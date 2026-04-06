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
