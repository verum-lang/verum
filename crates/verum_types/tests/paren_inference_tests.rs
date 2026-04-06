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
use verum_ast::{expr::*, literal::Literal, span::Span};

use verum_types::infer::*;
use verum_types::ty::Type;

#[test]
fn test_infer_parenthesized_expr() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Test simple parenthesized literal: (42)
    let inner = Expr::literal(Literal::int(42, span));
    let paren_expr = Expr::new(ExprKind::Paren(Box::new(inner)), span);

    let result = checker.synth_expr(&paren_expr).unwrap();
    assert_eq!(result.ty, Type::int());
}

#[test]
fn test_infer_nested_parenthesized_expr() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Test nested parentheses: (((42)))
    let inner = Expr::literal(Literal::int(42, span));
    let paren1 = Expr::new(ExprKind::Paren(Box::new(inner)), span);
    let paren2 = Expr::new(ExprKind::Paren(Box::new(paren1)), span);
    let paren3 = Expr::new(ExprKind::Paren(Box::new(paren2)), span);

    let result = checker.synth_expr(&paren3).unwrap();
    assert_eq!(result.ty, Type::int());
}

#[test]
fn test_infer_parenthesized_binop() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Test (1 + 2) + 3
    let left = Box::new(Expr::literal(Literal::int(1, span)));
    let right = Box::new(Expr::literal(Literal::int(2, span)));
    let inner_binop = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left,
            right,
        },
        span,
    );

    let paren_expr = Expr::new(ExprKind::Paren(Box::new(inner_binop)), span);
    let outer_right = Box::new(Expr::literal(Literal::int(3, span)));
    let outer_expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Box::new(paren_expr),
            right: outer_right,
        },
        span,
    );

    let result = checker.synth_expr(&outer_expr).unwrap();
    assert_eq!(result.ty, Type::int());
}

#[test]
fn test_infer_parenthesized_bool() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Test (true)
    let inner = Expr::literal(Literal::bool(true, span));
    let paren_expr = Expr::new(ExprKind::Paren(Box::new(inner)), span);

    let result = checker.synth_expr(&paren_expr).unwrap();
    assert_eq!(result.ty, Type::bool());
}

#[test]
fn test_infer_parenthesized_comparison() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Test (1 < 2)
    let left = Box::new(Expr::literal(Literal::int(1, span)));
    let right = Box::new(Expr::literal(Literal::int(2, span)));
    let comparison = Expr::new(
        ExprKind::Binary {
            op: BinOp::Lt,
            left,
            right,
        },
        span,
    );

    let paren_expr = Expr::new(ExprKind::Paren(Box::new(comparison)), span);

    let result = checker.synth_expr(&paren_expr).unwrap();
    assert_eq!(result.ty, Type::bool());
}
