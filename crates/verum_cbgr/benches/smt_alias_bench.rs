//! Benchmarks for SMT-based alias verification
//!
//! Performance target: <500μs per query with caching

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use verum_cbgr::analysis::RefId;
use verum_cbgr::smt_alias_verification::{
    ArrayIndex, PointerConstraint, SmtAliasVerifier, SmtAliasVerifierBuilder,
};

// =============================================================================
// Benchmark Group 1: Basic SMT Queries
// =============================================================================

fn bench_simple_stack_alloc_verification(c: &mut Criterion) {
    let mut group = c.benchmark_group("smt_simple_stack");

    group.bench_function("different_allocations", |b| {
        let mut verifier = SmtAliasVerifier::new();
        let ptr1 = PointerConstraint::stack_alloc(1, 0);
        let ptr2 = PointerConstraint::stack_alloc(2, 0);

        b.iter(|| {
            let result = verifier.verify_no_alias(
                black_box(RefId(1)),
                black_box(RefId(2)),
                black_box(&ptr1),
                black_box(&ptr2),
            );
            black_box(result);
        });
    });

    group.bench_function("same_alloc_different_offsets", |b| {
        let mut verifier = SmtAliasVerifier::new();
        let ptr1 = PointerConstraint::stack_alloc(1, 0);
        let ptr2 = PointerConstraint::stack_alloc(1, 8);

        b.iter(|| {
            let result = verifier.verify_no_alias(
                black_box(RefId(1)),
                black_box(RefId(2)),
                black_box(&ptr1),
                black_box(&ptr2),
            );
            black_box(result);
        });
    });

    group.bench_function("stack_vs_heap", |b| {
        let mut verifier = SmtAliasVerifier::new();
        let stack = PointerConstraint::stack_alloc(1, 0);
        let heap = PointerConstraint::heap_alloc(1, 0);

        b.iter(|| {
            let result = verifier.verify_no_alias(
                black_box(RefId(1)),
                black_box(RefId(2)),
                black_box(&stack),
                black_box(&heap),
            );
            black_box(result);
        });
    });

    group.finish();
}

// =============================================================================
// Benchmark Group 2: Field Access Verification
// =============================================================================

fn bench_field_access_verification(c: &mut Criterion) {
    let mut group = c.benchmark_group("smt_field_access");

    for num_fields in [2, 5, 10, 20].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(num_fields),
            num_fields,
            |b, &num_fields| {
                let mut verifier = SmtAliasVerifier::new();
                let base = PointerConstraint::stack_alloc(1, 0);

                // Create field constraints
                let field1 = PointerConstraint::field(base.clone(), 0, "field_0".into());
                let field2 =
                    PointerConstraint::field(base, (num_fields * 8) as u64, "field_n".into());

                b.iter(|| {
                    let result = verifier.verify_no_alias(
                        black_box(RefId(1)),
                        black_box(RefId(2)),
                        black_box(&field1),
                        black_box(&field2),
                    );
                    black_box(result);
                });
            },
        );
    }

    group.finish();
}

// =============================================================================
// Benchmark Group 3: Array Access Verification
// =============================================================================

fn bench_array_access_verification(c: &mut Criterion) {
    let mut group = c.benchmark_group("smt_array_access");

    group.bench_function("concrete_indices", |b| {
        let mut verifier = SmtAliasVerifier::new();
        let base = PointerConstraint::heap_alloc(1, 0);
        let elem0 = PointerConstraint::array_element(base.clone(), ArrayIndex::concrete(0), 4);
        let elem100 = PointerConstraint::array_element(base, ArrayIndex::concrete(100), 4);

        b.iter(|| {
            let result = verifier.verify_no_alias(
                black_box(RefId(1)),
                black_box(RefId(2)),
                black_box(&elem0),
                black_box(&elem100),
            );
            black_box(result);
        });
    });

    group.bench_function("symbolic_indices", |b| {
        let mut verifier = SmtAliasVerifier::new();
        let base = PointerConstraint::heap_alloc(1, 0);
        let elem_i =
            PointerConstraint::array_element(base.clone(), ArrayIndex::symbolic("i".into()), 4);
        let elem_j = PointerConstraint::array_element(base, ArrayIndex::symbolic("j".into()), 4);

        b.iter(|| {
            let result = verifier.verify_no_alias(
                black_box(RefId(1)),
                black_box(RefId(2)),
                black_box(&elem_i),
                black_box(&elem_j),
            );
            black_box(result);
        });
    });

    group.bench_function("bounded_symbolic_indices", |b| {
        let mut verifier = SmtAliasVerifier::new();
        let base = PointerConstraint::heap_alloc(1, 0);
        let elem_low = PointerConstraint::array_element(
            base.clone(),
            ArrayIndex::symbolic_bounded("i".into(), 0, 10),
            4,
        );
        let elem_high = PointerConstraint::array_element(
            base,
            ArrayIndex::symbolic_bounded("j".into(), 20, 30),
            4,
        );

        b.iter(|| {
            let result = verifier.verify_no_alias(
                black_box(RefId(1)),
                black_box(RefId(2)),
                black_box(&elem_low),
                black_box(&elem_high),
            );
            black_box(result);
        });
    });

    group.finish();
}

// =============================================================================
// Benchmark Group 4: Cache Performance
// =============================================================================

fn bench_cache_performance(c: &mut Criterion) {
    let mut group = c.benchmark_group("smt_cache");

    group.bench_function("cold_cache", |b| {
        let ptr1 = PointerConstraint::stack_alloc(1, 0);
        let ptr2 = PointerConstraint::stack_alloc(2, 0);

        b.iter(|| {
            // Create new verifier each time (cold cache)
            let mut verifier = SmtAliasVerifier::new();
            let result = verifier.verify_no_alias(
                black_box(RefId(1)),
                black_box(RefId(2)),
                black_box(&ptr1),
                black_box(&ptr2),
            );
            black_box(result);
        });
    });

    group.bench_function("warm_cache", |b| {
        let mut verifier = SmtAliasVerifier::new();
        let ptr1 = PointerConstraint::stack_alloc(1, 0);
        let ptr2 = PointerConstraint::stack_alloc(2, 0);

        // Warm up cache
        verifier.verify_no_alias(RefId(1), RefId(2), &ptr1, &ptr2);

        b.iter(|| {
            // Same verifier (warm cache)
            let result = verifier.verify_no_alias(
                black_box(RefId(1)),
                black_box(RefId(2)),
                black_box(&ptr1),
                black_box(&ptr2),
            );
            black_box(result);
        });
    });

    group.bench_function("mixed_cache_hit_rate", |b| {
        let mut verifier = SmtAliasVerifier::new();

        // Create 10 different constraint pairs
        let pairs: Vec<(PointerConstraint, PointerConstraint)> = (0..10)
            .map(|i| {
                (
                    PointerConstraint::stack_alloc(i, 0),
                    PointerConstraint::stack_alloc(i + 100, 0),
                )
            })
            .collect();

        // Warm up with half
        for i in 0..5 {
            verifier.verify_no_alias(
                RefId(i),
                RefId(i + 100),
                &pairs[i as usize].0,
                &pairs[i as usize].1,
            );
        }

        let mut counter = 0;
        b.iter(|| {
            // Access in pattern that creates 50% cache hit rate
            let idx = counter % 10;
            let result = verifier.verify_no_alias(
                black_box(RefId(idx)),
                black_box(RefId(idx + 100)),
                black_box(&pairs[idx as usize].0),
                black_box(&pairs[idx as usize].1),
            );
            counter += 1;
            black_box(result);
        });
    });

    group.finish();
}

// =============================================================================
// Benchmark Group 5: Complex Scenarios
// =============================================================================

fn bench_complex_pointer_expressions(c: &mut Criterion) {
    let mut group = c.benchmark_group("smt_complex");

    group.bench_function("nested_field_access", |b| {
        let mut verifier = SmtAliasVerifier::new();
        let base = PointerConstraint::stack_alloc(1, 0);
        let level1 = PointerConstraint::field(base.clone(), 0, "a".into());
        let level2 = PointerConstraint::field(level1, 8, "b".into());
        let level3 = PointerConstraint::field(level2, 16, "c".into());
        let other = PointerConstraint::field(base, 100, "other".into());

        b.iter(|| {
            let result = verifier.verify_no_alias(
                black_box(RefId(1)),
                black_box(RefId(2)),
                black_box(&level3),
                black_box(&other),
            );
            black_box(result);
        });
    });

    group.bench_function("pointer_arithmetic", |b| {
        let mut verifier = SmtAliasVerifier::new();
        let base = PointerConstraint::stack_alloc(1, 0);
        let offset1 = PointerConstraint::add_offset(base.clone(), 8);
        let offset2 = PointerConstraint::add_offset(base, 16);

        b.iter(|| {
            let result = verifier.verify_no_alias(
                black_box(RefId(1)),
                black_box(RefId(2)),
                black_box(&offset1),
                black_box(&offset2),
            );
            black_box(result);
        });
    });

    group.bench_function("mixed_field_and_array", |b| {
        let mut verifier = SmtAliasVerifier::new();
        let base = PointerConstraint::stack_alloc(1, 0);
        let field = PointerConstraint::field(base.clone(), 0, "array_field".into());
        let array_elem = PointerConstraint::array_element(field, ArrayIndex::concrete(5), 4);
        let other_field = PointerConstraint::field(base, 100, "other".into());

        b.iter(|| {
            let result = verifier.verify_no_alias(
                black_box(RefId(1)),
                black_box(RefId(2)),
                black_box(&array_elem),
                black_box(&other_field),
            );
            black_box(result);
        });
    });

    group.finish();
}

// =============================================================================
// Benchmark Group 6: Batch Verification
// =============================================================================

fn bench_batch_verification(c: &mut Criterion) {
    let mut group = c.benchmark_group("smt_batch");

    for num_pairs in [10, 50, 100, 200].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(num_pairs),
            num_pairs,
            |b, &num_pairs| {
                let mut verifier = SmtAliasVerifier::new();

                // Create pairs of constraints
                let pairs: Vec<(RefId, RefId, PointerConstraint, PointerConstraint)> = (0
                    ..num_pairs)
                    .map(|i| {
                        (
                            RefId(i),
                            RefId(i + 1000),
                            PointerConstraint::stack_alloc(i, 0),
                            PointerConstraint::stack_alloc(i + 1000, 0),
                        )
                    })
                    .collect();

                b.iter(|| {
                    for (ref1, ref2, ptr1, ptr2) in &pairs {
                        let result = verifier.verify_no_alias(
                            black_box(*ref1),
                            black_box(*ref2),
                            black_box(ptr1),
                            black_box(ptr2),
                        );
                        black_box(result);
                    }
                });
            },
        );
    }

    group.finish();
}

// =============================================================================
// Benchmark Group 7: Verifier Configuration Impact
// =============================================================================

fn bench_verifier_configuration(c: &mut Criterion) {
    let mut group = c.benchmark_group("smt_config");

    group.bench_function("default_config", |b| {
        let mut verifier = SmtAliasVerifier::new();
        let ptr1 = PointerConstraint::stack_alloc(1, 0);
        let ptr2 = PointerConstraint::stack_alloc(2, 0);

        b.iter(|| {
            let result = verifier.verify_no_alias(
                black_box(RefId(1)),
                black_box(RefId(2)),
                black_box(&ptr1),
                black_box(&ptr2),
            );
            black_box(result);
        });
    });

    group.bench_function("custom_timeout_50ms", |b| {
        let mut verifier = SmtAliasVerifierBuilder::new().with_timeout(50).build();
        let ptr1 = PointerConstraint::stack_alloc(1, 0);
        let ptr2 = PointerConstraint::stack_alloc(2, 0);

        b.iter(|| {
            let result = verifier.verify_no_alias(
                black_box(RefId(1)),
                black_box(RefId(2)),
                black_box(&ptr1),
                black_box(&ptr2),
            );
            black_box(result);
        });
    });

    group.bench_function("custom_timeout_200ms", |b| {
        let mut verifier = SmtAliasVerifierBuilder::new().with_timeout(200).build();
        let ptr1 = PointerConstraint::stack_alloc(1, 0);
        let ptr2 = PointerConstraint::stack_alloc(2, 0);

        b.iter(|| {
            let result = verifier.verify_no_alias(
                black_box(RefId(1)),
                black_box(RefId(2)),
                black_box(&ptr1),
                black_box(&ptr2),
            );
            black_box(result);
        });
    });

    group.bench_function("small_cache_size_100", |b| {
        let mut verifier = SmtAliasVerifierBuilder::new().with_cache_size(100).build();
        let ptr1 = PointerConstraint::stack_alloc(1, 0);
        let ptr2 = PointerConstraint::stack_alloc(2, 0);

        b.iter(|| {
            let result = verifier.verify_no_alias(
                black_box(RefId(1)),
                black_box(RefId(2)),
                black_box(&ptr1),
                black_box(&ptr2),
            );
            black_box(result);
        });
    });

    group.bench_function("large_cache_size_5000", |b| {
        let mut verifier = SmtAliasVerifierBuilder::new().with_cache_size(5000).build();
        let ptr1 = PointerConstraint::stack_alloc(1, 0);
        let ptr2 = PointerConstraint::stack_alloc(2, 0);

        b.iter(|| {
            let result = verifier.verify_no_alias(
                black_box(RefId(1)),
                black_box(RefId(2)),
                black_box(&ptr1),
                black_box(&ptr2),
            );
            black_box(result);
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_simple_stack_alloc_verification,
    bench_field_access_verification,
    bench_array_access_verification,
    bench_cache_performance,
    bench_complex_pointer_expressions,
    bench_batch_verification,
    bench_verifier_configuration,
);

criterion_main!(benches);
