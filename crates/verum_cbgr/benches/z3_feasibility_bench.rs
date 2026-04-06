//! Performance benchmarks for Z3 feasibility checking
//!
//! This benchmark suite measures the performance of Z3 SMT solver integration
//! for path feasibility checking in CBGR escape analysis.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use verum_cbgr::analysis::{BlockId, PathPredicate};
use verum_cbgr::z3_feasibility::Z3FeasibilityChecker;

// ==================== Helper Functions ====================

/// Create a simple predicate: block_id
fn simple_predicate(block_id: usize) -> PathPredicate {
    PathPredicate::BlockTrue(BlockId(block_id as u64))
}

/// Create a conjunction of N predicates: block_1 AND block_2 AND ... AND block_N
fn conjunction_predicate(n: usize) -> PathPredicate {
    let mut pred = PathPredicate::BlockTrue(BlockId(1));
    for i in 2..=n {
        pred = PathPredicate::And(Box::new(pred), Box::new(PathPredicate::BlockTrue(BlockId(i as u64))));
    }
    pred
}

/// Create a disjunction of N predicates: block_1 OR block_2 OR ... OR block_N
fn disjunction_predicate(n: usize) -> PathPredicate {
    let mut pred = PathPredicate::BlockTrue(BlockId(1));
    for i in 2..=n {
        pred = PathPredicate::Or(Box::new(pred), Box::new(PathPredicate::BlockTrue(BlockId(i as u64))));
    }
    pred
}

/// Create a nested predicate with depth D
fn nested_predicate(depth: usize, block_id: usize) -> PathPredicate {
    let mut pred = PathPredicate::BlockTrue(BlockId(block_id as u64));
    for _ in 0..depth {
        pred = PathPredicate::Not(Box::new(PathPredicate::Not(Box::new(pred))));
    }
    pred
}

/// Create a contradiction: block_id AND !block_id
fn contradiction_predicate(block_id: usize) -> PathPredicate {
    PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(block_id as u64))),
        Box::new(PathPredicate::BlockFalse(BlockId(block_id as u64))),
    )
}

/// Create a tautology: block_id OR !block_id
fn tautology_predicate(block_id: usize) -> PathPredicate {
    PathPredicate::Or(
        Box::new(PathPredicate::BlockTrue(BlockId(block_id as u64))),
        Box::new(PathPredicate::BlockFalse(BlockId(block_id as u64))),
    )
}

/// Create a complex nested expression
fn complex_predicate(size: usize) -> PathPredicate {
    // ((block_1 AND block_2) OR (block_3 AND block_4)) AND ... (repeated size times)
    let mut pred = PathPredicate::True;
    for i in 0..size {
        let left_and = PathPredicate::And(
            Box::new(PathPredicate::BlockTrue(BlockId((i * 4 + 1) as u64))),
            Box::new(PathPredicate::BlockTrue(BlockId((i * 4 + 2) as u64))),
        );
        let right_and = PathPredicate::And(
            Box::new(PathPredicate::BlockTrue(BlockId((i * 4 + 3) as u64))),
            Box::new(PathPredicate::BlockTrue(BlockId((i * 4 + 4) as u64))),
        );
        let or_expr = PathPredicate::Or(Box::new(left_and), Box::new(right_and));
        pred = PathPredicate::And(Box::new(pred), Box::new(or_expr));
    }
    pred
}

// ==================== Simple Predicate Benchmarks ====================

fn bench_simple_predicates(c: &mut Criterion) {
    let mut group = c.benchmark_group("simple_predicates");

    // Benchmark True
    group.bench_function("true", |b| {
        let mut checker = Z3FeasibilityChecker::new();
        b.iter(|| {
            black_box(checker.check_feasible(&PathPredicate::True));
        });
    });

    // Benchmark False
    group.bench_function("false", |b| {
        let mut checker = Z3FeasibilityChecker::new();
        b.iter(|| {
            black_box(checker.check_feasible(&PathPredicate::False));
        });
    });

    // Benchmark single block
    group.bench_function("single_block", |b| {
        let mut checker = Z3FeasibilityChecker::new();
        let pred = simple_predicate(42);
        b.iter(|| {
            black_box(checker.check_feasible(&pred));
        });
    });

    // Benchmark contradiction
    group.bench_function("contradiction", |b| {
        let mut checker = Z3FeasibilityChecker::new();
        let pred = contradiction_predicate(42);
        b.iter(|| {
            black_box(checker.check_feasible(&pred));
        });
    });

    // Benchmark tautology
    group.bench_function("tautology", |b| {
        let mut checker = Z3FeasibilityChecker::new();
        let pred = tautology_predicate(42);
        b.iter(|| {
            black_box(checker.check_feasible(&pred));
        });
    });

    group.finish();
}

// ==================== Conjunction Benchmarks ====================

fn bench_conjunctions(c: &mut Criterion) {
    let mut group = c.benchmark_group("conjunctions");

    for size in [10, 50, 100, 500, 1000].iter() {
        group.throughput(Throughput::Elements(*size as u64));

        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let mut checker = Z3FeasibilityChecker::new();
            let pred = conjunction_predicate(size);
            b.iter(|| {
                black_box(checker.check_feasible(&pred));
            });
        });
    }

    group.finish();
}

// ==================== Disjunction Benchmarks ====================

fn bench_disjunctions(c: &mut Criterion) {
    let mut group = c.benchmark_group("disjunctions");

    for size in [10, 50, 100, 500, 1000].iter() {
        group.throughput(Throughput::Elements(*size as u64));

        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let mut checker = Z3FeasibilityChecker::new();
            let pred = disjunction_predicate(size);
            b.iter(|| {
                black_box(checker.check_feasible(&pred));
            });
        });
    }

    group.finish();
}

// ==================== Nested Predicate Benchmarks ====================

fn bench_nested_predicates(c: &mut Criterion) {
    let mut group = c.benchmark_group("nested_predicates");

    for depth in [1, 5, 10, 20, 50].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(depth), depth, |b, &depth| {
            let mut checker = Z3FeasibilityChecker::new();
            let pred = nested_predicate(depth, 42);
            b.iter(|| {
                black_box(checker.check_feasible(&pred));
            });
        });
    }

    group.finish();
}

// ==================== Complex Predicate Benchmarks ====================

fn bench_complex_predicates(c: &mut Criterion) {
    let mut group = c.benchmark_group("complex_predicates");

    for size in [1, 5, 10, 20, 50].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let mut checker = Z3FeasibilityChecker::new();
            let pred = complex_predicate(size);
            b.iter(|| {
                black_box(checker.check_feasible(&pred));
            });
        });
    }

    group.finish();
}

// ==================== Cache Performance Benchmarks ====================

fn bench_cache_effectiveness(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_effectiveness");

    // Benchmark cache hits
    group.bench_function("cache_hit", |b| {
        let mut checker = Z3FeasibilityChecker::new();
        let pred = simple_predicate(42);

        // Warm up cache
        checker.check_feasible(&pred);

        b.iter(|| {
            black_box(checker.check_feasible(&pred));
        });
    });

    // Benchmark cache misses
    group.bench_function("cache_miss", |b| {
        let mut checker = Z3FeasibilityChecker::new();
        let mut counter = 0;

        b.iter(|| {
            counter += 1;
            let pred = simple_predicate(counter);
            black_box(checker.check_feasible(&pred));
        });
    });

    // Benchmark mixed workload (80% hit rate)
    group.bench_function("mixed_80pct_hit", |b| {
        let mut checker = Z3FeasibilityChecker::new();
        let predicates: Vec<PathPredicate> = (0..10).map(|i| simple_predicate(i)).collect();

        // Warm up cache
        for pred in &predicates {
            checker.check_feasible(pred);
        }

        let mut idx = 0;
        b.iter(|| {
            idx += 1;
            let pred_idx = if idx % 10 < 8 {
                // 80% hits: use cached predicates
                idx % 10
            } else {
                // 20% misses: use new predicates
                10 + (idx / 10)
            };

            if pred_idx < 10 {
                black_box(checker.check_feasible(&predicates[pred_idx]));
            } else {
                let pred = simple_predicate(pred_idx);
                black_box(checker.check_feasible(&pred));
            }
        });
    });

    group.finish();
}

// ==================== Cache Eviction Benchmarks ====================

fn bench_cache_eviction(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_eviction");

    // Benchmark with small cache (frequent evictions)
    group.bench_function("small_cache_100", |b| {
        let mut checker = Z3FeasibilityChecker::with_config(100, 100);
        let mut counter = 0;

        b.iter(|| {
            counter += 1;
            let pred = simple_predicate(counter % 200); // 2x cache size
            black_box(checker.check_feasible(&pred));
        });
    });

    // Benchmark with large cache (rare evictions)
    group.bench_function("large_cache_5000", |b| {
        let mut checker = Z3FeasibilityChecker::with_config(5000, 100);
        let mut counter = 0;

        b.iter(|| {
            counter += 1;
            let pred = simple_predicate(counter % 200);
            black_box(checker.check_feasible(&pred));
        });
    });

    group.finish();
}

// ==================== Comparison with Heuristic ====================

fn bench_z3_vs_heuristic(c: &mut Criterion) {
    let mut group = c.benchmark_group("z3_vs_heuristic");

    // Simple predicate (heuristic should be faster)
    group.bench_function("simple_z3", |b| {
        let mut checker = Z3FeasibilityChecker::new();
        let pred = simple_predicate(42);
        b.iter(|| {
            black_box(checker.check_feasible(&pred));
        });
    });

    group.bench_function("simple_heuristic", |b| {
        let pred = simple_predicate(42);
        b.iter(|| {
            black_box(pred.is_satisfiable());
        });
    });

    // Contradiction (both should detect it)
    group.bench_function("contradiction_z3", |b| {
        let mut checker = Z3FeasibilityChecker::new();
        let pred = contradiction_predicate(42);
        b.iter(|| {
            black_box(checker.check_feasible(&pred));
        });
    });

    group.bench_function("contradiction_heuristic", |b| {
        let pred = contradiction_predicate(42);
        b.iter(|| {
            black_box(pred.is_satisfiable());
        });
    });

    // Complex predicate (Z3 should be more precise)
    group.bench_function("complex_z3", |b| {
        let mut checker = Z3FeasibilityChecker::new();
        let pred = complex_predicate(10);
        b.iter(|| {
            black_box(checker.check_feasible(&pred));
        });
    });

    group.bench_function("complex_heuristic", |b| {
        let pred = complex_predicate(10);
        b.iter(|| {
            black_box(pred.is_satisfiable());
        });
    });

    group.finish();
}

// ==================== Real-World Workload Simulation ====================

fn bench_realistic_workload(c: &mut Criterion) {
    let mut group = c.benchmark_group("realistic_workload");

    // Simulate typical escape analysis workload:
    // - Mix of simple and complex predicates
    // - High cache hit rate (>90%)
    // - Occasional complex queries
    group.bench_function("typical_escape_analysis", |b| {
        let mut checker = Z3FeasibilityChecker::new();

        // Pre-populate cache with common predicates
        for i in 0..50 {
            let pred = simple_predicate(i);
            checker.check_feasible(&pred);
        }

        let mut counter = 0;
        b.iter(|| {
            counter += 1;
            let pred = if counter % 100 < 90 {
                // 90% simple cached predicates
                simple_predicate(counter % 50)
            } else if counter % 100 < 95 {
                // 5% conjunctions
                conjunction_predicate(5)
            } else {
                // 5% complex predicates
                complex_predicate(3)
            };

            black_box(checker.check_feasible(&pred));
        });
    });

    group.finish();
}

// ==================== Throughput Benchmarks ====================

fn bench_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("throughput");

    // Measure queries per second
    group.bench_function("qps_simple_cached", |b| {
        let mut checker = Z3FeasibilityChecker::new();
        let pred = simple_predicate(42);
        checker.check_feasible(&pred); // Warm up cache

        b.iter(|| {
            black_box(checker.check_feasible(&pred));
        });
    });

    group.bench_function("qps_simple_uncached", |b| {
        let mut checker = Z3FeasibilityChecker::new();
        let mut counter = 0;

        b.iter(|| {
            counter += 1;
            let pred = simple_predicate(counter);
            black_box(checker.check_feasible(&pred));
        });
    });

    group.bench_function("qps_complex", |b| {
        let mut checker = Z3FeasibilityChecker::new();
        let pred = complex_predicate(10);

        b.iter(|| {
            black_box(checker.check_feasible(&pred));
        });
    });

    group.finish();
}

// ==================== Criterion Configuration ====================

criterion_group!(
    benches,
    bench_simple_predicates,
    bench_conjunctions,
    bench_disjunctions,
    bench_nested_predicates,
    bench_complex_predicates,
    bench_cache_effectiveness,
    bench_cache_eviction,
    bench_z3_vs_heuristic,
    bench_realistic_workload,
    bench_throughput,
);

criterion_main!(benches);
