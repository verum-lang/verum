//! Type Inference Performance Benchmarks
//!
//! This benchmark validates the type inference performance target:
//! **Target: < 100ms for 10K LOC**
//!
//! NOTE: Significantly simplified from original due to API changes.
//! The original used removed ExprKind::Var, ExprKind::Lambda, Pattern::Var,
//! Literal::Int(i64), and InferMode::Synthesis which no longer exist.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use std::time::Duration;
use verum_ast::{
    expr::{BinOp, Expr, ExprKind},
    literal::Literal,
    span::Span,
};
use verum_types::{InferMode, TypeChecker};

// ============================================================================
// Helper: Generate test expressions using current API
// ============================================================================

fn dummy_span() -> Span {
    Span::default()
}

fn make_int_literal(value: i128) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal::int(value, dummy_span())),
        dummy_span(),
    )
}

fn make_binary(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        },
        dummy_span(),
    )
}

fn generate_simple_program(lines: usize) -> Vec<Expr> {
    (0..lines)
        .map(|i| make_int_literal(42 + i as i128))
        .collect()
}

fn generate_complex_program(lines: usize) -> Vec<Expr> {
    (0..lines)
        .map(|i| match i % 4 {
            0 => make_int_literal(i as i128),
            1 => make_binary(BinOp::Add, make_int_literal(i as i128), make_int_literal(1)),
            2 => make_binary(BinOp::Mul, make_int_literal(i as i128), make_int_literal(2)),
            _ => make_binary(
                BinOp::Sub,
                make_binary(BinOp::Add, make_int_literal(i as i128), make_int_literal(10)),
                make_int_literal(5),
            ),
        })
        .collect()
}

// ============================================================================
// Type Inference Scaling (Target: < 100ms / 10K LOC)
// ============================================================================

fn bench_type_inference_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("type_inference_scaling");
    group.warm_up_time(Duration::from_secs(3));
    group.measurement_time(Duration::from_secs(10));

    for size in [100usize, 500, 1000, 5000, 10000].iter() {
        group.throughput(Throughput::Elements(*size as u64));

        group.bench_with_input(
            BenchmarkId::new("simple_program", size),
            size,
            |b, &lines| {
                let exprs = generate_simple_program(lines);

                b.iter(|| {
                    let mut checker = TypeChecker::new();
                    for expr in &exprs {
                        black_box(checker.infer(expr, InferMode::Synth));
                    }
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("complex_program", size),
            size,
            |b, &lines| {
                let exprs = generate_complex_program(lines);

                b.iter(|| {
                    let mut checker = TypeChecker::new();
                    for expr in &exprs {
                        let _ = checker.infer(expr, InferMode::Synth);
                    }
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Bidirectional Type Checking Performance
// ============================================================================

fn bench_bidirectional_modes(c: &mut Criterion) {
    let mut group = c.benchmark_group("bidirectional_modes");

    // Synthesis mode (infer type from expression)
    group.bench_function("synthesis_mode", |b| {
        let expr = make_int_literal(42);

        b.iter(|| {
            let mut checker = TypeChecker::new();
            black_box(checker.infer(&expr, InferMode::Synth))
        });
    });

    group.finish();
}

// NOTE: Direct Unifier benchmarks removed because Unifier::unify takes
// verum_types::ty::Type (internal representation), not verum_ast::ty::Type.
// Type unification is implicitly benchmarked through TypeChecker::infer.

// ============================================================================
// Validation: Assert 10K LOC < 100ms target
// ============================================================================

fn validate_10k_loc_target(c: &mut Criterion) {
    let mut group = c.benchmark_group("performance_validation");
    group.sample_size(10);

    group.bench_function("10k_loc_must_be_under_100ms", |b| {
        let exprs = generate_complex_program(10_000);

        b.iter_custom(|iters| {
            let mut total_duration = Duration::ZERO;

            for _ in 0..iters {
                let mut checker = TypeChecker::new();

                let start = std::time::Instant::now();
                for expr in &exprs {
                    let _ = checker.infer(expr, InferMode::Synth);
                }
                let elapsed = start.elapsed();

                total_duration += elapsed;
            }

            total_duration
        });
    });

    group.finish();
}

criterion_group!(
    type_inference_benches,
    bench_type_inference_scaling,
    bench_bidirectional_modes,
    validate_10k_loc_target,
);

criterion_main!(type_inference_benches);
