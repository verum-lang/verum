//! Heap memory management with CBGR integration.
//!
//! The heap provides memory allocation for interpreter objects with:
//! - Object headers for type info and GC
//! - Generation counters for CBGR memory safety
//! - Epoch tracking for cross-allocator validation
//! - Simple bump allocation
//!
//! # CBGR Integration
//!
//! This heap implements full CBGR (Compile-time Borrow checking with Generational
//! References) semantics:
//!
//! - **Generation**: 32-bit counter incremented on each allocation, used to detect
//!   use-after-free when a slot is reused.
//! - **Epoch**: 16-bit value from global epoch counter, prevents ABA problem when
//!   generation wraps around.
//! - **Capabilities**: 16-bit flags for read/write/delegate permissions.
//!
//! # Object Layout
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                        OBJECT LAYOUT (24 bytes header)          │
//! ├─────────────────────────────────────────────────────────────────┤
//! │  ┌──────────────────────────────────────────────────────────┐  │
//! │  │                   ObjectHeader (24 bytes)                 │  │
//! │  │  type_id: TypeId (4)                                      │  │
//! │  │  generation: u32 (4)                                      │  │
//! │  │  flags: ObjectFlags (2)                                   │  │
//! │  │  refcount: u16 (2)                                        │  │
//! │  │  size: u32 (4)                                            │  │
//! │  │  epoch: u16 (2) + capabilities: u16 (2) + _pad: u32 (4)   │  │
//! │  └──────────────────────────────────────────────────────────┘  │
//! │  ┌──────────────────────────────────────────────────────────┐  │
//! │  │                      Object Data                          │  │
//! │  │  (type-specific fields, arrays, etc.)                    │  │
//! │  └──────────────────────────────────────────────────────────┘  │
//! └─────────────────────────────────────────────────────────────────┘
//! ```

use std::alloc::{alloc, dealloc, Layout};
use std::ptr::NonNull;

use bitflags::bitflags;

use crate::types::TypeId;
use crate::value::Value;
use super::error::{CbgrViolationKind, InterpreterError, InterpreterResult};

/// Size of object header in bytes.
pub const OBJECT_HEADER_SIZE: usize = 24;

/// Default heap size (16 MB).
pub const DEFAULT_HEAP_SIZE: usize = 16 * 1024 * 1024;

/// Minimum alignment for objects.
pub const MIN_ALIGNMENT: usize = 8;

/// Maximum single allocation size (1 GB).
///
/// Prevents DoS attacks via requesting extremely large allocations
/// (e.g., 2^63 element arrays). Any single allocation request exceeding
/// this limit is rejected with OutOfMemory.
pub const MAX_ALLOCATION_SIZE: usize = 1024 * 1024 * 1024;

bitflags! {
    /// Object flags for runtime state.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct ObjectFlags: u16 {
        /// Object is mutable.
        const MUTABLE = 0b0000_0001;
        /// Object is currently borrowed.
        const BORROWED = 0b0000_0010;
        /// Object is mutably borrowed.
        const BORROWED_MUT = 0b0000_0100;
        /// Object has been marked for GC.
        const MARKED = 0b0000_1000;
        /// Object is pinned (cannot move).
        const PINNED = 0b0001_0000;
        /// Object has been freed (for debug).
        const FREED = 0b0010_0000;
        /// Object contains references (needs tracing).
        const HAS_REFS = 0b0100_0000;
        /// Object has a finalizer.
        const HAS_FINALIZER = 0b1000_0000;
    }
}

/// Object header placed before object data.
///
/// Layout matches CBGR requirements with generation, epoch, and capabilities.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ObjectHeader {
    /// Type ID for this object.
    pub type_id: TypeId,

    /// CBGR generation counter (32-bit).
    /// Incremented on each allocation to detect use-after-free.
    pub generation: u32,

    /// Object flags.
    pub flags: ObjectFlags,

    /// Reference count (for non-CBGR objects).
    pub refcount: u16,

    /// Object size (data portion only).
    pub size: u32,

    /// CBGR epoch (16-bit) - prevents ABA problem on generation wrap.
    pub epoch: u16,

    /// CBGR capabilities (16-bit) - read/write/delegate permissions.
    pub capabilities: u16,

    /// Padding for 8-byte alignment.
    _padding: u32,
}

/// Default capabilities for new objects (READ + WRITE).
const DEFAULT_CAPS: u16 = 0x0003; // READ (0x01) | WRITE (0x02)

impl ObjectHeader {
    /// Creates a new object header with CBGR tracking.
    pub fn new(type_id: TypeId, generation: u32, size: u32) -> Self {
        // Get current epoch from global counter
        let epoch = (verum_common::cbgr::current_epoch() & 0xFFFF) as u16;
        Self {
            type_id,
            generation,
            flags: ObjectFlags::empty(),
            refcount: 1,
            size,
            epoch,
            capabilities: DEFAULT_CAPS,
            _padding: 0,
        }
    }

    /// Creates a new object header with specific epoch.
    pub fn with_epoch(type_id: TypeId, generation: u32, size: u32, epoch: u16) -> Self {
        Self {
            type_id,
            generation,
            flags: ObjectFlags::empty(),
            refcount: 1,
            size,
            epoch,
            capabilities: DEFAULT_CAPS,
            _padding: 0,
        }
    }

    /// Returns true if the object is valid (not freed).
    pub fn is_valid(&self) -> bool {
        !self.flags.contains(ObjectFlags::FREED)
    }

    /// Increments the reference count.
    pub fn incref(&mut self) {
        self.refcount = self.refcount.saturating_add(1);
    }

    /// Decrements the reference count. Returns true if count reaches zero.
    pub fn decref(&mut self) -> bool {
        self.refcount = self.refcount.saturating_sub(1);
        self.refcount == 0
    }

    /// Validates a CBGR reference against this header.
    ///
    /// Returns Ok(()) if valid, or an error describing the violation.
    pub fn validate(&self, expected_gen: u32, expected_epoch: u16) -> InterpreterResult<()> {
        if !self.is_valid() {
            return Err(InterpreterError::CbgrViolation {
                kind: CbgrViolationKind::UseAfterFree,
                ptr: 0,
            });
        }

        if self.generation != expected_gen {
            return Err(InterpreterError::CbgrViolation {
                kind: CbgrViolationKind::GenerationMismatch,
                ptr: 0,
            });
        }

        // Epoch validation with window check
        let epoch_diff = self.epoch.wrapping_sub(expected_epoch);
        if epoch_diff > 0x7FFF && epoch_diff != 0 {
            return Err(InterpreterError::CbgrViolation {
                kind: CbgrViolationKind::EpochExpired,
                ptr: 0,
            });
        }

        Ok(())
    }

    /// Check if the header has a specific capability.
    pub fn has_capability(&self, cap: u16) -> bool {
        (self.capabilities & cap) == cap
    }

    /// Set capabilities.
    pub fn set_capabilities(&mut self, caps: u16) {
        self.capabilities = caps;
    }

    /// Attenuate capabilities (can only remove, not add).
    pub fn attenuate_capabilities(&mut self, mask: u16) {
        self.capabilities &= mask;
    }
}

/// Heap-allocated object (type-erased).
///
/// An `Object` is a pointer to memory that starts with an `ObjectHeader`
/// followed by type-specific data.
#[repr(transparent)]
#[derive(Debug)]
pub struct Object {
    /// Pointer to object header.
    ptr: NonNull<ObjectHeader>,
}

impl Object {
    /// Creates a new object from a raw pointer.
    ///
    /// # Safety
    ///
    /// The pointer must point to valid ObjectHeader followed by data.
    pub unsafe fn from_raw(ptr: *mut ObjectHeader) -> Option<Self> {
        NonNull::new(ptr).map(|ptr| Self { ptr })
    }

    /// Returns a pointer to the header.
    pub fn header(&self) -> &ObjectHeader {
        unsafe { self.ptr.as_ref() }
    }

    /// Returns a mutable pointer to the header.
    pub fn header_mut(&mut self) -> &mut ObjectHeader {
        unsafe { self.ptr.as_mut() }
    }

    /// Returns a pointer to the data portion.
    pub fn data_ptr(&self) -> *mut u8 {
        unsafe { (self.ptr.as_ptr() as *mut u8).add(OBJECT_HEADER_SIZE) }
    }

    /// Returns the raw pointer.
    pub fn as_ptr(&self) -> *mut ObjectHeader {
        self.ptr.as_ptr()
    }

    /// Returns the type ID.
    pub fn type_id(&self) -> TypeId {
        self.header().type_id
    }

    /// Returns the generation counter.
    pub fn generation(&self) -> u32 {
        self.header().generation
    }

    /// Returns the epoch for CBGR validation.
    pub fn epoch(&self) -> u16 {
        self.header().epoch
    }

    /// Returns the capabilities flags.
    pub fn capabilities(&self) -> u16 {
        self.header().capabilities
    }

    /// Returns the reference count.
    pub fn refcount(&self) -> u16 {
        self.header().refcount
    }

    /// Returns the data size.
    pub fn size(&self) -> u32 {
        self.header().size
    }

    /// Returns a safe slice over the data portion of this object.
    ///
    /// This bounds-checks the size field against a maximum to prevent
    /// reading uninitialized memory if the header is corrupted.
    pub fn data_slice(&self) -> &[u8] {
        let size = self.header().size as usize;
        let ptr = self.data_ptr();
        if ptr.is_null() || size == 0 {
            return &[];
        }
        // Sanity: cap at 256MB to prevent corrupted headers from causing huge reads
        let capped = size.min(256 * 1024 * 1024);
        // SAFETY: data_ptr points to OBJECT_HEADER_SIZE bytes after the allocation start.
        // The allocation was made via alloc_with_init which allocates header + size bytes.
        // We cap the size to prevent corrupted metadata from reading beyond the allocation.
        unsafe { std::slice::from_raw_parts(ptr, capped) }
    }

    /// Validates this object against expected generation and epoch.
    pub fn validate(&self, expected_gen: u32, expected_epoch: u16) -> InterpreterResult<()> {
        self.header().validate(expected_gen, expected_epoch)
    }

    /// Check if the object has a specific capability.
    pub fn has_capability(&self, cap: u16) -> bool {
        self.header().has_capability(cap)
    }
}

/// Heap allocator for interpreter objects.
///
/// Uses simple bump allocation for fast allocation.
/// Collection is mark-sweep when threshold is reached.
pub struct Heap {
    /// Current generation counter.
    generation: u32,

    /// All allocated objects (for GC tracing).
    objects: Vec<NonNull<ObjectHeader>>,

    /// Total allocated bytes.
    allocated: usize,

    /// Collection threshold.
    threshold: usize,

    /// Statistics.
    stats: HeapStats,
}

/// Heap statistics including CBGR validation metrics.
#[derive(Debug, Clone, Default)]
pub struct HeapStats {
    /// Total allocations.
    pub total_allocs: u64,

    /// Total frees.
    pub total_frees: u64,

    /// Total bytes allocated.
    pub total_bytes: u64,

    /// Peak memory usage.
    pub peak_bytes: usize,

    /// Number of GC collections.
    pub collections: u64,

    /// CBGR validation checks performed.
    pub cbgr_validations: u64,

    /// CBGR validation failures.
    pub cbgr_failures: u64,

    /// Generation mismatches detected.
    pub generation_mismatches: u64,

    /// Epoch violations detected.
    pub epoch_violations: u64,
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}

impl Heap {
    /// Creates a new heap with default settings.
    pub fn new() -> Self {
        Self::with_threshold(DEFAULT_HEAP_SIZE)
    }

    /// Creates a new heap with the specified collection threshold.
    pub fn with_threshold(threshold: usize) -> Self {
        Self {
            generation: 1,
            objects: Vec::with_capacity(1024),
            allocated: 0,
            threshold,
            stats: HeapStats::default(),
        }
    }

    /// Allocates an object of the given type and size.
    pub fn alloc(&mut self, type_id: TypeId, size: usize) -> InterpreterResult<Object> {
        self.alloc_with_init(type_id, size, |_| {})
    }

    /// Allocates an object with custom initialization.
    pub fn alloc_with_init<F>(
        &mut self,
        type_id: TypeId,
        size: usize,
        init: F,
    ) -> InterpreterResult<Object>
    where
        F: FnOnce(&mut [u8]),
    {
        // Guard against unbounded allocation (DoS prevention)
        if size > MAX_ALLOCATION_SIZE {
            return Err(InterpreterError::OutOfMemory {
                requested: size,
                available: self.threshold.saturating_sub(self.allocated),
            });
        }

        // Guard against u32 truncation: ObjectHeader stores size as u32
        if size > u32::MAX as usize {
            return Err(InterpreterError::OutOfMemory {
                requested: size,
                available: self.threshold.saturating_sub(self.allocated),
            });
        }

        let total_size = OBJECT_HEADER_SIZE + size;
        let layout = Layout::from_size_align(total_size, MIN_ALIGNMENT)
            .map_err(|_| InterpreterError::OutOfMemory {
                requested: total_size,
                available: self.threshold.saturating_sub(self.allocated),
            })?;

        // Check threshold (simple GC trigger)
        if self.allocated + total_size > self.threshold {
            // In a real implementation, trigger GC here
            // For now, just extend threshold
            self.threshold = (self.threshold * 2).max(self.allocated + total_size);
        }

        // Allocate memory
        let ptr = unsafe { alloc(layout) };
        if ptr.is_null() {
            return Err(InterpreterError::OutOfMemory {
                requested: total_size,
                available: 0,
            });
        }

        // Get next generation
        let generation = self.next_generation();

        // Initialize header
        let header_ptr = ptr as *mut ObjectHeader;
        unsafe {
            header_ptr.write(ObjectHeader::new(type_id, generation, size as u32));
        }

        // Initialize data
        let data_ptr = unsafe { ptr.add(OBJECT_HEADER_SIZE) };
        unsafe {
            std::ptr::write_bytes(data_ptr, 0, size);
        }
        init(unsafe { std::slice::from_raw_parts_mut(data_ptr, size) });

        // Track object
        let nn_ptr = NonNull::new(header_ptr).ok_or(InterpreterError::OutOfMemory {
            requested: total_size,
            available: 0,
        })?;
        self.objects.push(nn_ptr);

        // Update stats
        self.allocated += total_size;
        self.stats.total_allocs += 1;
        self.stats.total_bytes += total_size as u64;
        self.stats.peak_bytes = self.stats.peak_bytes.max(self.allocated);

        Ok(Object { ptr: nn_ptr })
    }

    /// Allocates an array of values.
    pub fn alloc_array(&mut self, element_type: TypeId, length: usize) -> InterpreterResult<Object> {
        let size = length * std::mem::size_of::<Value>();
        self.alloc(element_type, size)
    }

    /// Frees an object.
    ///
    /// # Safety
    ///
    /// The object must have been allocated by this heap and must not be
    /// accessed after freeing.
    pub unsafe fn free(&mut self, obj: Object) {
        let header = obj.header();
        let total_size = OBJECT_HEADER_SIZE + header.size as usize;

        // SAFETY: The caller guarantees the object was allocated by this heap
        // with the same alignment.
        unsafe {
            let layout = Layout::from_size_align_unchecked(total_size, MIN_ALIGNMENT);

            // Mark as freed
            (*obj.ptr.as_ptr()).flags |= ObjectFlags::FREED;

            // Deallocate
            dealloc(obj.ptr.as_ptr() as *mut u8, layout);
        }

        // Update stats
        self.allocated = self.allocated.saturating_sub(total_size);
        self.stats.total_frees += 1;

        // Remove from tracking (expensive, but correct)
        self.objects.retain(|p| *p != obj.ptr);
    }

    /// Returns the next generation number.
    ///
    /// When generation reaches GEN_MAX, advances the global epoch and
    /// resets to GEN_INITIAL to prevent generation counter reuse within
    /// the same epoch (ABA prevention).
    pub fn next_generation(&mut self) -> u32 {
        let result = self.generation;
        if self.generation >= verum_common::cbgr::GEN_MAX {
            // SAFETY: Force epoch advance before allowing generation reuse.
            // This invalidates all references from the current epoch, preventing
            // ABA attacks where a new allocation gets the same generation as a
            // freed object.
            let new_epoch = verum_common::cbgr::advance_epoch();

            // Verify epoch actually advanced (protects against epoch counter exhaustion)
            debug_assert!(
                new_epoch > 0,
                "Epoch advance must produce a non-zero epoch"
            );

            self.generation = verum_common::cbgr::GEN_INITIAL;
        } else {
            // SAFETY: checked addition - if wrapping_add would exceed GEN_MAX,
            // the >= check above catches it on the next call.
            self.generation = self.generation.wrapping_add(1);
        }
        result
    }

    /// Returns current statistics.
    pub fn stats(&self) -> &HeapStats {
        &self.stats
    }

    /// Returns total allocated bytes.
    pub fn allocated(&self) -> usize {
        self.allocated
    }

    /// Returns the number of live objects.
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    /// Validates a CBGR reference against an object.
    ///
    /// This performs full CBGR validation including generation and epoch checks.
    /// Stats are updated for monitoring.
    pub fn validate_reference(
        &mut self,
        obj: &Object,
        expected_gen: u32,
        expected_epoch: u16,
    ) -> InterpreterResult<()> {
        self.stats.cbgr_validations += 1;

        match obj.validate(expected_gen, expected_epoch) {
            Ok(()) => Ok(()),
            Err(InterpreterError::CbgrViolation { kind, .. }) => {
                self.stats.cbgr_failures += 1;
                match kind {
                    CbgrViolationKind::GenerationMismatch => {
                        self.stats.generation_mismatches += 1;
                    }
                    CbgrViolationKind::EpochExpired => {
                        self.stats.epoch_violations += 1;
                    }
                    _ => {}
                }
                Err(InterpreterError::CbgrViolation {
                    kind,
                    ptr: obj.as_ptr() as usize,
                })
            }
            Err(e) => Err(e),
        }
    }

    /// Gets the current epoch for new references.
    pub fn current_epoch(&self) -> u16 {
        (verum_common::cbgr::current_epoch() & 0xFFFF) as u16
    }

    /// Clears all objects (for reset).
    ///
    /// # Safety
    ///
    /// All references to heap objects become invalid.
    pub unsafe fn clear(&mut self) {
        for obj_ptr in self.objects.drain(..) {
            // SAFETY: All objects were allocated by this heap with MIN_ALIGNMENT.
            unsafe {
                let header = obj_ptr.as_ref();
                let total_size = OBJECT_HEADER_SIZE + header.size as usize;
                let layout = Layout::from_size_align_unchecked(total_size, MIN_ALIGNMENT);
                dealloc(obj_ptr.as_ptr() as *mut u8, layout);
            }
        }
        self.allocated = 0;
        self.generation = 1;
    }

    /// Gets an Object from a data pointer.
    ///
    /// Given a pointer to the data portion of an object (after the header),
    /// this reconstructs the Object wrapper for CBGR operations.
    ///
    /// # Safety
    ///
    /// The pointer must have been returned by `Object::data_ptr()` for
    /// an object allocated from this heap.
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn get_object(&self, data_ptr: *mut u8) -> Option<Object> {
        if data_ptr.is_null() {
            return None;
        }

        // SAFETY: Validate pointer arithmetic won't underflow.
        // ObjectHeader requires OBJECT_HEADER_SIZE bytes before the data pointer.
        if (data_ptr as usize) < OBJECT_HEADER_SIZE {
            return None;
        }

        // Calculate header pointer by subtracting header size
        let header_ptr = unsafe { data_ptr.sub(OBJECT_HEADER_SIZE) as *mut ObjectHeader };

        // SAFETY: Check alignment to prevent type confusion via misaligned pointers
        if !(header_ptr as usize).is_multiple_of(std::mem::align_of::<ObjectHeader>()) {
            return None;
        }

        // Verify this is a valid heap object by checking if it's in our object list
        // This is O(n) but provides safety; in production could use a hash set
        let header_nonnull = std::ptr::NonNull::new(header_ptr)?;
        if !self.objects.contains(&header_nonnull) {
            return None;
        }

        // SAFETY: Pointer is in our object list, so header is valid to read.
        // Validate type tag to prevent object forgery via pointer reconstruction.
        let header = unsafe { &*header_ptr };

        // Reject freed objects - prevents type confusion via dangling pointers
        if header.flags.contains(ObjectFlags::FREED) {
            return None;
        }

        // Validate generation is in valid range (not unallocated sentinel)
        if header.generation == 0 {
            return None;
        }

        // SAFETY: All validation checks passed - object is genuine and alive
        unsafe { Object::from_raw(header_ptr) }
    }

    /// Creates a TokenStream heap object from serialized bytes.
    ///
    /// This is used by the MetaQuote instruction handler to create TokenStream
    /// objects directly from pre-serialized bytes stored in the constant pool.
    ///
    /// # Arguments
    ///
    /// * `serialized_data` - Pre-serialized TokenStream bytes
    ///
    /// # Returns
    ///
    /// A heap-allocated Object containing the serialized TokenStream data.
    ///
    /// # Performance
    ///
    /// O(n) where n = serialized data size. Just a single memcpy.
    pub fn alloc_token_stream(&mut self, serialized_data: &[u8]) -> InterpreterResult<Object> {
        self.alloc_with_init(crate::types::TypeId::TOKEN_STREAM, serialized_data.len(), |buf| {
            buf.copy_from_slice(serialized_data);
        })
    }
}

impl Drop for Heap {
    fn drop(&mut self) {
        // Free all remaining objects
        unsafe { self.clear() };
    }
}

// ============================================================================
// Specialized Object Types (for future use)
// ============================================================================

/// List object (growable array).
#[repr(C)]
#[allow(dead_code)]
pub struct ListObject {
    /// Current length.
    pub len: usize,
    /// Capacity.
    pub cap: usize,
    /// Pointer to elements (Value array).
    pub elements: *mut Value,
}

/// Map object (hash map).
#[repr(C)]
#[allow(dead_code)]
pub struct MapObject {
    /// Number of entries.
    pub len: usize,
    /// Capacity.
    pub cap: usize,
    /// Pointer to entries.
    pub entries: *mut MapEntry,
}

/// Map entry.
#[repr(C)]
#[allow(dead_code)]
pub struct MapEntry {
    /// Hash of key.
    pub hash: u64,
    /// Key value.
    pub key: Value,
    /// Value.
    pub value: Value,
}

/// Closure object.
#[repr(C)]
#[allow(dead_code)]
pub struct ClosureObject {
    /// Function ID.
    pub function: crate::module::FunctionId,
    /// Number of captured values.
    pub capture_count: u16,
    /// Captured values follow (inline array).
    _captures: [Value; 0],
}

#[allow(dead_code)]
impl ClosureObject {
    /// Returns captured values.
    pub fn captures(&self) -> &[Value] {
        unsafe {
            std::slice::from_raw_parts(
                self._captures.as_ptr(),
                self.capture_count as usize,
            )
        }
    }

    /// Returns mutable captured values.
    pub fn captures_mut(&mut self) -> &mut [Value] {
        unsafe {
            std::slice::from_raw_parts_mut(
                self._captures.as_mut_ptr(),
                self.capture_count as usize,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_object_header_size() {
        assert_eq!(std::mem::size_of::<ObjectHeader>(), OBJECT_HEADER_SIZE);
    }

    #[test]
    fn test_heap_creation() {
        let heap = Heap::new();
        assert_eq!(heap.allocated(), 0);
        assert_eq!(heap.object_count(), 0);
    }

    #[test]
    fn test_alloc() {
        let mut heap = Heap::new();

        let obj = heap.alloc(TypeId::INT, 64).unwrap();
        assert_eq!(obj.type_id(), TypeId::INT);
        assert_eq!(obj.size(), 64);
        assert_eq!(heap.object_count(), 1);
        assert!(heap.allocated() > 0);
    }

    #[test]
    fn test_alloc_with_init() {
        let mut heap = Heap::new();

        let obj = heap.alloc_with_init(TypeId::TEXT, 16, |data| {
            data.copy_from_slice(b"Hello, World!!\0\0");
        }).unwrap();

        let data = unsafe { std::slice::from_raw_parts(obj.data_ptr(), 16) };
        assert_eq!(data, b"Hello, World!!\0\0");
    }

    #[test]
    fn test_generation_increment() {
        let mut heap = Heap::new();

        let obj1 = heap.alloc(TypeId::INT, 8).unwrap();
        let obj2 = heap.alloc(TypeId::INT, 8).unwrap();

        // Generations should be different
        assert_ne!(obj1.generation(), obj2.generation());
    }

    #[test]
    fn test_free() {
        let mut heap = Heap::new();

        let obj = heap.alloc(TypeId::INT, 64).unwrap();
        let initial_alloc = heap.allocated();

        unsafe { heap.free(obj) };

        assert!(heap.allocated() < initial_alloc);
        assert_eq!(heap.object_count(), 0);
    }

    #[test]
    fn test_object_flags() {
        let mut flags = ObjectFlags::empty();
        assert!(!flags.contains(ObjectFlags::MUTABLE));

        flags |= ObjectFlags::MUTABLE;
        assert!(flags.contains(ObjectFlags::MUTABLE));

        flags |= ObjectFlags::BORROWED;
        assert!(flags.contains(ObjectFlags::MUTABLE | ObjectFlags::BORROWED));
    }

    #[test]
    fn test_refcount() {
        let mut header = ObjectHeader::new(TypeId::INT, 0, 8);
        assert_eq!(header.refcount, 1);

        header.incref();
        assert_eq!(header.refcount, 2);

        header.incref();
        assert_eq!(header.refcount, 3);

        assert!(!header.decref());
        assert_eq!(header.refcount, 2);

        assert!(!header.decref());
        assert_eq!(header.refcount, 1);

        assert!(header.decref()); // Reaches zero
        assert_eq!(header.refcount, 0);
    }

    #[test]
    fn test_stats() {
        let mut heap = Heap::new();

        heap.alloc(TypeId::INT, 64).unwrap();
        heap.alloc(TypeId::FLOAT, 128).unwrap();

        let stats = heap.stats();
        assert_eq!(stats.total_allocs, 2);
        assert!(stats.total_bytes > 0);
    }

    #[test]
    fn test_clear() {
        let mut heap = Heap::new();

        for _ in 0..10 {
            heap.alloc(TypeId::INT, 64).unwrap();
        }

        assert_eq!(heap.object_count(), 10);

        unsafe { heap.clear() };

        assert_eq!(heap.object_count(), 0);
        assert_eq!(heap.allocated(), 0);
    }

    #[test]
    fn test_alloc_array() {
        let mut heap = Heap::new();

        let obj = heap.alloc_array(TypeId::INT, 100).unwrap();
        let expected_size = 100 * std::mem::size_of::<Value>();
        assert_eq!(obj.size() as usize, expected_size);
    }
}
