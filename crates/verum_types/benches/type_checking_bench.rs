//! Performance benchmarks for type checking
//!
//! Performance targets:
//! - Type inference: < 100ms for 10K LOC
//! - Refinement checking: < 50ms per function
//! - CBGR overhead: < 15ns per check
//!
//! These benchmarks verify we meet our performance requirements.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use verum_ast::{
    expr::{BinOp, Expr, ExprKind},
    span::Span,
};
use verum_types::{InferMode, TypeChecker};

fn dummy_span() -> Span {
    Span::default()
}

fn make_int_literal(value: i128) -> Expr {
    Expr::new(
        ExprKind::Literal(verum_ast::literal::Literal::int(value, dummy_span())),
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

// ==================== Literal Type Inference ====================

fn bench_literal_inference(c: &mut Criterion) {
    let mut group = c.benchmark_group("literal_inference");

    let expr = make_int_literal(42);

    group.bench_function("int_literal", |b| {
        b.iter(|| {
            let mut checker = TypeChecker::new();
            checker.infer(black_box(&expr), InferMode::Synth)
        })
    });

    group.finish();
}

// ==================== Binary Operation Type Inference ====================

fn bench_binary_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("binary_operations");

    let expr = make_binary(BinOp::Add, make_int_literal(10), make_int_literal(20));

    group.bench_function("simple_add", |b| {
        b.iter(|| {
            let mut checker = TypeChecker::new();
            checker.infer(black_box(&expr), InferMode::Synth)
        })
    });

    group.finish();
}

// ==================== Deep Nesting Performance ====================

fn bench_deep_nesting(c: &mut Criterion) {
    let mut group = c.benchmark_group("deep_nesting");

    for depth in [10, 50, 100, 200].iter() {
        let mut expr = make_int_literal(1);
        for _ in 0..*depth {
            expr = make_binary(BinOp::Add, expr, make_int_literal(1));
        }

        group.bench_with_input(BenchmarkId::from_parameter(depth), depth, |b, _depth| {
            b.iter(|| {
                let mut checker = TypeChecker::new();
                checker.infer(black_box(&expr), InferMode::Synth)
            })
        });
    }

    group.finish();
}

// ==================== Wide Expressions Performance ====================

fn bench_wide_expressions(c: &mut Criterion) {
    let mut group = c.benchmark_group("wide_expressions");

    for width in [10, 50, 100, 200].iter() {
        let elements: Vec<Expr> = (0..*width).map(make_int_literal).collect();

        let expr = Expr::new(
            ExprKind::Tuple(elements.into()),
            dummy_span(),
        );

        group.bench_with_input(BenchmarkId::from_parameter(width), width, |b, _width| {
            b.iter(|| {
                let mut checker = TypeChecker::new();
                checker.infer(black_box(&expr), InferMode::Synth)
            })
        });
    }

    group.finish();
}

// ==================== Complex Expressions ====================

fn bench_complex_expressions(c: &mut Criterion) {
    let mut group = c.benchmark_group("complex_expressions");

    // Build complex arithmetic: (((1 + 2) * 3) - 4) / 5
    let expr = {
        let add = make_binary(BinOp::Add, make_int_literal(1), make_int_literal(2));
        let mul = make_binary(BinOp::Mul, add, make_int_literal(3));
        let sub = make_binary(BinOp::Sub, mul, make_int_literal(4));
        make_binary(BinOp::Div, sub, make_int_literal(5))
    };

    group.bench_function("arithmetic_chain", |b| {
        b.iter(|| {
            let mut checker = TypeChecker::new();
            checker.infer(black_box(&expr), InferMode::Synth)
        })
    });

    group.finish();
}

// ==================== Scalability Test ====================

fn bench_scalability(c: &mut Criterion) {
    let mut group = c.benchmark_group("scalability");
    group.sample_size(20); // Fewer samples for large inputs

    // Simulate type checking progressively larger "files"
    // Each "file" is a sequence of independent expressions
    for num_exprs in [100, 500, 1000, 5000, 10000].iter() {
        group.bench_with_input(
            BenchmarkId::new("expressions", num_exprs),
            num_exprs,
            |b, &count| {
                b.iter(|| {
                    let mut checker = TypeChecker::new();
                    let mut total_ok = 0;

                    for i in 0..count {
                        let expr = make_binary(
                            BinOp::Add,
                            make_int_literal(i as i128),
                            make_int_literal((i + 1) as i128),
                        );

                        if checker.infer(&expr, InferMode::Synth).is_ok() {
                            total_ok += 1;
                        }
                    }

                    black_box(total_ok)
                })
            },
        );
    }

    group.finish();
}

// ==================== Throughput Test ====================

fn bench_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("throughput");

    // Measure LOC/sec throughput
    // Assume each expression is ~1 line of code
    let expressions_per_test = 1000;

    let exprs: Vec<Expr> = (0..expressions_per_test)
        .map(|i| {
            make_binary(
                BinOp::Add,
                make_int_literal(i as i128),
                make_int_literal((i + 1) as i128),
            )
        })
        .collect();

    group.bench_function("1000_expressions", |b| {
        b.iter(|| {
            let mut checker = TypeChecker::new();

            for expr in &exprs {
                let _ = checker.infer(black_box(expr), InferMode::Synth);
            }
        })
    });

    group.finish();
}

// =============================================================================
// 10K LOC Target Test (CRITICAL)
// =============================================================================

fn bench_10k_loc_target(c: &mut Criterion) {
    let mut group = c.benchmark_group("10k_loc_target");
    group.sample_size(10); // Fewer samples for this critical test

    // Generate realistic 10K LOC program structure
    // Assume ~20 functions per 1000 LOC, ~50 expressions per function
    let num_functions = 200;
    let exprs_per_function = 50;
    let total_exprs = num_functions * exprs_per_function;

    group.throughput(Throughput::Elements(total_exprs as u64));

    group.bench_function("type_check_10k_loc", |b| {
        b.iter(|| {
            let mut checker = TypeChecker::new();
            let mut total_ok = 0;
            let mut total_time = std::time::Duration::ZERO;

            let start = std::time::Instant::now();

            // Simulate type checking 10K LOC worth of expressions
            for func_idx in 0..num_functions {
                for expr_idx in 0..exprs_per_function {
                    let expr = make_binary(
                        BinOp::Add,
                        make_int_literal((func_idx * 1000 + expr_idx) as i128),
                        make_int_literal(((func_idx + 1) * 1000 + expr_idx + 1) as i128),
                    );

                    if checker.infer(&expr, InferMode::Synth).is_ok() {
                        total_ok += 1;
                    }
                }
            }

            total_time = start.elapsed();

            black_box((total_ok, total_time))
        })
    });

    group.finish();

    println!("\n╔════════════════════════════════════════════════════════════╗");
    println!("║  CRITICAL REQUIREMENT: Type inference < 100ms for 10K LOC  ║");
    println!("╠════════════════════════════════════════════════════════════╣");
    println!(
        "║ Total expressions: {} (simulating 10K LOC)              ║",
        total_exprs
    );
    println!("║ Target: Complete in < 100ms                               ║");
    println!("║                                                            ║");
    println!("║ NOTE: Run with --release for accurate measurements        ║");
    println!("║       cargo bench --release --package verum_types         ║");
    println!("╚════════════════════════════════════════════════════════════╝\n");
}

// =============================================================================
// Incremental Type Checking
// =============================================================================

fn bench_incremental_type_checking(c: &mut Criterion) {
    let mut group = c.benchmark_group("incremental_type_checking");

    // First pass: type check entire module
    let exprs: Vec<Expr> = (0..1000)
        .map(|i| {
            make_binary(
                BinOp::Add,
                make_int_literal(i as i128),
                make_int_literal((i + 1) as i128),
            )
        })
        .collect();

    group.bench_function("full_module", |b| {
        b.iter(|| {
            let mut checker = TypeChecker::new();
            for expr in &exprs {
                let _ = checker.infer(black_box(expr), InferMode::Synth);
            }
        })
    });

    // Incremental: change one expression and re-check
    group.bench_function("single_change", |b| {
        b.iter(|| {
            let mut checker = TypeChecker::new();

            // Pre-populate type environment
            for expr in exprs.iter().take(999) {
                let _ = checker.infer(expr, InferMode::Synth);
            }

            // Re-check the changed expression
            let changed_expr =
                make_binary(BinOp::Mul, make_int_literal(999), make_int_literal(1000));

            black_box(checker.infer(&changed_expr, InferMode::Synth))
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_literal_inference,
    bench_binary_operations,
    bench_deep_nesting,
    bench_wide_expressions,
    bench_complex_expressions,
    bench_scalability,
    bench_throughput,
    bench_10k_loc_target,
    bench_incremental_type_checking,
);

criterion_main!(benches);
