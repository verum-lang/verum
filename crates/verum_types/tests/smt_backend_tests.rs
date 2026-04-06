#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
use verum_ast::expr::{BinOp, Expr, ExprKind};
use verum_ast::literal::Literal;
use verum_ast::span::Span;
use verum_common::Heap;
use verum_types::refinement::SmtBackend;
use verum_types::smt_backend::*;

fn make_bool(b: bool) -> Expr {
    Expr::literal(Literal::bool(b, Span::dummy()))
}

fn make_int(n: i64) -> Expr {
    Expr::literal(Literal::int(n as i128, Span::dummy()))
}

fn make_binary(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        },
        Span::dummy(),
    )
}

fn make_var(name: &str) -> Expr {
    use smallvec::smallvec;
    use verum_ast::Ident;
    use verum_ast::ty::{Path, PathSegment};

    let ident = Ident::new(name, Span::dummy());
    let path = Path {
        segments: smallvec![PathSegment::Name(ident)],
        span: Span::dummy(),
    };

    Expr::new(ExprKind::Path(path), Span::dummy())
}

#[test]
fn test_backend_creation() {
    let backend = Z3Backend::new();
    let stats = backend.stats();
    assert_eq!(stats.total_queries, 0);
}

#[test]
fn test_trivial_check() {
    let mut backend = Z3Backend::new();
    let true_expr = make_bool(true);

    let result = backend.check(&true_expr);
    assert!(result.is_ok());

    let stats = backend.stats();
    assert_eq!(stats.total_queries, 1);
}

#[test]
fn test_comparison_subsumption() {
    // Test: x > 10 implies x > 0
    let x = make_var("x");
    let ten = make_int(10);
    let zero = make_int(0);

    let x_gt_10 = make_binary(BinOp::Gt, x.clone(), ten);
    let x_gt_0 = make_binary(BinOp::Gt, x.clone(), zero);

    let result = check_subsumption_smt(&x_gt_10, &x_gt_0, 100);
    assert!(result.is_ok());
    assert!(result.unwrap()); // Should be valid
}

#[test]
fn test_invalid_subsumption() {
    // Test: x > 0 does NOT imply x > 10
    let x = make_var("x");
    let ten = make_int(10);
    let zero = make_int(0);

    let x_gt_0 = make_binary(BinOp::Gt, x.clone(), zero);
    let x_gt_10 = make_binary(BinOp::Gt, x.clone(), ten);

    let result = check_subsumption_smt(&x_gt_0, &x_gt_10, 100);
    assert!(result.is_ok());
    assert!(!result.unwrap()); // Should be invalid
}

#[test]
fn test_stats_tracking() {
    let mut backend = Z3Backend::new();

    // Use a more complex expression that will take measurable time
    let x = make_var("x");
    let y = make_var("y");
    let ten = make_int(10);
    let zero = make_int(0);

    let x_gt_10 = make_binary(BinOp::Gt, x.clone(), ten);
    let y_gt_0 = make_binary(BinOp::Gt, y.clone(), zero);
    let expr = make_binary(BinOp::And, x_gt_10, y_gt_0);

    let _ = backend.check(&expr);
    let _ = backend.check(&expr);

    let stats = backend.stats();
    assert_eq!(stats.total_queries, 2);
    // Note: Time tracking may still be 0ms for very fast operations
    // This is acceptable as the subsumption checker is optimized
}
