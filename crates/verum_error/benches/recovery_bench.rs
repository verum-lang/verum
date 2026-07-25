//! Benchmarks for recovery strategies (Level 3: Fault Tolerance)
//!
//! Verifies performance targets for Level 3 fault tolerance mechanisms:
//! - Circuit Breaker state check: ~10-20ns
//! - Health check: ~5-10μs (excluding check operation)
//! - Backoff calculation: ~50ns

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use std::time::Duration;
use verum_error::recovery::{
    BackoffStrategy, CircuitBreaker, CircuitBreakerConfig, HealthCheckConfig, HealthMonitor,
};

fn circuit_breaker_allow_request(c: &mut Criterion) {
    let breaker = CircuitBreaker::default_config();
    c.bench_function("circuit_breaker_allow_request_closed", |b| {
        b.iter(|| breaker.allow_request())
    });
}

fn circuit_breaker_record_success(c: &mut Criterion) {
    let breaker = CircuitBreaker::default_config();
    c.bench_function("circuit_breaker_record_success", |b| {
        b.iter(|| breaker.record_success())
    });
}

fn circuit_breaker_record_failure(c: &mut Criterion) {
    let breaker = CircuitBreaker::new(CircuitBreakerConfig {
        failure_threshold: 10000, // High threshold to avoid state transition
        timeout: Duration::from_secs(60),
        required_successes: 2,
        error_predicate: None,
    });
    c.bench_function("circuit_breaker_record_failure", |b| {
        b.iter(|| breaker.record_failure())
    });
}

fn circuit_breaker_state_transitions(c: &mut Criterion) {
    let mut group = c.benchmark_group("circuit_breaker_state_transitions");

    for failure_threshold in [1, 5, 10].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(failure_threshold),
            failure_threshold,
            |b, &threshold| {
                let breaker = CircuitBreaker::new(CircuitBreakerConfig {
                    failure_threshold: threshold,
                    timeout: Duration::from_secs(60),
                    required_successes: 2,
                    error_predicate: None,
                });
                b.iter(|| {
                    for _ in 0..threshold {
                        breaker.record_failure();
                    }
                    breaker.reset();
                });
            },
        );
    }
    group.finish();
}

fn backoff_calculation(c: &mut Criterion) {
    let fixed = BackoffStrategy::Fixed {
        delay: Duration::from_millis(100),
    };
    let exponential = BackoffStrategy::Exponential {
        base: Duration::from_millis(100),
        max: Duration::from_secs(10),
    };
    let linear = BackoffStrategy::Linear {
        base: Duration::from_millis(100),
        increment: Duration::from_millis(50),
    };
    let fibonacci = BackoffStrategy::Fibonacci {
        base: Duration::from_millis(100),
    };

    c.bench_function("backoff_fixed", |b| b.iter(|| fixed.delay(black_box(5))));
    c.bench_function("backoff_exponential", |b| {
        b.iter(|| exponential.delay(black_box(5)))
    });
    c.bench_function("backoff_linear", |b| b.iter(|| linear.delay(black_box(5))));
    c.bench_function("backoff_fibonacci", |b| {
        b.iter(|| fibonacci.delay(black_box(5)))
    });
}

fn health_monitor_record_success(c: &mut Criterion) {
    let monitor = HealthMonitor::default_config();
    c.bench_function("health_monitor_record_success", |b| {
        b.iter(|| monitor.record_success())
    });
}

fn health_monitor_record_failure(c: &mut Criterion) {
    let monitor = HealthMonitor::new(HealthCheckConfig {
        interval: Duration::from_secs(30),
        timeout: Duration::from_secs(5),
        failure_threshold: 1000, // High threshold to avoid state transition
        success_threshold: 2,
    });
    c.bench_function("health_monitor_record_failure", |b| {
        b.iter(|| monitor.record_failure())
    });
}

fn health_monitor_status_check(c: &mut Criterion) {
    let monitor = HealthMonitor::default_config();
    c.bench_function("health_monitor_status_check", |b| {
        b.iter(|| monitor.status())
    });
}

fn health_monitor_metrics_retrieval(c: &mut Criterion) {
    let monitor = HealthMonitor::default_config();
    c.bench_function("health_monitor_metrics_retrieval", |b| {
        b.iter(|| monitor.metrics())
    });
}

fn health_monitor_is_check_due(c: &mut Criterion) {
    let monitor = HealthMonitor::default_config();
    c.bench_function("health_monitor_is_check_due", |b| {
        b.iter(|| monitor.is_check_due())
    });
}

fn circuit_breaker_stats(c: &mut Criterion) {
    let breaker = CircuitBreaker::default_config();
    // Record some activity
    for _ in 0..3 {
        breaker.record_failure();
    }
    c.bench_function("circuit_breaker_stats", |b| b.iter(|| breaker.stats()));
}

criterion_group!(
    benches,
    circuit_breaker_allow_request,
    circuit_breaker_record_success,
    circuit_breaker_record_failure,
    circuit_breaker_state_transitions,
    circuit_breaker_stats,
    backoff_calculation,
    health_monitor_record_success,
    health_monitor_record_failure,
    health_monitor_status_check,
    health_monitor_metrics_retrieval,
    health_monitor_is_check_due,
);
criterion_main!(benches);
