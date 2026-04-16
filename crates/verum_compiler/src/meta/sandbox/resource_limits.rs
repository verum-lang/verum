//! Resource Limits for Meta Sandbox
//!
//! Manages execution limits (iterations, recursion, memory, timeout)
//! and provides RAII guards for tracking.
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).
//! Meta context unification: all compile-time features desugar to meta-system
//! operations, providing one coherent model with convenient syntax sugar.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

use super::errors::SandboxError;

/// Resource limits configuration
#[derive(Debug, Clone, Copy)]
pub struct ResourceLimits {
    /// Maximum number of iterations (infinite loop protection)
    pub max_iterations: usize,
    /// Maximum recursion depth (stack overflow protection)
    pub max_recursion_depth: usize,
    /// Maximum memory allocation in bytes
    pub max_memory_bytes: usize,
    /// Maximum execution time in milliseconds
    pub timeout_ms: u64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_iterations: 1_000_000,
            max_recursion_depth: 1000, // Overridden by [meta].macro_recursion_limit when wired
            max_memory_bytes: 100 * 1024 * 1024, // 100 MB
            timeout_ms: 30_000,                  // 30 seconds
        }
    }
}

/// Resource limiter for tracking execution state
#[derive(Debug)]
pub struct ResourceLimiter {
    /// Configuration limits
    pub limits: ResourceLimits,

    /// Current execution state (tracked atomically for thread safety)
    current_iterations: AtomicUsize,
    current_recursion: AtomicUsize,
    current_memory: AtomicUsize,

    /// Whether asset loading is enabled for current context
    build_assets_enabled: AtomicBool,
}

impl ResourceLimiter {
    /// Create a new resource limiter with default limits
    pub fn new() -> Self {
        Self::with_limits(ResourceLimits::default())
    }

    /// Create a new resource limiter with custom limits
    pub fn with_limits(limits: ResourceLimits) -> Self {
        Self {
            limits,
            current_iterations: AtomicUsize::new(0),
            current_recursion: AtomicUsize::new(0),
            current_memory: AtomicUsize::new(0),
            build_assets_enabled: AtomicBool::new(false),
        }
    }

    /// Reset execution state counters
    pub fn reset_execution_state(&self) {
        self.current_iterations.store(0, Ordering::SeqCst);
        self.current_recursion.store(0, Ordering::SeqCst);
        self.current_memory.store(0, Ordering::SeqCst);
    }

    /// Check if iteration limit has been exceeded
    pub fn check_iteration_limit(&self) -> Result<(), SandboxError> {
        let current = self.current_iterations.fetch_add(1, Ordering::SeqCst);
        if current >= self.limits.max_iterations {
            Err(SandboxError::IterationLimitExceeded {
                iterations: current,
                limit: self.limits.max_iterations,
            })
        } else {
            Ok(())
        }
    }

    /// Check if recursion limit has been exceeded
    pub fn check_recursion_limit(&self) -> Result<(), SandboxError> {
        let current = self.current_recursion.load(Ordering::SeqCst);
        if current >= self.limits.max_recursion_depth {
            Err(SandboxError::StackOverflow {
                depth: current,
                limit: self.limits.max_recursion_depth,
            })
        } else {
            Ok(())
        }
    }

    /// Check if memory limit has been exceeded
    pub fn check_memory_limit(&self, bytes: usize) -> Result<(), SandboxError> {
        let current = self.current_memory.fetch_add(bytes, Ordering::SeqCst);
        if current >= self.limits.max_memory_bytes {
            Err(SandboxError::MemoryLimitExceeded {
                allocated: current,
                limit: self.limits.max_memory_bytes,
            })
        } else {
            Ok(())
        }
    }

    /// Check if execution timeout has been exceeded
    pub fn check_timeout(&self, start: Instant) -> Result<(), SandboxError> {
        let elapsed = start.elapsed();
        let elapsed_ms = elapsed.as_millis() as u64;
        if elapsed_ms > self.limits.timeout_ms {
            Err(SandboxError::Timeout {
                elapsed_ms,
                limit_ms: self.limits.timeout_ms,
            })
        } else {
            Ok(())
        }
    }

    /// Get current iteration count
    pub fn current_iterations(&self) -> usize {
        self.current_iterations.load(Ordering::SeqCst)
    }

    /// Get current recursion depth
    pub fn current_recursion_depth(&self) -> usize {
        self.current_recursion.load(Ordering::SeqCst)
    }

    /// Get current memory usage
    pub fn current_memory_usage(&self) -> usize {
        self.current_memory.load(Ordering::SeqCst)
    }

    // ========================================================================
    // Asset loading control
    // ========================================================================

    /// Enable asset loading for the current execution context
    pub fn enable_asset_loading(&self) {
        self.build_assets_enabled.store(true, Ordering::SeqCst);
    }

    /// Disable asset loading for the current execution context
    pub fn disable_asset_loading(&self) {
        self.build_assets_enabled.store(false, Ordering::SeqCst);
    }

    /// Check if asset loading is currently allowed
    pub fn is_asset_loading_allowed(&self) -> bool {
        self.build_assets_enabled.load(Ordering::SeqCst)
    }

    /// Execute a function with asset loading enabled
    pub fn with_asset_loading<F, R>(&self, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        let was_allowed = self.build_assets_enabled.swap(true, Ordering::SeqCst);
        let result = f();
        self.build_assets_enabled
            .store(was_allowed, Ordering::SeqCst);
        result
    }

    // ========================================================================
    // Internal state access for guards
    // ========================================================================

    pub(crate) fn increment_recursion(&self) -> usize {
        self.current_recursion.fetch_add(1, Ordering::SeqCst)
    }

    pub(crate) fn decrement_recursion(&self) {
        self.current_recursion.fetch_sub(1, Ordering::SeqCst);
    }

    #[allow(dead_code)]
    pub(crate) fn add_memory(&self, bytes: usize) {
        self.current_memory.fetch_add(bytes, Ordering::SeqCst);
    }

    pub(crate) fn sub_memory(&self, bytes: usize) {
        self.current_memory.fetch_sub(bytes, Ordering::SeqCst);
    }
}

impl Clone for ResourceLimiter {
    fn clone(&self) -> Self {
        Self {
            limits: self.limits,
            current_iterations: AtomicUsize::new(self.current_iterations.load(Ordering::Relaxed)),
            current_recursion: AtomicUsize::new(self.current_recursion.load(Ordering::Relaxed)),
            current_memory: AtomicUsize::new(self.current_memory.load(Ordering::Relaxed)),
            build_assets_enabled: AtomicBool::new(
                self.build_assets_enabled.load(Ordering::Relaxed),
            ),
        }
    }
}

impl Default for ResourceLimiter {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// RAII Guards
// ============================================================================

/// RAII guard for tracking recursion depth
///
/// Automatically increments recursion counter on creation and decrements on drop.
/// This ensures accurate recursion tracking even in the presence of early returns
/// or panics.
pub struct RecursionGuard<'a> {
    limiter: &'a ResourceLimiter,
}

impl<'a> RecursionGuard<'a> {
    /// Create a new recursion guard
    ///
    /// Increments the recursion counter and checks against the limit.
    pub fn new(limiter: &'a ResourceLimiter) -> Result<Self, SandboxError> {
        let depth = limiter.increment_recursion();
        if depth >= limiter.limits.max_recursion_depth {
            // Decrement on error
            limiter.decrement_recursion();
            Err(SandboxError::StackOverflow {
                depth,
                limit: limiter.limits.max_recursion_depth,
            })
        } else {
            Ok(Self { limiter })
        }
    }

    /// Get current recursion depth
    pub fn depth(&self) -> usize {
        self.limiter.current_recursion_depth()
    }
}

impl<'a> Drop for RecursionGuard<'a> {
    fn drop(&mut self) {
        self.limiter.decrement_recursion();
    }
}

/// RAII guard for tracking memory allocation
///
/// Automatically adds to memory counter on creation and subtracts on drop.
pub struct MemoryGuard<'a> {
    limiter: &'a ResourceLimiter,
    bytes: usize,
}

impl<'a> MemoryGuard<'a> {
    /// Create a new memory guard
    ///
    /// Adds bytes to the memory counter and checks against the limit.
    pub fn new(limiter: &'a ResourceLimiter, bytes: usize) -> Result<Self, SandboxError> {
        limiter.check_memory_limit(bytes)?;
        Ok(Self { limiter, bytes })
    }

    /// Get current memory usage
    pub fn usage(&self) -> usize {
        self.limiter.current_memory_usage()
    }
}

impl<'a> Drop for MemoryGuard<'a> {
    fn drop(&mut self) {
        self.limiter.sub_memory(self.bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recursion_guard() {
        let limiter = ResourceLimiter::new();

        // Create a guard
        let guard1 = RecursionGuard::new(&limiter).unwrap();
        assert_eq!(guard1.depth(), 1);

        // Create nested guard
        let guard2 = RecursionGuard::new(&limiter).unwrap();
        assert_eq!(guard2.depth(), 2);

        // Drop inner guard
        drop(guard2);
        assert_eq!(limiter.current_recursion_depth(), 1);

        // Drop outer guard
        drop(guard1);
        assert_eq!(limiter.current_recursion_depth(), 0);
    }

    #[test]
    fn test_memory_guard() {
        let limiter = ResourceLimiter::new();

        // Allocate some memory
        let guard1 = MemoryGuard::new(&limiter, 1000).unwrap();
        assert_eq!(guard1.usage(), 1000);

        // Allocate more memory
        let guard2 = MemoryGuard::new(&limiter, 500).unwrap();
        assert_eq!(guard2.usage(), 1500);

        // Drop first allocation
        drop(guard1);
        assert_eq!(limiter.current_memory_usage(), 500);

        // Drop second allocation
        drop(guard2);
        assert_eq!(limiter.current_memory_usage(), 0);
    }

    #[test]
    fn test_iteration_limit() {
        let limits = ResourceLimits {
            max_iterations: 10,
            ..Default::default()
        };
        let limiter = ResourceLimiter::with_limits(limits);

        for _ in 0..10 {
            assert!(limiter.check_iteration_limit().is_ok());
        }

        // 11th iteration should fail
        assert!(matches!(
            limiter.check_iteration_limit(),
            Err(SandboxError::IterationLimitExceeded { .. })
        ));
    }

    #[test]
    fn test_timeout() {
        let limits = ResourceLimits {
            timeout_ms: 0, // Immediate timeout
            ..Default::default()
        };
        let limiter = ResourceLimiter::with_limits(limits);

        let start = Instant::now();
        std::thread::sleep(std::time::Duration::from_millis(1));

        assert!(matches!(
            limiter.check_timeout(start),
            Err(SandboxError::Timeout { .. })
        ));
    }

    #[test]
    fn test_asset_loading_control() {
        let limiter = ResourceLimiter::new();

        assert!(!limiter.is_asset_loading_allowed());

        limiter.enable_asset_loading();
        assert!(limiter.is_asset_loading_allowed());

        limiter.disable_asset_loading();
        assert!(!limiter.is_asset_loading_allowed());
    }

    #[test]
    fn test_with_asset_loading() {
        let limiter = ResourceLimiter::new();

        assert!(!limiter.is_asset_loading_allowed());

        let result = limiter.with_asset_loading(|| {
            assert!(limiter.is_asset_loading_allowed());
            42
        });

        assert_eq!(result, 42);
        assert!(!limiter.is_asset_loading_allowed());
    }
}
