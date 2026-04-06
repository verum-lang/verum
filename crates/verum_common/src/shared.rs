//! Thread-safe reference-counted smart pointer with weak reference support
//!
//! Shared<T> implements thread-safe atomic reference counting with CBGR generation
//! tracking. SharedInner<T> stores {strong_count: AtomicUsize, weak_count: AtomicUsize, data: T}.
//! Shared<T> size = 8 bytes (pointer). SharedInner overhead = 24 bytes (with CBGR tracking) + sizeof(T).
//!
//! This module provides `Shared<T>` (atomic reference counting) and `Weak<T>`
//! (weak references) with full CBGR integration for generation tracking.
//!
//! # Performance Targets (from spec)
//! - Shared<T> creation: <50ns overhead vs raw allocation
//! - downgrade: <20ns
//! - upgrade: <50ns (successful), <10ns (failed)
//! - Memory layout: 16 bytes overhead (2 atomic counters)
//!
//! # Memory Ordering
//! - Strong increment: Relaxed ordering
//! - Strong decrement: Release + Acquire fence on zero
//! - Weak increment: Relaxed ordering
//! - Weak decrement: Release + Acquire fence on zero
//! - Upgrade check: Acquire ordering

use std::fmt;
use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::ops::Deref;
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicU16, AtomicU32, AtomicUsize, Ordering, fence};

// ============================================================================
// SharedInner<T> - Internal allocation structure
// ============================================================================

/// Internal structure for Shared<T> allocations
///
/// Memory layout (SharedInner<T> = {strong_count, weak_count, generation, epoch, data}):
/// ```text
/// ┌───────────────────┐
/// │ strong_count: AtomicUsize │ (8 bytes)
/// │ weak_count: AtomicUsize   │ (8 bytes)
/// │ generation: AtomicU32     │ (4 bytes) - CBGR integration
/// │ epoch: AtomicU16          │ (2 bytes) - CBGR integration
/// │ _padding: u16              │ (2 bytes) - alignment
/// │ data: ManuallyDrop<T>      │ (sizeof(T) bytes)
/// └───────────────────┘
/// Total overhead: 24 bytes (with CBGR tracking)
/// ```
#[repr(C)]
struct SharedInner<T: ?Sized> {
    /// Number of Shared<T> instances pointing to this allocation
    strong_count: AtomicUsize,

    /// Number of Weak<T> instances pointing to this allocation
    weak_count: AtomicUsize,

    /// CBGR generation counter (incremented on deallocation)
    /// Enables integration with ExecutionEnv's CBGR tracking
    generation: AtomicU32,

    /// CBGR epoch counter (for wraparound protection)
    epoch: AtomicU16,

    /// Padding for alignment
    _padding: u16,

    /// The actual data (ManuallyDrop to prevent double-drop when weak refs exist)
    /// SAFETY: We manually drop this in Shared::drop when strong_count reaches 0
    data: ManuallyDrop<T>,
}

impl<T> SharedInner<T> {
    /// Create new SharedInner with initial counts
    fn new(data: T) -> Self {
        SharedInner {
            strong_count: AtomicUsize::new(1),
            weak_count: AtomicUsize::new(0),
            generation: AtomicU32::new(1),
            epoch: AtomicU16::new(0),
            _padding: 0,
            data: ManuallyDrop::new(data),
        }
    }
}

impl<T: ?Sized> SharedInner<T> {
    /// Get current generation (for CBGR integration)
    #[inline]
    fn generation(&self) -> u32 {
        self.generation.load(Ordering::Acquire)
    }

    /// Get current epoch (for CBGR integration)
    #[inline]
    fn epoch(&self) -> u16 {
        self.epoch.load(Ordering::Acquire)
    }
}

// ============================================================================
// Shared<T> - Thread-safe reference-counted pointer
// ============================================================================

/// A thread-safe reference-counted smart pointer.
///
/// `Shared<T>` provides shared ownership of a value of type `T`, allocated on
/// the heap. Invoking `clone` on `Shared` produces a new `Shared` instance that
/// points to the same allocation, while increasing a reference count. When the
/// last `Shared` pointer to a given allocation is dropped, the value stored in
/// that allocation is also dropped.
///
/// # Thread Safety
///
/// Unlike `Counted<T>` (single-threaded), `Shared<T>` is fully thread-safe and
/// can be shared across threads. The reference count is updated atomically.
///
/// # Weak References
///
/// Weak references (`Weak<T>`) can be created from `Shared<T>` using `downgrade()`.
/// Weak references don't prevent the value from being dropped, but can be upgraded
/// to strong references if the value still exists.
///
/// # Examples
///
/// ```
/// use verum_common::shared::Shared;
///
/// let shared = Shared::new(42);
/// let shared2 = shared.clone();
/// assert_eq!(*shared, 42);
/// assert_eq!(Shared::strong_count(&shared), 2);
/// ```
///
/// Thread-safe atomic reference-counted pointer with CBGR generation tracking.
/// new() allocates SharedInner with strong=1, weak=0. clone() does Relaxed fetch_add
/// on strong. drop() does Release fetch_sub, Acquire fence when reaching 0, drops data,
/// then deallocates if weak=0. get_mut() returns &mut T only if strong=1 and weak=0.
/// try_unwrap() returns T only if strong=1 and weak=0. downgrade() does Relaxed
/// fetch_add on weak. upgrade() uses CAS loop with Acquire on strong_count, returns
/// None if count=0.
pub struct Shared<T: ?Sized> {
    /// Non-null pointer to SharedInner<T>
    ptr: NonNull<SharedInner<T>>,

    /// Phantom data to ensure proper variance and drop check
    _phantom: PhantomData<T>,
}

// SAFETY: Shared<T> is Send if T is Send + Sync
unsafe impl<T: Send + Sync + ?Sized> Send for Shared<T> {}

// SAFETY: Shared<T> is Sync if T is Send + Sync
unsafe impl<T: Send + Sync + ?Sized> Sync for Shared<T> {}

impl<T> Shared<T> {
    /// Constructs a new `Shared<T>`.
    ///
    /// # Performance
    /// - Target: <50ns overhead vs raw allocation
    /// - Actual: Heap allocation + atomic initialization
    ///
    /// # Examples
    ///
    /// ```
    /// use verum_common::shared::Shared;
    ///
    /// let shared = Shared::new(42);
    /// assert_eq!(*shared, 42);
    /// ```
    ///
    /// API contract: Allocates SharedInner with strong=1, weak=0.
    #[inline]
    pub fn new(data: T) -> Self {
        let inner = Box::new(SharedInner::new(data));
        let ptr =
            NonNull::new(Box::into_raw(inner)).expect("Box::into_raw should never return null");

        Shared {
            ptr,
            _phantom: PhantomData,
        }
    }

    /// Gets the number of strong (`Shared`) pointers to this allocation.
    ///
    /// # Performance
    /// - ~5ns (atomic load with Acquire ordering)
    ///
    /// # Examples
    ///
    /// ```
    /// use verum_common::shared::Shared;
    ///
    /// let shared = Shared::new(5);
    /// let shared2 = shared.clone();
    /// assert_eq!(Shared::strong_count(&shared), 2);
    /// ```
    ///
    /// API contract: Returns current strong reference count via Acquire load.
    #[inline]
    pub fn strong_count(this: &Self) -> usize {
        // SAFETY: ptr is always valid for the lifetime of Shared
        unsafe { this.ptr.as_ref().strong_count.load(Ordering::Acquire) }
    }

    /// Gets the number of weak (`Weak`) pointers to this allocation.
    ///
    /// # Performance
    /// - ~5ns (atomic load with Acquire ordering)
    ///
    /// # Examples
    ///
    /// ```
    /// use verum_common::shared::Shared;
    ///
    /// let shared = Shared::new(5);
    /// let weak = Shared::downgrade(&shared);
    /// assert_eq!(Shared::weak_count(&shared), 1);
    /// ```
    ///
    /// API contract: Returns current weak reference count via Acquire load.
    #[inline]
    pub fn weak_count(this: &Self) -> usize {
        // SAFETY: ptr is always valid for the lifetime of Shared
        unsafe { this.ptr.as_ref().weak_count.load(Ordering::Acquire) }
    }

    /// Creates a new `Weak` pointer to this allocation.
    ///
    /// # Performance
    /// - Target: <20ns
    /// - Actual: Atomic increment (Relaxed) + pointer copy
    ///
    /// # Examples
    ///
    /// ```
    /// use verum_common::shared::Shared;
    ///
    /// let shared = Shared::new(5);
    /// let weak = Shared::downgrade(&shared);
    /// assert!(weak.upgrade().is_some());
    /// ```
    ///
    /// API contract: Creates Weak by doing Relaxed fetch_add on weak_count.
    #[inline]
    pub fn downgrade(this: &Self) -> Weak<T> {
        // SAFETY: ptr is always valid for the lifetime of Shared
        // Memory Ordering: Relaxed — no synchronization needed for increment,
        // only counting references. The actual data visibility is handled by
        // the Acquire fence in upgrade() and drop().
        unsafe {
            this.ptr.as_ref().weak_count.fetch_add(1, Ordering::Relaxed);
        }

        Weak {
            ptr: this.ptr,
            _phantom: PhantomData,
        }
    }

    /// Returns a mutable reference into the given `Shared`, if there are
    /// no other `Shared` or `Weak` pointers to the same allocation.
    ///
    /// Returns `None` otherwise, because it is not safe to mutate a shared value.
    ///
    /// # Examples
    ///
    /// ```
    /// use verum_common::shared::Shared;
    /// use verum_common::Maybe;
    ///
    /// let mut shared = Shared::new(5);
    ///
    /// match Shared::get_mut(&mut shared) {
    ///     Some(val) => *val = 42,
    ///     None => {},
    /// }
    ///
    /// assert_eq!(*shared, 42);
    /// ```
    ///
    /// API contract: Returns &mut T only if strong=1 and weak=0 (exclusive ownership).
    pub fn get_mut(this: &mut Self) -> Option<&mut T> {
        // SAFETY: ptr is always valid
        unsafe {
            let inner = this.ptr.as_ref();

            // Check if we're the only strong reference AND no weak references exist
            if inner.strong_count.load(Ordering::Acquire) == 1
                && inner.weak_count.load(Ordering::Acquire) == 0
            {
                // SAFETY: We have exclusive ownership
                // ManuallyDrop<T> has same layout as T, so we can get mutable ref through it
                Some(&mut *(*this.ptr.as_ptr()).data)
            } else {
                None
            }
        }
    }

    /// Tries to unwrap the `Shared` into the inner value, if there are no other
    /// `Shared` or `Weak` pointers to the same allocation.
    ///
    /// Returns `Err(self)` if there are other pointers.
    ///
    /// # Examples
    ///
    /// ```
    /// use verum_common::shared::Shared;
    /// use verum_common::Result;
    ///
    /// let shared = Shared::new(5);
    ///
    /// match Shared::try_unwrap(shared) {
    ///     Ok(val) => assert_eq!(val, 5),
    ///     Err(_) => panic!("Other references exist"),
    /// }
    /// ```
    ///
    /// API contract: Returns T only if strong=1 and weak=0 (sole owner). Otherwise returns Err(self).
    pub fn try_unwrap(this: Self) -> crate::Result<T, Self> {
        // SAFETY: ptr is always valid
        unsafe {
            let inner = this.ptr.as_ref();

            // Check if we're the only strong reference AND no weak references exist
            if inner.strong_count.load(Ordering::Acquire) == 1
                && inner.weak_count.load(Ordering::Acquire) == 0
            {
                // SAFETY: We have exclusive ownership, read the data from ManuallyDrop
                // We need to manually take the value out of ManuallyDrop
                let data = ManuallyDrop::into_inner(ptr::read(&inner.data));

                // Get the pointer before we forget `this`
                let ptr = this.ptr.as_ptr();

                // Don't run drop on `this` - we're taking ownership of the data
                std::mem::forget(this);

                // Deallocate the SharedInner
                // SAFETY: The data field is ManuallyDrop, so Box::from_raw won't drop it
                // We already took ownership of the inner value above
                let _ = Box::from_raw(ptr);

                Ok(data)
            } else {
                Err(this)
            }
        }
    }
}

impl<T: ?Sized> Shared<T> {
    /// Get a reference to the inner SharedInner<T>
    #[inline]
    fn inner_ref(&self) -> &SharedInner<T> {
        // SAFETY: ptr is always valid for the lifetime of Shared
        unsafe { self.ptr.as_ref() }
    }

    /// Get current CBGR generation for this allocation.
    ///
    /// This is used for integration with ExecutionEnv's MemoryContext.
    ///
    /// # Performance
    /// - ~5ns (atomic load with Acquire ordering)
    #[inline]
    pub fn generation(&self) -> u32 {
        self.inner_ref().generation()
    }

    /// Get current CBGR epoch for this allocation.
    ///
    /// This is used for integration with ExecutionEnv's MemoryContext.
    ///
    /// # Performance
    /// - ~5ns (atomic load with Acquire ordering)
    #[inline]
    pub fn epoch(&self) -> u16 {
        self.inner_ref().epoch()
    }

    /// Creates a new `Shared` from an `Arc` (for compatibility).
    ///
    /// NOTE: This is a compatibility shim. The Arc data is cloned.
    #[inline]
    pub fn from_arc(arc: std::sync::Arc<T>) -> Self
    where
        T: Clone + Sized,
    {
        Shared::new((*arc).clone())
    }

    /// Converts to an `Arc` (for compatibility).
    ///
    /// NOTE: This is a compatibility shim. Creates a new Arc with cloned data.
    #[inline]
    pub fn into_arc(self) -> std::sync::Arc<T>
    where
        T: Clone + Sized,
    {
        std::sync::Arc::new((*self).clone())
    }

    /// Gets a reference to an `Arc` (for compatibility).
    ///
    /// NOTE: This method creates a temporary Arc with cloned data.
    /// Not efficient - prefer using Shared directly.
    #[inline]
    pub fn as_arc(&self) -> std::sync::Arc<T>
    where
        T: Clone + Sized,
    {
        std::sync::Arc::new((**self).clone())
    }
}

// Note: CBGR integration methods (register_with_context, unregister_from_context)
// could be added here in the future to enable full MemoryContext tracking.
// The generation() and epoch() methods above provide the necessary data for integration.

impl<T: ?Sized> Clone for Shared<T> {
    /// Makes a clone of the `Shared` pointer.
    ///
    /// This creates another pointer to the same allocation, increasing the
    /// strong reference count.
    ///
    /// # Performance
    /// - ~10ns (atomic increment with Relaxed ordering + pointer copy)
    ///
    /// # Memory Ordering
    /// - Relaxed: Multiple threads can increment independently
    /// - No synchronization needed - just counting references
    ///
    /// Memory Ordering: Clone uses Relaxed ordering for the increment — no
    /// synchronization is needed because we're only counting references, not
    /// accessing the shared data. Multiple threads can increment independently.
    #[inline]
    fn clone(&self) -> Self {
        // SAFETY: ptr is always valid
        // Memory Ordering: Relaxed — no synchronization needed for increment.
        unsafe {
            self.ptr
                .as_ref()
                .strong_count
                .fetch_add(1, Ordering::Relaxed);
        }

        Shared {
            ptr: self.ptr,
            _phantom: PhantomData,
        }
    }
}

impl<T: ?Sized> Deref for Shared<T> {
    type Target = T;

    /// Dereferences the value.
    ///
    /// # Performance
    /// - ~1ns (direct pointer dereference, no atomic operations)
    #[inline]
    fn deref(&self) -> &T {
        // SAFETY: ptr is always valid for the lifetime of Shared
        // ManuallyDrop<T> has same layout as T, so we can deref through it
        unsafe { &self.ptr.as_ref().data }
    }
}

impl<T: ?Sized> Drop for Shared<T> {
    /// Drops the `Shared`.
    ///
    /// This will decrement the strong reference count. If the strong reference
    /// count reaches zero, the value will be dropped. If both strong and weak
    /// counts reach zero, the allocation will be freed.
    ///
    /// # Memory Ordering
    /// - Release: Synchronizes with Acquire fence in the last drop
    /// - Ensures all writes to shared data visible before deallocation
    /// - Acquire fence: Synchronizes with all prior Release decrements
    ///
    /// Memory Ordering: Drop uses Release for decrement + Acquire fence before
    /// deallocation. Release ensures all writes to shared data are visible before
    /// deallocation. Acquire fence synchronizes with all prior Release decrements,
    /// ensuring we see all other threads' writes before dropping data.
    fn drop(&mut self) {
        // SAFETY: ptr is always valid until drop completes
        unsafe {
            let inner = self.ptr.as_ref();

            // Decrement strong count with Release ordering
            // SAFETY: We know count is at least 1 (we hold a reference)
            // Memory Ordering: Release — synchronize with last drop's Acquire fence
            if inner.strong_count.fetch_sub(1, Ordering::Release) == 1 {
                // We were the last strong owner

                // Acquire fence: Synchronize with all prior Release decrements
                // SAFETY: Ensures we see all other threads' writes before deallocating
                fence(Ordering::Acquire);

                // ALWAYS drop the data when we're the last strong reference
                // SAFETY: We are the last strong owner, data will not be accessed again
                // We use ManuallyDrop::drop to explicitly drop the data
                ManuallyDrop::drop(&mut (*self.ptr.as_ptr()).data);

                // Check if we should deallocate the SharedInner allocation
                // (only if no weak references remain)
                if inner.weak_count.load(Ordering::Acquire) == 0 {
                    // No weak references, safe to deallocate the entire allocation
                    // SAFETY: We are the last owner and no weak refs remain
                    // The data field is already dropped above, Box::from_raw will only
                    // deallocate the memory (ManuallyDrop prevents double-drop)
                    let _ = Box::from_raw(self.ptr.as_ptr());
                }
                // If weak references exist, they will deallocate the SharedInner
                // when the last weak reference is dropped
            }
        }
    }
}

impl<T: fmt::Debug + ?Sized> fmt::Debug for Shared<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<T: fmt::Display + ?Sized> fmt::Display for Shared<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

impl<T: PartialEq + ?Sized> PartialEq for Shared<T> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl<T: Eq + ?Sized> Eq for Shared<T> {}

// ============================================================================
// Weak<T> - Weak reference that doesn't prevent deallocation
// ============================================================================

/// A weak reference to a `Shared<T>`.
///
/// `Weak` is a version of `Shared` that doesn't keep the value alive.
/// Instead, it can be upgraded to a `Shared` if the value still exists.
///
/// This is useful for breaking reference cycles: if you have a cycle of
/// `Shared` pointers, the values will never be deallocated. By using `Weak`
/// for some of the references, you can break the cycle.
///
/// # Examples
///
/// ```
/// use verum_common::shared::{Shared, Weak};
///
/// let strong = Shared::new(5);
/// let weak = Shared::downgrade(&strong);
///
/// // Value still exists, upgrade succeeds
/// assert!(weak.upgrade().is_some());
///
/// // Drop the strong reference
/// drop(strong);
///
/// // Value is gone, upgrade fails
/// assert!(weak.upgrade().is_none());
/// ```
///
/// Weak reference to a Shared<T> allocation. Does not prevent deallocation of
/// the data, but keeps the SharedInner allocation alive for upgrade attempts.
/// Weak clone uses Relaxed ordering. Weak drop uses Release/Acquire like Shared.
pub struct Weak<T: ?Sized> {
    /// Pointer to SharedInner<T> (may be dangling if all strong refs dropped)
    ptr: NonNull<SharedInner<T>>,

    /// Phantom data to ensure proper variance and drop check
    _phantom: PhantomData<T>,
}

// SAFETY: Weak<T> is Send if T is Send + Sync
unsafe impl<T: Send + Sync + ?Sized> Send for Weak<T> {}

// SAFETY: Weak<T> is Sync if T is Send + Sync
unsafe impl<T: Send + Sync + ?Sized> Sync for Weak<T> {}

impl<T: ?Sized> Weak<T> {
    /// Attempts to upgrade the `Weak` pointer to a `Shared`, delaying
    /// dropping of the inner value if successful.
    ///
    /// Returns `None` if the inner value has since been dropped.
    ///
    /// # Performance
    /// - Target: <50ns (successful), <10ns (failed)
    /// - Actual: Atomic CAS loop (successful) or single load (failed)
    ///
    /// # Memory Ordering
    /// - Acquire: Synchronizes with potential last drop
    /// - Ensures visibility of all writes before the value was dropped
    ///
    /// # Examples
    ///
    /// ```
    /// use verum_common::shared::Shared;
    /// use verum_common::Maybe;
    ///
    /// let strong = Shared::new(5);
    /// let weak = Shared::downgrade(&strong);
    ///
    /// match weak.upgrade() {
    ///     Some(shared) => assert_eq!(*shared, 5),
    ///     None => panic!("Value was dropped"),
    /// }
    /// ```
    ///
    /// API contract: Uses CAS loop with Acquire on strong_count. Returns None if
    /// count=0 (data already dropped). Acquire ordering synchronizes with the Release
    /// store from the last strong reference drop, ensuring visibility of deallocation.
    pub fn upgrade(&self) -> Option<Shared<T>> {
        // SAFETY: ptr may be dangling, but we only access the atomic counters
        // which are valid until the allocation is freed (requires both counts to be 0)
        unsafe {
            let inner = self.ptr.as_ref();

            // Try to increment strong_count (loop until success or count is 0)
            loop {
                // SAFETY: Acquire ordering required to synchronize with Release store
                // from the last strong reference drop. Without Acquire, we might not
                // see the deallocation and attempt to use freed memory.
                let count = inner.strong_count.load(Ordering::Acquire);

                if count == 0 {
                    // No strong references left, can't upgrade
                    // Performance: <10ns (single load + branch)
                    return None;
                }

                // Try to increment (may fail if another thread drops last Shared)
                // Memory Ordering: Acquire — synchronize with potential last drop
                match inner.strong_count.compare_exchange_weak(
                    count,
                    count + 1,
                    Ordering::Acquire, // Success: synchronize with last drop
                    Ordering::Acquire, // Failure: must also use Acquire to see deallocation
                ) {
                    Ok(_) => {
                        // Successfully upgraded!
                        // Performance: <50ns (CAS loop + Shared construction)
                        return Some(Shared {
                            ptr: self.ptr,
                            _phantom: PhantomData,
                        });
                    }
                    Err(_) => {
                        // Another thread modified count, retry
                        continue;
                    }
                }
            }
        }
    }

    /// Gets the number of strong (`Shared`) pointers to this allocation.
    ///
    /// # Performance
    /// - ~5ns (atomic load with Acquire ordering)
    ///
    /// API contract: Returns current strong count via Acquire load.
    #[inline]
    pub fn strong_count(&self) -> usize {
        // SAFETY: ptr may be dangling, but atomic counters are valid
        unsafe { self.ptr.as_ref().strong_count.load(Ordering::Acquire) }
    }

    /// Gets the number of weak (`Weak`) pointers to this allocation.
    ///
    /// # Performance
    /// - ~5ns (atomic load with Acquire ordering)
    ///
    /// API contract: Returns current weak count via Acquire load.
    #[inline]
    pub fn weak_count(&self) -> usize {
        // SAFETY: ptr may be dangling, but atomic counters are valid
        unsafe { self.ptr.as_ref().weak_count.load(Ordering::Acquire) }
    }
}

impl<T: ?Sized> Clone for Weak<T> {
    /// Makes a clone of the `Weak` pointer.
    ///
    /// This creates another weak pointer to the same allocation.
    ///
    /// # Performance
    /// - ~10ns (atomic increment with Relaxed ordering + pointer copy)
    ///
    /// Memory Ordering: Weak clone uses Relaxed ordering — no synchronization
    /// needed for increment, just counting weak references.
    #[inline]
    fn clone(&self) -> Self {
        // SAFETY: ptr may be dangling, but atomic counters are valid
        // Memory Ordering: Relaxed (no synchronization needed for increment)
        unsafe {
            self.ptr.as_ref().weak_count.fetch_add(1, Ordering::Relaxed);
        }

        Weak {
            ptr: self.ptr,
            _phantom: PhantomData,
        }
    }
}

impl<T: ?Sized> Drop for Weak<T> {
    /// Drops the `Weak`.
    ///
    /// This will decrement the weak reference count. If both strong and weak
    /// counts reach zero, the allocation will be freed.
    ///
    /// # Memory Ordering
    /// - Release: Synchronizes with Acquire fence in the last drop
    /// - Acquire fence: Ensures we see all writes before deallocating
    ///
    /// Memory Ordering: Weak drop uses Release for decrement + Acquire fence before
    /// deallocation. Release ensures visibility of writes. Acquire fence ensures we
    /// see all other threads' writes before deallocating the SharedInner allocation.
    fn drop(&mut self) {
        // SAFETY: ptr may be dangling, but atomic counters are valid until
        // both strong_count and weak_count reach 0
        unsafe {
            let inner = self.ptr.as_ref();

            // Decrement weak count with Release ordering
            // Memory Ordering: Release — synchronize with last drop's Acquire fence
            if inner.weak_count.fetch_sub(1, Ordering::Release) == 1 {
                // We were the last weak reference

                // Only deallocate if strong_count is also 0
                if inner.strong_count.load(Ordering::Acquire) == 0 {
                    // Acquire fence: Synchronize with all prior Release decrements
                    // SAFETY: Ensures we see all other threads' writes before deallocating
                    fence(Ordering::Acquire);

                    // Both counts are 0, safe to deallocate
                    // SAFETY: data was already dropped when last Shared was dropped (ManuallyDrop::drop)
                    // The data field is ManuallyDrop<T>, so Box::from_raw will only deallocate memory
                    let _ = Box::from_raw(self.ptr.as_ptr());
                }
            }
        }
    }
}

impl<T: fmt::Debug + ?Sized> fmt::Debug for Weak<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(Weak)")
    }
}
