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
// Tests for precondition module
// Migrated from src/precondition.rs per CLAUDE.md standards
// FIXED (Session 24): contains_old function made public

use verum_ast::{BinOp, Expr, ExprKind, Ident, Literal, Path, Span};
use verum_common::{Heap, List, Maybe, Text};
use verum_smt::{
    Context, RslClause, RslClauseKind, Translator, assert_precondition, contains_old,
    contains_result, format_precondition_violation, validate_precondition,
};

fn make_simple_expr() -> Expr {
    // Create: x > 0
    let left = Expr::path(Path::from_ident(Ident::new(Text::from("x"), Span::dummy())));
    let right = Expr::literal(Literal::int(0, Span::dummy()));

    Expr::new(
        ExprKind::Binary {
            op: BinOp::Gt,
            left: Heap::new(left),
            right: Heap::new(right),
        },
        Span::dummy(),
    )
}

#[test]
fn test_validate_precondition_valid() {
    let expr = make_simple_expr();
    let result = validate_precondition(&expr);
    assert!(result.is_ok());
}

#[test]
fn test_validate_precondition_with_result() {
    let expr = Expr::path(Path::from_ident(Ident::new(
        Text::from("result"),
        Span::dummy(),
    )));

    let result = validate_precondition(&expr);
    assert!(result.is_err());
}

#[test]
fn test_contains_result_true() {
    let expr = Expr::path(Path::from_ident(Ident::new(
        Text::from("result"),
        Span::dummy(),
    )));
    assert!(contains_result(&expr));
}

#[test]
fn test_contains_result_false() {
    let expr = make_simple_expr();
    assert!(!contains_result(&expr));
}

#[test]
fn test_contains_old_with_call() {
    let old_func = Expr::path(Path::from_ident(Ident::new(
        Text::from("old"),
        Span::dummy(),
    )));
    let arg = Expr::path(Path::from_ident(Ident::new(Text::from("x"), Span::dummy())));

    let expr = Expr::new(
        ExprKind::Call {
            func: Heap::new(old_func),
            type_args: Vec::new().into(),
            args: List::from(vec![arg]),
        },
        Span::dummy(),
    );

    assert!(contains_old(&expr));
}

#[test]
fn test_assert_precondition() {
    let ctx = Context::new();
    let translator = Translator::new(&ctx);
    let solver = ctx.solver();

    let expr = make_simple_expr();
    let clause = RslClause {
        kind: RslClauseKind::Requires,
        expr,
        label: Maybe::None,
        span: Span::dummy(),
    };

    let result = assert_precondition(&translator, &solver, &clause);
    assert!(result.is_ok());
}

#[test]
fn test_format_precondition_violation() {
    let expr = make_simple_expr();
    let clause = RslClause {
        kind: RslClauseKind::Requires,
        expr,
        label: Maybe::None,
        span: Span::dummy(),
    };

    let message = format_precondition_violation(&clause, "test_function");
    assert!(message.as_str().contains("test_function"));
    assert!(message.as_str().contains("Precondition violated"));
}
