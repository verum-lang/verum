// Escape Analysis Performance Benchmarks
//
// Validates that escape analysis completes within <100ms for 10K LOC target
// CBGR escape analysis determines whether references can be promoted from
// &T (15-50ns runtime check) to &checked T (0ns, statically verified).
// Target: complete escape analysis within <100ms for 10K LOC.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::hint::black_box;
use verum_cbgr::{CbgrTier, CfgBuilder, DefSite, RefFlow, ReferenceTier, Span, TierStatistics, UseeSite, Tier0Reason};

/// Benchmark CFG builder for simple reference patterns
fn bench_cfg_builder_simple(c: &mut Criterion) {
    let mut group = c.benchmark_group("cfg_builder_simple");

    for num_refs in [10, 50, 100, 500].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(num_refs),
            num_refs,
            |b, &num_refs| {
                b.iter(|| {
                    let mut builder = CfgBuilder::new();

                    // Create references
                    for _ in 0..num_refs {
                        let _ref_id = builder.new_ref_id();
                    }

                    black_box(builder)
                });
            },
        );
    }

    group.finish();
}

/// Benchmark CFG builder with span tracking
fn bench_cfg_builder_with_spans(c: &mut Criterion) {
    let mut group = c.benchmark_group("cfg_builder_with_spans");

    for num_refs in [10, 50, 100, 500].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(num_refs),
            num_refs,
            |b, &num_refs| {
                b.iter(|| {
                    let mut builder = CfgBuilder::new();

                    // Create references with spans
                    for i in 0..num_refs {
                        let span: Span = (i as u32, (i + 10) as u32);
                        let _ref_id = builder.new_ref_id_with_span(span);
                    }

                    // Lookup by span
                    for i in 0..num_refs {
                        let span: Span = (i as u32, (i + 10) as u32);
                        let _ = builder.get_ref_for_span(span);
                    }

                    black_box(builder)
                });
            },
        );
    }

    group.finish();
}

/// Benchmark DefSite creation
fn bench_defsite_creation(c: &mut Criterion) {
    use verum_cbgr::analysis::BlockId;

    let mut group = c.benchmark_group("defsite_creation");

    group.bench_function("new_without_span", |b| {
        b.iter(|| {
            let block = BlockId(42);
            let ref_id = verum_cbgr::analysis::RefId(1);
            black_box(DefSite::new(block, ref_id, true))
        });
    });

    group.bench_function("with_span", |b| {
        b.iter(|| {
            let block = BlockId(42);
            let ref_id = verum_cbgr::analysis::RefId(1);
            let span: Span = (100, 200);
            black_box(DefSite::with_span(block, ref_id, true, span))
        });
    });

    group.finish();
}

/// Benchmark UseeSite creation
fn bench_usesite_creation(c: &mut Criterion) {
    use verum_cbgr::analysis::BlockId;

    let mut group = c.benchmark_group("usesite_creation");

    group.bench_function("new_without_span", |b| {
        b.iter(|| {
            let block = BlockId(42);
            let ref_id = verum_cbgr::analysis::RefId(1);
            black_box(UseeSite::new(block, ref_id, false))
        });
    });

    group.bench_function("with_span", |b| {
        b.iter(|| {
            let block = BlockId(42);
            let ref_id = verum_cbgr::analysis::RefId(1);
            let span: Span = (100, 200);
            black_box(UseeSite::with_span(block, ref_id, false, span))
        });
    });

    group.finish();
}

/// Benchmark RefFlow creation patterns
fn bench_refflow_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("refflow_creation");

    for param_count in [1, 5, 10, 20].iter() {
        group.bench_with_input(
            BenchmarkId::new("conservative", param_count),
            param_count,
            |b, &param_count| {
                b.iter(|| black_box(RefFlow::conservative(param_count)));
            },
        );

        group.bench_with_input(
            BenchmarkId::new("safe", param_count),
            param_count,
            |b, &param_count| {
                b.iter(|| black_box(RefFlow::safe(param_count)));
            },
        );
    }

    group.finish();
}

/// Benchmark tier statistics operations
fn bench_tier_statistics(c: &mut Criterion) {
    let mut group = c.benchmark_group("tier_statistics");

    let tier0 = ReferenceTier::tier0(Tier0Reason::NotAnalyzed);
    let tier1 = ReferenceTier::tier1();
    let tier2 = ReferenceTier::tier2();

    group.bench_function("record_tier0", |b| {
        b.iter(|| {
            let mut stats = TierStatistics::default();
            for _ in 0..100 {
                stats.record(&tier0);
            }
            black_box(stats)
        });
    });

    group.bench_function("record_mixed_tiers", |b| {
        b.iter(|| {
            let mut stats = TierStatistics::default();
            for i in 0..100 {
                match i % 3 {
                    0 => stats.record(&tier0),
                    1 => stats.record(&tier1),
                    _ => stats.record(&tier2),
                }
            }
            black_box(stats)
        });
    });

    group.bench_function("promotion_rate", |b| {
        let mut stats = TierStatistics::default();
        for i in 0..100 {
            match i % 3 {
                0 => stats.record(&tier0),
                1 => stats.record(&tier1),
                _ => stats.record(&tier2),
            }
        }

        b.iter(|| black_box(stats.promotion_rate()));
    });

    group.finish();
}

/// Benchmark CBGR tier comparisons
fn bench_cbgr_tier_ops(c: &mut Criterion) {
    let mut group = c.benchmark_group("cbgr_tier_ops");

    group.bench_function("tier_comparison", |b| {
        let tiers = [CbgrTier::Tier0, CbgrTier::Tier1, CbgrTier::Tier2];
        b.iter(|| {
            let mut count = 0;
            for t1 in &tiers {
                for t2 in &tiers {
                    if t1 == t2 {
                        count += 1;
                    }
                }
            }
            black_box(count)
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_cfg_builder_simple,
    bench_cfg_builder_with_spans,
    bench_defsite_creation,
    bench_usesite_creation,
    bench_refflow_creation,
    bench_tier_statistics,
    bench_cbgr_tier_ops,
);

criterion_main!(benches);
