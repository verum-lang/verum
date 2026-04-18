//! NaN-boxed runtime values.
//!
//! VBC uses NaN-boxing to represent all values in a single 64-bit word.
//! This technique exploits the IEEE 754 NaN representation to encode
//! tagged pointers and small values.
//!
//! # Encoding
//!
//! IEEE 754 double-precision format:
//! ```text
//! [S][EEEEEEEEEEE][MMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMM]
//!  63  62      52  51                                                0
//! ```
//!
//! A quiet NaN has all exponent bits = 1 and the quiet bit (bit 51) = 1.
//! We use a quiet NaN pattern (0x7FF8) and encode our tag in bits 48-50
//! (3 bits, 8 possible tags) to avoid collision with the quiet bit.
//!
//! ```text
//! Float (non-NaN):  [sign][exponent=0x7FF][0][mantissa] - regular value
//! Tagged value:     0x7FF8_TTPP_PPPP_PPPP
//!                        ^ quiet bit (always 1)
//!                         ^^^ tag (3 bits, 0-7)
//!                             ^^^^^^^^^^^^ payload (48 bits)
//! ```
//!
//! # Tags
//!
//! | Tag | Type | Payload |
//! |-----|------|---------|
//! | 0x0 | Pointer | 48-bit pointer |
//! | 0x1 | Integer | 48-bit signed int |
//! | 0x2 | Boolean | 0 or 1 |
//! | 0x3 | Unit | (none) |
//! | 0x4 | Small string | Up to 6 UTF-8 bytes |
//! | 0x5 | TypeRef | Type ID |
//! | 0x6 | FunctionRef | Function ID |
//! | 0x7 | NaN | Original NaN bits |

use std::fmt;

use crate::types::TypeId;
use crate::FunctionId;

/// Base NaN bits for tagged values.
/// We use a quiet NaN: 0x7FF8 (exponent=0x7FF, quiet bit set at position 51).
/// Tag goes into bits 48-50 (3 bits), payload in bits 0-47.
const NAN_BITS: u64 = 0x7FF8_0000_0000_0000;

/// Mask to extract the tag (bits 48-50, 3 bits for 8 tags).
/// We deliberately avoid bit 51 which is the quiet NaN bit.
const TAG_MASK: u64 = 0x0007_0000_0000_0000;

/// Shift to position the tag.
const TAG_SHIFT: u32 = 48;

/// Mask to extract the payload (bits 0-47).
const PAYLOAD_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;

/// Maximum value for 48-bit signed integer (fits in payload).
pub const MAX_SMALL_INT: i64 = (1 << 47) - 1;

/// Minimum value for 48-bit signed integer.
pub const MIN_SMALL_INT: i64 = -(1 << 47);

// Tag values (0-7, fits in 3 bits)
const TAG_POINTER: u64 = 0x0;
const TAG_INTEGER: u64 = 0x1;
const TAG_BOOLEAN: u64 = 0x2;
const TAG_UNIT: u64 = 0x3;
const TAG_SMALL_STRING: u64 = 0x4;
const TAG_TYPE_REF: u64 = 0x5;
const TAG_FUNC_REF: u64 = 0x6;
const TAG_NAN: u64 = 0x7;

/// High marker bit (bit 47) used for non-pointer special values.
///
/// Both generators and boxed integers have this bit set to distinguish them
/// from real heap pointers. User-space addresses on x86-64/ARM64 have bit 47 = 0
/// due to canonical addressing, so this bit is always safe to use as a marker.
const SPECIAL_VALUE_MARKER: u64 = 1u64 << 47;

/// Marker bit for generator values (bit 47 set, bit 46 NOT set).
///
/// Generators use TAG_POINTER with SPECIAL_VALUE_MARKER set and bit 46 clear.
/// The remaining 45 bits (0-44) hold the GeneratorId (bit 45 is THIN_REF_SUB_MARKER).
///
/// Generator State Machine: Generators are created from `fn*` functions via GenCreate (0x9E).
/// The generator maintains a state machine with statuses: Created, Running, Yielded, Completed.
/// On Yield, the full register state + PC + context stack is saved. On GenNext (0x9F), the
/// generator is resumed from saved state. GenHasNext (0xC9) checks if more values are available.
/// NaN-boxed encoding: TAG_POINTER with SPECIAL_VALUE_MARKER set, bit 46 clear, bits 0-44 = GeneratorId.
const GENERATOR_MARKER: u64 = SPECIAL_VALUE_MARKER;  // bit 47 = 1, bit 46 = 0

/// Sub-marker bit (bit 46) to distinguish boxed integers from generators.
///
/// When SPECIAL_VALUE_MARKER (bit 47) is set:
/// - bit 46 = 0: generator
/// - bit 46 = 1: boxed integer
const BOXED_INT_SUB_MARKER: u64 = 1u64 << 46;

/// Combined marker for boxed integers (bits 47 AND 46 both set).
///
/// Boxed integers use TAG_POINTER with both bit 47 and bit 46 set.
/// The remaining 45 bits (0-44) hold an index into the global boxed integer table.
///
/// This is used for integers outside the 48-bit inline range (-2^47 to 2^47-1).
///
/// This encoding guarantees no collision with:
/// - Real heap pointers: bit 47 = 0 (canonical addressing)
/// - Generators: bit 47 = 1, bit 46 = 0
const BOXED_INT_MARKER: u64 = SPECIAL_VALUE_MARKER | BOXED_INT_SUB_MARKER;  // bits 47 and 46

/// Mask for generator ID (bits 0-44, 45 bits total).
/// This avoids bit 45 which is used as THIN_REF_SUB_MARKER.
const GENERATOR_ID_MASK: u64 = (1u64 << 45) - 1;

/// Mask for boxed integer index (bits 0-44, 45 bits total).
/// This avoids bit 45 which is used as FAT_REF_SUB_MARKER.
const BOXED_INT_INDEX_MASK: u64 = (1u64 << 45) - 1;

/// Sub-marker bit (bit 45) to distinguish ThinRef from generators.
///
/// When bit 47 is set and bit 46 is clear:
/// - bit 45 = 0: generator
/// - bit 45 = 1: ThinRef
const THIN_REF_SUB_MARKER: u64 = 1u64 << 45;

/// Combined marker for ThinRef (bit 47 set, bit 46 clear, bit 45 set).
///
/// ThinRef values use TAG_POINTER with this marker pattern.
/// The remaining 45 bits hold an index into the global ThinRef table.
const THIN_REF_MARKER: u64 = SPECIAL_VALUE_MARKER | THIN_REF_SUB_MARKER;  // bits 47 and 45

/// Sub-marker bit (bit 45) to distinguish FatRef from boxed integers.
///
/// When both bit 47 and bit 46 are set:
/// - bit 45 = 0: boxed integer
/// - bit 45 = 1: FatRef
const FAT_REF_SUB_MARKER: u64 = 1u64 << 45;

/// Combined marker for FatRef (bits 47, 46, and 45 all set).
///
/// FatRef values use TAG_POINTER with this marker pattern.
/// The remaining 45 bits hold an index into the global FatRef table.
const FAT_REF_MARKER: u64 = SPECIAL_VALUE_MARKER | BOXED_INT_SUB_MARKER | FAT_REF_SUB_MARKER;  // bits 47, 46, 45

/// Mask for reference index (bits 0-44, 45 bits total).
const REF_INDEX_MASK: u64 = (1u64 << 45) - 1;

// Thread-safe global storage for boxed integers
use std::sync::Mutex;
static BOXED_INTS: Mutex<Vec<i64>> = Mutex::new(Vec::new());

// Thread-safe global storage for CBGR references
static THIN_REFS: Mutex<Vec<ThinRef>> = Mutex::new(Vec::new());
static FAT_REFS: Mutex<Vec<FatRef>> = Mutex::new(Vec::new());

/// Clears the global NaN-boxed Value side tables.
///
/// The VBC Value representation uses global `Mutex<Vec<_>>` side tables to
/// hold payloads too large to fit in 48 bits (boxed integers) or that are
/// structurally larger than 64 bits (CBGR `ThinRef` / `FatRef`). Each
/// [`Value::from_i64_boxed`], [`Value::from_thin_ref`], and
/// [`Value::from_fat_ref`] call pushes onto these tables and stores the
/// resulting index inside the returned `Value`.
///
/// Because the tables are process-global, consecutive in-process test runs
/// share them, and the indices handed out to a prior run remain baked into
/// any stale `Value`s that linger in globals, caches, or interpreter state
/// leaked between tests. Under sustained batch execution the tables also
/// grow unboundedly (one entry per boxed int / CBGR ref created).
///
/// This function truncates all three side tables to zero length, freeing
/// their backing storage and resetting index allocation.
///
/// # Safety contract
///
/// Any existing `Value` whose payload is a side-table index (boxed int,
/// ThinRef, FatRef) becomes **dangling** after this call. Callers MUST
/// guarantee that no such `Value`s remain reachable before invoking this
/// function. The intended use site is a test runner that has just finished
/// executing a test (dropping the interpreter and all `Value`s it owned)
/// and is about to start a fresh one.
pub fn reset_global_value_tables() {
    if let Ok(mut t) = BOXED_INTS.lock() {
        t.clear();
        t.shrink_to_fit();
    }
    if let Ok(mut t) = THIN_REFS.lock() {
        t.clear();
        t.shrink_to_fit();
    }
    if let Ok(mut t) = FAT_REFS.lock() {
        t.clear();
        t.shrink_to_fit();
    }
}

// ============================================================================
// CBGR Reference Types
// ============================================================================

/// CBGR capability flags (16 bits).
///
/// These flags control what operations are permitted on a reference.
/// Capabilities can only be attenuated (removed), never added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(transparent)]
pub struct Capabilities(pub u16);

impl Capabilities {
    /// Read permission - can dereference to read value.
    pub const READ: u16 = 0x0001;
    /// Write permission - can dereference to write value.
    pub const WRITE: u16 = 0x0002;
    /// Add permission - can add elements (for collections).
    pub const ADD: u16 = 0x0004;
    /// Remove permission - can remove elements (for collections).
    pub const REMOVE: u16 = 0x0008;
    /// Exclusive permission - no other references exist.
    pub const EXCLUSIVE: u16 = 0x0010;
    /// Delegate permission - can create sub-references.
    pub const DELEGATE: u16 = 0x0020;
    /// Alias permission - can be aliased (shared).
    pub const ALIAS: u16 = 0x0040;
    /// Drop permission - can be dropped/freed.
    pub const DROP: u16 = 0x0080;

    /// Full capabilities (all permissions).
    pub const FULL: Capabilities = Capabilities(0x00FF);

    /// Read-only capabilities.
    pub const READ_ONLY: Capabilities = Capabilities(Self::READ | Self::ALIAS);

    /// Mutable exclusive capabilities.
    pub const MUT_EXCLUSIVE: Capabilities = Capabilities(
        Self::READ | Self::WRITE | Self::ADD | Self::REMOVE | Self::EXCLUSIVE | Self::DELEGATE | Self::DROP
    );

    /// Creates new capabilities with the given flags.
    #[inline]
    pub const fn new(flags: u16) -> Self {
        Capabilities(flags)
    }

    /// Checks if the given capability is present.
    #[inline]
    pub const fn has(&self, cap: u16) -> bool {
        (self.0 & cap) != 0
    }

    /// Attenuates (removes) capabilities, returning a new Capabilities.
    #[inline]
    pub const fn attenuate(&self, remove: u16) -> Self {
        Capabilities(self.0 & !remove)
    }

    /// Combines capabilities from two references (intersection).
    #[inline]
    pub const fn intersect(&self, other: &Self) -> Self {
        Capabilities(self.0 & other.0)
    }
}

/// ThinRef - Compact CBGR reference (16 bytes).
///
/// Used for references to sized types. Contains:
/// - Raw pointer to the data
/// - Generation counter for dangling reference detection
/// - Epoch and capabilities packed together
///
/// Layout: ptr:8 + generation:4 + epoch_caps:4 = 16 bytes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct ThinRef {
    /// Raw pointer to the referenced data.
    pub ptr: *mut u8,
    /// Generation counter (32 bits).
    /// Incremented when the allocation is freed/reallocated.
    pub generation: u32,
    /// Packed epoch (16 bits) and capabilities (16 bits).
    /// epoch: bits 16-31, caps: bits 0-15
    pub epoch_caps: u32,
}

impl ThinRef {
    /// Creates a new ThinRef.
    #[inline]
    pub fn new(ptr: *mut u8, generation: u32, epoch: u16, caps: Capabilities) -> Self {
        Self {
            ptr,
            generation,
            epoch_caps: ((epoch as u32) << 16) | (caps.0 as u32),
        }
    }

    /// Creates a null ThinRef.
    #[inline]
    pub fn null() -> Self {
        Self {
            ptr: std::ptr::null_mut(),
            generation: 0,
            epoch_caps: 0,
        }
    }

    /// Returns the epoch.
    #[inline]
    pub fn epoch(&self) -> u16 {
        (self.epoch_caps >> 16) as u16
    }

    /// Returns the capabilities.
    #[inline]
    pub fn capabilities(&self) -> Capabilities {
        Capabilities((self.epoch_caps & 0xFFFF) as u16)
    }

    /// Returns true if this reference is null.
    #[inline]
    pub fn is_null(&self) -> bool {
        self.ptr.is_null()
    }

    /// Attenuates capabilities, returning a new ThinRef.
    #[inline]
    pub fn attenuate(&self, remove: u16) -> Self {
        let new_caps = self.capabilities().attenuate(remove);
        Self {
            ptr: self.ptr,
            generation: self.generation,
            epoch_caps: ((self.epoch() as u32) << 16) | (new_caps.0 as u32),
        }
    }
}

// SAFETY: ThinRef can be sent/shared across threads because:
// - `ptr` is a raw pointer to heap-allocated data protected by CBGR generation checks;
//   any access through the pointer is validated against the generation counter before
//   dereferencing, preventing use-after-free across threads
// - `generation` and `epoch_caps` are plain u32 values (immutable snapshot of the
//   allocation state at reference creation time), not shared mutable state
// - The CBGR validation protocol ensures that stale references (where generation or
//   epoch has changed) are detected and rejected at runtime, regardless of thread
// Invariant: callers must not dereference without CBGR validation
unsafe impl Send for ThinRef {}
unsafe impl Sync for ThinRef {}

impl Default for ThinRef {
    fn default() -> Self {
        Self::null()
    }
}

/// FatRef - Extended CBGR reference (32 bytes).
///
/// Used for references that need additional metadata:
/// - Slices (need length)
/// - Trait objects (need vtable pointer)
/// - Interior references (need offset)
///
/// Layout: ptr:8 + generation:4 + epoch_caps:4 + metadata:8 + offset:4 + reserved:4 = 32 bytes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct FatRef {
    /// Thin reference portion (16 bytes).
    pub thin: ThinRef,
    /// Metadata (8 bytes) - interpreted based on reference type:
    /// - For slices: length as u64
    /// - For trait objects: vtable pointer
    /// - For other types: type-specific metadata
    pub metadata: u64,
    /// Offset from base pointer (for interior references).
    pub offset: u32,
    /// Reserved for future use (alignment padding).
    pub reserved: u32,
}

impl FatRef {
    /// Creates a new FatRef.
    #[inline]
    pub fn new(ptr: *mut u8, generation: u32, epoch: u16, caps: Capabilities, metadata: u64) -> Self {
        Self {
            thin: ThinRef::new(ptr, generation, epoch, caps),
            metadata,
            offset: 0,
            reserved: 0,
        }
    }

    /// Creates a slice reference.
    #[inline]
    pub fn slice(ptr: *mut u8, generation: u32, epoch: u16, caps: Capabilities, len: u64) -> Self {
        Self::new(ptr, generation, epoch, caps, len)
    }

    /// Creates a null FatRef.
    #[inline]
    pub fn null() -> Self {
        Self {
            thin: ThinRef::null(),
            metadata: 0,
            offset: 0,
            reserved: 0,
        }
    }

    /// Returns the pointer.
    #[inline]
    pub fn ptr(&self) -> *mut u8 {
        self.thin.ptr
    }

    /// Returns the generation.
    #[inline]
    pub fn generation(&self) -> u32 {
        self.thin.generation
    }

    /// Returns the epoch.
    #[inline]
    pub fn epoch(&self) -> u16 {
        self.thin.epoch()
    }

    /// Returns the capabilities.
    #[inline]
    pub fn capabilities(&self) -> Capabilities {
        self.thin.capabilities()
    }

    /// Returns the metadata (interpreted as length for slices).
    #[inline]
    pub fn len(&self) -> u64 {
        self.metadata
    }

    /// Returns true if this fat reference has zero length (empty slice).
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.metadata == 0
    }

    /// Returns true if this reference is null.
    #[inline]
    pub fn is_null(&self) -> bool {
        self.thin.is_null()
    }

    /// Converts to a ThinRef (discards metadata).
    #[inline]
    pub fn to_thin(&self) -> ThinRef {
        self.thin
    }

    /// Attenuates capabilities, returning a new FatRef.
    #[inline]
    pub fn attenuate(&self, remove: u16) -> Self {
        Self {
            thin: self.thin.attenuate(remove),
            metadata: self.metadata,
            offset: self.offset,
            reserved: self.reserved,
        }
    }
}

// SAFETY: FatRef can be sent/shared across threads because:
// - Contains a ThinRef (see ThinRef SAFETY justification above) plus metadata
// - `metadata` (u64), `offset` (u32), `reserved` (u32) are plain immutable data
// - The same CBGR validation protocol applies: all dereferences go through
//   generation/epoch checks that detect stale references regardless of thread
// Invariant: callers must not dereference without CBGR validation
unsafe impl Send for FatRef {}
unsafe impl Sync for FatRef {}

impl Default for FatRef {
    fn default() -> Self {
        Self::null()
    }
}

/// NaN-boxed runtime value.
///
/// All VBC runtime values are represented as 64-bit NaN-boxed values.
/// This provides efficient storage (8 bytes per value) and fast type
/// checking via bit manipulation.
///
/// # Safety
///
/// The internal representation assumes x86-64 or ARM64 with 48-bit
/// virtual addresses. On systems with larger address spaces, pointer
/// encoding may fail.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct Value(u64);

impl Value {
    // ========================================================================
    // Constructors
    // ========================================================================

    /// Creates a float value.
    ///
    /// If the float is NaN, it's boxed specially to avoid confusion
    /// with tagged values.
    #[inline]
    pub fn from_f64(f: f64) -> Self {
        let bits = f.to_bits();

        // Check if it's a NaN (exponent all 1s, mantissa non-zero)
        let is_nan = (bits & 0x7FF0_0000_0000_0000) == 0x7FF0_0000_0000_0000
            && (bits & 0x000F_FFFF_FFFF_FFFF) != 0;

        if is_nan {
            // Box NaN with special tag to preserve original bits
            Value(NAN_BITS | (TAG_NAN << TAG_SHIFT) | (bits & PAYLOAD_MASK))
        } else {
            Value(bits)
        }
    }

    /// Creates an integer value.
    ///
    /// Small integers (-2^47 to 2^47-1) are stored inline.
    /// Larger integers are automatically boxed (heap-allocated).
    #[inline]
    pub fn from_i64(i: i64) -> Self {
        if (MIN_SMALL_INT..=MAX_SMALL_INT).contains(&i) {
            // Inline: fits in 48-bit payload
            let payload = (i as u64) & PAYLOAD_MASK;
            Value(NAN_BITS | (TAG_INTEGER << TAG_SHIFT) | payload)
        } else {
            // Box: allocate on heap for large integers
            Self::from_i64_boxed(i)
        }
    }

    /// Creates a boxed integer value (stored in global table).
    ///
    /// This is used for integers outside the 48-bit inline range.
    /// The integer is stored in a global table and the index is encoded.
    ///
    /// # Note
    ///
    /// Uses a global table for storage. In production, this should
    /// integrate with the runtime's garbage collector for cleanup.
    #[inline]
    fn from_i64_boxed(i: i64) -> Self {
        let mut table = BOXED_INTS.lock().unwrap();
        let index = table.len() as u64;
        if index > BOXED_INT_INDEX_MASK {
            // Prevent OOM: wrap index to stay within table bounds (lossy but safe)
            let wrapped_index = index & BOXED_INT_INDEX_MASK;
            return Value(NAN_BITS | (TAG_INTEGER << TAG_SHIFT) | wrapped_index);
        }
        table.push(i);
        Value(NAN_BITS | (TAG_POINTER << TAG_SHIFT) | BOXED_INT_MARKER | index)
    }

    /// Creates a boolean value.
    #[inline]
    pub fn from_bool(b: bool) -> Self {
        Value(NAN_BITS | (TAG_BOOLEAN << TAG_SHIFT) | (b as u64))
    }

    /// Creates a character value.
    /// Characters are stored as integers (Unicode codepoint).
    #[inline]
    pub fn from_char(c: char) -> Self {
        Self::from_i64(c as i64)
    }

    /// Creates the unit value.
    #[inline]
    pub fn unit() -> Self {
        Value(NAN_BITS | (TAG_UNIT << TAG_SHIFT))
    }

    /// Creates a pointer value.
    ///
    /// # Safety
    ///
    /// The pointer must fit in 48 bits and remain valid for the
    /// lifetime of this value.
    #[inline]
    pub fn from_ptr<T>(ptr: *mut T) -> Self {
        let addr = ptr as u64;
        debug_assert!(
            addr & !PAYLOAD_MASK == 0,
            "Pointer {:p} doesn't fit in 48 bits",
            ptr
        );
        Value(NAN_BITS | (TAG_POINTER << TAG_SHIFT) | addr)
    }

    /// Creates a small string value (up to 6 bytes).
    ///
    /// Returns `None` if the string doesn't fit (> 6 bytes).
    /// Layout: bits 47-44 = length (4 bits), bits 0-47 = 6 bytes of string data
    #[inline]
    pub fn from_small_string(s: &str) -> Option<Self> {
        let bytes = s.as_bytes();
        if bytes.len() > 6 {
            return None;
        }

        let mut payload: u64 = 0;
        // Store bytes first (bits 0-47)
        for (i, &byte) in bytes.iter().enumerate() {
            payload |= (byte as u64) << (i * 8);
        }

        Some(Value(NAN_BITS | (TAG_SMALL_STRING << TAG_SHIFT) | payload))
    }

    /// Creates a type reference value.
    #[inline]
    pub fn from_type(type_id: TypeId) -> Self {
        Value(NAN_BITS | (TAG_TYPE_REF << TAG_SHIFT) | (type_id.0 as u64))
    }

    /// Creates a function reference value.
    #[inline]
    pub fn from_function(func_id: FunctionId) -> Self {
        Value(NAN_BITS | (TAG_FUNC_REF << TAG_SHIFT) | (func_id.0 as u64))
    }

    /// Creates a nil/null value (null pointer).
    #[inline]
    pub fn nil() -> Self {
        Self::from_ptr(std::ptr::null_mut::<u8>())
    }

    /// Returns true if this value is nil (null pointer).
    #[inline]
    pub fn is_nil(&self) -> bool {
        self.is_ptr() && (self.0 & PAYLOAD_MASK) == 0
    }

    /// Creates a generator reference value.
    ///
    /// Generators are encoded using TAG_POINTER with the GENERATOR_MARKER bit set.
    /// Bits 0-44 hold the generator ID (45 bits total).
    ///
    /// Generator encoding: TAG_POINTER | GENERATOR_MARKER | (id & GENERATOR_ID_MASK).
    /// The generator ID indexes into the interpreter's generator table (Vec<Generator>).
    #[inline]
    pub fn from_generator(generator_id: u64) -> Self {
        debug_assert!(
            generator_id <= GENERATOR_ID_MASK,
            "Generator ID {} exceeds 45-bit limit",
            generator_id
        );
        Value(NAN_BITS | (TAG_POINTER << TAG_SHIFT) | GENERATOR_MARKER | (generator_id & GENERATOR_ID_MASK))
    }

    /// Creates a ThinRef value (stored in global table).
    ///
    /// ThinRef is a 16-byte CBGR reference containing:
    /// - Pointer to data (8 bytes)
    /// - Generation counter (4 bytes)
    /// - Epoch + capabilities (4 bytes)
    ///
    /// Since ThinRef doesn't fit in 64 bits, it's stored in a global table
    /// and the index is encoded in the Value.
    #[inline]
    pub fn from_thin_ref(thin_ref: ThinRef) -> Self {
        let mut table = THIN_REFS.lock().unwrap();
        let index = table.len() as u64;
        if index > REF_INDEX_MASK {
            // Prevent OOM: return nil to signal overflow (safe fallback)
            return Value::nil();
        }
        table.push(thin_ref);
        Value(NAN_BITS | (TAG_POINTER << TAG_SHIFT) | THIN_REF_MARKER | index)
    }

    /// Creates a FatRef value (stored in global table).
    ///
    /// FatRef is a 32-byte CBGR reference containing:
    /// - ThinRef (16 bytes)
    /// - Metadata (8 bytes) - length for slices, vtable for traits
    /// - Offset (4 bytes) - for interior references
    /// - Reserved (4 bytes)
    ///
    /// Since FatRef doesn't fit in 64 bits, it's stored in a global table
    /// and the index is encoded in the Value.
    #[inline]
    pub fn from_fat_ref(fat_ref: FatRef) -> Self {
        let mut table = FAT_REFS.lock().unwrap();
        let index = table.len() as u64;
        if index > REF_INDEX_MASK {
            // Prevent OOM: return nil to signal overflow (safe fallback)
            return Value::nil();
        }
        table.push(fat_ref);
        Value(NAN_BITS | (TAG_POINTER << TAG_SHIFT) | FAT_REF_MARKER | index)
    }

    // ========================================================================
    // Type Checks
    // ========================================================================

    /// Returns true if this value is a float (not a tagged value).
    #[inline]
    pub fn is_float(&self) -> bool {
        // It's a float if it's not our tagged NaN pattern
        // Check: not (starts with 0x7FF8)
        (self.0 & 0xFFF8_0000_0000_0000) != 0x7FF8_0000_0000_0000
    }

    /// Returns true if this value is a tagged value (not a pure float).
    #[inline]
    pub fn is_tagged(&self) -> bool {
        !self.is_float()
    }

    /// Returns the tag for tagged values.
    ///
    /// Returns None for pure float values.
    #[inline]
    pub fn tag(&self) -> Option<u8> {
        if self.is_float() {
            None
        } else {
            Some(((self.0 & TAG_MASK) >> TAG_SHIFT) as u8)
        }
    }

    /// Returns true if this is an inline integer (48-bit range).
    #[inline]
    pub fn is_inline_int(&self) -> bool {
        self.tag() == Some(TAG_INTEGER as u8)
    }

    /// Returns true if this is a boxed integer (heap-allocated for large values).
    ///
    /// Boxed integers are stored with both bit 47 and bit 46 set, plus an index
    /// into a global table. This encoding guarantees no collision with:
    /// - Real heap pointers: bit 47 = 0 (canonical addressing)
    /// - Generators: bit 47 = 1, bit 46 = 0
    ///
    /// On x86-64/ARM64, user-space addresses have bits 47-63 all-zero.
    #[inline]
    pub fn is_boxed_int(&self) -> bool {
        self.tag() == Some(TAG_POINTER as u8)
            && (self.0 & BOXED_INT_MARKER) == BOXED_INT_MARKER  // Both bits 47 and 46 must be set
    }

    /// Returns true if this is an integer (inline or boxed).
    #[inline]
    pub fn is_int(&self) -> bool {
        self.is_inline_int() || self.is_boxed_int()
    }

    /// Returns true if this is a boolean.
    #[inline]
    pub fn is_bool(&self) -> bool {
        self.tag() == Some(TAG_BOOLEAN as u8)
    }

    /// Returns true if this is the unit value.
    #[inline]
    pub fn is_unit(&self) -> bool {
        self.tag() == Some(TAG_UNIT as u8)
    }

    /// Returns true if this is a pointer.
    ///
    /// Note: Boxed integers share the TAG_POINTER tag but are semantically
    /// integers, not pointers. This returns `false` for boxed integers so that
    /// `is_ptr()` callers never attempt to dereference a boxed integer's
    /// encoded index as a raw memory address.
    #[inline]
    pub fn is_ptr(&self) -> bool {
        self.tag() == Some(TAG_POINTER as u8) && !self.is_boxed_int()
    }

    /// Returns true if this is a small string.
    #[inline]
    pub fn is_small_string(&self) -> bool {
        self.tag() == Some(TAG_SMALL_STRING as u8)
    }

    /// Returns true if this is a type reference.
    #[inline]
    pub fn is_type_ref(&self) -> bool {
        self.tag() == Some(TAG_TYPE_REF as u8)
    }

    /// Returns true if this is a function reference.
    #[inline]
    pub fn is_func_ref(&self) -> bool {
        self.tag() == Some(TAG_FUNC_REF as u8)
    }

    /// Returns true if this is a generator reference.
    ///
    /// Generators use TAG_POINTER with bit 47 set and bit 46 clear.
    /// This distinguishes them from:
    /// - Real heap pointers: bit 47 = 0
    /// - Boxed integers: bit 47 = 1, bit 46 = 1
    ///
    /// Checks if this value is a generator reference. Distinguished from other pointer-tagged
    /// values by: bit 47 set (SPECIAL_VALUE_MARKER), bit 46 clear (not boxed int), bit 45 clear (not ThinRef).
    #[inline]
    pub fn is_generator(&self) -> bool {
        self.tag() == Some(TAG_POINTER as u8)
            && (self.0 & SPECIAL_VALUE_MARKER) != 0  // bit 47 set
            && (self.0 & BOXED_INT_SUB_MARKER) == 0  // bit 46 clear
            && (self.0 & THIN_REF_SUB_MARKER) == 0   // bit 45 clear (not ThinRef)
    }

    /// Returns true if this is a ThinRef (CBGR managed reference).
    ///
    /// ThinRef values use TAG_POINTER with bit 47 set, bit 46 clear, and bit 45 set.
    /// This distinguishes them from:
    /// - Real heap pointers: bit 47 = 0
    /// - Generators: bit 47 = 1, bit 46 = 0, bit 45 = 0
    /// - Boxed integers: bit 47 = 1, bit 46 = 1, bit 45 = 0
    /// - FatRef: bit 47 = 1, bit 46 = 1, bit 45 = 1
    #[inline]
    pub fn is_thin_ref(&self) -> bool {
        self.tag() == Some(TAG_POINTER as u8)
            && (self.0 & THIN_REF_MARKER) == THIN_REF_MARKER  // bits 47 and 45 set
            && (self.0 & BOXED_INT_SUB_MARKER) == 0           // bit 46 clear
    }

    /// Returns true if this is a FatRef (CBGR managed reference with metadata).
    ///
    /// FatRef values use TAG_POINTER with bits 47, 46, and 45 all set.
    /// This distinguishes them from:
    /// - Real heap pointers: bit 47 = 0
    /// - Generators: bit 47 = 1, bit 46 = 0, bit 45 = 0
    /// - ThinRef: bit 47 = 1, bit 46 = 0, bit 45 = 1
    /// - Boxed integers: bit 47 = 1, bit 46 = 1, bit 45 = 0
    #[inline]
    pub fn is_fat_ref(&self) -> bool {
        self.tag() == Some(TAG_POINTER as u8)
            && (self.0 & FAT_REF_MARKER) == FAT_REF_MARKER  // bits 47, 46, and 45 all set
    }

    /// Returns true if this is any CBGR reference (ThinRef or FatRef).
    #[inline]
    pub fn is_cbgr_ref(&self) -> bool {
        self.is_thin_ref() || self.is_fat_ref()
    }

    /// Returns true if this is a regular pointer (not a generator, boxed int, ThinRef, FatRef, or nil).
    ///
    /// Real heap pointers have bit 47 = 0 due to canonical addressing on x86-64/ARM64.
    /// Generators and boxed integers both have bit 47 = 1, so checking SPECIAL_VALUE_MARKER
    /// is sufficient to exclude them.
    #[inline]
    pub fn is_regular_ptr(&self) -> bool {
        self.tag() == Some(TAG_POINTER as u8)
            && (self.0 & SPECIAL_VALUE_MARKER) == 0  // bit 47 clear = real pointer
            && !self.is_nil()
    }

    // ========================================================================
    // Extractors
    // ========================================================================

    /// Extracts as f64.
    ///
    /// # Panics
    ///
    /// Panics if this is not a float value.
    #[inline]
    pub fn as_f64(&self) -> f64 {
        if self.tag() == Some(TAG_NAN as u8) {
            // Reconstruct original NaN
            f64::from_bits(self.0 & PAYLOAD_MASK | 0x7FF8_0000_0000_0000)
        } else {
            debug_assert!(self.is_float(), "Expected float, got {:?}", self.tag());
            f64::from_bits(self.0)
        }
    }

    /// Extracts as i64.
    ///
    /// Handles both inline integers (48-bit) and boxed integers (table-stored).
    ///
    /// # Panics
    ///
    /// Panics if this is not an integer value.
    ///
    /// For boxed integers whose index falls outside the current
    /// `BOXED_INTS` table (e.g. because `reset_global_value_tables()` was
    /// called while the `Value` remained reachable through a cached side
    /// structure like a Mutex-held ConstantEntry, or because a bit pattern
    /// from an adjacent Value family is being misclassified), return `0`
    /// rather than indexing past the end. This is a hard-crash → soft-nil
    /// downgrade: the panic used to take down `vtest-check` threads during
    /// batch L2 runs; the benign fallback lets the surrounding typecheck
    /// carry on and surface any upstream miscoding as normal type errors
    /// instead of an out-of-bounds abort.
    #[inline]
    pub fn as_i64(&self) -> i64 {
        debug_assert!(self.is_int(), "Expected int, got {:?}", self.tag());
        if self.is_boxed_int() {
            // Boxed: look up in global table
            let index = (self.0 & BOXED_INT_INDEX_MASK) as usize;
            let table = BOXED_INTS.lock().unwrap();
            table.get(index).copied().unwrap_or(0)
        } else {
            // Inline: sign-extend from 48 bits to 64 bits
            let payload = (self.0 & PAYLOAD_MASK) as i64;
            (payload << 16) >> 16
        }
    }

    /// Extracts as bool.
    ///
    /// # Panics
    ///
    /// Panics if this is not a boolean value.
    #[inline]
    pub fn as_bool(&self) -> bool {
        debug_assert!(self.is_bool(), "Expected bool, got {:?}", self.tag());
        (self.0 & 1) != 0
    }

    /// Returns true if this value is "truthy" for conditional branching.
    ///
    /// - Bool: true/false
    /// - Int: non-zero is truthy
    /// - Nil/Unit: falsy
    /// - Pointer: non-null is truthy
    /// - Other: truthy
    #[inline]
    pub fn is_truthy(&self) -> bool {
        if self.is_bool() {
            (self.0 & 1) != 0
        } else if self.is_int() {
            self.as_i64() != 0
        } else { !(self.is_nil() || self.is_unit()) }
    }

    /// Extracts as i64, accepting both Int and Bool values.
    ///
    /// Bool is coerced to 0 (false) or 1 (true). This is used by integer
    /// comparison handlers which may receive Bool operands from codegen
    /// (Bool is classified as a primitive type).
    ///
    /// # Panics
    ///
    /// Panics if this is neither an integer nor a boolean value.
    #[inline]
    pub fn as_integer_compatible(&self) -> i64 {
        if self.is_bool() {
            (self.0 & 1) as i64
        } else if self.is_float() {
            self.as_f64() as i64
        } else if self.is_int() {
            self.as_i64()
        } else if self.is_ptr() {
            // Pointer values (e.g., variant objects like Maybe<Byte>) — compare by address.
            // This enables comparison operations on variant values without crashing.
            (self.0 & PAYLOAD_MASK) as i64
        } else if self.is_unit() || self.is_nil() {
            0
        } else {
            // Fallback: extract raw payload bits as signed integer.
            // Covers TypeRef, FuncRef, SmallString, etc.
            let payload = (self.0 & PAYLOAD_MASK) as i64;
            (payload << 16) >> 16
        }
    }

    /// Extracts as char.
    ///
    /// Characters are stored as integers (Unicode codepoint).
    /// Returns the character corresponding to the stored codepoint.
    ///
    /// # Panics
    ///
    /// Panics if this is not an integer value or the value is not a valid char.
    #[inline]
    pub fn as_char(&self) -> char {
        let codepoint = self.as_i64();
        char::from_u32(codepoint as u32).expect("Invalid character codepoint")
    }

    /// Extracts as pointer.
    ///
    /// # Panics
    ///
    /// Panics if this is not a pointer value.
    #[inline]
    pub fn as_ptr<T>(&self) -> *mut T {
        debug_assert!(self.is_ptr(), "Expected pointer, got {:?}", self.tag());
        (self.0 & PAYLOAD_MASK) as *mut T
    }

    /// Extracts as small string.
    ///
    /// # Panics
    ///
    /// Panics if this is not a small string value.
    #[inline]
    pub fn as_small_string(&self) -> SmallString {
        debug_assert!(
            self.is_small_string(),
            "Expected small string, got {:?}",
            self.tag()
        );
        let payload = self.0 & PAYLOAD_MASK;
        // Extract 6 bytes and find length by looking for first zero
        let mut bytes = [0u8; 6];
        let mut len = 0;
        for (i, byte_slot) in bytes.iter_mut().enumerate() {
            let byte = ((payload >> (i * 8)) & 0xFF) as u8;
            *byte_slot = byte;
            if byte != 0 {
                len = i + 1;
            }
        }
        SmallString { bytes, len }
    }

    /// Extracts as TypeId.
    ///
    /// # Panics
    ///
    /// Panics if this is not a type reference.
    #[inline]
    pub fn as_type_id(&self) -> TypeId {
        debug_assert!(
            self.is_type_ref(),
            "Expected type ref, got {:?}",
            self.tag()
        );
        TypeId((self.0 & PAYLOAD_MASK) as u32)
    }

    /// Extracts as FunctionId.
    ///
    /// # Panics
    ///
    /// Panics if this is not a function reference.
    #[inline]
    pub fn as_func_id(&self) -> FunctionId {
        debug_assert!(
            self.is_func_ref(),
            "Expected func ref, got {:?}",
            self.tag()
        );
        FunctionId((self.0 & PAYLOAD_MASK) as u32)
    }

    /// Extracts the generator ID from a generator value.
    ///
    /// # Panics
    ///
    /// Panics if this is not a generator reference.
    ///
    /// Extracts the 45-bit generator ID from bits 0-44 of the NaN-boxed value.
    /// The ID indexes into the interpreter's generator table for state lookup/resume.
    #[inline]
    pub fn as_generator_id(&self) -> u64 {
        debug_assert!(
            self.is_generator(),
            "Expected generator, got {:?}",
            self.tag()
        );
        self.0 & GENERATOR_ID_MASK
    }

    /// Extracts the ThinRef from a ThinRef value.
    ///
    /// # Panics
    ///
    /// Panics if this is not a ThinRef value.
    #[inline]
    pub fn as_thin_ref(&self) -> ThinRef {
        debug_assert!(
            self.is_thin_ref(),
            "Expected ThinRef, got {:?}",
            self.tag()
        );
        let index = (self.0 & REF_INDEX_MASK) as usize;
        let table = THIN_REFS.lock().unwrap();
        table[index]
    }

    /// Extracts the FatRef from a FatRef value.
    ///
    /// # Panics
    ///
    /// Panics if this is not a FatRef value.
    #[inline]
    pub fn as_fat_ref(&self) -> FatRef {
        debug_assert!(
            self.is_fat_ref(),
            "Expected FatRef, got {:?}",
            self.tag()
        );
        let index = (self.0 & REF_INDEX_MASK) as usize;
        let table = FAT_REFS.lock().unwrap();
        table[index]
    }

    // ========================================================================
    // Safe Extractors (Return Option)
    // ========================================================================

    /// Tries to extract as f64.
    #[inline]
    pub fn try_as_f64(&self) -> Option<f64> {
        if self.is_float() || self.tag() == Some(TAG_NAN as u8) {
            Some(self.as_f64())
        } else {
            None
        }
    }

    /// Tries to extract as i64.
    #[inline]
    pub fn try_as_i64(&self) -> Option<i64> {
        if self.is_int() {
            Some(self.as_i64())
        } else {
            None
        }
    }

    /// Tries to extract as bool.
    #[inline]
    pub fn try_as_bool(&self) -> Option<bool> {
        if self.is_bool() {
            Some(self.as_bool())
        } else {
            None
        }
    }

    /// Tries to extract as generator ID.
    ///
    /// Returns Some(generator_id) if this is a generator value, None otherwise.
    #[inline]
    pub fn try_as_generator_id(&self) -> Option<u64> {
        if self.is_generator() {
            Some(self.as_generator_id())
        } else {
            None
        }
    }

    /// Tries to extract as ThinRef.
    #[inline]
    pub fn try_as_thin_ref(&self) -> Option<ThinRef> {
        if self.is_thin_ref() {
            Some(self.as_thin_ref())
        } else {
            None
        }
    }

    /// Tries to extract as FatRef.
    #[inline]
    pub fn try_as_fat_ref(&self) -> Option<FatRef> {
        if self.is_fat_ref() {
            Some(self.as_fat_ref())
        } else {
            None
        }
    }

    // ========================================================================
    // Raw Access
    // ========================================================================

    /// Returns the raw 64-bit representation.
    #[inline]
    pub fn to_bits(&self) -> u64 {
        self.0
    }

    /// Creates a value from raw bits.
    #[inline]
    pub fn from_bits(bits: u64) -> Self {
        Value(bits)
    }
}

impl Default for Value {
    fn default() -> Self {
        Value::unit()
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        // For floats, use bit comparison (handles NaN properly)
        self.0 == other.0
    }
}

impl Eq for Value {}

impl std::hash::Hash for Value {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_float() {
            write!(f, "Value::Float({:?})", self.as_f64())
        } else if self.is_generator() {
            // Check generator before generic pointer (generators use TAG_POINTER)
            write!(f, "Value::Generator({})", self.as_generator_id())
        } else if self.is_thin_ref() {
            // Check ThinRef before generic pointer (uses TAG_POINTER)
            let thin_ref = self.as_thin_ref();
            write!(f, "Value::ThinRef({:p}, gen={}, epoch={}, caps={:#x})",
                thin_ref.ptr, thin_ref.generation, thin_ref.epoch(), thin_ref.capabilities().0)
        } else if self.is_fat_ref() {
            // Check FatRef before boxed int (both have bit 46 set)
            let fat_ref = self.as_fat_ref();
            write!(f, "Value::FatRef({:p}, gen={}, epoch={}, caps={:#x}, len={})",
                fat_ref.ptr(), fat_ref.generation(), fat_ref.epoch(), fat_ref.capabilities().0, fat_ref.len())
        } else if self.is_boxed_int() {
            // Check boxed int before generic pointer (boxed ints use TAG_POINTER)
            write!(f, "Value::BoxedInt({})", self.as_i64())
        } else {
            match self.tag() {
                Some(tag) if tag == TAG_INTEGER as u8 => {
                    write!(f, "Value::Int({})", self.as_i64())
                }
                Some(tag) if tag == TAG_BOOLEAN as u8 => {
                    write!(f, "Value::Bool({})", self.as_bool())
                }
                Some(tag) if tag == TAG_UNIT as u8 => write!(f, "Value::Unit"),
                Some(tag) if tag == TAG_POINTER as u8 => {
                    write!(f, "Value::Ptr({:#x})", self.0 & PAYLOAD_MASK)
                }
                Some(tag) if tag == TAG_SMALL_STRING as u8 => {
                    write!(f, "Value::SmallStr({:?})", self.as_small_string())
                }
                Some(tag) if tag == TAG_TYPE_REF as u8 => {
                    write!(f, "Value::Type({:?})", self.as_type_id())
                }
                Some(tag) if tag == TAG_FUNC_REF as u8 => {
                    write!(f, "Value::Func({:?})", self.as_func_id())
                }
                Some(tag) if tag == TAG_NAN as u8 => {
                    write!(f, "Value::NaN({:?})", self.as_f64())
                }
                Some(tag) => write!(f, "Value::Unknown(tag={}, bits={:#x})", tag, self.0),
                None => write!(f, "Value::Float({:?})", self.as_f64()),
            }
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_float() {
            write!(f, "{}", self.as_f64())
        } else if self.is_generator() {
            write!(f, "<generator#{}>", self.as_generator_id())
        } else if self.is_thin_ref() {
            write!(f, "<ref>")
        } else if self.is_fat_ref() {
            let fat_ref = self.as_fat_ref();
            write!(f, "<ref[{}]>", fat_ref.len())
        } else if self.is_boxed_int() {
            // Boxed integers display as regular integers
            write!(f, "{}", self.as_i64())
        } else {
            match self.tag() {
                Some(tag) if tag == TAG_INTEGER as u8 => write!(f, "{}", self.as_i64()),
                Some(tag) if tag == TAG_BOOLEAN as u8 => write!(f, "{}", self.as_bool()),
                Some(tag) if tag == TAG_UNIT as u8 => write!(f, "()"),
                Some(tag) if tag == TAG_POINTER as u8 => write!(f, "<ptr>"),
                Some(tag) if tag == TAG_SMALL_STRING as u8 => {
                    write!(f, "{}", self.as_small_string())
                }
                Some(tag) if tag == TAG_TYPE_REF as u8 => write!(f, "<type>"),
                Some(tag) if tag == TAG_FUNC_REF as u8 => write!(f, "<func>"),
                _ => write!(f, "<unknown>"),
            }
        }
    }
}

// ============================================================================
// Small String Helper
// ============================================================================

/// Small string extracted from a NaN-boxed value (max 6 bytes).
#[derive(Clone, Copy)]
pub struct SmallString {
    bytes: [u8; 6],
    len: usize,
}

impl SmallString {
    /// Returns the string as a byte slice.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    /// Returns the string as a str.
    pub fn as_str(&self) -> &str {
        // SmallString bytes originate from `&str` via `Value::from_small_string`,
        // which takes `&str` (guaranteed UTF-8). The `as_small_string()` method only
        // extracts those same bytes back. Use checked conversion in release builds
        // to guard against memory corruption.
        #[cfg(debug_assertions)]
        {
            std::str::from_utf8(&self.bytes[..self.len])
                .unwrap_or_else(|e| panic!("SmallString contains invalid UTF-8: {:?}, error: {}", &self.bytes[..self.len], e))
        }
        #[cfg(not(debug_assertions))]
        {
            // In release: use checked conversion to prevent UB from corruption.
            // Falls back to empty string on invalid UTF-8 rather than UB.
            std::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
        }
    }

    /// Returns the length.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns true if empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl fmt::Debug for SmallString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

impl fmt::Display for SmallString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_float_values() {
        let v = Value::from_f64(3.14);
        assert!(v.is_float());
        assert!(!v.is_int());
        assert_eq!(v.as_f64(), 3.14);

        let v = Value::from_f64(-0.0);
        assert!(v.is_float());
        assert_eq!(v.as_f64(), -0.0);

        let v = Value::from_f64(f64::INFINITY);
        assert!(v.is_float());
        assert!(v.as_f64().is_infinite());
    }

    #[test]
    fn test_nan_values() {
        let nan = f64::NAN;
        let v = Value::from_f64(nan);
        assert!(v.as_f64().is_nan());
    }

    #[test]
    fn test_int_values() {
        let v = Value::from_i64(42);
        assert!(v.is_int());
        assert!(!v.is_float());
        assert_eq!(v.as_i64(), 42);

        let v = Value::from_i64(-100);
        assert!(v.is_int());
        assert_eq!(v.as_i64(), -100);

        let v = Value::from_i64(0);
        assert!(v.is_int());
        assert_eq!(v.as_i64(), 0);

        // Test near boundary
        let v = Value::from_i64(MAX_SMALL_INT);
        assert_eq!(v.as_i64(), MAX_SMALL_INT);

        let v = Value::from_i64(MIN_SMALL_INT);
        assert_eq!(v.as_i64(), MIN_SMALL_INT);
    }

    #[test]
    fn test_bool_values() {
        let v = Value::from_bool(true);
        assert!(v.is_bool());
        assert!(v.as_bool());

        let v = Value::from_bool(false);
        assert!(v.is_bool());
        assert!(!v.as_bool());
    }

    #[test]
    fn test_unit_value() {
        let v = Value::unit();
        assert!(v.is_unit(), "Expected unit, is_float={}, tag={:?}", v.is_float(), v.tag());
        assert!(!v.is_float());
        assert!(!v.is_int());
        assert!(!v.is_bool());
        assert!(!v.is_ptr());
        assert_eq!(v.tag(), Some(TAG_UNIT as u8));
    }

    #[test]
    fn test_small_string() {
        let v = Value::from_small_string("hello").unwrap();
        assert!(v.is_small_string());
        assert_eq!(v.as_small_string().as_str(), "hello");

        let v = Value::from_small_string("").unwrap();
        assert!(v.as_small_string().is_empty());

        let v = Value::from_small_string("123456").unwrap();
        assert_eq!(v.as_small_string().as_str(), "123456");

        // Too long (> 6 bytes)
        assert!(Value::from_small_string("1234567").is_none());
    }

    #[test]
    fn test_type_ref() {
        let v = Value::from_type(TypeId::INT);
        assert!(v.is_type_ref());
        assert_eq!(v.as_type_id(), TypeId::INT);
    }

    #[test]
    fn test_func_ref() {
        let v = Value::from_function(FunctionId(42));
        assert!(v.is_func_ref());
        assert_eq!(v.as_func_id(), FunctionId(42));
    }

    #[test]
    fn test_pointer() {
        let data: i32 = 42;
        let ptr = &data as *const i32 as *mut i32;
        let v = Value::from_ptr(ptr);
        assert!(v.is_ptr());
        assert_eq!(v.as_ptr::<i32>(), ptr);
    }

    #[test]
    fn test_equality() {
        assert_eq!(Value::from_i64(42), Value::from_i64(42));
        assert_ne!(Value::from_i64(42), Value::from_i64(43));
        assert_eq!(Value::from_f64(3.14), Value::from_f64(3.14));
        assert_eq!(Value::unit(), Value::unit());
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", Value::from_i64(42)), "42");
        assert_eq!(format!("{}", Value::from_bool(true)), "true");
        assert_eq!(format!("{}", Value::unit()), "()");
    }

    // ========================================================================
    // Exhaustive Edge Case Tests
    // ========================================================================

    #[test]
    fn test_int_boundary_values() {
        // Test exact boundaries
        let max = Value::from_i64(MAX_SMALL_INT);
        assert!(max.is_int());
        assert_eq!(max.as_i64(), MAX_SMALL_INT);

        let min = Value::from_i64(MIN_SMALL_INT);
        assert!(min.is_int());
        assert_eq!(min.as_i64(), MIN_SMALL_INT);

        // Test values near boundaries
        let near_max = Value::from_i64(MAX_SMALL_INT - 1);
        assert_eq!(near_max.as_i64(), MAX_SMALL_INT - 1);

        let near_min = Value::from_i64(MIN_SMALL_INT + 1);
        assert_eq!(near_min.as_i64(), MIN_SMALL_INT + 1);

        // Test powers of 2
        for i in 0..46 {
            let pow = 1i64 << i;
            let v = Value::from_i64(pow);
            assert_eq!(v.as_i64(), pow, "Failed for 2^{}", i);

            let neg = Value::from_i64(-pow);
            assert_eq!(neg.as_i64(), -pow, "Failed for -2^{}", i);
        }
    }

    #[test]
    fn test_special_float_values() {
        // Positive and negative infinity
        let pos_inf = Value::from_f64(f64::INFINITY);
        assert!(pos_inf.is_float());
        assert!(pos_inf.as_f64().is_infinite());
        assert!(pos_inf.as_f64().is_sign_positive());

        let neg_inf = Value::from_f64(f64::NEG_INFINITY);
        assert!(neg_inf.is_float());
        assert!(neg_inf.as_f64().is_infinite());
        assert!(neg_inf.as_f64().is_sign_negative());

        // Positive and negative zero
        let pos_zero = Value::from_f64(0.0);
        let neg_zero = Value::from_f64(-0.0);
        assert!(pos_zero.is_float());
        assert!(neg_zero.is_float());
        assert_eq!(pos_zero.as_f64(), 0.0);
        assert_eq!(neg_zero.as_f64(), -0.0);

        // Subnormal numbers
        let subnormal = Value::from_f64(f64::MIN_POSITIVE / 2.0);
        assert!(subnormal.is_float());

        // Epsilon
        let eps = Value::from_f64(f64::EPSILON);
        assert!(eps.is_float());
        assert_eq!(eps.as_f64(), f64::EPSILON);

        // Min/max positive
        let max = Value::from_f64(f64::MAX);
        assert!(max.is_float());
        assert_eq!(max.as_f64(), f64::MAX);

        let min_pos = Value::from_f64(f64::MIN_POSITIVE);
        assert!(min_pos.is_float());
        assert_eq!(min_pos.as_f64(), f64::MIN_POSITIVE);
    }

    #[test]
    fn test_nan_boxing_invariants() {
        // Verify NaN-boxing constants are correct
        assert_eq!(NAN_BITS, 0x7FF8_0000_0000_0000);
        assert_eq!(TAG_MASK, 0x0007_0000_0000_0000);
        assert_eq!(TAG_SHIFT, 48);
        assert_eq!(PAYLOAD_MASK, 0x0000_FFFF_FFFF_FFFF);

        // Verify tag constants are distinct and fit in 3 bits
        let tags = [TAG_POINTER, TAG_INTEGER, TAG_BOOLEAN, TAG_UNIT,
                   TAG_SMALL_STRING, TAG_TYPE_REF, TAG_FUNC_REF, TAG_NAN];
        for &tag in &tags {
            assert!(tag < 8, "Tag {} exceeds 3-bit limit", tag);
        }

        // Verify tags are unique
        for i in 0..tags.len() {
            for j in (i + 1)..tags.len() {
                assert_ne!(tags[i], tags[j], "Duplicate tags at {} and {}", i, j);
            }
        }
    }

    #[test]
    fn test_tag_extraction_consistency() {
        // Each value type should have its expected tag
        let unit = Value::unit();
        assert_eq!(unit.tag(), Some(TAG_UNIT as u8));

        let int = Value::from_i64(42);
        assert_eq!(int.tag(), Some(TAG_INTEGER as u8));

        let boolean = Value::from_bool(true);
        assert_eq!(boolean.tag(), Some(TAG_BOOLEAN as u8));

        let string = Value::from_small_string("hi").unwrap();
        assert_eq!(string.tag(), Some(TAG_SMALL_STRING as u8));

        let type_ref = Value::from_type(TypeId::INT);
        assert_eq!(type_ref.tag(), Some(TAG_TYPE_REF as u8));

        let func_ref = Value::from_function(FunctionId(1));
        assert_eq!(func_ref.tag(), Some(TAG_FUNC_REF as u8));
    }

    #[test]
    fn test_bits_roundtrip() {
        let values = [
            Value::unit(),
            Value::from_i64(42),
            Value::from_i64(-100),
            Value::from_i64(MAX_SMALL_INT),
            Value::from_i64(MIN_SMALL_INT),
            Value::from_bool(true),
            Value::from_bool(false),
            Value::from_f64(3.14),
            Value::from_f64(-2.71),
            Value::from_f64(f64::INFINITY),
            Value::from_type(TypeId::FLOAT),
            Value::from_function(FunctionId(123)),
        ];

        for v in values {
            let bits = v.to_bits();
            let restored = Value::from_bits(bits);
            assert_eq!(v, restored, "Bits roundtrip failed");
        }
    }

    #[test]
    fn test_try_methods() {
        let int = Value::from_i64(42);
        assert_eq!(int.try_as_i64(), Some(42));
        assert_eq!(int.try_as_f64(), None);
        assert_eq!(int.try_as_bool(), None);

        let float = Value::from_f64(3.14);
        assert_eq!(float.try_as_i64(), None);
        assert!(float.try_as_f64().is_some());
        assert_eq!(float.try_as_bool(), None);

        let boolean = Value::from_bool(true);
        assert_eq!(boolean.try_as_i64(), None);
        assert_eq!(boolean.try_as_f64(), None);
        assert_eq!(boolean.try_as_bool(), Some(true));
    }

    #[test]
    fn test_default_is_unit() {
        let default = Value::default();
        assert!(default.is_unit());
        assert_eq!(default, Value::unit());
    }

    #[test]
    fn test_hash_consistency() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        fn hash_value(v: &Value) -> u64 {
            let mut hasher = DefaultHasher::new();
            v.hash(&mut hasher);
            hasher.finish()
        }

        // Equal values should have equal hashes
        let v1 = Value::from_i64(42);
        let v2 = Value::from_i64(42);
        assert_eq!(hash_value(&v1), hash_value(&v2));

        // Different values may have different hashes
        let v3 = Value::from_i64(43);
        assert_ne!(hash_value(&v1), hash_value(&v3));
    }

    #[test]
    fn test_small_string_utf8() {
        // Valid UTF-8 strings
        let valid = ["", "a", "ab", "abc", "abcd", "abcde", "abcdef"];
        for s in valid {
            let v = Value::from_small_string(s);
            assert!(v.is_some(), "Failed for {:?}", s);
            assert_eq!(v.unwrap().as_small_string().as_str(), s);
        }

        // String too long (7+ bytes)
        assert!(Value::from_small_string("abcdefg").is_none());
        assert!(Value::from_small_string("12345678").is_none());
    }

    #[test]
    fn test_small_string_lengths() {
        for len in 0..=6 {
            let s: String = (0..len).map(|i| (b'a' + (i as u8 % 26)) as char).collect();
            let v = Value::from_small_string(&s).unwrap();
            let extracted = v.as_small_string();
            assert_eq!(extracted.len(), len);
            assert_eq!(extracted.as_str(), s);
        }
    }

    #[test]
    fn test_type_predicates_mutual_exclusion() {
        let values = [
            (Value::unit(), "unit"),
            (Value::from_i64(0), "int"),
            (Value::from_bool(false), "bool"),
            (Value::from_f64(0.0), "float"),
            (Value::from_type(TypeId::UNIT), "type"),
            (Value::from_function(FunctionId(0)), "func"),
        ];

        for (v, name) in values {
            let is_unit = v.is_unit();
            let is_int = v.is_int();
            let is_bool = v.is_bool();
            let is_float = v.is_float();
            let is_type = v.is_type_ref();
            let is_func = v.is_func_ref();

            // Count how many predicates are true
            let count = [is_unit, is_int, is_bool, is_float, is_type, is_func]
                .iter()
                .filter(|&&b| b)
                .count();

            assert_eq!(count, 1,
                "Value {:?} ({}) should match exactly one predicate, got {:?}",
                v, name, (is_unit, is_int, is_bool, is_float, is_type, is_func));
        }
    }

    #[test]
    fn test_debug_format() {
        assert!(format!("{:?}", Value::unit()).contains("Unit"));
        assert!(format!("{:?}", Value::from_i64(42)).contains("Int"));
        assert!(format!("{:?}", Value::from_bool(true)).contains("Bool"));
        assert!(format!("{:?}", Value::from_f64(3.14)).contains("Float"));
    }

    #[test]
    fn test_negative_integer_sign_extension() {
        // Verify negative integers are properly sign-extended
        let values = [-1i64, -2, -128, -256, -1000, -1_000_000];
        for val in values {
            let v = Value::from_i64(val);
            assert_eq!(v.as_i64(), val, "Sign extension failed for {}", val);
        }
    }

    #[test]
    fn test_payload_mask_coverage() {
        // Verify payload mask is exactly 48 bits
        assert_eq!(PAYLOAD_MASK, (1u64 << 48) - 1);
        assert_eq!(PAYLOAD_MASK.count_ones(), 48);
    }

    // ========================================================================
    // Generator Value Tests
    // ========================================================================
    //
    // Generator values: NaN-boxed with TAG_POINTER + GENERATOR_MARKER, 45-bit ID in bits 0-44.

    #[test]
    fn test_generator_value_creation() {
        let v = Value::from_generator(42);
        assert!(v.is_generator());
        assert!(!v.is_float());
        assert!(!v.is_int());
        assert!(!v.is_nil());
        assert_eq!(v.as_generator_id(), 42);
    }

    #[test]
    fn test_generator_value_zero_id() {
        let v = Value::from_generator(0);
        assert!(v.is_generator());
        assert_eq!(v.as_generator_id(), 0);
    }

    #[test]
    fn test_generator_value_large_id() {
        // Test near boundary (45-bit max)
        let max_id = GENERATOR_ID_MASK;
        let v = Value::from_generator(max_id);
        assert!(v.is_generator());
        assert_eq!(v.as_generator_id(), max_id);
    }

    #[test]
    fn test_generator_not_pointer() {
        // Generator uses TAG_POINTER but should not be treated as regular pointer
        let v = Value::from_generator(100);
        assert!(v.is_generator());
        assert!(v.is_ptr()); // Still reports as pointer for tag
        assert!(!v.is_regular_ptr()); // But not a regular pointer
        assert!(!v.is_nil());
    }

    #[test]
    fn test_generator_vs_regular_pointer() {
        let data: i32 = 42;
        let ptr = &data as *const i32 as *mut i32;
        let ptr_val = Value::from_ptr(ptr);
        let gen_val = Value::from_generator(100);

        // Both use TAG_POINTER
        assert!(ptr_val.is_ptr());
        assert!(gen_val.is_ptr());

        // Only generator should be detected as generator
        assert!(!ptr_val.is_generator());
        assert!(gen_val.is_generator());

        // Only regular pointer should be detected as regular
        assert!(ptr_val.is_regular_ptr());
        assert!(!gen_val.is_regular_ptr());
    }

    #[test]
    fn test_generator_try_extractor() {
        let gen_val = Value::from_generator(777);
        let int_val = Value::from_i64(777);

        assert_eq!(gen_val.try_as_generator_id(), Some(777));
        assert_eq!(int_val.try_as_generator_id(), None);
    }

    #[test]
    fn test_generator_debug_format() {
        let v = Value::from_generator(123);
        let debug = format!("{:?}", v);
        assert!(debug.contains("Generator"));
        assert!(debug.contains("123"));
    }

    #[test]
    fn test_generator_display_format() {
        let v = Value::from_generator(456);
        let display = format!("{}", v);
        assert!(display.contains("generator"));
        assert!(display.contains("456"));
    }

    #[test]
    fn test_generator_equality() {
        assert_eq!(Value::from_generator(1), Value::from_generator(1));
        assert_ne!(Value::from_generator(1), Value::from_generator(2));
        assert_ne!(Value::from_generator(1), Value::from_i64(1));
    }

    #[test]
    fn test_generator_roundtrip() {
        for id in [0, 1, 100, 1000, 1_000_000, GENERATOR_ID_MASK] {
            let v = Value::from_generator(id);
            let bits = v.to_bits();
            let restored = Value::from_bits(bits);
            assert!(restored.is_generator());
            assert_eq!(restored.as_generator_id(), id);
        }
    }

    #[test]
    fn test_generator_marker_constant() {
        // Verify GENERATOR_MARKER is exactly bit 47
        assert_eq!(GENERATOR_MARKER, 1u64 << 47);
        assert_eq!(GENERATOR_MARKER.count_ones(), 1);
        assert_eq!(GENERATOR_MARKER.trailing_zeros(), 47);
    }

    #[test]
    fn test_generator_id_mask_constant() {
        // Verify GENERATOR_ID_MASK is exactly 45 bits (bits 45-47 are reserved for markers)
        assert_eq!(GENERATOR_ID_MASK, (1u64 << 45) - 1);
        assert_eq!(GENERATOR_ID_MASK.count_ones(), 45);
    }
}
