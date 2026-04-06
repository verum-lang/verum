//! Performance benchmarks for SMT solving
//!
//! Performance targets from CLAUDE.md:
//! - SMT queries: < 10ms average
//! - Refinement checking: < 50ms per function
//!
//! NOTE: Rewritten to use current verify_refinement API.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use std::time::Duration;

use verum_ast::{
    expr::{BinOp, Expr, ExprKind},
    literal::Literal,
    span::Span,
    ty::{Ident, Path, PathSegment, RefinementPredicate, Type, TypeKind},
};
use verum_common::{Heap, Maybe};
use verum_smt::{Context, VerifyMode, verify_refinement};

fn make_int_literal(value: i64) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal::int(value as i128, Span::default())),
        Span::default(),
    )
}

fn make_var(name: &str) -> Expr {
    let ident = Ident::new(name.to_string(), Span::default());
    let segment = PathSegment::Name(ident);
    let path = Path {
        segments: vec![segment].into(),
        span: Span::default(),
    };
    Expr::new(ExprKind::Path(path), Span::default())
}

fn make_binary(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        },
        Span::default(),
    )
}

fn make_refinement_type(predicate: Expr) -> Type {
    let base = Type::new(TypeKind::Int, Span::default());
    Type::new(
        TypeKind::Refined {
            base: Heap::new(base),
            predicate: Heap::new(RefinementPredicate {
                expr: predicate,
                binding: Maybe::None,
                span: Span::default(),
            }),
        },
        Span::default(),
    )
}

// ==================== Simple Query Benchmarks ====================

fn bench_simple_queries(c: &mut Criterion) {
    let mut group = c.benchmark_group("simple_queries");
    group.measurement_time(Duration::from_secs(10));

    let ctx = Context::new();

    let pred = make_binary(BinOp::Gt, make_var("it"), make_int_literal(0));
    let ty = make_refinement_type(pred);

    group.bench_function("simple_comparison", |b| {
        b.iter(|| verify_refinement(black_box(&ctx), black_box(&ty), None, VerifyMode::Proof))
    });

    let lower = make_binary(BinOp::Ge, make_var("it"), make_int_literal(0));
    let upper = make_binary(BinOp::Lt, make_var("it"), make_int_literal(100));
    let pred = make_binary(BinOp::And, lower, upper);
    let ty = make_refinement_type(pred);

    group.bench_function("bounded_check", |b| {
        b.iter(|| verify_refinement(black_box(&ctx), black_box(&ty), None, VerifyMode::Proof))
    });

    group.finish();
}

// ==================== Complex Formula Benchmarks ====================

fn bench_complex_formulas(c: &mut Criterion) {
    let mut group = c.benchmark_group("complex_formulas");
    group.measurement_time(Duration::from_secs(10));

    let ctx = Context::new();

    for num_conjuncts in [2, 5, 10].iter() {
        let mut expr = make_binary(BinOp::Gt, make_var("it"), make_int_literal(0));
        for i in 1..*num_conjuncts {
            let constraint = make_binary(BinOp::Gt, make_var("it"), make_int_literal(i));
            expr = make_binary(BinOp::And, expr, constraint);
        }
        let ty = make_refinement_type(expr);

        group.bench_with_input(
            BenchmarkId::new("conjunctions", num_conjuncts),
            num_conjuncts,
            |b, _| b.iter(|| verify_refinement(black_box(&ctx), black_box(&ty), None, VerifyMode::Proof)),
        );
    }

    group.finish();
}

// ==================== Throughput Benchmarks ====================

fn bench_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("throughput");
    group.sample_size(50);
    group.measurement_time(Duration::from_secs(10));

    let ctx = Context::new();

    let queries: Vec<Type> = (0..100)
        .map(|i| {
            let pred = make_binary(BinOp::Gt, make_var("it"), make_int_literal(i));
            make_refinement_type(pred)
        })
        .collect();

    group.bench_function("100_queries", |b| {
        b.iter(|| {
            for ty in &queries {
                let _ = verify_refinement(black_box(&ctx), black_box(ty), None, VerifyMode::Proof);
            }
        })
    });

    group.finish();
}

// ==================== Average Time Test ====================

fn bench_average_time(c: &mut Criterion) {
    let mut group = c.benchmark_group("average_time");
    group.significance_level(0.05).sample_size(100);
    group.measurement_time(Duration::from_secs(10));

    let ctx = Context::new();

    let pred = make_binary(BinOp::Gt, make_var("it"), make_int_literal(0));
    let ty = make_refinement_type(pred);

    group.bench_function("target_10ms_average", |b| {
        b.iter(|| verify_refinement(black_box(&ctx), black_box(&ty), None, VerifyMode::Proof))
    });

    group.finish();
}

// ==================== Refinement Function Benchmark ====================

fn bench_refinement_function(c: &mut Criterion) {
    let mut group = c.benchmark_group("refinement_function");
    group.sample_size(50);
    group.measurement_time(Duration::from_secs(10));

    let ctx = Context::new();

    group.bench_function("typical_function", |b| {
        let types: Vec<Type> = (0..5)
            .map(|_| {
                let pred = make_binary(BinOp::Gt, make_var("it"), make_int_literal(0));
                make_refinement_type(pred)
            })
            .collect();

        b.iter(|| {
            for ty in &types {
                let _ = verify_refinement(black_box(&ctx), black_box(ty), None, VerifyMode::Proof);
            }
        })
    });

    group.bench_function("complex_function", |b| {
        let types: Vec<Type> = (0..20)
            .map(|_| {
                let lower = make_binary(BinOp::Ge, make_var("it"), make_int_literal(0));
                let upper = make_binary(BinOp::Le, make_var("it"), make_int_literal(100));
                let pred = make_binary(BinOp::And, lower, upper);
                make_refinement_type(pred)
            })
            .collect();

        b.iter(|| {
            for ty in &types {
                let _ = verify_refinement(black_box(&ctx), black_box(ty), None, VerifyMode::Proof);
            }
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_simple_queries,
    bench_complex_formulas,
    bench_throughput,
    bench_average_time,
    bench_refinement_function
);

criterion_main!(benches);
