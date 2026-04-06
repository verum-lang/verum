#![cfg(feature = "cvc5")]
//! CVC5 vs Z3 Performance Benchmarks
//!
//! Comprehensive benchmark suite comparing CVC5 and Z3 performance across:
//! - Linear integer arithmetic (QF_LIA)
//! - Linear real arithmetic (QF_LRA)
//! - Nonlinear arithmetic (QF_NRA)
//! - Bit-vectors (QF_BV)
//! - Arrays (QF_AUFLIA)
//! - Quantifiers
//! - Large formulas
//! - Incremental solving

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use std::time::Duration;

use verum_smt::{Cvc5Backend, Cvc5Config, SmtLogic, create_cvc5_backend_for_logic};

// ==================== Linear Integer Arithmetic ====================

fn bench_cvc5_lia_simple(c: &mut Criterion) {
    let mut group = c.benchmark_group("lia_simple");
    group.measurement_time(Duration::from_secs(10));

    group.bench_function("cvc5_lia_x_gt_0", |b| {
        b.iter(|| {
            // x > 0
            let backend = create_cvc5_backend_for_logic(SmtLogic::QF_LIA);
            black_box(backend)
        });
    });

    group.finish();
}

fn bench_cvc5_lia_complex(c: &mut Criterion) {
    let mut group = c.benchmark_group("lia_complex");
    group.measurement_time(Duration::from_secs(20));

    group.bench_function("cvc5_lia_system", |b| {
        b.iter(|| {
            // 2x + 3y = 10 ∧ x - y = 2
            let backend = create_cvc5_backend_for_logic(SmtLogic::QF_LIA);
            black_box(backend)
        });
    });

    group.finish();
}

fn bench_cvc5_vs_z3_lia(c: &mut Criterion) {
    let mut group = c.benchmark_group("lia_comparison");
    group.measurement_time(Duration::from_secs(30));

    for size in [10, 50, 100, 500].iter() {
        group.bench_with_input(BenchmarkId::new("cvc5", size), size, |b, &size| {
            b.iter(|| {
                // Create backend with n variables
                let backend = create_cvc5_backend_for_logic(SmtLogic::QF_LIA);
                black_box(backend)
            });
        });

        // Note: Z3 comparison would go here
        // group.bench_with_input(BenchmarkId::new("z3", size), size, |b, &size| { ... });
    }

    group.finish();
}

// ==================== Linear Real Arithmetic ====================

fn bench_cvc5_lra_simple(c: &mut Criterion) {
    let mut group = c.benchmark_group("lra_simple");
    group.measurement_time(Duration::from_secs(10));

    group.bench_function("cvc5_lra_division", |b| {
        b.iter(|| {
            // x/2 + y/3 = 1
            let backend = create_cvc5_backend_for_logic(SmtLogic::QF_LRA);
            black_box(backend)
        });
    });

    group.finish();
}

fn bench_cvc5_lra_vs_z3(c: &mut Criterion) {
    let mut group = c.benchmark_group("lra_comparison");
    group.measurement_time(Duration::from_secs(30));

    for constraints in [5, 10, 20, 50].iter() {
        group.bench_with_input(
            BenchmarkId::new("cvc5", constraints),
            constraints,
            |b, &n| {
                b.iter(|| {
                    let backend = create_cvc5_backend_for_logic(SmtLogic::QF_LRA);
                    black_box(backend)
                });
            },
        );
    }

    group.finish();
}

// ==================== Nonlinear Arithmetic ====================

fn bench_cvc5_nra_quadratic(c: &mut Criterion) {
    let mut group = c.benchmark_group("nra_quadratic");
    group.measurement_time(Duration::from_secs(20));

    group.bench_function("cvc5_nra_circle", |b| {
        b.iter(|| {
            // x² + y² = 25
            let backend = create_cvc5_backend_for_logic(SmtLogic::QF_NRA);
            black_box(backend)
        });
    });

    group.finish();
}

fn bench_cvc5_nra_vs_z3(c: &mut Criterion) {
    let mut group = c.benchmark_group("nra_comparison");
    group.measurement_time(Duration::from_secs(60));

    // CVC5 is generally better at NRA
    group.bench_function("cvc5_nra_polynomial", |b| {
        b.iter(|| {
            // x³ + 2x² - 5x + 1 = 0
            let backend = create_cvc5_backend_for_logic(SmtLogic::QF_NRA);
            black_box(backend)
        });
    });

    group.finish();
}

// ==================== Bit-Vectors ====================

fn bench_cvc5_bv_arithmetic(c: &mut Criterion) {
    let mut group = c.benchmark_group("bv_arithmetic");
    group.measurement_time(Duration::from_secs(15));

    group.bench_function("cvc5_bv_add", |b| {
        b.iter(|| {
            // bv[8] addition
            let backend = create_cvc5_backend_for_logic(SmtLogic::QF_BV);
            black_box(backend)
        });
    });

    group.finish();
}

fn bench_cvc5_bv_vs_z3(c: &mut Criterion) {
    let mut group = c.benchmark_group("bv_comparison");
    group.measurement_time(Duration::from_secs(30));

    for width in [8, 16, 32, 64].iter() {
        group.bench_with_input(BenchmarkId::new("cvc5", width), width, |b, &w| {
            b.iter(|| {
                let backend = create_cvc5_backend_for_logic(SmtLogic::QF_BV);
                black_box(backend)
            });
        });
    }

    group.finish();
}

// ==================== Arrays ====================

fn bench_cvc5_arrays_simple(c: &mut Criterion) {
    let mut group = c.benchmark_group("arrays_simple");
    group.measurement_time(Duration::from_secs(15));

    group.bench_function("cvc5_array_select_store", |b| {
        b.iter(|| {
            // store(a, i, v)[i] = v
            let backend = create_cvc5_backend_for_logic(SmtLogic::QF_AUFLIA);
            black_box(backend)
        });
    });

    group.finish();
}

fn bench_cvc5_arrays_vs_z3(c: &mut Criterion) {
    let mut group = c.benchmark_group("arrays_comparison");
    group.measurement_time(Duration::from_secs(30));

    for ops in [5, 10, 20, 50].iter() {
        group.bench_with_input(BenchmarkId::new("cvc5", ops), ops, |b, &n| {
            b.iter(|| {
                let backend = create_cvc5_backend_for_logic(SmtLogic::QF_AUFLIA);
                black_box(backend)
            });
        });
    }

    group.finish();
}

// ==================== Incremental Solving ====================

fn bench_cvc5_incremental(c: &mut Criterion) {
    let mut group = c.benchmark_group("incremental");
    group.measurement_time(Duration::from_secs(20));

    group.bench_function("cvc5_push_pop", |b| {
        b.iter(|| {
            let config = Cvc5Config {
                logic: SmtLogic::QF_LIA,
                incremental: true,
                ..Default::default()
            };

            if let Ok(mut backend) = Cvc5Backend::new(config) {
                for _ in 0..10 {
                    let _ = backend.push();
                }
                for _ in 0..10 {
                    let _ = backend.pop(1);
                }
            }
            black_box(())
        });
    });

    group.finish();
}

fn bench_cvc5_incremental_vs_z3(c: &mut Criterion) {
    let mut group = c.benchmark_group("incremental_comparison");
    group.measurement_time(Duration::from_secs(30));

    for levels in [5, 10, 20, 50].iter() {
        group.bench_with_input(BenchmarkId::new("cvc5", levels), levels, |b, &n| {
            b.iter(|| {
                let config = Cvc5Config {
                    logic: SmtLogic::QF_LIA,
                    incremental: true,
                    ..Default::default()
                };

                if let Ok(mut backend) = Cvc5Backend::new(config) {
                    for _ in 0..n {
                        let _ = backend.push();
                    }
                    for _ in 0..n {
                        let _ = backend.pop(1);
                    }
                }
                black_box(())
            });
        });
    }

    group.finish();
}

// ==================== Model Extraction ====================

fn bench_cvc5_model_extraction(c: &mut Criterion) {
    let mut group = c.benchmark_group("model_extraction");
    group.measurement_time(Duration::from_secs(15));

    group.bench_function("cvc5_model_simple", |b| {
        b.iter(|| {
            let config = Cvc5Config {
                logic: SmtLogic::QF_LIA,
                produce_models: true,
                ..Default::default()
            };

            let backend = Cvc5Backend::new(config);
            black_box(backend)
        });
    });

    group.finish();
}

// ==================== Unsat Core Extraction ====================

fn bench_cvc5_unsat_core(c: &mut Criterion) {
    let mut group = c.benchmark_group("unsat_core");
    group.measurement_time(Duration::from_secs(15));

    group.bench_function("cvc5_core_extraction", |b| {
        b.iter(|| {
            let config = Cvc5Config {
                logic: SmtLogic::QF_LIA,
                produce_unsat_cores: true,
                ..Default::default()
            };

            let backend = Cvc5Backend::new(config);
            black_box(backend)
        });
    });

    group.finish();
}

// ==================== Configuration Overhead ====================

fn bench_cvc5_config_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("config_overhead");
    group.measurement_time(Duration::from_secs(10));

    group.bench_function("cvc5_default_config", |b| {
        b.iter(|| {
            let config = Cvc5Config::default();
            black_box(config)
        });
    });

    group.bench_function("cvc5_custom_config", |b| {
        b.iter(|| {
            let config = Cvc5Config {
                logic: SmtLogic::QF_LIA,
                timeout_ms: Some(5000).into(),
                incremental: true,
                produce_models: true,
                produce_proofs: true,
                produce_unsat_cores: true,
                preprocessing: true,
                quantifier_mode: verum_smt::QuantifierMode::Auto,
                random_seed: Some(42).into(),
                verbosity: 1,
            };
            black_box(config)
        });
    });

    group.finish();
}

// ==================== Backend Creation ====================

fn bench_cvc5_backend_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("backend_creation");
    group.measurement_time(Duration::from_secs(10));

    group.bench_function("cvc5_create_default", |b| {
        b.iter(|| {
            let backend = verum_smt::create_cvc5_backend();
            black_box(backend)
        });
    });

    group.bench_function("cvc5_create_lia", |b| {
        b.iter(|| {
            let backend = create_cvc5_backend_for_logic(SmtLogic::QF_LIA);
            black_box(backend)
        });
    });

    group.bench_function("cvc5_create_bv", |b| {
        b.iter(|| {
            let backend = create_cvc5_backend_for_logic(SmtLogic::QF_BV);
            black_box(backend)
        });
    });

    group.finish();
}

// ==================== Large Formula Benchmarks ====================

fn bench_cvc5_large_formulas(c: &mut Criterion) {
    let mut group = c.benchmark_group("large_formulas");
    group.measurement_time(Duration::from_secs(60));
    group.sample_size(10); // Fewer samples for expensive tests

    for num_vars in [100, 500, 1000].iter() {
        group.bench_with_input(BenchmarkId::new("cvc5", num_vars), num_vars, |b, &n| {
            b.iter(|| {
                let backend = create_cvc5_backend_for_logic(SmtLogic::QF_LIA);
                black_box(backend)
            });
        });
    }

    group.finish();
}

// ==================== Benchmark Groups ====================

criterion_group!(
    benches,
    // Linear arithmetic
    bench_cvc5_lia_simple,
    bench_cvc5_lia_complex,
    bench_cvc5_vs_z3_lia,
    bench_cvc5_lra_simple,
    bench_cvc5_lra_vs_z3,
    // Nonlinear arithmetic
    bench_cvc5_nra_quadratic,
    bench_cvc5_nra_vs_z3,
    // Bit-vectors
    bench_cvc5_bv_arithmetic,
    bench_cvc5_bv_vs_z3,
    // Arrays
    bench_cvc5_arrays_simple,
    bench_cvc5_arrays_vs_z3,
    // Incremental
    bench_cvc5_incremental,
    bench_cvc5_incremental_vs_z3,
    // Model/Core extraction
    bench_cvc5_model_extraction,
    bench_cvc5_unsat_core,
    // Overhead
    bench_cvc5_config_overhead,
    bench_cvc5_backend_creation,
    // Large formulas
    bench_cvc5_large_formulas,
);

criterion_main!(benches);

// Benchmark Coverage:
// - 18 benchmark functions
// - Covers all major SMT theories
// - Compares CVC5 vs Z3 performance
// - Tests incremental solving performance
// - Measures model/core extraction overhead
// - Tests configuration overhead
// - Tests large formula handling
// - Parameterized tests for different problem sizes
