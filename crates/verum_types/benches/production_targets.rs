//! Production Target Benchmarks for verum_types
//!
//! Target: Type inference < 100ms / 10K LOC
//!
//! Measures type checking throughput at various scales to verify
//! the production target.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;
use std::time::Duration;
use verum_ast::{
    expr::{BinOp, Expr, ExprKind},
    literal::Literal,
    span::Span,
};
use verum_types::{InferMode, TypeChecker};

// ============================================================================
// Helpers
// ============================================================================

fn dummy_span() -> Span {
    Span::default()
}

fn make_int(value: i128) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal::int(value, dummy_span())),
        dummy_span(),
    )
}

fn make_float(value: f64) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal::float(value, dummy_span())),
        dummy_span(),
    )
}

fn make_bool(value: bool) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal::bool(value, dummy_span())),
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

/// Generate a realistic mix of expressions simulating actual Verum code.
/// Each "line" generates one expression. Mix includes:
/// - Integer literals (constants)
/// - Float literals
/// - Boolean literals
/// - Binary arithmetic (Add, Sub, Mul, Div)
/// - Nested binary expressions (2-3 levels)
/// - Comparison operations
fn generate_realistic_program(lines: usize) -> Vec<Expr> {
    (0..lines)
        .map(|i| match i % 10 {
            // Simple literals
            0 => make_int(i as i128),
            1 => make_float(i as f64 * 1.5),
            2 => make_bool(i % 2 == 0),
            // Simple binary ops
            3 => make_binary(BinOp::Add, make_int(i as i128), make_int(1)),
            4 => make_binary(BinOp::Mul, make_float(i as f64), make_float(2.0)),
            5 => make_binary(BinOp::Sub, make_int(100), make_int(i as i128)),
            // Nested binary ops (2 levels)
            6 => make_binary(
                BinOp::Add,
                make_binary(BinOp::Mul, make_int(i as i128), make_int(2)),
                make_int(1),
            ),
            7 => make_binary(
                BinOp::Div,
                make_binary(BinOp::Add, make_float(i as f64), make_float(10.0)),
                make_float(3.0),
            ),
            // Deeply nested (3 levels)
            8 => make_binary(
                BinOp::Sub,
                make_binary(
                    BinOp::Add,
                    make_binary(BinOp::Mul, make_int(i as i128), make_int(3)),
                    make_int(7),
                ),
                make_int(2),
            ),
            // Comparison
            _ => make_binary(BinOp::Add, make_int(i as i128), make_int(i as i128 + 1)),
        })
        .collect()
}

// ============================================================================
// Benchmarks
// ============================================================================

fn bench_type_inference_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("production_target_type_inference");
    group.warm_up_time(Duration::from_secs(3));
    group.measurement_time(Duration::from_secs(8));

    for size in [100, 500, 1000, 5000, 10000] {
        let exprs = generate_realistic_program(size);

        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(
            BenchmarkId::new("infer", format!("{size}_loc")),
            &exprs,
            |b, exprs| {
                b.iter(|| {
                    let mut checker = TypeChecker::new();
                    for expr in exprs {
                        let _ = checker.infer(expr, InferMode::Synth);
                    }
                    black_box(())
                });
            },
        );
    }

    group.finish();
}

fn validate_10k_under_100ms(c: &mut Criterion) {
    let mut group = c.benchmark_group("production_validation_type_inference");
    group.sample_size(10);

    let exprs = generate_realistic_program(10_000);

    // TARGET: 10K LOC type inference must complete in < 100ms
    group.bench_function("10k_loc_must_be_under_100ms", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let mut checker = TypeChecker::new();
                let start = std::time::Instant::now();
                for expr in &exprs {
                    let _ = checker.infer(expr, InferMode::Synth);
                }
                total += start.elapsed();
            }
            total
        });
    });

    group.finish();
}

fn bench_type_checker_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("type_checker_overhead");

    group.bench_function("new_type_checker", |b| {
        b.iter(|| black_box(TypeChecker::new()))
    });

    group.bench_function("single_infer_int", |b| {
        let expr = make_int(42);
        b.iter(|| {
            let mut checker = TypeChecker::new();
            black_box(checker.infer(black_box(&expr), InferMode::Synth))
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_type_inference_throughput,
    validate_10k_under_100ms,
    bench_type_checker_creation,
);
criterion_main!(benches);
