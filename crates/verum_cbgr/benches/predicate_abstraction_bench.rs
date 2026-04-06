//! Performance benchmarks for predicate abstraction
//!
//! This benchmark suite measures:
//! - Path explosion scenarios (measure reduction)
//! - Abstraction overhead (time cost)
//! - Cache effectiveness
//! - Precision vs performance trade-off

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use verum_cbgr::analysis::{BlockId, PathCondition, PathPredicate};
use verum_cbgr::predicate_abstraction::PredicateAbstractor;
use verum_cbgr::z3_feasibility::Z3FeasibilityCheckerBuilder;
use verum_common::List;

// ============================================================================
// Benchmark 1: Path Explosion Scenarios
// ============================================================================

fn bench_path_explosion(c: &mut Criterion) {
    let mut group = c.benchmark_group("path_explosion");

    for num_paths in [10, 50, 100, 500, 1000] {
        group.throughput(Throughput::Elements(num_paths as u64));

        // Benchmark WITHOUT abstraction
        group.bench_with_input(
            BenchmarkId::new("without_abstraction", num_paths),
            &num_paths,
            |b, &n| {
                b.iter(|| {
                    // Create n similar paths
                    let mut paths = List::new();
                    for i in 0..n {
                        let pred = PathPredicate::BlockTrue(BlockId(i as u64));
                        paths.push(PathCondition::with_predicate(pred));
                    }
                    black_box(paths)
                });
            },
        );

        // Benchmark WITH abstraction
        group.bench_with_input(
            BenchmarkId::new("with_abstraction", num_paths),
            &num_paths,
            |b, &n| {
                b.iter(|| {
                    let mut abstractor = PredicateAbstractor::default();
                    let mut paths = List::new();
                    for i in 0..n {
                        let pred = PathPredicate::BlockTrue(BlockId(i as u64));
                        paths.push(PathCondition::with_predicate(pred));
                    }
                    let merged = abstractor.merge_similar_paths(paths);
                    black_box(merged)
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Benchmark 2: Abstraction Overhead
// ============================================================================

fn bench_abstraction_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("abstraction_overhead");

    let predicates = vec![
        ("simple_block", PathPredicate::BlockTrue(BlockId(42))),
        (
            "simple_and",
            PathPredicate::And(
                Box::new(PathPredicate::BlockTrue(BlockId(1))),
                Box::new(PathPredicate::BlockTrue(BlockId(2))),
            ),
        ),
        (
            "nested_and",
            PathPredicate::And(
                Box::new(PathPredicate::And(
                    Box::new(PathPredicate::BlockTrue(BlockId(1))),
                    Box::new(PathPredicate::BlockTrue(BlockId(2))),
                )),
                Box::new(PathPredicate::And(
                    Box::new(PathPredicate::BlockTrue(BlockId(3))),
                    Box::new(PathPredicate::BlockTrue(BlockId(4))),
                )),
            ),
        ),
        (
            "complex_formula",
            PathPredicate::Or(
                Box::new(PathPredicate::And(
                    Box::new(PathPredicate::BlockTrue(BlockId(1))),
                    Box::new(PathPredicate::Not(Box::new(PathPredicate::BlockTrue(BlockId(2))))),
                )),
                Box::new(PathPredicate::And(
                    Box::new(PathPredicate::BlockTrue(BlockId(3))),
                    Box::new(PathPredicate::BlockTrue(BlockId(4))),
                )),
            ),
        ),
    ];

    for (name, pred) in predicates {
        for level in 0..=4 {
            group.bench_with_input(
                BenchmarkId::new(format!("{}_level_{}", name, level), level),
                &(pred.clone(), level),
                |b, (p, l)| {
                    let mut abstractor = PredicateAbstractor::default();
                    b.iter(|| {
                        let result = abstractor.abstract_predicate(black_box(p), *l);
                        black_box(result)
                    });
                },
            );
        }
    }

    group.finish();
}

// ============================================================================
// Benchmark 3: Cache Effectiveness
// ============================================================================

fn bench_cache_effectiveness(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_effectiveness");

    let pred = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
        Box::new(PathPredicate::BlockTrue(BlockId(2))),
    );

    // Cold cache (first access)
    group.bench_function("cold_cache", |b| {
        b.iter(|| {
            let mut abstractor = PredicateAbstractor::default();
            let result = abstractor.abstract_predicate(black_box(&pred), 1);
            black_box(result)
        });
    });

    // Warm cache (repeated access)
    group.bench_function("warm_cache", |b| {
        let mut abstractor = PredicateAbstractor::default();
        // Prime the cache
        abstractor.abstract_predicate(&pred, 1);

        b.iter(|| {
            let result = abstractor.abstract_predicate(black_box(&pred), 1);
            black_box(result)
        });
    });

    // Cache with many entries
    group.bench_function("large_cache", |b| {
        let mut abstractor = PredicateAbstractor::default();

        // Fill cache with many predicates
        for i in 0..1000 {
            let p = PathPredicate::BlockTrue(BlockId(i));
            abstractor.abstract_predicate(&p, 1);
        }

        b.iter(|| {
            let result = abstractor.abstract_predicate(black_box(&pred), 1);
            black_box(result)
        });
    });

    group.finish();
}

// ============================================================================
// Benchmark 4: Similarity Checking
// ============================================================================

fn bench_similarity_checking(c: &mut Criterion) {
    let mut group = c.benchmark_group("similarity_checking");

    let p1 = PathPredicate::BlockTrue(BlockId(1));
    let p2 = PathPredicate::BlockTrue(BlockId(2));

    let complex1 = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
        Box::new(PathPredicate::BlockTrue(BlockId(2))),
    );

    let complex2 = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(3))),
        Box::new(PathPredicate::BlockTrue(BlockId(4))),
    );

    group.bench_function("simple_similar", |b| {
        let mut abstractor = PredicateAbstractor::default();
        b.iter(|| {
            let result = abstractor.are_similar(black_box(&p1), black_box(&p2));
            black_box(result)
        });
    });

    group.bench_function("complex_similar", |b| {
        let mut abstractor = PredicateAbstractor::default();
        b.iter(|| {
            let result = abstractor.are_similar(black_box(&complex1), black_box(&complex2));
            black_box(result)
        });
    });

    group.bench_function("identical", |b| {
        let mut abstractor = PredicateAbstractor::default();
        b.iter(|| {
            let result = abstractor.are_similar(black_box(&p1), black_box(&p1));
            black_box(result)
        });
    });

    group.finish();
}

// ============================================================================
// Benchmark 5: Path Merging Performance
// ============================================================================

fn bench_path_merging(c: &mut Criterion) {
    let mut group = c.benchmark_group("path_merging");

    for num_paths in [10, 50, 100, 200] {
        group.throughput(Throughput::Elements(num_paths as u64));

        group.bench_with_input(
            BenchmarkId::new("merge_similar", num_paths),
            &num_paths,
            |b, &n| {
                b.iter(|| {
                    let mut abstractor = PredicateAbstractor::default();
                    let mut paths = List::new();
                    // Create similar paths that will be merged
                    for i in 0..n {
                        let pred = PathPredicate::BlockTrue(BlockId(i as u64));
                        paths.push(PathCondition::with_predicate(pred));
                    }
                    let merged = abstractor.merge_similar_paths(black_box(paths));
                    black_box(merged)
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("merge_diverse", num_paths),
            &num_paths,
            |b, &n| {
                b.iter(|| {
                    let mut abstractor = PredicateAbstractor::default();
                    let mut paths = List::new();
                    // Create diverse paths that won't be merged
                    for i in 0..n {
                        let pred = if i % 2 == 0 {
                            PathPredicate::BlockTrue(BlockId(i as u64))
                        } else {
                            PathPredicate::BlockFalse(BlockId(i as u64))
                        };
                        paths.push(PathCondition::with_predicate(pred));
                    }
                    let merged = abstractor.merge_similar_paths(black_box(paths));
                    black_box(merged)
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Benchmark 6: Widening Performance
// ============================================================================

fn bench_widening(c: &mut Criterion) {
    let mut group = c.benchmark_group("widening");

    let pred = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
        Box::new(PathPredicate::BlockTrue(BlockId(2))),
    );

    group.bench_function("first_widening", |b| {
        b.iter(|| {
            let mut abstractor = PredicateAbstractor::default();
            let result = abstractor.abstract_predicate(black_box(&pred), 3);
            black_box(result)
        });
    });

    group.bench_function("repeated_widening", |b| {
        let mut abstractor = PredicateAbstractor::default();
        b.iter(|| {
            let result = abstractor.abstract_predicate(black_box(&pred), 3);
            black_box(result)
        });
    });

    group.finish();
}

// ============================================================================
// Benchmark 7: Normalization Performance
// ============================================================================

fn bench_normalization(c: &mut Criterion) {
    let mut group = c.benchmark_group("normalization");

    // Double negation
    let double_neg = PathPredicate::Not(Box::new(PathPredicate::Not(Box::new(
        PathPredicate::BlockTrue(BlockId(1)),
    ))));

    // Commutative AND
    let comm_and = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(2))),
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
    );

    // Complex nested
    let complex = PathPredicate::Or(
        Box::new(PathPredicate::And(
            Box::new(PathPredicate::BlockTrue(BlockId(3))),
            Box::new(PathPredicate::BlockTrue(BlockId(1))),
        )),
        Box::new(PathPredicate::And(
            Box::new(PathPredicate::BlockTrue(BlockId(2))),
            Box::new(PathPredicate::BlockTrue(BlockId(4))),
        )),
    );

    group.bench_function("double_negation", |b| {
        let mut abstractor = PredicateAbstractor::default();
        b.iter(|| {
            let result = abstractor.abstract_predicate(black_box(&double_neg), 1);
            black_box(result)
        });
    });

    group.bench_function("commutative_and", |b| {
        let mut abstractor = PredicateAbstractor::default();
        b.iter(|| {
            let result = abstractor.abstract_predicate(black_box(&comm_and), 1);
            black_box(result)
        });
    });

    group.bench_function("complex_nested", |b| {
        let mut abstractor = PredicateAbstractor::default();
        b.iter(|| {
            let result = abstractor.abstract_predicate(black_box(&complex), 1);
            black_box(result)
        });
    });

    group.finish();
}

// ============================================================================
// Benchmark 8: Z3 Integration Performance
// ============================================================================

fn bench_z3_integration(c: &mut Criterion) {
    let mut group = c.benchmark_group("z3_integration");

    let p1 = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
        Box::new(PathPredicate::BlockTrue(BlockId(2))),
    );

    let p2 = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(2))),
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
    );

    group.bench_function("equivalence_check", |b| {
        let mut abstractor = PredicateAbstractor::default();
        let mut z3 = Z3FeasibilityCheckerBuilder::new().build();

        b.iter(|| {
            let result = abstractor.check_equivalence_z3(black_box(&p1), black_box(&p2), &mut z3);
            black_box(result)
        });
    });

    let stronger = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
        Box::new(PathPredicate::BlockTrue(BlockId(2))),
    );

    let weaker = PathPredicate::BlockTrue(BlockId(1));

    group.bench_function("subsumption_check", |b| {
        let mut abstractor = PredicateAbstractor::default();
        let mut z3 = Z3FeasibilityCheckerBuilder::new().build();

        b.iter(|| {
            let result =
                abstractor.check_subsumption_z3(black_box(&stronger), black_box(&weaker), &mut z3);
            black_box(result)
        });
    });

    group.finish();
}

// ============================================================================
// Benchmark 9: Precision vs Performance Trade-off
// ============================================================================

fn bench_precision_vs_performance(c: &mut Criterion) {
    let mut group = c.benchmark_group("precision_vs_performance");

    // Create complex predicate
    let mut complex_pred = PathPredicate::BlockTrue(BlockId(1));
    for i in 2..=10 {
        complex_pred = PathPredicate::And(
            Box::new(complex_pred),
            Box::new(PathPredicate::BlockTrue(BlockId(i))),
        );
    }

    for level in 0..=4 {
        group.bench_with_input(
            BenchmarkId::new("abstraction_level", level),
            &level,
            |b, &l| {
                let mut abstractor = PredicateAbstractor::default();
                b.iter(|| {
                    let result = abstractor.abstract_predicate(black_box(&complex_pred), l);
                    black_box(result)
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Benchmark 10: Memory Overhead
// ============================================================================

fn bench_memory_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_overhead");

    group.bench_function("abstractor_creation", |b| {
        b.iter(|| {
            let abstractor = PredicateAbstractor::default();
            black_box(abstractor)
        });
    });

    group.bench_function("cache_growth", |b| {
        b.iter(|| {
            let mut abstractor = PredicateAbstractor::default();
            for i in 0..1000 {
                let pred = PathPredicate::BlockTrue(BlockId(i));
                abstractor.abstract_predicate(&pred, 1);
            }
            black_box(abstractor)
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_path_explosion,
    bench_abstraction_overhead,
    bench_cache_effectiveness,
    bench_similarity_checking,
    bench_path_merging,
    bench_widening,
    bench_normalization,
    bench_z3_integration,
    bench_precision_vs_performance,
    bench_memory_overhead,
);

criterion_main!(benches);
