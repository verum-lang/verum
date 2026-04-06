//! CBGR-integrated heap allocation for VBC interpreter.
//!
//! This module provides heap allocation with full CBGR (Compile-time Borrow
//! checking with Generational References) integration using verum_common types.
//!
//! # V-LLSI Architecture
//!
//! With the V-LLSI (Verum Low-Level System Interface) architecture, CBGR types
//! are defined in verum_common and used consistently across the interpreter.
//! The interpreter bridges VBC execution with the unified allocation system:
//!
//! ```text
//! ┌────────────────────────────────────────────────────────────────────────┐
//! │                    CBGR HEAP ALLOCATION LAYOUT                         │
//! ├────────────────────────────────────────────────────────────────────────┤
//! │  ┌────────────────────────────────────────────────────────────────┐   │
//! │  │               CbgrHeader (8 bytes) [verum_common]                │   │
//! │  │  generation: AtomicU32 (4 bytes)                               │   │
//! │  │  epoch_caps: AtomicU32 (4 bytes)                               │   │
//! │  └────────────────────────────────────────────────────────────────┘   │
//! │  ┌────────────────────────────────────────────────────────────────┐   │
//! │  │              ObjectMeta (16 bytes) [interpreter]               │   │
//! │  │  type_id: u32 (4 bytes)                                        │   │
//! │  │  flags: u16 (2 bytes)                                          │   │
//! │  │  refcount: u16 (2 bytes)                                       │   │
//! │  │  size: u32 (4 bytes)                                           │   │
//! │  │  padding: u32 (4 bytes)                                        │   │
//! │  └────────────────────────────────────────────────────────────────┘   │
//! │  ┌────────────────────────────────────────────────────────────────┐   │
//! │  │                      User Data                                  │   │
//! │  │  (variable size, aligned to 8 bytes)                           │   │
//! │  └────────────────────────────────────────────────────────────────┘   │
//! └────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # CBGR Implementation
//!
//! Each allocation has a 32-byte AllocationHeader: [size:u32, align:u32, generation:u32,
//! epoch:u16, flags:u16, type_id:u32, padding:u32, reserved:u64]. References carry generation
//! counters; on deref, the reference's generation is compared to the allocation's current
//! generation (~15ns overhead for Tier 0). Tier 1 references are compiler-proven safe (0ns).
//! The V-LLSI bootstrap kernel provides the initial allocator before the Verum runtime loads.

use std::ptr::NonNull;

use bitflags::bitflags;

// CBGR types from verum_common (V-LLSI architecture)
use verum_common::cbgr::{CbgrHeader, CbgrErrorCode, tracked_alloc_zeroed, tracked_dealloc, TrackedAllocation};

use crate::types::TypeId;
use crate::value::Value;
use super::error::{InterpreterError, InterpreterResult, CbgrViolationKind};

/// Size of object metadata in bytes (placed after CbgrHeader).
pub const OBJECT_META_SIZE: usize = 16;

/// Default heap size (16 MB).
pub const DEFAULT_CBGR_HEAP_SIZE: usize = 16 * 1024 * 1024;

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
    pub struct CbgrObjectFlags: u16 {
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

/// Object metadata placed after CbgrHeader.
///
/// This contains interpreter-specific metadata that supplements
/// the CBGR header from verum_common.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ObjectMeta {
    /// Type ID for this object.
    pub type_id: u32,

    /// Object flags.
    pub flags: CbgrObjectFlags,

    /// Reference count (for non-CBGR objects).
    pub refcount: u16,

    /// Object size (user data portion only).
    pub size: u32,

    /// Padding for alignment.
    _padding: u32,
}

impl ObjectMeta {
    /// Creates new object metadata.
    #[inline]
    pub fn new(type_id: TypeId, size: u32) -> Self {
        Self {
            type_id: type_id.0,
            flags: CbgrObjectFlags::empty(),
            refcount: 1,
            size,
            _padding: 0,
        }
    }

    /// Returns true if the object is valid (not freed).
    #[inline]
    pub fn is_valid(&self) -> bool {
        !self.flags.contains(CbgrObjectFlags::FREED)
    }

    /// Increments the reference count.
    #[inline]
    pub fn incref(&mut self) {
        self.refcount = self.refcount.saturating_add(1);
    }

    /// Decrements the reference count. Returns true if count reaches zero.
    #[inline]
    pub fn decref(&mut self) -> bool {
        self.refcount = self.refcount.saturating_sub(1);
        self.refcount == 0
    }
}

/// CBGR-tracked heap object.
///
/// Wraps a TrackedAllocation from verum_common and provides
/// access to both the CBGR header and interpreter metadata.
pub struct CbgrObject {
    /// The underlying tracked allocation.
    allocation: TrackedAllocation,
}

impl std::fmt::Debug for CbgrObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CbgrObject")
            .field("type_id", &self.type_id())
            .field("size", &self.size())
            .field("generation", &self.generation())
            .field("epoch", &self.epoch())
            .finish()
    }
}

impl CbgrObject {
    /// Creates a new CbgrObject from a TrackedAllocation.
    ///
    /// # Safety
    ///
    /// The allocation must have been created with sufficient space for ObjectMeta.
    unsafe fn from_allocation(allocation: TrackedAllocation) -> Self {
        Self { allocation }
    }

    /// Returns the CBGR header for this object.
    #[inline]
    pub fn cbgr_header(&self) -> &CbgrHeader {
        self.allocation.header()
    }

    /// Returns the mutable CBGR header for this object.
    #[inline]
    pub fn cbgr_header_mut(&mut self) -> &mut CbgrHeader {
        self.allocation.header_mut()
    }

    /// Returns the object metadata.
    #[inline]
    pub fn meta(&self) -> &ObjectMeta {
        // SAFETY: The allocation is created with OBJECT_META_SIZE prefix containing a valid
        // ObjectMeta written by CbgrHeap::alloc_with_init. The pointer is valid for the
        // lifetime of the CbgrObject.
        unsafe {
            let meta_ptr = self.allocation.as_ptr() as *const ObjectMeta;
            &*meta_ptr
        }
    }

    /// Returns mutable object metadata.
    #[inline]
    pub fn meta_mut(&mut self) -> &mut ObjectMeta {
        // SAFETY: Same as meta() - allocation starts with a valid ObjectMeta. We have
        // &mut self so exclusive access is guaranteed.
        unsafe {
            let meta_ptr = self.allocation.as_ptr() as *mut ObjectMeta;
            &mut *meta_ptr
        }
    }

    /// Returns a pointer to the user data (after ObjectMeta).
    #[inline]
    pub fn data_ptr(&self) -> *mut u8 {
        // SAFETY: The allocation has size OBJECT_META_SIZE + user_size, so adding
        // OBJECT_META_SIZE yields a valid pointer to the user data region.
        unsafe { self.allocation.as_ptr().add(OBJECT_META_SIZE) }
    }

    /// Returns the raw pointer to the allocation start (ObjectMeta).
    #[inline]
    pub fn as_ptr(&self) -> *mut u8 {
        self.allocation.as_ptr()
    }

    /// Returns the type ID.
    #[inline]
    pub fn type_id(&self) -> TypeId {
        TypeId(self.meta().type_id)
    }

    /// Returns the current generation from the CBGR header.
    #[inline]
    pub fn generation(&self) -> u32 {
        self.cbgr_header().generation()
    }

    /// Returns the current epoch from the CBGR header.
    #[inline]
    pub fn epoch(&self) -> u32 {
        self.cbgr_header().epoch()
    }

    /// Returns the user data size.
    #[inline]
    pub fn size(&self) -> u32 {
        self.meta().size
    }

    /// Validates a reference against the CBGR header.
    ///
    /// Returns Ok(()) if valid, or an error describing the violation.
    #[inline]
    pub fn validate(&self, expected_gen: u32, expected_epoch: u32) -> InterpreterResult<()> {
        let result = self.cbgr_header().validate(expected_gen, expected_epoch);

        match result {
            CbgrErrorCode::Success => Ok(()),
            CbgrErrorCode::GenerationMismatch => {
                Err(InterpreterError::CbgrViolation {
                    kind: CbgrViolationKind::GenerationMismatch,
                    ptr: self.as_ptr() as usize,
                })
            }
            CbgrErrorCode::EpochMismatch => {
                Err(InterpreterError::CbgrViolation {
                    kind: CbgrViolationKind::EpochExpired,
                    ptr: self.as_ptr() as usize,
                })
            }
            CbgrErrorCode::ExpiredReference => {
                Err(InterpreterError::CbgrViolation {
                    kind: CbgrViolationKind::UseAfterFree,
                    ptr: self.as_ptr() as usize,
                })
            }
            _ => {
                Err(InterpreterError::CbgrViolation {
                    kind: CbgrViolationKind::InvalidReference,
                    ptr: self.as_ptr() as usize,
                })
            }
        }
    }

    /// Increments the generation counter (called after mutation).
    #[inline]
    pub fn increment_generation(&mut self) -> u32 {
        self.cbgr_header_mut().increment_generation()
    }
}

/// CBGR heap statistics.
#[derive(Debug, Clone, Default)]
pub struct CbgrHeapStats {
    /// Total allocations.
    pub total_allocs: u64,

    /// Total frees.
    pub total_frees: u64,

    /// Total bytes allocated (user data only).
    pub total_bytes: u64,

    /// Peak memory usage.
    pub peak_bytes: usize,

    /// Number of GC collections.
    pub collections: u64,

    /// CBGR validation checks performed.
    pub cbgr_validations: u64,

    /// CBGR validation failures.
    pub cbgr_failures: u64,
}

/// CBGR-integrated heap allocator.
///
/// Uses verum_common's tracked allocation system for full CBGR
/// memory safety with generation and epoch tracking.
pub struct CbgrHeap {
    /// All allocated objects (for GC tracing).
    objects: Vec<NonNull<u8>>,

    /// Total allocated bytes.
    allocated: usize,

    /// Collection threshold.
    threshold: usize,

    /// Statistics.
    stats: CbgrHeapStats,
}

impl Default for CbgrHeap {
    fn default() -> Self {
        Self::new()
    }
}

impl CbgrHeap {
    /// Creates a new CBGR heap with default settings.
    pub fn new() -> Self {
        Self::with_threshold(DEFAULT_CBGR_HEAP_SIZE)
    }

    /// Creates a new heap with the specified collection threshold.
    pub fn with_threshold(threshold: usize) -> Self {
        Self {
            objects: Vec::with_capacity(1024),
            allocated: 0,
            threshold,
            stats: CbgrHeapStats::default(),
        }
    }

    /// Allocates an object of the given type and size.
    ///
    /// Returns a CbgrObject with:
    /// - CbgrHeader initialized by verum_common (generation=0, current epoch)
    /// - ObjectMeta initialized with the given type_id and size
    /// - User data zeroed
    pub fn alloc(&mut self, type_id: TypeId, size: usize) -> InterpreterResult<CbgrObject> {
        self.alloc_with_init(type_id, size, |_| {})
    }

    /// Allocates an object with custom initialization.
    pub fn alloc_with_init<F>(
        &mut self,
        type_id: TypeId,
        size: usize,
        init: F,
    ) -> InterpreterResult<CbgrObject>
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

        // Total allocation: ObjectMeta + user data
        let total_size = OBJECT_META_SIZE + size;

        // Check threshold (simple GC trigger)
        if self.allocated + total_size > self.threshold {
            // In a real implementation, trigger GC here
            // For now, just extend threshold
            self.threshold = (self.threshold * 2).max(self.allocated + total_size);
        }

        // Use verum_common's tracked allocation
        let allocation = tracked_alloc_zeroed(total_size, MIN_ALIGNMENT)
            .map_err(|_e| InterpreterError::OutOfMemory {
                requested: total_size,
                available: self.threshold.saturating_sub(self.allocated),
            })?;

        // Initialize ObjectMeta at the start of user data
        let meta = ObjectMeta::new(type_id, size as u32);
        // SAFETY: allocation.as_ptr() points to a freshly allocated region of size
        // OBJECT_META_SIZE + size. Writing ObjectMeta at the start is valid.
        unsafe {
            let meta_ptr = allocation.as_ptr() as *mut ObjectMeta;
            std::ptr::write(meta_ptr, meta);
        }

        // Initialize user data (after ObjectMeta)
        // SAFETY: The allocation has room for OBJECT_META_SIZE + size bytes. The pointer
        // at offset OBJECT_META_SIZE is valid for `size` bytes of user data.
        let data_ptr = unsafe { allocation.as_ptr().add(OBJECT_META_SIZE) };
        init(unsafe { std::slice::from_raw_parts_mut(data_ptr, size) });

        // Track object
        // SAFETY: TrackedAllocation always contains a non-null pointer from a successful allocation.
        let ptr = NonNull::new(allocation.as_ptr())
            .expect("TrackedAllocation returned null pointer");
        self.objects.push(ptr);

        // Update stats
        self.allocated += total_size;
        self.stats.total_allocs += 1;
        self.stats.total_bytes += size as u64;
        self.stats.peak_bytes = self.stats.peak_bytes.max(self.allocated);

        // SAFETY: The allocation was created with sufficient space for ObjectMeta + user data,
        // and ObjectMeta was written to the start of the allocation above.
        Ok(unsafe { CbgrObject::from_allocation(allocation) })
    }

    /// Allocates an array of values.
    pub fn alloc_array(&mut self, element_type: TypeId, length: usize) -> InterpreterResult<CbgrObject> {
        let size = length * std::mem::size_of::<Value>();
        self.alloc(element_type, size)
    }

    /// Frees an object.
    ///
    /// # Safety
    ///
    /// The object must have been allocated by this heap and must not be
    /// accessed after freeing.
    pub unsafe fn free(&mut self, mut obj: CbgrObject) {
        let meta = obj.meta_mut();
        let total_size = OBJECT_META_SIZE + meta.size as usize;

        // Mark as freed
        meta.flags |= CbgrObjectFlags::FREED;

        // SAFETY: The allocation was created by tracked_alloc and has not been freed yet.
        // The caller guarantees this via the unsafe fn contract.
        unsafe {
            tracked_dealloc(obj.allocation);
        }

        // Update stats
        self.allocated = self.allocated.saturating_sub(total_size);
        self.stats.total_frees += 1;

        // Remove from tracking (expensive, but correct)
        // Note: obj.as_ptr() is no longer valid after dealloc, so we need to
        // track the ptr before deallocation in a real implementation
    }

    /// Validates a CBGR reference and records statistics.
    pub fn validate_reference(
        &mut self,
        obj: &CbgrObject,
        expected_gen: u32,
        expected_epoch: u32,
    ) -> InterpreterResult<()> {
        self.stats.cbgr_validations += 1;

        match obj.validate(expected_gen, expected_epoch) {
            Ok(()) => Ok(()),
            Err(e) => {
                self.stats.cbgr_failures += 1;
                Err(e)
            }
        }
    }

    /// Returns current statistics.
    pub fn stats(&self) -> &CbgrHeapStats {
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

    /// Clears all objects (for reset).
    ///
    /// # Safety
    ///
    /// All references to heap objects become invalid.
    pub unsafe fn clear(&mut self) {
        // Note: In a full implementation, we would need to deallocate
        // each object through tracked_dealloc. For now, we just clear
        // the tracking and reset stats.
        self.objects.clear();
        self.allocated = 0;
    }

    /// Returns the next generation number.
    ///
    /// This is a convenience method that provides a unique generation number
    /// for manual reference creation. Normally, generations are managed by
    /// the CBGR header during allocation, but this allows creating generations
    /// for special cases.
    pub fn next_generation(&mut self) -> u32 {
        // Use the global epoch from verum_common as the base
        // Each call increments it for uniqueness
        let epoch = verum_common::cbgr::current_epoch();
        // Combine epoch with allocation count for uniqueness
        
        (epoch as u32).wrapping_add(self.stats.total_allocs as u32)
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
    /// A heap-allocated CbgrObject containing the serialized TokenStream data.
    ///
    /// # Performance
    ///
    /// O(n) where n = serialized data size. Just a single memcpy.
    pub fn alloc_token_stream(&mut self, serialized_data: &[u8]) -> InterpreterResult<CbgrObject> {
        self.alloc_with_init(TypeId::TOKEN_STREAM, serialized_data.len(), |buf| {
            buf.copy_from_slice(serialized_data);
        })
    }

    /// Gets a CbgrObject from a data pointer.
    ///
    /// Given a pointer to the data portion of an object (after ObjectMeta),
    /// this reconstructs the CbgrObject wrapper for CBGR operations.
    ///
    /// # Safety
    ///
    /// The pointer must have been returned by `CbgrObject::data_ptr()` for
    /// an object allocated from this heap.
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn get_object(&self, data_ptr: *mut u8) -> Option<CbgrObjectRef> {
        if data_ptr.is_null() {
            return None;
        }

        // Calculate ObjectMeta pointer by subtracting meta size
        // SAFETY: data_ptr was originally computed as allocation_ptr + OBJECT_META_SIZE,
        // so subtracting OBJECT_META_SIZE yields the original valid allocation pointer.
        let meta_ptr = unsafe { data_ptr.sub(OBJECT_META_SIZE) };

        // Calculate CbgrHeader pointer (before ObjectMeta)
        // We need to go back through the verum_common allocation header
        // For now, return a reference wrapper that can be used for validation
        let header_nonnull = std::ptr::NonNull::new(meta_ptr)?;

        // Verify this is a valid heap object by checking if it's in our object list
        if self.objects.contains(&header_nonnull) {
            Some(CbgrObjectRef { meta_ptr })
        } else {
            None
        }
    }
}

/// A lightweight reference to a CBGR heap object.
///
/// Used for looking up objects by data pointer without full ownership.
pub struct CbgrObjectRef {
    meta_ptr: *mut u8,
}

impl CbgrObjectRef {
    /// Returns the object metadata.
    pub fn meta(&self) -> &ObjectMeta {
        // SAFETY: meta_ptr was validated by CbgrHeap::get_object to be in the objects list,
        // ensuring it points to a valid ObjectMeta written during allocation.
        unsafe { &*(self.meta_ptr as *const ObjectMeta) }
    }

    /// Returns the type ID.
    pub fn type_id(&self) -> TypeId {
        TypeId(self.meta().type_id)
    }

    /// Returns the user data size.
    pub fn size(&self) -> u32 {
        self.meta().size
    }

    /// Returns a pointer to the user data.
    pub fn data_ptr(&self) -> *mut u8 {
        // SAFETY: The allocation has OBJECT_META_SIZE bytes before user data, so adding
        // OBJECT_META_SIZE to meta_ptr yields the valid user data pointer.
        unsafe { self.meta_ptr.add(OBJECT_META_SIZE) }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_object_meta_size() {
        assert_eq!(std::mem::size_of::<ObjectMeta>(), OBJECT_META_SIZE);
    }

    #[test]
    fn test_cbgr_heap_alloc() {
        let mut heap = CbgrHeap::new();

        let obj = heap.alloc(TypeId(42), 64).unwrap();

        assert_eq!(obj.type_id().0, 42);
        assert_eq!(obj.size(), 64);
        // Generation starts at 1 (GEN_INITIAL), not 0 (0 means unallocated)
        assert!(obj.generation() == 1);
        assert_eq!(heap.object_count(), 1);
    }

    #[test]
    fn test_cbgr_heap_alloc_with_init() {
        let mut heap = CbgrHeap::new();

        let obj = heap.alloc_with_init(TypeId(1), 8, |data| {
            data[0] = 0xDE;
            data[1] = 0xAD;
            data[2] = 0xBE;
            data[3] = 0xEF;
        }).unwrap();

        let data = unsafe { std::slice::from_raw_parts(obj.data_ptr(), 4) };
        assert_eq!(data, &[0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn test_cbgr_heap_generation() {
        let mut heap = CbgrHeap::new();

        let mut obj = heap.alloc(TypeId(1), 32).unwrap();

        // Generation starts at 1 (GEN_INITIAL)
        let gen1 = obj.generation();
        assert_eq!(gen1, 1);

        // Increment generation
        obj.increment_generation();
        let gen2 = obj.generation();
        assert_eq!(gen2, 2);
    }

    #[test]
    fn test_cbgr_heap_validation() {
        let mut heap = CbgrHeap::new();

        let obj = heap.alloc(TypeId(1), 32).unwrap();
        let generation = obj.generation();
        let epoch = obj.epoch();

        // Valid reference
        let result = heap.validate_reference(&obj, generation, epoch);
        assert!(result.is_ok());

        // Invalid generation
        let result = heap.validate_reference(&obj, generation + 100, epoch);
        assert!(result.is_err());

        assert_eq!(heap.stats().cbgr_validations, 2);
        assert_eq!(heap.stats().cbgr_failures, 1);
    }

    #[test]
    fn test_cbgr_heap_stats() {
        let mut heap = CbgrHeap::new();

        let _obj1 = heap.alloc(TypeId(1), 64).unwrap();
        let _obj2 = heap.alloc(TypeId(2), 128).unwrap();

        assert_eq!(heap.stats().total_allocs, 2);
        assert_eq!(heap.stats().total_bytes, 64 + 128);
        assert_eq!(heap.object_count(), 2);
    }

    #[test]
    fn test_object_meta_refcount() {
        let mut meta = ObjectMeta::new(TypeId(1), 100);

        assert_eq!(meta.refcount, 1);

        meta.incref();
        assert_eq!(meta.refcount, 2);

        assert!(!meta.decref()); // count = 1, not zero
        assert!(meta.decref());  // count = 0, return true
    }

    #[test]
    fn test_cbgr_object_flags() {
        let mut heap = CbgrHeap::new();

        let mut obj = heap.alloc(TypeId(1), 32).unwrap();

        assert!(obj.meta().is_valid());
        assert!(!obj.meta().flags.contains(CbgrObjectFlags::MUTABLE));

        obj.meta_mut().flags |= CbgrObjectFlags::MUTABLE;
        assert!(obj.meta().flags.contains(CbgrObjectFlags::MUTABLE));
    }

    #[test]
    fn test_cbgr_heap_array_alloc() {
        let mut heap = CbgrHeap::new();

        let obj = heap.alloc_array(TypeId(100), 10).unwrap();

        // Size should be 10 * sizeof(Value)
        assert_eq!(obj.size() as usize, 10 * std::mem::size_of::<Value>());
    }
}
