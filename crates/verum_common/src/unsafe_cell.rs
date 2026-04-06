//! UnsafeCell - The core primitive for interior mutability
//!
//! Interior Mutability: UnsafeCell<T> opts out of the shared reference immutability
//! guarantee — &UnsafeCell<T> may point to data being mutated. This is the ONLY legal
//! way to obtain aliasable mutable data. It has zero runtime overhead and same layout as T.
//! UnsafeCell is NOT Sync (cannot be safely shared across threads). It is the foundation
//! for Cell<T> and Mutable<T>.
//!
//! UnsafeCell<T> is the fundamental building block for all interior mutability
//! patterns in Verum. It is a compiler primitive that opts out of the shared
//! reference immutability guarantee.
//!
//! # Key Properties
//! - Zero runtime overhead
//! - NOT Sync (cannot be shared across threads safely)
//! - Provides raw pointer access to inner value
//! - Foundation for Cell<T> and Mutable<T>
//!
//! # Safety
//! UnsafeCell provides no synchronization or runtime checks. Users must ensure:
//! - No data races occur
//! - Aliasing rules are manually upheld
//! - Access is properly synchronized if used across threads

use std::cell::UnsafeCell as StdUnsafeCell;

/// The core primitive for interior mutability.
///
/// `UnsafeCell<T>` opts-out of the immutability guarantee for `&T`: a shared
/// reference `&UnsafeCell<T>` may point to data that is being mutated. This is
/// the only legal way to obtain aliasable data that is considered mutable.
///
/// # Memory Layout
/// ```text
/// UnsafeCell<T> has the same memory layout as T
/// No overhead, #[repr(transparent)]
/// ```
///
/// # Examples
///
/// ```
/// use verum_common::UnsafeCell;
///
/// let cell = UnsafeCell::new(5);
///
/// unsafe {
///     *cell.get() = 10;
///     assert_eq!(*cell.get(), 10);
/// }
/// ```
///
/// UnsafeCell<T> is #[repr(transparent)] over T — same memory layout, zero overhead.
/// Provides raw pointer access to inner value for interior mutability patterns.
#[repr(transparent)]
pub struct UnsafeCell<T: ?Sized> {
    value: StdUnsafeCell<T>,
}

impl<T> UnsafeCell<T> {
    /// Constructs a new instance of `UnsafeCell` which will wrap the specified value.
    ///
    /// # Examples
    ///
    /// ```
    /// use verum_common::UnsafeCell;
    ///
    /// let cell = UnsafeCell::new(42);
    /// ```
    ///
    /// UnsafeCell<T> is #[repr(transparent)] over T — same memory layout, zero overhead.
/// Provides raw pointer access to inner value for interior mutability patterns.
    #[inline]
    pub const fn new(value: T) -> UnsafeCell<T> {
        UnsafeCell {
            value: StdUnsafeCell::new(value),
        }
    }

    /// Unwraps the value.
    ///
    /// # Examples
    ///
    /// ```
    /// use verum_common::UnsafeCell;
    ///
    /// let cell = UnsafeCell::new(42);
    /// let value = cell.into_inner();
    /// assert_eq!(value, 42);
    /// ```
    ///
    /// UnsafeCell<T> is #[repr(transparent)] over T — same memory layout, zero overhead.
/// Provides raw pointer access to inner value for interior mutability patterns.
    #[inline]
    pub fn into_inner(self) -> T {
        self.value.into_inner()
    }
}

impl<T: ?Sized> UnsafeCell<T> {
    /// Gets a mutable pointer to the wrapped value.
    ///
    /// This can be cast to a pointer of any kind. Ensure that the access is
    /// unique (no active references, mutable or not) when calling this method,
    /// and that the access is not used to violate aliasing rules.
    ///
    /// # Safety
    ///
    /// The caller must ensure that:
    /// - No data races occur
    /// - Aliasing rules are manually upheld
    /// - Access is properly synchronized if used across threads
    ///
    /// # Examples
    ///
    /// ```
    /// use verum_common::UnsafeCell;
    ///
    /// let cell = UnsafeCell::new(5);
    ///
    /// unsafe {
    ///     let ptr = cell.get();
    ///     *ptr = 10;
    ///     assert_eq!(*ptr, 10);
    /// }
    /// ```
    ///
    /// UnsafeCell<T> is #[repr(transparent)] over T — same memory layout, zero overhead.
/// Provides raw pointer access to inner value for interior mutability patterns.
    #[inline]
    pub const fn get(&self) -> *mut T {
        self.value.get()
    }

    /// Returns a mutable reference to the underlying data.
    ///
    /// This call borrows the `UnsafeCell` mutably (at compile-time) which
    /// guarantees that we possess the only reference.
    ///
    /// # Examples
    ///
    /// ```
    /// use verum_common::UnsafeCell;
    ///
    /// let mut cell = UnsafeCell::new(5);
    /// *cell.get_mut() = 10;
    /// assert_eq!(cell.into_inner(), 10);
    /// ```
    ///
    /// UnsafeCell<T> is #[repr(transparent)] over T — same memory layout, zero overhead.
/// Provides raw pointer access to inner value for interior mutability patterns.
    #[inline]
    pub fn get_mut(&mut self) -> &mut T {
        self.value.get_mut()
    }

    /// Gets a mutable pointer to the wrapped value.
    ///
    /// This is the same as `get`, but the returned pointer is covariant.
    ///
    /// # Safety
    ///
    /// See [`get`](UnsafeCell::get) for safety requirements.
    ///
    /// UnsafeCell<T> is #[repr(transparent)] over T — same memory layout, zero overhead.
/// Provides raw pointer access to inner value for interior mutability patterns.
    #[inline]
    pub const fn raw_get(this: *const Self) -> *mut T {
        // SAFETY: Forwarding to std::cell::UnsafeCell implementation
        // SAFETY: Raw pointer access to UnsafeCell inner value
        // - Precondition 1: `this` points to a valid UnsafeCell (caller's responsibility)
        // - Precondition 2: Access must not violate aliasing rules (caller's responsibility)
        // - Proof: We cast the const pointer to UnsafeCell to a const pointer to the inner
        //   StdUnsafeCell, then call its get() method which returns *mut T. This is safe
        //   because: (1) UnsafeCell is #[repr(transparent)] over StdUnsafeCell, so the
        //   pointer cast preserves validity and alignment, (2) StdUnsafeCell::get is
        //   specifically designed for this use case and handles the const-to-mut pointer
        //   cast correctly, (3) we're not dereferencing anything ourselves - just forwarding
        //   the pointer access to the standard library implementation which upholds all
        //   necessary invariants. The caller must ensure the returned pointer is used safely.
        // - Note: We use `as` cast instead of `.cast::<>()` to support ?Sized types
        unsafe { (*(this as *const StdUnsafeCell<T>)).get() }
    }
}

// UnsafeCell is Send if T is Send
// SAFETY: UnsafeCell itself doesn't add any thread-safety issues beyond those of T
// Interior mutability rule: UnsafeCell is Send if T is Send, since it doesn't
// add thread-safety concerns beyond those of T itself.
unsafe impl<T: ?Sized + Send> Send for UnsafeCell<T> {}

// UnsafeCell is explicitly NOT Sync, even if T is Sync
// This is the key property that enables interior mutability
// &UnsafeCell<T> can be used to mutate T, so it cannot be safely shared across threads
// Interior mutability rule: UnsafeCell is explicitly NOT Sync. Since &UnsafeCell<T>
// can be used to mutate T, it cannot be safely shared across threads.

impl<T: Default> Default for UnsafeCell<T> {
    /// Creates an `UnsafeCell`, with the `Default` value for T.
    #[inline]
    fn default() -> UnsafeCell<T> {
        UnsafeCell::new(Default::default())
    }
}

impl<T> From<T> for UnsafeCell<T> {
    /// Creates an `UnsafeCell<T>` from the given value.
    #[inline]
    fn from(t: T) -> UnsafeCell<T> {
        UnsafeCell::new(t)
    }
}

// Clone support if T is Clone
impl<T: Clone> Clone for UnsafeCell<T> {
    #[inline]
    fn clone(&self) -> UnsafeCell<T> {
        // SAFETY: We're creating a new UnsafeCell with a cloned value
        // SAFETY: Cloning value from UnsafeCell
        // - Precondition 1: T implements Clone
        // - Precondition 2: self.get() points to valid T (UnsafeCell invariant)
        // - Proof: We obtain a const pointer via get(), dereference it to get &T,
        //   then call clone() to produce a new T value. This is safe because:
        //   (1) UnsafeCell maintains the invariant that its internal pointer is
        //   always valid and properly aligned, (2) we're only reading the value
        //   (not mutating), which is safe even with potential aliasing, (3) the
        //   clone() trait method only requires &T and T: Clone, both satisfied here.
        //   The resulting clone is placed in a new UnsafeCell with independent state.
        UnsafeCell::new(unsafe { (*self.get()).clone() })
    }
}

// PartialEq support if T is PartialEq
impl<T: PartialEq + Copy> PartialEq for UnsafeCell<T> {
    #[inline]
    fn eq(&self, other: &UnsafeCell<T>) -> bool {
        // SAFETY: We're comparing Copy values
        // SAFETY: Reading and comparing Copy values from UnsafeCells
        // - Precondition 1: T implements Copy and PartialEq
        // - Precondition 2: self.get() and other.get() point to valid T instances
        // - Proof: We dereference both pointers to get T values and compare them.
        //   This is safe because: (1) T is Copy, so reading creates a new independent
        //   value rather than moving/borrowing, (2) UnsafeCell guarantees its pointer
        //   is always valid and aligned, (3) we're only reading (not mutating), which
        //   cannot violate memory safety, (4) PartialEq for T only requires &T arguments,
        //   and dereferencing a Copy type is always safe. No aliasing issues arise since
        //   we copy the values before comparison.
        unsafe { *self.get() == *other.get() }
    }
}

// Eq support if T is Eq
impl<T: Eq + Copy> Eq for UnsafeCell<T> {}

// Debug support for testing (reads the value, safe for Copy types)
impl<T: Copy + std::fmt::Debug> std::fmt::Debug for UnsafeCell<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // SAFETY: T is Copy, so reading is safe
        // SAFETY: Reading Copy value for Debug formatting
        // - Precondition 1: T implements Copy and Debug
        // - Precondition 2: self.get() returns valid pointer to T
        // - Proof: We dereference the pointer to read the value for formatting.
        //   This is safe because: (1) T is Copy, so reading creates an independent
        //   copy without ownership issues, (2) UnsafeCell guarantees the pointer
        //   is valid and aligned, (3) we're only reading (not mutating), which
        //   cannot violate safety even with potential aliasing from interior
        //   mutability, (4) Debug::fmt only requires &T, satisfied by the Copy.
        f.debug_struct("UnsafeCell")
            .field("value", unsafe { &*self.get() })
            .finish()
    }
}
