#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
// Tests for recovery module
// Migrated from src/recovery.rs per CLAUDE.md standards

use std::time::Duration;
use verum_error::recovery::*;

/// Helper fibonacci function for testing (mirrors internal implementation)
fn fibonacci(n: usize) -> usize {
    match n {
        0 | 1 => 1,
        n => {
            let (mut a, mut b) = (1, 1);
            for _ in 2..=n {
                (a, b) = (b, a + b);
            }
            b
        }
    }
}

#[test]
fn test_backoff_fixed() {
    let backoff = BackoffStrategy::Fixed {
        delay: Duration::from_millis(100),
    };

    assert_eq!(backoff.delay(0), Duration::from_millis(100));
    assert_eq!(backoff.delay(5), Duration::from_millis(100));
}

#[test]
fn test_backoff_exponential() {
    let backoff = BackoffStrategy::Exponential {
        base: Duration::from_millis(100),
        max: Duration::from_secs(10),
    };

    assert_eq!(backoff.delay(0), Duration::from_millis(100));
    assert_eq!(backoff.delay(1), Duration::from_millis(200));
    assert_eq!(backoff.delay(2), Duration::from_millis(400));
    assert_eq!(backoff.delay(3), Duration::from_millis(800));
}

#[test]
fn test_fibonacci() {
    assert_eq!(fibonacci(0), 1);
    assert_eq!(fibonacci(1), 1);
    assert_eq!(fibonacci(2), 2);
    assert_eq!(fibonacci(3), 3);
    assert_eq!(fibonacci(4), 5);
    assert_eq!(fibonacci(5), 8);
}

#[test]
fn test_circuit_breaker_closed() {
    let breaker = CircuitBreaker::default_config();

    assert_eq!(breaker.state(), CircuitState::Closed);
    assert!(breaker.allow_request());
}

#[test]
fn test_circuit_breaker_opens() {
    let breaker = CircuitBreaker::new(CircuitBreakerConfig {
        failure_threshold: 3,
        timeout: Duration::from_secs(60),
        required_successes: 2,
        error_predicate: None,
    });

    // Record failures
    breaker.record_failure();
    assert_eq!(breaker.state(), CircuitState::Closed);

    breaker.record_failure();
    assert_eq!(breaker.state(), CircuitState::Closed);

    breaker.record_failure();
    assert_eq!(breaker.state(), CircuitState::Open);
    assert!(!breaker.allow_request());
}

#[test]
fn test_circuit_breaker_half_open() {
    let breaker = CircuitBreaker::new(CircuitBreakerConfig {
        failure_threshold: 1,
        timeout: Duration::from_millis(10),
        required_successes: 2,
        error_predicate: None,
    });

    // Open the circuit
    breaker.record_failure();
    assert_eq!(breaker.state(), CircuitState::Open);

    // Wait for timeout
    std::thread::sleep(Duration::from_millis(15));

    // Should transition to half-open
    assert!(breaker.allow_request());
    assert_eq!(breaker.state(), CircuitState::HalfOpen);

    // Record successes
    breaker.record_success();
    assert_eq!(breaker.state(), CircuitState::HalfOpen);

    breaker.record_success();
    assert_eq!(breaker.state(), CircuitState::Closed);
}

#[test]
fn test_circuit_breaker_reset() {
    let breaker = CircuitBreaker::default_config();

    breaker.record_failure();
    breaker.record_failure();
    breaker.record_failure();

    breaker.reset();

    assert_eq!(breaker.state(), CircuitState::Closed);
    let stats = breaker.stats();
    assert_eq!(stats.failure_count, 0);
}

// =============================================================================
// SupervisionConfig wiring tests
// =============================================================================
//
// Pin: SupervisionConfig.{strategy, max_restarts, within} reach
// the public `should_permit_restart` + `strategy()` accessors.
// Pre-fix all three fields were stored on the struct + asserted
// in tests but no production code path consulted them — every
// caller of supervision logic routed through `RestartStrategy`
// (a separate enum) without consulting SupervisionConfig.

mod supervision_config_wiring {
    use super::*;
    use std::time::Instant;

    #[test]
    fn should_permit_restart_under_limit() {
        // Pin: a child can restart up to max_restarts times within
        // the configured window. Under the limit → permitted.
        let cfg = SupervisionConfig::default(); // 10 restarts / 60s
        let now = Instant::now();
        assert!(cfg.should_permit_restart(0, now));
        assert!(cfg.should_permit_restart(5, now));
        assert!(cfg.should_permit_restart(9, now));
    }

    #[test]
    fn should_permit_restart_at_limit_blocks() {
        // Pin: restart_count == max_restarts → not permitted.
        let cfg = SupervisionConfig::default();
        let now = Instant::now();
        assert!(!cfg.should_permit_restart(10, now));
        assert!(!cfg.should_permit_restart(100, now));
    }

    #[test]
    fn should_permit_restart_after_window_resets() {
        // Pin: when window has elapsed, restart is permitted
        // regardless of count — caller is expected to reset its
        // counter on the next attempt. Use a tiny window to
        // exercise the elapsed branch deterministically.
        let cfg = SupervisionConfig {
            strategy: SupervisionStrategy::OneForOne,
            max_restarts: 1,
            within: Duration::from_nanos(1),
        };
        let past = Instant::now();
        std::thread::sleep(Duration::from_millis(2));
        // Even with restart_count > max_restarts, the elapsed
        // window resets and permits restart.
        assert!(cfg.should_permit_restart(100, past));
    }

    #[test]
    fn strategy_accessor_returns_configured_value() {
        // Pin: strategy() exposes the raw enum without the field
        // becoming inert. Round-trip every variant.
        for strat in [
            SupervisionStrategy::OneForOne,
            SupervisionStrategy::OneForAll,
            SupervisionStrategy::RestForOne,
        ] {
            let cfg = SupervisionConfig {
                strategy: strat,
                max_restarts: 5,
                within: Duration::from_secs(30),
            };
            assert_eq!(cfg.strategy(), strat);
        }
    }

    #[test]
    fn preset_constructors_carry_distinct_limits() {
        // Pin: the preset constructors set distinct
        // (max_restarts, within) pairs that should_permit_restart
        // honours.
        let critical = SupervisionConfig::critical(); // 5/30s
        let resilient = SupervisionConfig::resilient(); // 15/120s
        let now = Instant::now();
        // critical permits 4 restarts but blocks at 5.
        assert!(critical.should_permit_restart(4, now));
        assert!(!critical.should_permit_restart(5, now));
        // resilient permits up to 14, blocks at 15.
        assert!(resilient.should_permit_restart(14, now));
        assert!(!resilient.should_permit_restart(15, now));
    }
}
