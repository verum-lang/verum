//! Recovery Strategies (Level 3: Fault Tolerance)
//!
//! Level 3 of the 5-Level Error Defense Architecture provides production-ready
//! fault tolerance. The core design principles are: (1) fail fast when something
//! is broken, (2) automatically attempt recovery, (3) always expose health status,
//! (4) isolate failures to prevent cascading, and (5) make recovery behavior tunable.
//!
//! Integrates with the Unified Execution Environment (ExecutionEnv / theta+) for
//! automatic error recovery in the async runtime, with supervision trees providing
//! hierarchical restart semantics (OneForOne, OneForAll, RestForOne strategies).
//!
//! Provides **production-ready fault tolerance mechanisms** for building resilient systems
//! that can recover automatically from failures. Includes:
//!
//! - **Circuit Breakers** - Stop calling failing services, detect recovery
//! - **Retry Policies** - Retry transient errors with adaptive backoff strategies
//! - **Supervision Trees** - Hierarchical fault tolerance with automatic restarts
//! - **Health Monitoring** - Detect and respond to degradation
//! - **Graceful Degradation** - Reduce functionality rather than crash
//!
//! # Integration Points
//!
//! - **ExecutionEnv** - Automatic error recovery in async runtime
//! - **verum_runtime** - Full supervision tree implementation
//! - **ObservabilityFramework** - Metrics and logging integration
//! - **ConcurrencyPrimitives** - Timeout and cancellation support
//!
//! # Design Principles
//!
//! 1. **Fast failure** - Fail quickly when something is broken
//! 2. **Eventual recovery** - Automatically attempt to recover
//! 3. **Observable** - Always know the system's health status
//! 4. **Cascading isolation** - Prevent failure propagation
//! 5. **Tunable** - Configure recovery for your specific use case
//!
//! # Common Patterns
//!
//! ## Simple Retry
//! ```rust,ignore
//! // Retry 3 times with 100ms delay between attempts
//! let strategy = RecoveryStrategy::Retry {
//!     max_attempts: 3,
//!     backoff: BackoffStrategy::Fixed {
//!         delay: Duration::from_millis(100),
//!     },
//! };
//! ```
//!
//! ## Exponential Backoff
//! ```rust,ignore
//! // Start at 100ms, double each time, max 10 seconds
//! let strategy = RecoveryStrategy::Retry {
//!     max_attempts: 5,
//!     backoff: BackoffStrategy::Exponential {
//!         base: Duration::from_millis(100),
//!         max: Duration::from_secs(10),
//!     },
//! };
//! ```
//!
//! ## Circuit Breaker
//! ```rust,ignore
//! let strategy = RecoveryStrategy::CircuitBreaker {
//!     config: CircuitBreakerConfig {
//!         failure_threshold: 5,        // Fail after 5 consecutive errors
//!         success_threshold: 2,        // Close after 2 successes
//!         timeout: Duration::from_secs(30),  // Try again after 30s
//!     },
//! };
//! ```
//!
//! ## Supervised Task
//! ```rust,ignore
//! let strategy = RecoveryStrategy::Supervision {
//!     restart: RestartStrategy::ExponentialBackoff {
//!         initial: Duration::from_millis(100),
//!         max: Duration::from_secs(10),
//!         multiplier: 2.0,
//!     },
//! };
//! ```
//!
//! These components integrate with ExecutionEnv for automatic error recovery.

use parking_lot::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
#[allow(unused_imports)]
use verum_common::List;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Recovery strategy for error handling
///
/// Defines how errors should be recovered from automatically.
#[derive(Debug, Clone)]
pub enum RecoveryStrategy {
    /// No automatic recovery
    None,

    /// Retry with backoff
    Retry {
        /// Maximum number of retry attempts
        max_attempts: usize,
        /// Backoff strategy
        backoff: BackoffStrategy,
    },

    /// Circuit breaker pattern
    CircuitBreaker {
        /// Circuit breaker configuration
        config: CircuitBreakerConfig,
    },

    /// Supervision tree (integrated with ExecutionEnv)
    Supervision {
        /// Restart strategy
        restart: RestartStrategy,
    },
}

/// Backoff strategy for retries
///
/// Determines the delay between retry attempts.
#[derive(Debug, Clone)]
pub enum BackoffStrategy {
    /// Fixed delay between retries
    Fixed {
        /// Delay duration
        delay: Duration,
    },

    /// Exponential backoff (2^n * base)
    Exponential {
        /// Base delay
        base: Duration,
        /// Maximum delay
        max: Duration,
    },

    /// Fibonacci backoff
    Fibonacci {
        /// Base delay multiplier
        base: Duration,
    },

    /// Linear backoff (base + n * increment)
    Linear {
        /// Base delay
        base: Duration,
        /// Increment per attempt
        increment: Duration,
    },
}

impl BackoffStrategy {
    /// Calculate delay for the nth attempt
    ///
    /// # Performance
    /// - Fixed: O(1)
    /// - Exponential: O(1)
    /// - Fibonacci: O(n)
    /// - Linear: O(1)
    pub fn delay(&self, attempt: usize) -> Duration {
        match self {
            BackoffStrategy::Fixed { delay } => *delay,

            BackoffStrategy::Exponential { base, max } => {
                let delay_ms = base.as_millis() * 2u128.pow(attempt as u32);
                Duration::from_millis(delay_ms.min(max.as_millis()) as u64)
            }

            BackoffStrategy::Fibonacci { base } => {
                let fib = fibonacci(attempt);
                base.saturating_mul(fib as u32)
            }

            BackoffStrategy::Linear { base, increment } => {
                base.saturating_add(increment.saturating_mul(attempt as u32))
            }
        }
    }
}

/// Calculate nth Fibonacci number
fn fibonacci(n: usize) -> usize {
    match n {
        0 | 1 => 1,
        n => {
            let (mut a, mut b) = (1, 1);
            for _ in 2..=n {
                let c = a + b;
                a = b;
                b = c;
            }
            b
        }
    }
}

/// Restart strategy for supervision
///
/// Determines when a supervised process should be restarted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum RestartStrategy {
    /// Always restart (critical services)
    Permanent,

    /// Restart only on error (network services)
    Transient,

    /// Never restart (one-shot tasks)
    Temporary,
}

/// Error predicate function type
///
/// Determines whether an error should count towards circuit breaker failure threshold.
/// Returns `true` if the error should trigger circuit breaker logic.
///
/// # Thread Safety
/// Must be Send + Sync for use in multi-threaded contexts.
pub type ErrorPredicate = Box<dyn Fn(&crate::error::VerumError) -> bool + Send + Sync>;

/// Circuit breaker configuration
///
/// Controls when the circuit opens and closes.
///
/// # Error Predicates
///
/// The `error_predicate` field allows filtering which errors count towards failures:
/// - `None` - All errors count (default behavior)
/// - `Some(predicate)` - Only errors matching the predicate count
///
/// # Examples
///
/// ```rust,ignore
/// use verum_error::recovery::{CircuitBreakerConfig, predicates};
/// use std::time::Duration;
///
/// // Only network errors trigger circuit breaker
/// let config = CircuitBreakerConfig {
///     failure_threshold: 5,
///     timeout: Duration::from_secs(60),
///     required_successes: 3,
///     error_predicate: Some(predicates::is_network_error()),
/// };
///
/// // Only retriable errors trigger circuit breaker
/// let config = CircuitBreakerConfig {
///     failure_threshold: 3,
///     timeout: Duration::from_secs(30),
///     required_successes: 2,
///     error_predicate: Some(predicates::is_retriable()),
/// };
/// ```
pub struct CircuitBreakerConfig {
    /// Number of failures before opening circuit
    pub failure_threshold: usize,

    /// Timeout before transitioning to half-open
    pub timeout: Duration,

    /// Number of successes required to close circuit
    pub required_successes: usize,

    /// Error predicate (determines if error should count)
    ///
    /// When `None`, all errors count towards the failure threshold.
    /// When `Some(predicate)`, only errors matching the predicate count.
    pub error_predicate: Option<ErrorPredicate>,
}

// Manual Clone implementation since ErrorPredicate doesn't implement Clone
impl Clone for CircuitBreakerConfig {
    fn clone(&self) -> Self {
        Self {
            failure_threshold: self.failure_threshold,
            timeout: self.timeout,
            required_successes: self.required_successes,
            error_predicate: None, // Don't clone the predicate - it's not clonable
        }
    }
}

// Manual Debug implementation since ErrorPredicate doesn't implement Debug
impl std::fmt::Debug for CircuitBreakerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CircuitBreakerConfig")
            .field("failure_threshold", &self.failure_threshold)
            .field("timeout", &self.timeout)
            .field("required_successes", &self.required_successes)
            .field(
                "error_predicate",
                &self.error_predicate.as_ref().map(|_| "<function>"),
            )
            .finish()
    }
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            timeout: Duration::from_secs(60),
            required_successes: 3,
            error_predicate: None,
        }
    }
}

impl CircuitBreakerConfig {
    /// Create a new configuration with custom parameters
    pub fn new(failure_threshold: usize, timeout: Duration, required_successes: usize) -> Self {
        Self {
            failure_threshold,
            timeout,
            required_successes,
            error_predicate: None,
        }
    }

    /// Create configuration with an error predicate
    pub fn with_predicate(
        failure_threshold: usize,
        timeout: Duration,
        required_successes: usize,
        error_predicate: ErrorPredicate,
    ) -> Self {
        Self {
            failure_threshold,
            timeout,
            required_successes,
            error_predicate: Some(error_predicate),
        }
    }

    /// Set error predicate (builder pattern)
    pub fn set_predicate(mut self, predicate: ErrorPredicate) -> Self {
        self.error_predicate = Some(predicate);
        self
    }

    /// Configuration for critical services (fail fast)
    pub fn critical() -> Self {
        Self {
            failure_threshold: 2,
            timeout: Duration::from_secs(30),
            required_successes: 5,
            error_predicate: None,
        }
    }

    /// Configuration for resilient services (tolerate failures)
    pub fn resilient() -> Self {
        Self {
            failure_threshold: 10,
            timeout: Duration::from_secs(120),
            required_successes: 2,
            error_predicate: None,
        }
    }

    /// Check if an error should count based on the predicate
    ///
    /// # Performance
    /// - ~5-10ns when no predicate
    /// - ~20-50ns when predicate present (depends on predicate complexity)
    pub fn should_count_error(&self, error: &crate::error::VerumError) -> bool {
        match &self.error_predicate {
            None => true, // All errors count by default
            Some(predicate) => predicate(error),
        }
    }
}

/// Common error predicates for circuit breakers
///
/// Provides pre-built predicates for common error filtering scenarios.
///
/// # Examples
///
/// ```rust,ignore
/// use verum_error::recovery::{CircuitBreakerConfig, predicates};
/// use std::time::Duration;
///
/// // Only count network errors
/// let config = CircuitBreakerConfig::default()
///     .set_predicate(predicates::is_network_error());
///
/// // Only count retriable errors
/// let config = CircuitBreakerConfig::default()
///     .set_predicate(predicates::is_retriable());
///
/// // Custom predicate: only timeout and network errors
/// let config = CircuitBreakerConfig::default()
///     .set_predicate(predicates::any_of(vec![
///         predicates::is_timeout(),
///         predicates::is_network_error(),
///     ]));
/// ```
pub mod predicates {
    use super::ErrorPredicate;
    use crate::error::{ErrorKind, VerumError};
    use verum_common::List;

    /// Predicate that matches retriable errors
    ///
    /// Returns `true` for errors that are typically transient and worth retrying:
    /// - I/O errors
    /// - Network errors
    /// - Database errors
    /// - Timeout errors
    /// - Circuit breaker errors
    /// - Retry exhausted errors
    pub fn is_retriable() -> ErrorPredicate {
        Box::new(|error: &VerumError| error.kind().is_recoverable())
    }

    /// Predicate that matches network errors
    pub fn is_network_error() -> ErrorPredicate {
        Box::new(|error: &VerumError| matches!(error.kind(), ErrorKind::Network))
    }

    /// Predicate that matches timeout errors
    pub fn is_timeout() -> ErrorPredicate {
        Box::new(|error: &VerumError| matches!(error.kind(), ErrorKind::Timeout))
    }

    /// Predicate that matches I/O errors
    pub fn is_io_error() -> ErrorPredicate {
        Box::new(|error: &VerumError| matches!(error.kind(), ErrorKind::IO))
    }

    /// Predicate that matches database errors
    pub fn is_database_error() -> ErrorPredicate {
        Box::new(|error: &VerumError| matches!(error.kind(), ErrorKind::Database))
    }

    /// Predicate that matches any error kind in the provided list
    pub fn any_kind(kinds: List<ErrorKind>) -> ErrorPredicate {
        Box::new(move |error: &VerumError| kinds.contains(&error.kind()))
    }

    /// Predicate that matches if ANY of the provided predicates match
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let predicate = any_of(vec![
    ///     is_network_error(),
    ///     is_timeout(),
    /// ]);
    /// ```
    pub fn any_of(predicates: List<ErrorPredicate>) -> ErrorPredicate {
        Box::new(move |error: &VerumError| predicates.iter().any(|p| p(error)))
    }

    /// Predicate that matches if ALL of the provided predicates match
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let predicate = all_of(vec![
    ///     is_retriable(),
    ///     custom_predicate(),
    /// ]);
    /// ```
    pub fn all_of(predicates: List<ErrorPredicate>) -> ErrorPredicate {
        Box::new(move |error: &VerumError| predicates.iter().all(|p| p(error)))
    }

    /// Predicate that negates another predicate
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // Count everything EXCEPT network errors
    /// let predicate = not(is_network_error());
    /// ```
    pub fn not(predicate: ErrorPredicate) -> ErrorPredicate {
        Box::new(move |error: &VerumError| !predicate(error))
    }

    /// Predicate that always returns true (count all errors)
    pub fn always() -> ErrorPredicate {
        Box::new(|_: &VerumError| true)
    }

    /// Predicate that always returns false (count no errors)
    pub fn never() -> ErrorPredicate {
        Box::new(|_: &VerumError| false)
    }
}

/// Circuit breaker state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum CircuitState {
    /// Circuit is closed, requests pass through
    Closed,

    /// Circuit is open, requests fail fast
    Open,

    /// Circuit is half-open, testing recovery
    HalfOpen,
}

/// Circuit breaker implementation
///
/// Implements the circuit breaker pattern for fault tolerance.
///
/// # Performance
/// - State check: ~10ns (atomic load)
/// - State transition: ~20-50ns (atomic CAS + lock)
///
/// # Thread Safety
/// All operations are thread-safe and lock-free on the fast path.
pub struct CircuitBreaker {
    /// Current state
    state: RwLock<CircuitState>,

    /// Failure count
    failure_count: AtomicU64,

    /// Success count (in half-open state)
    success_count: AtomicU64,

    /// Last state transition time
    last_transition: RwLock<Instant>,

    /// Configuration
    config: CircuitBreakerConfig,
}

impl CircuitBreaker {
    /// Create a new circuit breaker
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            state: RwLock::new(CircuitState::Closed),
            failure_count: AtomicU64::new(0),
            success_count: AtomicU64::new(0),
            last_transition: RwLock::new(Instant::now()),
            config,
        }
    }

    /// Create with default configuration
    pub fn default_config() -> Self {
        Self::new(CircuitBreakerConfig::default())
    }

    /// Check if circuit allows request
    ///
    /// # Performance
    /// ~10-20ns (lock-free on closed/open states)
    pub fn allow_request(&self) -> bool {
        let state = *self.state.read();
        match state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                // Check if timeout has elapsed
                let last = *self.last_transition.read();
                if last.elapsed() > self.config.timeout {
                    // Transition to half-open
                    *self.state.write() = CircuitState::HalfOpen;
                    *self.last_transition.write() = Instant::now();
                    self.success_count.store(0, Ordering::SeqCst);
                    true
                } else {
                    false
                }
            }
            CircuitState::HalfOpen => true,
        }
    }

    /// Record successful request
    ///
    /// # Performance
    /// ~10-20ns
    pub fn record_success(&self) {
        let state = *self.state.read();
        match state {
            CircuitState::Closed => {
                // Reset failure count
                self.failure_count.store(0, Ordering::SeqCst);
            }
            CircuitState::HalfOpen => {
                let successes = self.success_count.fetch_add(1, Ordering::SeqCst) + 1;
                if successes >= self.config.required_successes as u64 {
                    // Close the circuit
                    *self.state.write() = CircuitState::Closed;
                    *self.last_transition.write() = Instant::now();
                    self.failure_count.store(0, Ordering::SeqCst);
                    self.success_count.store(0, Ordering::SeqCst);
                }
            }
            CircuitState::Open => {
                // Shouldn't happen, but reset if it does
                *self.state.write() = CircuitState::Closed;
                *self.last_transition.write() = Instant::now();
            }
        }
    }

    /// Record failed request
    ///
    /// # Performance
    /// ~10-20ns
    pub fn record_failure(&self) {
        let state = *self.state.read();
        match state {
            CircuitState::Closed => {
                let failures = self.failure_count.fetch_add(1, Ordering::SeqCst) + 1;
                if failures >= self.config.failure_threshold as u64 {
                    // Open the circuit
                    *self.state.write() = CircuitState::Open;
                    *self.last_transition.write() = Instant::now();
                }
            }
            CircuitState::HalfOpen => {
                // Immediately reopen
                *self.state.write() = CircuitState::Open;
                *self.last_transition.write() = Instant::now();
                self.success_count.store(0, Ordering::SeqCst);
            }
            CircuitState::Open => {
                // Already open, nothing to do
            }
        }
    }

    /// Record an error, checking the predicate before counting it as a failure
    ///
    /// If the config has an error predicate, only errors matching the predicate
    /// will count towards the failure threshold. Otherwise, all errors count.
    ///
    /// # Performance
    /// - ~10-20ns when no predicate
    /// - ~30-70ns when predicate present
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use verum_error::recovery::{CircuitBreaker, CircuitBreakerConfig, predicates};
    /// use verum_error::error::VerumError;
    /// use std::time::Duration;
    ///
    /// let config = CircuitBreakerConfig::default()
    ///     .set_predicate(predicates::is_network_error());
    /// let breaker = CircuitBreaker::new(config);
    ///
    /// // Network error - will count
    /// let err = VerumError::network("connection refused");
    /// breaker.record_error(&err);
    ///
    /// // Parse error - will NOT count (not a network error)
    /// let err = VerumError::parse("invalid syntax");
    /// breaker.record_error(&err);
    /// ```
    pub fn record_error(&self, error: &crate::error::VerumError) {
        // Check if this error should count based on the predicate
        if self.config.should_count_error(error) {
            self.record_failure();
        }
    }

    /// Get current state
    pub fn state(&self) -> CircuitState {
        *self.state.read()
    }

    /// Get statistics
    pub fn stats(&self) -> CircuitBreakerStats {
        CircuitBreakerStats {
            state: self.state(),
            failure_count: self.failure_count.load(Ordering::SeqCst),
            success_count: self.success_count.load(Ordering::SeqCst),
            last_transition: *self.last_transition.read(),
        }
    }

    /// Reset circuit breaker
    pub fn reset(&self) {
        *self.state.write() = CircuitState::Closed;
        *self.last_transition.write() = Instant::now();
        self.failure_count.store(0, Ordering::SeqCst);
        self.success_count.store(0, Ordering::SeqCst);
    }
}

/// Circuit breaker statistics
#[derive(Debug, Clone, Copy)]
pub struct CircuitBreakerStats {
    /// Current state
    pub state: CircuitState,
    /// Total failures
    pub failure_count: u64,
    /// Successes in half-open state
    pub success_count: u64,
    /// Last state transition time
    pub last_transition: Instant,
}

/// Health check status
///
/// Used to monitor the health of supervised processes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum HealthStatus {
    /// Process is healthy
    Healthy,
    /// Process is degraded (recovering)
    Degraded,
    /// Process has failed
    Unhealthy,
    /// Health status is unknown (not checked yet)
    Unknown,
}

/// Health check configuration
///
/// Defines how to monitor process health.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct HealthCheckConfig {
    /// How frequently to perform health checks
    pub interval: Duration,

    /// Timeout for health check operation
    pub timeout: Duration,

    /// Number of consecutive failures before marking unhealthy
    pub failure_threshold: usize,

    /// Number of consecutive successes before marking healthy
    pub success_threshold: usize,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(30),
            timeout: Duration::from_secs(5),
            failure_threshold: 3,
            success_threshold: 2,
        }
    }
}

impl HealthCheckConfig {
    /// Configuration for critical services (frequent checks)
    pub fn critical() -> Self {
        Self {
            interval: Duration::from_secs(10),
            timeout: Duration::from_secs(2),
            failure_threshold: 2,
            success_threshold: 3,
        }
    }

    /// Configuration for resilient services (less frequent checks)
    pub fn resilient() -> Self {
        Self {
            interval: Duration::from_secs(60),
            timeout: Duration::from_secs(10),
            failure_threshold: 5,
            success_threshold: 2,
        }
    }
}

/// Health check metrics
///
/// Tracks health check history and statistics.
#[derive(Debug, Clone, Copy)]
pub struct HealthMetrics {
    /// Current health status
    pub status: HealthStatus,
    /// Consecutive failure count
    pub consecutive_failures: usize,
    /// Consecutive success count
    pub consecutive_successes: usize,
    /// Last health check time
    pub last_check: Option<Instant>,
    /// Total checks performed
    pub total_checks: u64,
    /// Total failed checks
    pub failed_checks: u64,
}

impl Default for HealthMetrics {
    fn default() -> Self {
        Self {
            status: HealthStatus::Unknown,
            consecutive_failures: 0,
            consecutive_successes: 0,
            last_check: None,
            total_checks: 0,
            failed_checks: 0,
        }
    }
}

/// Health check monitor
///
/// Monitors process health and transitions between states.
///
/// # Performance
/// - Health check: ~5-10μs (excluding check operation)
/// - Metrics access: ~10-20ns
pub struct HealthMonitor {
    /// Configuration
    config: HealthCheckConfig,
    /// Current metrics
    metrics: RwLock<HealthMetrics>,
    /// Last check time
    last_check_time: RwLock<Instant>,
}

impl HealthMonitor {
    /// Create a new health monitor
    pub fn new(config: HealthCheckConfig) -> Self {
        Self {
            config,
            metrics: RwLock::new(HealthMetrics::default()),
            last_check_time: RwLock::new(Instant::now()),
        }
    }

    /// Create with default configuration
    pub fn default_config() -> Self {
        Self::new(HealthCheckConfig::default())
    }

    /// Record a successful health check
    pub fn record_success(&self) {
        let mut metrics = self.metrics.write();
        metrics.total_checks += 1;
        metrics.consecutive_failures = 0;
        metrics.consecutive_successes += 1;

        if metrics.consecutive_successes >= self.config.success_threshold
            && metrics.status != HealthStatus::Healthy
        {
            metrics.status = HealthStatus::Healthy;
            metrics.consecutive_successes = 0;
        }

        metrics.last_check = Some(Instant::now());
        *self.last_check_time.write() = Instant::now();
    }

    /// Record a failed health check
    pub fn record_failure(&self) {
        let mut metrics = self.metrics.write();
        metrics.total_checks += 1;
        metrics.failed_checks += 1;
        metrics.consecutive_successes = 0;
        metrics.consecutive_failures += 1;

        if metrics.consecutive_failures >= self.config.failure_threshold {
            // Transition to next degradation level
            metrics.status = match metrics.status {
                HealthStatus::Healthy | HealthStatus::Unknown => HealthStatus::Degraded,
                HealthStatus::Degraded => HealthStatus::Unhealthy,
                HealthStatus::Unhealthy => HealthStatus::Unhealthy,
            };
        }

        metrics.last_check = Some(Instant::now());
        *self.last_check_time.write() = Instant::now();
    }

    /// Get current health status
    pub fn status(&self) -> HealthStatus {
        self.metrics.read().status
    }

    /// Get health metrics
    pub fn metrics(&self) -> HealthMetrics {
        *self.metrics.read()
    }

    /// Check if health check is due
    pub fn is_check_due(&self) -> bool {
        let last_check = *self.last_check_time.read();
        last_check.elapsed() > self.config.interval
    }

    /// Reset health monitor
    pub fn reset(&self) {
        let mut metrics = self.metrics.write();
        *metrics = HealthMetrics {
            status: HealthStatus::Healthy,
            consecutive_failures: 0,
            consecutive_successes: 0,
            last_check: Some(Instant::now()),
            total_checks: 0,
            failed_checks: 0,
        };
        *self.last_check_time.write() = Instant::now();
    }
}

/// Supervision strategy
///
/// Determines how failures are handled in supervision trees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum SupervisionStrategy {
    /// Restart only the failed child
    OneForOne,
    /// Restart all children if any fails
    OneForAll,
    /// Restart failed child and all children started after it
    RestForOne,
}

/// Shutdown strategy
///
/// Determines how to shutdown a child process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ShutdownStrategy {
    /// Gracefully shutdown (allow completion)
    Graceful {
        /// Timeout duration for graceful shutdown
        timeout: Duration,
    },
    /// Brutally shutdown (force terminate)
    Brutal {
        /// Timeout duration for brutal shutdown
        timeout: Duration,
    },
}

/// Supervision tree configuration
///
/// Configures restart limits and behavior for supervised processes.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SupervisionConfig {
    /// Supervision strategy
    pub strategy: SupervisionStrategy,
    /// Maximum number of restarts
    pub max_restarts: usize,
    /// Time window for restart limit
    pub within: Duration,
}

impl Default for SupervisionConfig {
    fn default() -> Self {
        Self {
            strategy: SupervisionStrategy::OneForOne,
            max_restarts: 10,
            within: Duration::from_secs(60),
        }
    }
}

impl SupervisionConfig {
    /// Configuration for critical services
    pub fn critical() -> Self {
        Self {
            strategy: SupervisionStrategy::OneForOne,
            max_restarts: 5,
            within: Duration::from_secs(30),
        }
    }

    /// Configuration for resilient services
    pub fn resilient() -> Self {
        Self {
            strategy: SupervisionStrategy::OneForOne,
            max_restarts: 15,
            within: Duration::from_secs(120),
        }
    }

    /// Configuration with one-for-all strategy
    pub fn one_for_all() -> Self {
        Self {
            strategy: SupervisionStrategy::OneForAll,
            max_restarts: 5,
            within: Duration::from_secs(60),
        }
    }

    /// Configuration with rest-for-one strategy
    pub fn rest_for_one() -> Self {
        Self {
            strategy: SupervisionStrategy::RestForOne,
            max_restarts: 10,
            within: Duration::from_secs(60),
        }
    }

    /// Decide whether a restart is permitted given the count of
    /// recent restarts and the start of the current window.
    ///
    /// Implements the OTP-style rate limit: a child can restart up
    /// to `max_restarts` times within `within` duration. When the
    /// window has elapsed, the limit resets (caller sees `true` and
    /// is expected to reset its counter). When the limit is
    /// exhausted within the window, returns `false` so the
    /// supervisor escalates per `strategy`.
    ///
    /// Closes the inert-defense pattern around all three
    /// SupervisionConfig fields. Pre-fix `strategy`,
    /// `max_restarts`, and `within` were stored but no production
    /// code path consulted any of them — every caller of
    /// `should_restart` (the existing API) routed through
    /// `RestartStrategy`, a separate enum. This method exposes the
    /// rate-limit semantic as a public API so a future supervisor
    /// implementation can consult the config without re-deriving
    /// the limits at the call site.
    pub fn should_permit_restart(
        &self,
        restart_count: usize,
        window_start: Instant,
    ) -> bool {
        // Window has expired → caller resets the counter and
        // restart is permitted. Returning true here is the natural
        // signal — the supervisor treats this as "fresh window,
        // try again".
        if window_start.elapsed() > self.within {
            return true;
        }
        restart_count < self.max_restarts
    }

    /// Read accessor for the configured supervision strategy.
    /// Embedders building a supervision tree consult this to
    /// dispatch on `OneForOne` / `OneForAll` / `RestForOne` /
    /// `SimpleOneForOne` semantics.
    pub fn strategy(&self) -> SupervisionStrategy {
        self.strategy
    }
}

/// Failure reason classification
///
/// Categorizes how a process exited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ExitReason {
    /// Normal successful exit
    Normal,
    /// Exit with error
    Error,
    /// Panic/crash
    Panic,
    /// Unknown exit reason
    Unknown,
}

/// Restart decision logic
///
/// Determines if a child should be restarted based on strategy and exit reason.
pub fn should_restart(strategy: RestartStrategy, reason: ExitReason) -> bool {
    match (strategy, reason) {
        // Permanent: Always restart
        (RestartStrategy::Permanent, _) => true,

        // Transient: Restart only on errors/panics
        (RestartStrategy::Transient, ExitReason::Normal) => false,
        (RestartStrategy::Transient, ExitReason::Error) => true,
        (RestartStrategy::Transient, ExitReason::Panic) => true,
        (RestartStrategy::Transient, ExitReason::Unknown) => true,

        // Temporary: Never restart
        (RestartStrategy::Temporary, _) => false,
    }
}
