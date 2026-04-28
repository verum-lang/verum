//! CBGR (Counter-Based Generational References) Core Types
//!
//! This module provides Rust-side CBGR types for the VBC interpreter and LLVM
//! lowering. These types MUST match the stdlib definitions in `core/mem/*.vr`.
//!
//! # Architecture (VBC-first)
//!
//! ```text
//! Source → VBC Bytecode → ┬─ Tier 0: Interpreter (uses THIS module)
//!                         ├─ Tier 1-2: VBC → LLVM JIT
//!                         └─ Tier 3: VBC → LLVM IR/MLIR → AOT
//! ```
//!
//! # Crate Responsibilities
//!
//! | Crate | Purpose |
//! |-------|---------|
//! | **verum_common/cbgr.rs** | Rust types for interpreter & LLVM lowering |
//! | **core/mem/*.vr** | Source of truth - runtime implementation |
//! | **verum_cbgr** | Compile-time analysis (escape, lifetime, tier) |
//! | **verum_vbc/cbgr.rs** | Codegen strategies (tier selection) |
//!
//! # Memory Layout (MUST match core/mem/header.vr)
//!
//! ```text
//! AllocationHeader: 32 bytes (cache-line optimized)
//! ┌────────────────────────────────────────────────────────────────────┐
//! │ Offset  Size  Field           Description                          │
//! │ ────────────────────────────────────────────────────────────────── │
//! │ 0       4     size            Allocation size in bytes             │
//! │ 4       4     alignment       Alignment requirement                │
//! │ 8       4     generation      Generation counter (atomic)          │
//! │ 12      2     epoch           Epoch counter (atomic)               │
//! │ 14      2     capabilities    Capability flags (atomic)            │
//! │ 16      4     type_id         Runtime type identifier              │
//! │ 20      4     flags           Allocation state flags               │
//! │ 24      8     reserved        Reserved for future use              │
//! └────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Capability Bits (16-bit, matches core/mem/capability.vr)
//!
//! | Bit | Name        | Description                              |
//! |-----|-------------|------------------------------------------|
//! | 0   | READ        | Can read data through reference          |
//! | 1   | WRITE       | Can write data through reference         |
//! | 2   | EXECUTE     | Can execute (function pointers)          |
//! | 3   | DELEGATE    | Can create sub-references                |
//! | 4   | REVOKE      | Can invalidate/revoke reference          |
//! | 5   | BORROWED    | Non-owning borrowed reference            |
//! | 6   | MUTABLE     | Can obtain mutable access                |
//! | 7   | NO_ESCAPE   | Cannot escape scope (enables SBGL opt)   |
//!
//! # Implementation References
//!
//! - core/mem/header.vr (source of truth for AllocationHeader layout)
//! - core/mem/epoch.vr (global epoch management)
//! - core/mem/capability.vr (capability bit definitions)

use core::sync::atomic::{AtomicU16, AtomicU32, AtomicU64, Ordering};

// =============================================================================
// Constants (MUST match core/mem/header.vr)
// =============================================================================

/// Generation value for unallocated memory.
/// Matches stdlib: `GEN_UNALLOCATED: UInt32 = 0`
pub const GEN_UNALLOCATED: u32 = 0;

/// Initial generation value for freshly allocated memory.
/// Matches stdlib: `GEN_INITIAL: UInt32 = 1`
pub const GEN_INITIAL: u32 = 1;

/// Maximum generation value before wrapping.
/// Matches stdlib: `GEN_MAX: UInt32 = 0xFFFF_FFFE`
/// Note: Uses 0xFFFF_FFFE (not 0x7FFF_FFFF) to leave room for wraparound detection.
pub const GEN_MAX: u32 = 0xFFFF_FFFE;

/// Generation value indicating permanent/immortal allocation.
/// These allocations never get deallocated (static data, leaked boxes).
pub const GEN_PERMANENT: u32 = u32::MAX;

// =============================================================================
// Capability Bits
// =============================================================================

/// Capability bits (lower 16 bits of epoch_caps in CbgrHeader).
/// 8 capability flags (bits 0-7): READ, WRITE, EXECUTE, DELEGATE, REVOKE,
/// BORROWED, MUTABLE, NO_ESCAPE. Upper 16 bits store the epoch counter.
pub mod caps {
    /// Can read data through reference (bit 0)
    pub const READ: u32 = 1 << 0;
    /// Can write data through reference (bit 1)
    pub const WRITE: u32 = 1 << 1;
    /// Can execute - for function pointers (bit 2)
    pub const EXECUTE: u32 = 1 << 2;
    /// Can create sub-references/capabilities (bit 3)
    pub const DELEGATE: u32 = 1 << 3;
    /// Can revoke/invalidate references (bit 4)
    pub const REVOKE: u32 = 1 << 4;
    /// Non-owning borrowed reference (bit 5)
    pub const BORROWED: u32 = 1 << 5;
    /// Can obtain mutable access (bit 6)
    pub const MUTABLE: u32 = 1 << 6;
    /// Cannot escape scope - enables SBGL optimization (bit 7)
    pub const NO_ESCAPE: u32 = 1 << 7;

    /// All standard capabilities (bits 0-7)
    pub const ALL: u32 = (1 << 8) - 1;
    /// Mask for capability bits (lower 16 bits)
    pub const MASK: u32 = 0xFFFF;
    /// Shift for epoch bits (upper 16 bits)
    pub const EPOCH_SHIFT: u32 = 16;

    // Common capability combinations
    /// Read-only reference
    pub const READ_ONLY: u32 = READ | BORROWED;
    /// Read-write reference
    pub const READ_WRITE: u32 = READ | WRITE;
    /// Exclusive mutable reference
    pub const EXCLUSIVE: u32 = READ | WRITE | MUTABLE;
    /// Full ownership capabilities
    pub const OWNER: u32 = READ | WRITE | MUTABLE | DELEGATE | REVOKE;
}

/// Capability preset for CBGR references
///
/// These are common capability combinations. For fine-grained control,
/// use the `caps::` constants directly.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Capability {
    /// No capabilities
    #[default]
    None = 0,
    /// Read-only borrowed reference
    ReadOnly = caps::READ_ONLY,
    /// Read-write reference (shared mutable)
    ReadWrite = caps::READ_WRITE,
    /// Exclusive mutable reference
    Exclusive = caps::EXCLUSIVE,
    /// Full ownership (read + write + mutable + delegate + revoke)
    Owner = caps::OWNER,
    /// All capabilities (bits 0-7)
    All = caps::ALL,
}

impl Capability {
    /// Full capabilities constant
    pub const ALL: Capability = Capability::All;

    /// Check if a capability bit is present in a capability bitfield
    #[inline]
    pub fn is_present(self, bitfield: u32) -> bool {
        let mask = self as u32;
        (bitfield & mask) == mask
    }

    /// Check if specific capability flag is set
    #[inline]
    pub fn has_flag(bitfield: u32, flag: u32) -> bool {
        (bitfield & flag) != 0
    }

    /// Get raw u32 value
    #[inline]
    pub fn as_u32(self) -> u32 {
        self as u32
    }

    /// Create from raw capability bits
    #[inline]
    pub fn from_bits(bits: u32) -> Self {
        match bits & caps::MASK {
            0 => Self::None,
            x if x == caps::READ_ONLY => Self::ReadOnly,
            x if x == caps::READ_WRITE => Self::ReadWrite,
            x if x == caps::EXCLUSIVE => Self::Exclusive,
            x if x == caps::OWNER => Self::Owner,
            x if x == caps::ALL => Self::All,
            _ => Self::None, // Custom combination, default to None preset
        }
    }
}

// =============================================================================
// CBGR Error Code
// =============================================================================

/// Error codes returned by CBGR operations
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CbgrErrorCode {
    /// Operation completed successfully
    Success = 0,
    /// CBGR generation mismatch (stale reference)
    GenerationMismatch = 1,
    /// CBGR epoch mismatch (cross-epoch reference)
    EpochMismatch = 2,
    /// CBGR reference has expired
    ExpiredReference = 3,
    /// Null pointer
    NullPointer = 4,
    /// Out of bounds access
    OutOfBounds = 5,
    /// Allocation failed
    AllocationFailed = 6,
    /// Invalid alignment
    InvalidAlignment = 7,
}

impl CbgrErrorCode {
    /// Convert to u32 for FFI
    #[inline(always)]
    pub const fn as_u32(self) -> u32 {
        self as u32
    }

    /// Check if this is a success code
    #[inline(always)]
    pub const fn is_success(self) -> bool {
        matches!(self, Self::Success)
    }
}

// =============================================================================
// CbgrHeader (8 bytes) - Compact header for VBC interpreter
// =============================================================================

/// Compact CBGR tracking header (8 bytes) for VBC interpreter.
///
/// **IMPORTANT**: This is an internal-only type for the VBC interpreter's
/// heap management. For the full runtime header that matches stdlib, use
/// [`AllocationHeader`] (32 bytes).
///
/// # Layout
///
/// ```text
/// ┌────────────────────────────────────────────────────────────────────┐
/// │ generation: AtomicU32 (4 bytes) - GEN_UNALLOCATED(0) = invalid     │
/// │ epoch_caps: AtomicU32 (4 bytes) - epoch:16 | capabilities:16       │
/// └────────────────────────────────────────────────────────────────────┘
/// ```
///
/// # Validity Model
///
/// An allocation is valid when `generation > 0` (GEN_INITIAL or higher).
/// Invalid/deallocated memory has `generation = GEN_UNALLOCATED (0)`.
///
/// # Note on Epoch
///
/// The epoch field is 16 bits (stored in upper 16 bits of epoch_caps).
/// This is the truncated value from the 64-bit global epoch. Comparison
/// uses only these 16 bits: `expected_epoch == (global_epoch & 0xFFFF)`.
#[repr(C)]
pub struct CbgrHeader {
    /// Packed atomic field: generation (upper 32 bits) | epoch_caps (lower 32 bits).
    ///
    /// Packing into a single AtomicU64 eliminates TOCTOU races between generation
    /// and epoch checks during validation, ensuring both are read atomically.
    ///
    /// Layout: [generation:u32][epoch:u16 | caps:u16]
    packed: AtomicU64,
}

impl CbgrHeader {
    /// Size of the header in bytes
    pub const SIZE: usize = 8;

    /// Pack generation and epoch_caps into a single u64
    #[inline(always)]
    const fn pack(generation: u32, epoch_caps: u32) -> u64 {
        ((generation as u64) << 32) | (epoch_caps as u64)
    }

    /// Unpack generation from packed u64
    #[inline(always)]
    const fn unpack_generation(packed: u64) -> u32 {
        (packed >> 32) as u32
    }

    /// Unpack epoch_caps from packed u64
    #[inline(always)]
    const fn unpack_epoch_caps(packed: u64) -> u32 {
        packed as u32
    }

    /// Create a new CBGR header with initial generation (valid)
    #[inline]
    pub const fn new(epoch: u32) -> Self {
        let epoch_caps = (epoch << caps::EPOCH_SHIFT) | caps::READ_WRITE;
        Self {
            packed: AtomicU64::new(Self::pack(GEN_INITIAL, epoch_caps)),
        }
    }

    /// Create with explicit initial generation and capabilities
    #[inline]
    pub fn with_generation(epoch: u32, generation: u32, capabilities: u32) -> Self {
        let epoch_caps = (epoch << caps::EPOCH_SHIFT) | (capabilities & caps::MASK);
        Self {
            packed: AtomicU64::new(Self::pack(generation, epoch_caps)),
        }
    }

    /// Get the current generation
    #[inline(always)]
    pub fn generation(&self) -> u32 {
        Self::unpack_generation(self.packed.load(Ordering::Acquire))
    }

    /// Get the epoch
    #[inline(always)]
    pub fn epoch(&self) -> u32 {
        let epoch_caps = Self::unpack_epoch_caps(self.packed.load(Ordering::Acquire));
        epoch_caps >> caps::EPOCH_SHIFT
    }

    /// Get the capabilities
    #[inline(always)]
    pub fn capabilities(&self) -> u32 {
        let epoch_caps = Self::unpack_epoch_caps(self.packed.load(Ordering::Acquire));
        epoch_caps & caps::MASK
    }

    /// Increment the generation (called after mutation).
    /// Returns the new generation value.
    ///
    /// Handles wraparound: when generation reaches GEN_MAX, advances the global
    /// epoch and resets to GEN_INITIAL. Uses compare_exchange to prevent the
    /// generation from transiently holding GEN_MAX+1 or wrapping to
    /// GEN_UNALLOCATED (0), which could allow use-after-free via stale
    /// weak reference upgrades matching a reused generation value.
    #[inline(always)]
    pub fn increment_generation(&self) -> u32 {
        loop {
            let old_packed = self.packed.load(Ordering::Acquire);
            let old_gen = Self::unpack_generation(old_packed);

            if old_gen >= GEN_MAX {
                // Generation exhausted -- advance global epoch and reset atomically.
                // CAS ensures only one thread performs the reset.
                let old_epoch_caps = Self::unpack_epoch_caps(old_packed);
                let capabilities = old_epoch_caps & caps::MASK;
                let new_epoch = (current_epoch() & 0xFFFF) as u32;
                let new_epoch_caps = (new_epoch << caps::EPOCH_SHIFT) | capabilities;
                let new_packed = Self::pack(GEN_INITIAL, new_epoch_caps);
                if self.packed.compare_exchange(
                    old_packed, new_packed, Ordering::AcqRel, Ordering::Relaxed
                ).is_ok() {
                    advance_epoch();
                }
                return GEN_INITIAL;
            }

            let new_gen = old_gen + 1; // Safe: old_gen < GEN_MAX so no overflow
            let new_packed = old_packed.wrapping_add(1u64 << 32);
            if self.packed.compare_exchange(
                old_packed, new_packed, Ordering::AcqRel, Ordering::Relaxed
            ).is_ok() {
                return new_gen;
            }
            // CAS failed due to concurrent modification, retry
        }
    }

    /// Validate a reference against expected generation and epoch.
    ///
    /// This is the hot path - must be < 15ns.
    /// Uses a single atomic load to read both generation and epoch together,
    /// eliminating TOCTOU races.
    #[inline(always)]
    pub fn validate(&self, expected_gen: u32, expected_epoch: u32) -> CbgrErrorCode {
        // Single atomic load for both generation and epoch - no TOCTOU possible
        let packed = self.packed.load(Ordering::Acquire);
        let actual_gen = Self::unpack_generation(packed);

        // Check validity first - generation 0 means deallocated
        if actual_gen == GEN_UNALLOCATED {
            return CbgrErrorCode::ExpiredReference;
        }

        if actual_gen != expected_gen {
            return CbgrErrorCode::GenerationMismatch;
        }

        let epoch_caps = Self::unpack_epoch_caps(packed);
        let actual_epoch = epoch_caps >> caps::EPOCH_SHIFT;

        if actual_epoch != expected_epoch {
            return CbgrErrorCode::EpochMismatch;
        }

        CbgrErrorCode::Success
    }

    /// Invalidate this allocation (called on dealloc).
    ///
    /// Sets generation to GEN_UNALLOCATED (0) to mark as invalid.
    /// Preserves epoch and capabilities in the lower 32 bits.
    #[inline]
    pub fn invalidate(&self) {
        let old = self.packed.load(Ordering::Acquire);
        let epoch_caps = Self::unpack_epoch_caps(old);
        self.packed.store(Self::pack(GEN_UNALLOCATED, epoch_caps), Ordering::Release);
    }

    /// Check if still valid (generation > 0)
    #[inline(always)]
    pub fn is_valid(&self) -> bool {
        Self::unpack_generation(self.packed.load(Ordering::Acquire)) != GEN_UNALLOCATED
    }
}

// =============================================================================
// AllocationHeader (32 bytes) - MUST match core/mem/header.vr
// =============================================================================

/// Full allocation header with metadata (32 bytes).
///
/// **IMPORTANT**: This layout MUST match `core/mem/header.vr` exactly!
///
/// # Layout (matches core/mem/header.vr)
///
/// ```text
/// Offset  Size  Field           Description
/// ──────────────────────────────────────────────────────────────────────
/// 0       4     size            Allocation size in bytes (excluding header)
/// 4       4     alignment       Alignment requirement
/// 8       4     generation      Current generation counter (atomic)
/// 12      2     epoch           Epoch counter for wraparound safety (atomic)
/// 14      2     capabilities    Capability flags (atomic)
/// 16      4     type_id         Runtime type identifier
/// 20      4     flags           Allocation state flags
/// 24      8     reserved        Reserved for future use
/// ──────────────────────────────────────────────────────────────────────
/// Total: 32 bytes (half cache line, 32-byte aligned)
/// ```
#[repr(C, align(32))]
pub struct AllocationHeader {
    /// Allocation size in bytes (excluding header).
    /// Matches stdlib: `size: UInt32`
    size: u32,
    /// Alignment requirement.
    /// Matches stdlib: `alignment: UInt32`
    alignment: u32,
    /// Current generation counter (atomic).
    /// Matches stdlib: `generation: UInt32` (with atomic semantics)
    generation: AtomicU32,
    /// Epoch counter for wraparound protection (atomic).
    /// Matches stdlib: `epoch: UInt16` (with atomic semantics)
    epoch: AtomicU16,
    /// Capability flags (atomic).
    /// Matches stdlib: `capabilities: UInt16` (with atomic semantics)
    capabilities: AtomicU16,
    /// Runtime type identifier.
    /// Matches stdlib: `type_id: UInt32`
    type_id: u32,
    /// Allocation state flags.
    /// Matches stdlib: `flags: UInt32`
    flags: AtomicU32,
    /// Reserved for future use.
    /// Matches stdlib: `reserved: [UInt32; 2]`
    _reserved: [u32; 2],
}

impl AllocationHeader {
    /// Size of the header in bytes.
    /// Matches stdlib: `HEADER_SIZE: Int = 32`
    pub const SIZE: usize = 32;

    /// Header alignment (cache-line half).
    /// Matches stdlib: `HEADER_ALIGN: Int = 32`
    pub const ALIGN: usize = 32;

    /// Create a new allocation header.
    ///
    /// Matches stdlib: `AllocationHeader.new(size, alignment, type_id, capabilities)`
    #[inline]
    pub fn new(size: u32, alignment: u32, type_id: u32, capabilities: u16) -> Self {
        Self {
            size,
            alignment,
            generation: AtomicU32::new(GEN_INITIAL),
            epoch: AtomicU16::new(current_epoch_u16()),
            capabilities: AtomicU16::new(capabilities),
            type_id,
            flags: AtomicU32::new(0),
            _reserved: [0, 0],
        }
    }

    /// Create header with preset capability.
    #[inline]
    pub fn with_capability(size: u32, alignment: u32, type_id: u32, cap: Capability) -> Self {
        Self::new(size, alignment, type_id, cap.as_u32() as u16)
    }

    /// Get the user data size.
    /// Matches stdlib: `get_size(&self) -> UInt32`
    #[inline(always)]
    pub fn size(&self) -> u32 {
        self.size
    }

    /// Get the alignment.
    /// Matches stdlib: `get_alignment(&self) -> UInt32`
    #[inline(always)]
    pub fn alignment(&self) -> u32 {
        self.alignment
    }

    /// Get the type ID.
    /// Matches stdlib: `get_type_id(&self) -> UInt32`
    #[inline(always)]
    pub fn type_id(&self) -> u32 {
        self.type_id
    }

    /// Load the capabilities with specified ordering.
    /// Matches stdlib: `load_capabilities(&self) -> UInt16`
    #[inline(always)]
    pub fn load_capabilities(&self, ordering: Ordering) -> u16 {
        self.capabilities.load(ordering)
    }

    /// Store capabilities with specified ordering.
    /// Matches stdlib: `store_capabilities(&mut self, caps: UInt16)`
    #[inline(always)]
    pub fn store_capabilities(&self, caps: u16, ordering: Ordering) {
        self.capabilities.store(caps, ordering);
    }

    /// Load the generation with specified ordering.
    /// Matches stdlib: `load_generation(&self) -> UInt32`
    #[inline(always)]
    pub fn load_generation(&self, ordering: Ordering) -> u32 {
        self.generation.load(ordering)
    }

    /// Store generation counter.
    /// Matches stdlib: `store_generation(&mut self, gen: UInt32)`
    #[inline(always)]
    pub fn store_generation(&self, generation: u32, ordering: Ordering) {
        self.generation.store(generation, ordering);
    }

    /// Load the epoch with specified ordering.
    /// Matches stdlib: `load_epoch(&self) -> UInt16`
    #[inline(always)]
    pub fn load_epoch(&self, ordering: Ordering) -> u16 {
        self.epoch.load(ordering)
    }

    /// Store epoch counter.
    /// Matches stdlib: `store_epoch(&mut self, epoch: UInt16)`
    #[inline(always)]
    pub fn store_epoch(&self, epoch: u16, ordering: Ordering) {
        self.epoch.store(epoch, ordering);
    }

    /// Load generation and epoch together (optimized path).
    /// Matches stdlib: `load_generation_epoch_fast(&self) -> (UInt32, UInt16)`
    ///
    /// This is the HOT PATH for validation - ~15ns.
    #[inline(always)]
    pub fn load_generation_epoch_fast(&self, ordering: Ordering) -> (u32, u16) {
        // Load generation with Acquire ordering (provides memory fence)
        let generation = self.generation.load(ordering);
        // Epoch can use Relaxed since generation load provides the fence
        let epoch = self.epoch.load(Ordering::Relaxed);
        (generation, epoch)
    }

    /// Increment generation and return NEW value.
    /// Matches stdlib: `increment_generation(&mut self) -> UInt32`
    ///
    /// Handles wraparound by advancing global epoch and resetting to GEN_INITIAL
    /// when generation reaches GEN_MAX. Uses compare_exchange to prevent the
    /// generation from transiently reaching GEN_MAX+1 (0xFFFFFFFF) or wrapping
    /// to GEN_UNALLOCATED (0), which could allow use-after-free via stale
    /// weak reference upgrades matching a reused generation value.
    #[inline(always)]
    pub fn increment_generation(&self, ordering: Ordering) -> u32 {
        loop {
            let old_gen = self.generation.load(ordering);
            if old_gen >= GEN_MAX {
                // Generation exhausted -- advance global epoch and reset.
                // CAS ensures only one thread performs the reset.
                if self.generation.compare_exchange(
                    old_gen, GEN_INITIAL, Ordering::AcqRel, Ordering::Relaxed
                ).is_ok() {
                    advance_epoch();
                    self.epoch.store(current_epoch_u16(), Ordering::Release);
                }
                return GEN_INITIAL;
            }
            let new_gen = old_gen + 1; // Safe: old_gen < GEN_MAX so no overflow
            if self.generation.compare_exchange(
                old_gen, new_gen, Ordering::AcqRel, Ordering::Relaxed
            ).is_ok() {
                return new_gen;
            }
            // CAS failed due to concurrent modification, retry
        }
    }

    /// Validate reference against expected generation and epoch.
    ///
    /// This is the core CBGR validation - must be < 15ns.
    ///
    /// Uses a double-check pattern to mitigate TOCTOU: after checking epoch,
    /// re-reads generation to detect concurrent modifications. If generation
    /// changed between the two reads, returns GenerationMismatch.
    #[inline(always)]
    pub fn validate(&self, expected_gen: u32, expected_epoch: u16) -> CbgrErrorCode {
        let actual_gen = self.generation.load(Ordering::Acquire);

        // Check validity first - generation 0 means deallocated
        if actual_gen == GEN_UNALLOCATED {
            return CbgrErrorCode::ExpiredReference;
        }

        if actual_gen != expected_gen {
            return CbgrErrorCode::GenerationMismatch;
        }

        let actual_epoch = self.epoch.load(Ordering::Acquire);
        if actual_epoch != expected_epoch {
            return CbgrErrorCode::EpochMismatch;
        }

        // Double-check: re-read generation to detect concurrent mutation
        // between the generation and epoch reads above (TOCTOU mitigation)
        let recheck_gen = self.generation.load(Ordering::Acquire);
        if recheck_gen != actual_gen {
            return CbgrErrorCode::GenerationMismatch;
        }

        CbgrErrorCode::Success
    }

    /// Invalidate this allocation (called on dealloc).
    /// Sets generation to GEN_UNALLOCATED (0).
    #[inline]
    pub fn invalidate(&self) {
        self.generation.store(GEN_UNALLOCATED, Ordering::Release);
    }

    /// Check if still valid (generation > 0).
    #[inline(always)]
    pub fn is_valid(&self) -> bool {
        self.generation.load(Ordering::Acquire) != GEN_UNALLOCATED
    }

    /// Try to get the allocation header from a user data pointer.
    ///
    /// # Safety
    ///
    /// The pointer must have been allocated with this header layout.
    #[inline]
    pub unsafe fn try_from_user_ptr(user_ptr: *const u8) -> Option<*const Self> {
        if user_ptr.is_null() {
            return None;
        }
        // SAFETY: Caller guarantees user_ptr was allocated with this header layout
        let header_ptr = unsafe { user_ptr.sub(Self::SIZE) as *const Self };
        Some(header_ptr)
    }

    /// Get a mutable reference from user pointer.
    ///
    /// # Safety
    ///
    /// The pointer must have been allocated with this header layout.
    #[inline]
    pub unsafe fn try_from_user_ptr_mut(user_ptr: *mut u8) -> Option<*mut Self> {
        if user_ptr.is_null() {
            return None;
        }
        // SAFETY: Caller guarantees user_ptr was allocated with this header layout
        let header_ptr = unsafe { user_ptr.sub(Self::SIZE) as *mut Self };
        Some(header_ptr)
    }

    /// Get pointer to user data (after header).
    /// Matches stdlib: `user_ptr(&self) -> &unsafe Byte`
    ///
    /// # Safety
    ///
    /// The caller must ensure that the header was allocated with sufficient
    /// space for user data following it. The returned pointer is only valid
    /// for the lifetime of the allocation.
    #[inline]
    pub unsafe fn user_ptr(&self) -> *mut u8 {
        // SAFETY: Caller guarantees header was allocated with space for user data
        unsafe { (self as *const Self as *mut u8).add(Self::SIZE) }
    }

    /// Get total allocation size (header + data).
    /// Matches stdlib: `total_size(&self) -> Int`
    #[inline]
    pub fn total_size(&self) -> usize {
        Self::SIZE + self.size as usize
    }
}

// =============================================================================
// TrackedAllocation - Wrapper for CBGR-tracked allocations
// =============================================================================

/// A tracked allocation with CBGR header
///
/// This wraps an allocation that has a CbgrHeader prepended to it.
pub struct TrackedAllocation {
    /// Pointer to the user data (after CbgrHeader)
    ptr: *mut u8,
}

impl TrackedAllocation {
    /// Create a TrackedAllocation from a user data pointer
    ///
    /// # Safety
    ///
    /// The pointer must have been allocated with a CbgrHeader prepended.
    #[inline]
    pub unsafe fn from_user_ptr(ptr: *mut u8) -> Self {
        Self { ptr }
    }

    /// Get the user data pointer
    #[inline(always)]
    pub fn as_ptr(&self) -> *mut u8 {
        self.ptr
    }

    /// Get the CBGR header
    #[inline]
    pub fn header(&self) -> &CbgrHeader {
        unsafe {
            let header_ptr = self.ptr.sub(CbgrHeader::SIZE) as *const CbgrHeader;
            &*header_ptr
        }
    }

    /// Get a mutable reference to the CBGR header
    #[inline]
    pub fn header_mut(&mut self) -> &mut CbgrHeader {
        unsafe {
            let header_ptr = self.ptr.sub(CbgrHeader::SIZE) as *mut CbgrHeader;
            &mut *header_ptr
        }
    }
}

// =============================================================================
// Memory Allocation Functions
// =============================================================================

/// Allocate zeroed memory with CBGR tracking header
///
/// Returns a TrackedAllocation if successful, None if allocation fails.
///
/// # Arguments
///
/// * `size` - Size of user data in bytes
/// * `align` - Alignment requirement (must be power of 2)
///
/// # Safety
///
/// This function is safe to call but the returned allocation must be
/// properly freed using `tracked_dealloc`.
pub fn tracked_alloc_zeroed(size: usize, align: usize) -> Result<TrackedAllocation, CbgrErrorCode> {
    use std::alloc::{Layout, alloc_zeroed};

    // Validate alignment
    if !align.is_power_of_two() || align == 0 {
        return Err(CbgrErrorCode::InvalidAlignment);
    }

    // Total size includes CbgrHeader
    let total_size = CbgrHeader::SIZE + size;
    let actual_align = align.max(std::mem::align_of::<CbgrHeader>());

    let layout = Layout::from_size_align(total_size, actual_align)
        .map_err(|_| CbgrErrorCode::AllocationFailed)?;

    let base_ptr = unsafe { alloc_zeroed(layout) };
    if base_ptr.is_null() {
        return Err(CbgrErrorCode::AllocationFailed);
    }

    // Initialize the CBGR header
    unsafe {
        let header_ptr = base_ptr as *mut CbgrHeader;
        std::ptr::write(header_ptr, CbgrHeader::new(0));

        // User data pointer is after the header
        let user_ptr = base_ptr.add(CbgrHeader::SIZE);
        Ok(TrackedAllocation::from_user_ptr(user_ptr))
    }
}

/// Deallocate a tracked allocation
///
/// # Safety
///
/// The allocation must have been created by `tracked_alloc_zeroed` and
/// must not have been deallocated already.
pub unsafe fn tracked_dealloc(allocation: TrackedAllocation) {
    #[allow(unused_imports)]
    use std::alloc::{Layout, dealloc};

    // This is a simplified version - in a real implementation we would
    // need to store the original size/alignment in the header or elsewhere
    // For now, we just mark it invalid but don't actually dealloc
    // (this is a stub implementation for compilation purposes)
    allocation.header().invalidate();

    // Note: Proper deallocation requires knowing the original layout
    // This would typically be stored in AllocationHeader
    let _ = allocation;
}

// =============================================================================
// Global Epoch (MUST match core/mem/epoch.vr)
// =============================================================================

/// Global epoch counter.
/// Matches stdlib: `GLOBAL_EPOCH.epoch: UInt64`
///
/// The epoch increments whenever any allocation's generation wraps around.
/// This prevents ABA problems where a deallocated slot is reused.
static GLOBAL_EPOCH: AtomicU64 = AtomicU64::new(0);

/// Get the current global epoch.
/// Matches stdlib: `current_epoch() -> UInt64`
#[inline(always)]
pub fn current_epoch() -> u64 {
    GLOBAL_EPOCH.load(Ordering::Acquire)
}

/// Advance to next epoch (called on generation wraparound).
/// Matches stdlib: `EpochManager.increment_epoch() -> UInt64`
///
/// Returns the new epoch value.
#[inline]
pub fn advance_epoch() -> u64 {
    GLOBAL_EPOCH.fetch_add(1, Ordering::AcqRel) + 1
}

/// Get current epoch truncated to 16 bits for header storage.
/// The AllocationHeader stores epoch as u16 to save space.
#[inline(always)]
pub fn current_epoch_u16() -> u16 {
    (current_epoch() & 0xFFFF) as u16
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cbgr_header_size() {
        assert_eq!(std::mem::size_of::<CbgrHeader>(), CbgrHeader::SIZE);
    }

    #[test]
    fn test_allocation_header_size() {
        assert_eq!(std::mem::size_of::<AllocationHeader>(), AllocationHeader::SIZE);
    }

    #[test]
    fn test_cbgr_header_new() {
        let header = CbgrHeader::new(42);
        // New headers start with GEN_INITIAL (1) and are valid
        assert_eq!(header.generation(), GEN_INITIAL);
        assert_eq!(header.epoch(), 42);
        assert!(header.is_valid());
    }

    #[test]
    fn test_generation_increment() {
        let header = CbgrHeader::new(0);
        assert_eq!(header.generation(), GEN_INITIAL);

        let new_gen = header.increment_generation();
        assert_eq!(new_gen, GEN_INITIAL + 1);
        assert_eq!(header.generation(), GEN_INITIAL + 1);
    }

    #[test]
    fn test_validation_success() {
        let header = CbgrHeader::new(5);
        // Validate with initial generation (1) and epoch (5)
        assert_eq!(header.validate(GEN_INITIAL, 5), CbgrErrorCode::Success);
    }

    #[test]
    fn test_validation_generation_mismatch() {
        let header = CbgrHeader::new(5);
        header.increment_generation();
        // Wrong generation (1 vs actual 2)
        assert_eq!(header.validate(GEN_INITIAL, 5), CbgrErrorCode::GenerationMismatch);
    }

    #[test]
    fn test_validation_epoch_mismatch() {
        let header = CbgrHeader::new(5);
        // Correct generation, wrong epoch
        assert_eq!(header.validate(GEN_INITIAL, 6), CbgrErrorCode::EpochMismatch);
    }

    #[test]
    fn test_invalidation() {
        let header = CbgrHeader::new(0);
        assert!(header.is_valid());

        header.invalidate();
        // After invalidation, generation is GEN_UNALLOCATED (0)
        assert!(!header.is_valid());
        assert_eq!(header.generation(), GEN_UNALLOCATED);
        // Any validation should fail with ExpiredReference
        assert_eq!(header.validate(GEN_INITIAL, 0), CbgrErrorCode::ExpiredReference);
    }

    #[test]
    fn test_tracked_alloc() {
        let result = tracked_alloc_zeroed(64, 8);
        assert!(result.is_ok());

        let alloc = result.unwrap();
        assert!(!alloc.as_ptr().is_null());
        assert_eq!(alloc.header().generation(), GEN_INITIAL);
        assert!(alloc.header().is_valid());
    }

    #[test]
    fn test_capability_presets() {
        // Test preset capability combinations
        assert!(Capability::has_flag(Capability::ReadOnly.as_u32(), caps::READ));
        assert!(Capability::has_flag(Capability::ReadOnly.as_u32(), caps::BORROWED));
        assert!(!Capability::has_flag(Capability::ReadOnly.as_u32(), caps::WRITE));

        assert!(Capability::has_flag(Capability::ReadWrite.as_u32(), caps::READ));
        assert!(Capability::has_flag(Capability::ReadWrite.as_u32(), caps::WRITE));

        assert!(Capability::has_flag(Capability::Exclusive.as_u32(), caps::MUTABLE));
        assert!(Capability::has_flag(Capability::Owner.as_u32(), caps::DELEGATE));
        assert!(Capability::has_flag(Capability::Owner.as_u32(), caps::REVOKE));

        // Test All contains all 8 capability bits
        assert_eq!(Capability::All.as_u32(), caps::ALL);
        assert_eq!(caps::ALL, 0xFF); // bits 0-7
    }

    #[test]
    fn test_epoch_caps_packing() {
        // Verify 16-bit epoch + 16-bit caps packing
        let header = CbgrHeader::with_generation(0x1234, 42, caps::READ | caps::WRITE | caps::EXECUTE);
        assert_eq!(header.epoch(), 0x1234);
        let caps = header.capabilities();
        assert!(caps & caps::READ != 0);
        assert!(caps & caps::WRITE != 0);
        assert!(caps & caps::EXECUTE != 0);
        assert!(caps & caps::DELEGATE == 0);
    }

    #[test]
    fn test_capability_from_bits() {
        assert_eq!(Capability::from_bits(0), Capability::None);
        assert_eq!(Capability::from_bits(caps::READ_ONLY), Capability::ReadOnly);
        assert_eq!(Capability::from_bits(caps::READ_WRITE), Capability::ReadWrite);
        assert_eq!(Capability::from_bits(caps::EXCLUSIVE), Capability::Exclusive);
        assert_eq!(Capability::from_bits(caps::OWNER), Capability::Owner);
        assert_eq!(Capability::from_bits(caps::ALL), Capability::All);
    }

    // =========================================================================
    // Round 2 §7.1 — Generation counter race (DEFENSE CONFIRMED guardrail)
    // =========================================================================
    //
    // Red-team scenario: N threads concurrently call `increment_generation`
    // on the same `CbgrHeader`.  The CAS-loop must produce exactly N
    // distinct increments — no lost updates, no duplicate values, no
    // value past `GEN_MAX` (the wraparound branch resets cleanly to
    // `GEN_INITIAL`).  A regression to a non-atomic add or to AcqRel
    // ordering being relaxed would surface as either a generation count
    // < N×threads (lost updates) or a generation past `GEN_MAX` (CAS
    // race on the wraparound branch).
    //
    // The test uses 8 threads × 5,000 increments each = 40,000 total.
    // We start at `GEN_INITIAL` (1), so the final generation is the
    // count of CAS-successes mod (GEN_MAX - GEN_INITIAL + 1) — for 40k
    // iterations and `GEN_MAX = u32::MAX - 1` the wraparound never
    // triggers, so the final generation equals exactly
    // `GEN_INITIAL + 40_000`.

    #[test]
    fn test_generation_counter_concurrent_stress() {
        use std::sync::Arc;
        use std::thread;

        const THREADS: u32 = 8;
        const ITERS_PER_THREAD: u32 = 5_000;
        const TOTAL: u32 = THREADS * ITERS_PER_THREAD;

        let header = Arc::new(CbgrHeader::new(0));
        let mut handles = Vec::with_capacity(THREADS as usize);

        for _ in 0..THREADS {
            let h = Arc::clone(&header);
            handles.push(thread::spawn(move || {
                for _ in 0..ITERS_PER_THREAD {
                    let _ = h.increment_generation();
                }
            }));
        }

        for h in handles {
            h.join().expect("worker thread panicked");
        }

        // Exactly TOTAL successful CAS-add operations must have landed.
        // No lost updates → final generation = GEN_INITIAL + TOTAL.
        let final_gen = header.generation();
        assert_eq!(
            final_gen,
            GEN_INITIAL + TOTAL,
            "lost updates under contention: expected {}, got {}",
            GEN_INITIAL + TOTAL,
            final_gen
        );

        // Sanity: the value must not have wrapped past GEN_MAX (we sized
        // the workload to stay well below).  If this fires, the
        // wraparound branch was hit unexpectedly — re-run with a
        // smaller workload and investigate.
        assert!(
            final_gen < GEN_MAX,
            "unexpected wraparound: final_gen {} >= GEN_MAX {}",
            final_gen,
            GEN_MAX
        );

        // Capabilities + epoch must be untouched by generation increments.
        // (The wraparound branch DOES touch them, but we asserted above
        // that we didn't hit it.)
        assert_eq!(header.epoch(), 0, "generation race must not touch epoch");
    }
}
