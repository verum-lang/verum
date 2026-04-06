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
// Tests for postcondition module
// Migrated from src/postcondition.rs per CLAUDE.md standards

use verum_smt::postcondition::*;

use verum_ast::{BinOp, Expr, ExprKind, Ident, Literal, Path, Span};
use verum_common::{Heap, List, Text};

fn make_simple_postcondition() -> Expr {
    // Create: result >= 0
    let left = Expr::path(Path::from_ident(Ident::new(
        Text::from("result"),
        Span::dummy(),
    )));
    let right = Expr::literal(Literal::int(0, Span::dummy()));

    Expr::new(
        ExprKind::Binary {
            op: BinOp::Ge,
            left: Heap::new(left),
            right: Heap::new(right),
        },
        Span::dummy(),
    )
}

#[test]
fn test_references_result_true() {
    let expr = make_simple_postcondition();
    assert!(references_result(&expr));
}

#[test]
fn test_references_result_false() {
    let expr = Expr::literal(Literal::int(42, Span::dummy()));
    assert!(!references_result(&expr));
}

#[test]
fn test_extract_old_calls() {
    // Create: result == old(balance)
    let old_func = Expr::path(Path::from_ident(Ident::new(
        Text::from("old"),
        Span::dummy(),
    )));
    let balance = Expr::path(Path::from_ident(Ident::new(
        Text::from("balance"),
        Span::dummy(),
    )));

    let old_call = Expr::new(
        ExprKind::Call {
            func: Heap::new(old_func),
            type_args: Vec::new().into(),
            args: List::from(vec![balance]),
        },
        Span::dummy(),
    );

    let result = Expr::path(Path::from_ident(Ident::new(
        Text::from("result"),
        Span::dummy(),
    )));

    let expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Eq,
            left: Heap::new(result),
            right: Heap::new(old_call),
        },
        Span::dummy(),
    );

    let old_calls = extract_old_calls(&expr);
    assert_eq!(old_calls.len(), 1);
}

#[test]
fn test_old_value_tracker() {
    let mut tracker = OldValueTracker::new();

    let var = z3::ast::Int::new_const("x");
    let dyn_var = z3::ast::Dynamic::from_ast(&var);

    tracker.capture("x".into(), dyn_var);

    assert!(tracker.contains("x"));
    assert!(tracker.get("x").is_some());
    assert!(!tracker.contains("y"));
}

#[test]
fn test_validate_postcondition() {
    let expr = make_simple_postcondition();
    let result = validate_postcondition(&expr);
    assert!(result.is_ok());
}
