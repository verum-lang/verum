//! SMT Solver Performance Benchmarks
//!
//! NOTE: Rewritten to use current API (verify_refinement, Translator).

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
use verum_smt::{Context, VerifyMode, verify_refinement, clear_cache};

fn create_int_literal(value: i64) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal::int(value as i128, Span::default())),
        Span::default(),
    )
}

fn create_var(name: &str) -> Expr {
    let ident = Ident::new(name.to_string(), Span::default());
    let segment = PathSegment::Name(ident);
    let path = Path {
        segments: vec![segment].into(),
        span: Span::default(),
    };
    Expr::new(ExprKind::Path(path), Span::default())
}

fn create_binary(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        },
        Span::default(),
    )
}

fn create_refinement_type(predicate: Expr) -> Type {
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

fn bench_simple_refinements(c: &mut Criterion) {
    let mut group = c.benchmark_group("simple_refinements");
    group.measurement_time(Duration::from_secs(10));

    let ctx = Context::new();

    let pred = create_binary(BinOp::Gt, create_var("it"), create_int_literal(0));
    let ty = create_refinement_type(pred);

    group.bench_function("x_gt_0", |b| {
        b.iter(|| verify_refinement(black_box(&ctx), black_box(&ty), None, VerifyMode::Proof))
    });

    let pred = create_binary(BinOp::Ge, create_var("it"), create_int_literal(1));
    let ty = create_refinement_type(pred);

    group.bench_function("x_ge_1", |b| {
        b.iter(|| verify_refinement(black_box(&ctx), black_box(&ty), None, VerifyMode::Proof))
    });

    let pred = create_binary(BinOp::Ne, create_var("it"), create_int_literal(0));
    let ty = create_refinement_type(pred);

    group.bench_function("x_ne_0", |b| {
        b.iter(|| verify_refinement(black_box(&ctx), black_box(&ty), None, VerifyMode::Proof))
    });

    group.finish();
}

fn bench_complex_refinements(c: &mut Criterion) {
    let mut group = c.benchmark_group("complex_refinements");
    group.measurement_time(Duration::from_secs(15));

    let ctx = Context::new();

    let left = create_binary(BinOp::Gt, create_var("it"), create_int_literal(0));
    let right = create_binary(BinOp::Lt, create_var("it"), create_int_literal(100));
    let pred = create_binary(BinOp::And, left, right);
    let ty = create_refinement_type(pred);

    group.bench_function("bounded_0_to_100", |b| {
        b.iter(|| verify_refinement(black_box(&ctx), black_box(&ty), None, VerifyMode::Proof))
    });

    let left = create_binary(BinOp::Ge, create_var("it"), create_int_literal(0));
    let right = create_binary(BinOp::Le, create_var("it"), create_int_literal(255));
    let pred = create_binary(BinOp::And, left, right);
    let ty = create_refinement_type(pred);

    group.bench_function("byte_range", |b| {
        b.iter(|| verify_refinement(black_box(&ctx), black_box(&ty), None, VerifyMode::Proof))
    });

    group.finish();
}

fn bench_verification_modes(c: &mut Criterion) {
    let mut group = c.benchmark_group("verification_modes");
    group.measurement_time(Duration::from_secs(10));

    let ctx = Context::new();

    let pred = create_binary(BinOp::Gt, create_var("it"), create_int_literal(0));
    let ty = create_refinement_type(pred);

    for mode in [VerifyMode::Runtime, VerifyMode::Proof, VerifyMode::Auto] {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{:?}", mode)),
            &mode,
            |b, &mode| b.iter(|| verify_refinement(black_box(&ctx), black_box(&ty), None, mode)),
        );
    }

    group.finish();
}

fn bench_cache_performance(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache");
    group.measurement_time(Duration::from_secs(10));

    let ctx = Context::new();

    let pred = create_binary(BinOp::Gt, create_var("it"), create_int_literal(0));
    let ty = create_refinement_type(pred);

    group.bench_function("first_check_cache_miss", |b| {
        b.iter(|| {
            clear_cache();
            verify_refinement(black_box(&ctx), black_box(&ty), None, VerifyMode::Proof)
        })
    });

    let _ = verify_refinement(&ctx, &ty, None, VerifyMode::Proof);

    group.bench_function("second_check_cache_hit", |b| {
        b.iter(|| verify_refinement(black_box(&ctx), black_box(&ty), None, VerifyMode::Proof))
    });

    group.finish();
}

fn bench_translation(c: &mut Criterion) {
    let mut group = c.benchmark_group("translation");
    group.measurement_time(Duration::from_secs(10));

    let ctx = Context::new();
    let translator = verum_smt::Translator::new(&ctx);

    let simple_expr = create_binary(BinOp::Gt, create_var("x"), create_int_literal(0));

    group.bench_function("translate_simple", |b| {
        b.iter(|| translator.translate_expr(black_box(&simple_expr)))
    });

    let left = create_binary(BinOp::Gt, create_var("x"), create_int_literal(0));
    let right = create_binary(BinOp::Lt, create_var("x"), create_int_literal(100));
    let complex_expr = create_binary(BinOp::And, left, right);

    group.bench_function("translate_complex", |b| {
        b.iter(|| translator.translate_expr(black_box(&complex_expr)))
    });

    group.finish();
}

criterion_group!(
    name = benches;
    config = Criterion::default()
        .sample_size(100)
        .warm_up_time(Duration::from_secs(3));
    targets =
        bench_simple_refinements,
        bench_complex_refinements,
        bench_verification_modes,
        bench_cache_performance,
        bench_translation
);

criterion_main!(benches);
