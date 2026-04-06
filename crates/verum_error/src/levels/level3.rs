//! Level 3: Fault Tolerance (Resilience Patterns)
//!
//! Level 3 provides automatic recovery from unexpected failures through
//! composition of resilience patterns. Key components:
//!
//! - **Supervision Trees**: Hierarchical fault tolerance with three strategies --
//!   OneForOne (restart only the failed child), OneForAll (restart all children),
//!   and RestForOne (restart the failed child plus all children started after it).
//!   Restart intensity is rate-limited (max_restarts within a time window).
//! - **Circuit Breakers**: 3-state machine (Closed -> Open -> HalfOpen -> Closed).
//!   Opens after `failure_threshold` consecutive errors, transitions to HalfOpen
//!   after `timeout`, and closes again after `required_successes` in HalfOpen state.
//! - **Retry Policies**: Fixed, exponential (2^n * base capped at max), fibonacci,
//!   and linear backoff strategies with configurable max attempts.
//! - **Health Monitoring**: Periodic health checks with configurable thresholds for
//!   Healthy -> Degraded -> Unhealthy state transitions.
//!
//! These integrate with ExecutionEnv for automatic error recovery in the async runtime.
//! Full supervision tree implementation is in verum_runtime.
//!
//! Implements **production-grade resilience patterns** that keep systems running
//! even when errors occur. This level provides:
//!
//! - **Automatic recovery** - circuit breakers, retries, exponential backoff
//! - **Supervision trees** - hierarchical fault tolerance
//! - **Health monitoring** - detect degradation before failure
//! - **Graceful degradation** - reduce functionality rather than crash
//! - **Chaos engineering** - deliberately inject faults to test resilience
//!
//! # Recovery Strategies
//!
//! - **Retry with backoff**: Exponential, linear, or custom backoff
//! - **Circuit breaker**: Stop calling failing service, attempt recovery
//! - **Bulkhead isolation**: Prevent cascading failures
//! - **Fallback**: Gracefully degrade to alternative implementation
//! - **Timeout**: Prevent hanging operations
//!
//! # Supervision Trees (verum_runtime)
//!
//! Hierarchical fault tolerance with automatic restart:
//!
//! ```rust,ignore
//! // Supervisor monitors children and restarts on failure
//! supervisor.spawn_child("worker", || async {
//!     do_work().await
//! }, RestartStrategy::ExponentialBackoff {
//!     initial: Duration::from_millis(100),
//!     max: Duration::from_secs(10),
//!     multiplier: 2.0,
//! })?;
//! ```
//!
//! # Health Monitoring
//!
//! Detect problems before they become failures:
//!
//! ```rust,ignore
//! let health = HealthMonitor::new();
//! health.check_cpu()? ;  // Alert if > 90%
//! health.check_memory()?;  // Alert if > 85%
//! health.check_latency()?; // Alert if p99 > 1s
//! ```
//!
//! # Best Practices
//!
//! 1. **Don't retry everything** - retry transient errors, not logic errors
//! 2. **Set timeouts** - prevent indefinite hangs
//! 3. **Monitor health** - detect issues early
//! 4. **Use circuit breakers** - prevent cascading failures
//! 5. **Log failures** - understand why recovery was needed
//! 6. **Test recovery** - chaos engineering finds weak points
//!
//! Re-exports recovery strategies from the recovery module.
//! Full supervision tree implementation is in verum_runtime.

pub use crate::recovery::{
    BackoffStrategy, CircuitBreaker, CircuitBreakerConfig, CircuitBreakerStats, CircuitState,
    ExitReason, HealthCheckConfig, HealthMetrics, HealthMonitor, HealthStatus, RecoveryStrategy,
    RestartStrategy, ShutdownStrategy, SupervisionConfig, SupervisionStrategy, should_restart,
};
