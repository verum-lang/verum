//! Comprehensive Error Handling Performance Benchmark Suite
//!
//! **CRITICAL REQUIREMENTS**: Verify all error handling performance targets
//!
//! # Performance Targets (from CLAUDE.md)
//! - Circuit breaker check overhead: 10-50ns
//! - Error creation and propagation: < 100ns
//! - Context chain building: < 50ns per level
//! - Recovery strategy execution: < 1μs
//!
//! Run with: cargo bench --package verum_error --bench error_comprehensive_bench --release

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use std::time::Duration;
use verum_error::{
    ErrorKind, VerumError,
    context::ContextError,
    levels::{level0, level1, level4},
    recovery::{
        BackoffStrategy, CircuitBreaker, CircuitBreakerConfig, HealthCheckConfig, HealthMonitor,
    },
};

// =============================================================================
// 1. Circuit Breaker Performance (Target: 10-50ns)
// =============================================================================

fn bench_circuit_breaker_10_50ns(c: &mut Criterion) {
    let mut group = c.benchmark_group("circuit_breaker_10_50ns_target");
    group.significance_level(0.01).sample_size(1000);

    let breaker = CircuitBreaker::default_config();

    // CRITICAL: Circuit breaker check must be 10-50ns
    group.bench_function("allow_request_hot_path", |b| {
        b.iter(|| {
            let allowed = breaker.allow_request();
            black_box(allowed)
        })
    });

    // Record success (fast path)
    group.bench_function("record_success", |b| {
        b.iter(|| {
            breaker.record_success();
        })
    });

    // Record failure (fast path)
    let breaker_failures = CircuitBreaker::new(CircuitBreakerConfig {
        failure_threshold: 10000,
        timeout: Duration::from_secs(60),
        required_successes: 2,
        error_predicate: None,
    });

    group.bench_function("record_failure", |b| {
        b.iter(|| {
            breaker_failures.record_failure();
        })
    });

    // State check
    group.bench_function("state_check", |b| {
        b.iter(|| {
            let state = breaker.state();
            black_box(state)
        })
    });

    // Stats retrieval
    group.bench_function("stats_retrieval", |b| {
        b.iter(|| {
            let stats = breaker.stats();
            black_box(stats)
        })
    });

    group.finish();

    println!("\n╔════════════════════════════════════════════════════════════╗");
    println!("║   CRITICAL: Circuit Breaker Check 10-50ns Target          ║");
    println!("╠════════════════════════════════════════════════════════════╣");
    println!("║ allow_request_hot_path:  Should be 10-50ns                ║");
    println!("║ record_success:          Should be < 20ns                 ║");
    println!("║ record_failure:          Should be < 30ns                 ║");
    println!("║ state_check:             Should be < 10ns                 ║");
    println!("╚════════════════════════════════════════════════════════════╝\n");
}

// =============================================================================
// 2. Error Creation and Propagation (Target: < 100ns)
// =============================================================================

fn bench_error_creation_propagation(c: &mut Criterion) {
    let mut group = c.benchmark_group("error_creation_propagation");
    group.significance_level(0.01).sample_size(500);

    // Error creation (different levels)
    group.bench_function("create_level0_error", |b| {
        b.iter(|| {
            let err = level0::RefinementError::new("x > 0");
            black_box(err)
        })
    });

    group.bench_function("create_level1_error", |b| {
        b.iter(|| {
            let err = level1::VerificationError::new("property holds");
            black_box(err)
        })
    });

    group.bench_function("create_level2_error", |b| {
        b.iter(|| {
            let err = VerumError::new("validation failed", ErrorKind::InvalidState);
            black_box(err)
        })
    });

    // Error with context
    group.bench_function("create_error_with_context", |b| {
        b.iter(|| {
            let err = VerumError::new("test error", ErrorKind::Other);
            let ctx_err = ContextError::new(err, "test_op context");
            black_box(ctx_err)
        })
    });

    // Error propagation simulation
    fn propagate_error_1_level() -> Result<(), VerumError> {
        Err(level0::RefinementError::new("test").into())
    }

    fn propagate_error_3_levels() -> Result<(), VerumError> {
        propagate_error_1_level()?;
        Ok(())
    }

    group.bench_function("propagate_1_level", |b| {
        b.iter(|| {
            let result = propagate_error_1_level();
            black_box(result)
        })
    });

    group.bench_function("propagate_3_levels", |b| {
        b.iter(|| {
            let result = propagate_error_3_levels();
            black_box(result)
        })
    });

    group.finish();

    println!("\n╔════════════════════════════════════════════════════════════╗");
    println!("║    Error Creation and Propagation (Target: < 100ns)       ║");
    println!("╠════════════════════════════════════════════════════════════╣");
    println!("║ All error creation operations should be < 100ns           ║");
    println!("║ Error propagation should add < 50ns per level             ║");
    println!("╚════════════════════════════════════════════════════════════╝\n");
}

// =============================================================================
// 3. Context Chain Building (Target: < 50ns per level)
// =============================================================================

fn bench_context_chain_building(c: &mut Criterion) {
    let mut group = c.benchmark_group("context_chain_building");
    group.significance_level(0.01);

    // Single context
    group.bench_function("build_context_1_field", |b| {
        b.iter(|| {
            let err = VerumError::new("test", ErrorKind::Other);
            let ctx = ContextError::new(err, "test_op");
            black_box(ctx)
        })
    });

    // Multiple fields (using add_structured)
    group.bench_function("build_context_5_fields", |b| {
        b.iter(|| {
            let err = VerumError::new("test", ErrorKind::Other);
            let ctx = ContextError::new(err, "test_op")
                .add_structured("key1", "value1")
                .add_structured("key2", "value2")
                .add_structured("key3", "value3")
                .add_structured("key4", "value4");
            black_box(ctx)
        })
    });

    // Context chain depth
    for depth in [5, 10, 20].iter() {
        group.bench_with_input(
            BenchmarkId::new("chain_depth", depth),
            depth,
            |b, &depth| {
                b.iter(|| {
                    let err = VerumError::new("test", ErrorKind::Other);
                    let mut ctx = ContextError::new(err, "root");
                    for i in 0..depth {
                        let key = format!("key{}", i);
                        let value = format!("value{}", i);
                        ctx = ctx.add_structured(key.as_str(), value.as_str());
                    }
                    black_box(ctx)
                })
            },
        );
    }

    group.finish();

    println!("\n╔════════════════════════════════════════════════════════════╗");
    println!("║     Context Chain Building (Target: < 50ns per level)     ║");
    println!("╠════════════════════════════════════════════════════════════╣");
    println!("║ Each context level should add < 50ns overhead             ║");
    println!("╚════════════════════════════════════════════════════════════╝\n");
}

// =============================================================================
// 4. Recovery Strategy Execution (Target: < 1μs)
// =============================================================================

fn bench_recovery_strategy(c: &mut Criterion) {
    let mut group = c.benchmark_group("recovery_strategy");

    // Backoff calculations
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

    group.bench_function("backoff_fixed", |b| {
        b.iter(|| {
            let delay = fixed.delay(black_box(5));
            black_box(delay)
        })
    });

    group.bench_function("backoff_exponential", |b| {
        b.iter(|| {
            let delay = exponential.delay(black_box(5));
            black_box(delay)
        })
    });

    group.bench_function("backoff_linear", |b| {
        b.iter(|| {
            let delay = linear.delay(black_box(5));
            black_box(delay)
        })
    });

    group.bench_function("backoff_fibonacci", |b| {
        b.iter(|| {
            let delay = fibonacci.delay(black_box(5));
            black_box(delay)
        })
    });

    // Sequential backoff calculations
    group.bench_function("backoff_sequence_10", |b| {
        b.iter(|| {
            let mut delays = Vec::new();
            for attempt in 0..10 {
                delays.push(exponential.delay(attempt));
            }
            black_box(delays)
        })
    });

    group.finish();
}

// =============================================================================
// 5. Health Monitor Performance
// =============================================================================

fn bench_health_monitor(c: &mut Criterion) {
    let mut group = c.benchmark_group("health_monitor");

    let monitor = HealthMonitor::default_config();

    // Record operations
    group.bench_function("record_success", |b| {
        b.iter(|| {
            monitor.record_success();
        })
    });

    let monitor_failures = HealthMonitor::new(HealthCheckConfig {
        interval: Duration::from_secs(30),
        timeout: Duration::from_secs(5),
        failure_threshold: 10000,
        success_threshold: 2,
    });

    group.bench_function("record_failure", |b| {
        b.iter(|| {
            monitor_failures.record_failure();
        })
    });

    // Status checks
    group.bench_function("status_check", |b| {
        b.iter(|| {
            let status = monitor.status();
            black_box(status)
        })
    });

    // Metrics retrieval
    group.bench_function("metrics_retrieval", |b| {
        b.iter(|| {
            let metrics = monitor.metrics();
            black_box(metrics)
        })
    });

    // Check due
    group.bench_function("is_check_due", |b| {
        b.iter(|| {
            let due = monitor.is_check_due();
            black_box(due)
        })
    });

    group.finish();
}

// =============================================================================
// 6. Error Level Performance
// =============================================================================

fn bench_error_levels(c: &mut Criterion) {
    let mut group = c.benchmark_group("error_levels");

    // Level 0: Refinement Error
    group.bench_function("level0_refinement", |b| {
        b.iter(|| {
            let err = level0::RefinementError::new("x > 0");
            black_box(err)
        })
    });

    // Level 1: Verification Error
    group.bench_function("level1_verification", |b| {
        b.iter(|| {
            let err = level1::VerificationError::new("property holds");
            black_box(err)
        })
    });

    // Level 2: Runtime Error (using VerumError)
    group.bench_function("level2_runtime", |b| {
        b.iter(|| {
            let err = VerumError::new("runtime error", ErrorKind::IO);
            black_box(err)
        })
    });

    // Level 3: Fault Tolerance (using VerumError with CircuitOpen kind)
    group.bench_function("level3_fault_tolerance", |b| {
        b.iter(|| {
            let err = VerumError::circuit_open("service unavailable");
            black_box(err)
        })
    });

    // Level 4: Security Error
    group.bench_function("level4_security", |b| {
        b.iter(|| {
            let err = level4::SecurityError::new("access denied");
            black_box(err)
        })
    });

    group.finish();
}

// =============================================================================
// 7. Concurrent Error Handling
// =============================================================================

fn bench_concurrent_error_handling(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent_error_handling");

    let breaker = std::sync::Arc::new(CircuitBreaker::default_config());

    // Concurrent circuit breaker checks
    group.bench_function("concurrent_breaker_4_threads", |b| {
        b.iter(|| {
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    let breaker = std::sync::Arc::clone(&breaker);
                    std::thread::spawn(move || {
                        for _ in 0..100 {
                            let allowed = breaker.allow_request();
                            black_box(allowed);
                        }
                    })
                })
                .collect();

            for handle in handles {
                handle.join().unwrap();
            }
        })
    });

    // Concurrent error creation
    group.bench_function("concurrent_error_creation_4_threads", |b| {
        b.iter(|| {
            let handles: Vec<_> = (0..4)
                .map(|i| {
                    std::thread::spawn(move || {
                        for j in 0..100 {
                            let err = level0::RefinementError::new(format!("error_{}_{}", i, j));
                            black_box(err);
                        }
                    })
                })
                .collect();

            for handle in handles {
                handle.join().unwrap();
            }
        })
    });

    group.finish();
}

// =============================================================================
// 8. Error Formatting Performance
// =============================================================================

fn bench_error_formatting(c: &mut Criterion) {
    let mut group = c.benchmark_group("error_formatting");

    let err = level0::RefinementError::new("x > 0").with_value("-5");
    let err_with_ctx = ContextError::new(
        VerumError::new("test error", ErrorKind::Other),
        "test_op context",
    )
    .add_structured("key1", "value1")
    .add_structured("key2", "value2");

    // Simple error formatting
    group.bench_function("format_simple_error", |b| {
        b.iter(|| {
            let formatted = format!("{}", err);
            black_box(formatted)
        })
    });

    // Error with context formatting
    group.bench_function("format_error_with_context", |b| {
        b.iter(|| {
            let formatted = format!("{}", err_with_ctx);
            black_box(formatted)
        })
    });

    // Debug formatting
    group.bench_function("format_debug", |b| {
        b.iter(|| {
            let formatted = format!("{:?}", err);
            black_box(formatted)
        })
    });

    group.finish();
}

// =============================================================================
// 9. Circuit Breaker State Transitions
// =============================================================================

fn bench_circuit_breaker_transitions(c: &mut Criterion) {
    let mut group = c.benchmark_group("circuit_breaker_transitions");

    // Test different failure thresholds
    for threshold in [5, 10, 20, 50].iter() {
        group.bench_with_input(
            BenchmarkId::new("transition", threshold),
            threshold,
            |b, &threshold| {
                let breaker = CircuitBreaker::new(CircuitBreakerConfig {
                    failure_threshold: threshold,
                    timeout: Duration::from_secs(60),
                    required_successes: 2,
                    error_predicate: None,
                });

                b.iter(|| {
                    // Trigger transition
                    for _ in 0..threshold {
                        breaker.record_failure();
                    }
                    // Reset
                    breaker.reset();
                })
            },
        );
    }

    group.finish();
}

// =============================================================================
// 10. Error Result Chaining
// =============================================================================

fn bench_error_result_chaining(c: &mut Criterion) {
    let mut group = c.benchmark_group("error_result_chaining");

    fn operation_1() -> Result<i32, VerumError> {
        Ok(42)
    }

    fn operation_2(x: i32) -> Result<i32, VerumError> {
        if x > 0 {
            Ok(x * 2)
        } else {
            Err(level0::RefinementError::new("x > 0").into())
        }
    }

    fn operation_3(x: i32) -> Result<i32, VerumError> {
        if x < 100 {
            Ok(x + 10)
        } else {
            Err(level1::VerificationError::new("x < 100").into())
        }
    }

    // Success path
    group.bench_function("chain_success_3_ops", |b| {
        b.iter(|| {
            let result = operation_1().and_then(operation_2).and_then(operation_3);
            black_box(result)
        })
    });

    // Error path (early return)
    fn failing_op_1() -> Result<i32, VerumError> {
        Err(level0::RefinementError::new("test").into())
    }

    group.bench_function("chain_error_early_return", |b| {
        b.iter(|| {
            let result = failing_op_1().and_then(operation_2).and_then(operation_3);
            black_box(result)
        })
    });

    group.finish();
}

// =============================================================================
// Criterion Configuration
// =============================================================================

criterion_group!(
    error_benches,
    bench_circuit_breaker_10_50ns,
    bench_error_creation_propagation,
    bench_context_chain_building,
    bench_recovery_strategy,
    bench_health_monitor,
    bench_error_levels,
    bench_concurrent_error_handling,
    bench_error_formatting,
    bench_circuit_breaker_transitions,
    bench_error_result_chaining,
);

criterion_main!(error_benches);
