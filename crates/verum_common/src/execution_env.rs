//! Unified Execution Environment (θ+) - Foundation Layer
//!
//! This module provides the foundation-layer `ExecutionEnv` type used across crates.
//!
//! # Thread Safety
//!
//! This foundation layer is Send + Sync.
//!
//! # Performance Characteristics
//!
//! - Creation: <100ns
//! - Fork: <50ns (shallow clone)
//! - Thread-local access: <10ns
//!
//! # Example
//!
//! ```rust
//! use verum_common::ExecutionEnv;
//!
//! // Create environment
//! let env = ExecutionEnv::new();
//!
//! // Fork for child tasks
//! let child_env = env.fork();
//!
//! // Thread-local access (requires "std" feature)
//! #[cfg(feature = "std")]
//! {
//!     ExecutionEnv::set_current(env.clone());
//!     let current = ExecutionEnv::current();
//!     ExecutionEnv::clear_current();
//! }
//! ```

/// Foundation-layer execution context management.
///
/// Thread-safe (Send + Sync), with fork semantics for child tasks.
#[derive(Clone, Debug, Default)]
pub struct ExecutionEnv {
    /// Environment generation for tracking forks
    ///
    /// Incremented on each fork() call. Root environments have generation 0.
    /// Used by the runtime to track environment lineage and for debugging.
    generation: u32,
}

impl ExecutionEnv {
    /// Create a new execution environment
    ///
    /// Creates a root environment with generation 0.
    ///
    /// # Performance
    /// - ~50ns (stack allocation only)
    ///
    /// # Example
    /// ```rust
    /// use verum_common::ExecutionEnv;
    ///
    /// let env = ExecutionEnv::new();
    /// assert_eq!(env.generation(), 0);
    /// ```
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Fork the execution environment for a child task
    ///
    /// Creates a new environment with incremented generation. In the foundation
    /// layer, this performs a shallow clone with generation tracking. The full
    /// implementation in `verum_runtime` extends this with:
    /// - Memory context isolation
    /// - Capability inheritance
    /// - Supervision tree linkage
    ///
    /// # Performance
    /// - ~30ns (generation increment + shallow clone)
    ///
    /// # Example
    /// ```rust
    /// use verum_common::ExecutionEnv;
    ///
    /// let parent = ExecutionEnv::new();
    /// let child = parent.fork();
    /// assert_eq!(child.generation(), 1);
    /// ```
    #[inline]
    pub fn fork(&self) -> Self {
        Self {
            generation: self.generation.saturating_add(1),
        }
    }

    /// Get the environment generation
    ///
    /// Returns the fork depth of this environment:
    /// - 0 for root environments created with `new()`
    /// - 1+ for forked environments
    ///
    /// # Example
    /// ```rust
    /// use verum_common::ExecutionEnv;
    ///
    /// let env = ExecutionEnv::new();
    /// assert_eq!(env.generation(), 0);
    ///
    /// let child = env.fork();
    /// assert_eq!(child.generation(), 1);
    ///
    /// let grandchild = child.fork();
    /// assert_eq!(grandchild.generation(), 2);
    /// ```
    #[inline]
    pub const fn generation(&self) -> u32 {
        self.generation
    }

    /// Check if this is a root environment
    ///
    /// Returns `true` if this environment was created with `new()` (not forked).
    ///
    /// # Example
    /// ```rust
    /// use verum_common::ExecutionEnv;
    ///
    /// let root = ExecutionEnv::new();
    /// assert!(root.is_root());
    ///
    /// let child = root.fork();
    /// assert!(!child.is_root());
    /// ```
    #[inline]
    pub const fn is_root(&self) -> bool {
        self.generation == 0
    }
}

// Thread-local storage (requires std feature)
#[cfg(feature = "std")]
mod thread_local_impl {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        /// Thread-local storage for the current execution environment
        ///
        /// This enables implicit context propagation within a thread.
        /// Each thread maintains its own environment, supporting task-local semantics.
        static CURRENT_ENV: RefCell<Option<ExecutionEnv>> = const { RefCell::new(None) };
    }

    impl ExecutionEnv {
        /// Get the current thread's execution environment
        ///
        /// Returns a clone of the current environment if one has been set via
        /// `set_current()`, or creates a new default environment if none exists.
        ///
        /// # Thread Safety
        ///
        /// This method is safe to call from any thread. Each thread has its own
        /// independent environment storage.
        ///
        /// # Performance
        /// - ~5ns when environment is set (clone + thread-local access)
        /// - ~50ns when creating default (allocation)
        ///
        /// # Example
        /// ```rust
        /// use verum_common::ExecutionEnv;
        ///
        /// // Initially returns a new default environment
        /// let env = ExecutionEnv::current();
        /// assert!(env.is_root());
        ///
        /// // After setting, returns clone of set environment
        /// let child = env.fork();
        /// ExecutionEnv::set_current(child.clone());
        /// let current = ExecutionEnv::current();
        /// assert!(!current.is_root());
        /// ExecutionEnv::clear_current();
        /// ```
        pub fn current() -> Self {
            CURRENT_ENV.with(|env| env.borrow().clone().unwrap_or_else(Self::default))
        }

        /// Check if a current environment has been set
        ///
        /// Returns `true` if `set_current()` has been called and the environment
        /// has not been cleared.
        ///
        /// # Example
        /// ```rust
        /// use verum_common::ExecutionEnv;
        ///
        /// assert!(!ExecutionEnv::has_current());
        ///
        /// let env = ExecutionEnv::new();
        /// ExecutionEnv::set_current(env);
        /// assert!(ExecutionEnv::has_current());
        ///
        /// ExecutionEnv::clear_current();
        /// assert!(!ExecutionEnv::has_current());
        /// ```
        pub fn has_current() -> bool {
            CURRENT_ENV.with(|env| env.borrow().is_some())
        }

        /// Set the current thread's execution environment
        ///
        /// This affects all subsequent calls to `current()` on this thread until
        /// `clear_current()` is called. Previous environment (if any) is dropped.
        ///
        /// # Thread Safety
        ///
        /// This only affects the calling thread. Other threads maintain their
        /// own independent environments.
        ///
        /// # Performance
        /// - ~5ns (thread-local write)
        ///
        /// # Example
        /// ```rust
        /// use verum_common::ExecutionEnv;
        ///
        /// let env = ExecutionEnv::new();
        /// let child = env.fork();
        ///
        /// ExecutionEnv::set_current(child);
        /// assert_eq!(ExecutionEnv::current().generation(), 1);
        ///
        /// ExecutionEnv::clear_current();
        /// ```
        pub fn set_current(env: Self) {
            CURRENT_ENV.with(|current| {
                *current.borrow_mut() = Some(env);
            });
        }

        /// Clear the current thread's execution environment
        ///
        /// After calling this, `current()` will return a new default environment.
        /// The previous environment is dropped.
        ///
        /// # Performance
        /// - ~5ns (thread-local write + Option drop)
        ///
        /// # Example
        /// ```rust
        /// use verum_common::ExecutionEnv;
        ///
        /// let env = ExecutionEnv::new().fork();
        /// ExecutionEnv::set_current(env);
        /// assert_eq!(ExecutionEnv::current().generation(), 1);
        ///
        /// ExecutionEnv::clear_current();
        /// assert_eq!(ExecutionEnv::current().generation(), 0);
        /// ```
        pub fn clear_current() {
            CURRENT_ENV.with(|current| {
                *current.borrow_mut() = None;
            });
        }

        /// Execute a closure with a temporary execution environment
        ///
        /// Sets the environment for the duration of the closure, then restores
        /// the previous environment (if any).
        ///
        /// # Example
        /// ```rust
        /// use verum_common::ExecutionEnv;
        ///
        /// let env = ExecutionEnv::new().fork();
        ///
        /// let result = ExecutionEnv::with_env(env, || {
        ///     let current = ExecutionEnv::current();
        ///     assert_eq!(current.generation(), 1);
        ///     42
        /// });
        ///
        /// assert_eq!(result, 42);
        /// assert!(!ExecutionEnv::has_current());
        /// ```
        pub fn with_env<F, R>(env: Self, f: F) -> R
        where
            F: FnOnce() -> R,
        {
            let previous = CURRENT_ENV.with(|current| current.borrow_mut().replace(env));
            let result = f();
            CURRENT_ENV.with(|current| {
                *current.borrow_mut() = previous;
            });
            result
        }
    }
}

// Explicit Send + Sync implementation
// SAFETY: ExecutionEnv contains only a u32 generation counter,
// which is trivially Send and Sync.
unsafe impl Send for ExecutionEnv {}
unsafe impl Sync for ExecutionEnv {}

impl PartialEq for ExecutionEnv {
    fn eq(&self, other: &Self) -> bool {
        self.generation == other.generation
    }
}

impl Eq for ExecutionEnv {}
