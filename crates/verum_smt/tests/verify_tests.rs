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
// Unit tests for verify.rs
//
// Migrated from src/verify.rs to comply with CLAUDE.md test organization.
#![allow(unexpected_cfgs)]

use verum_common::Heap;
use verum_smt::verify::*;

use std::time::Duration;
use verum_ast::{BinOp, Expr, ExprKind, Literal, Span, Type};
use verum_common::{List, Text};

#[test]
fn test_verify_mode_default() {
    assert_eq!(VerifyMode::default(), VerifyMode::Auto);
}

#[test]
fn test_estimate_complexity() {
    let span = Span::dummy();

    // Simple literal
    let simple = Expr::literal(Literal::int(42, span));
    assert!(estimate_expr_complexity(&simple) < 5);

    // Binary operation
    let left = Heap::new(Expr::literal(Literal::int(1, span)));
    let right = Heap::new(Expr::literal(Literal::int(2, span)));
    let binary = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left,
            right,
        },
        span,
    );
    assert!(estimate_expr_complexity(&binary) < 10);
}

#[test]
fn test_auto_mode() {
    let span = Span::dummy();
    let simple_type = Type::int(span);

    // Simple types should use Proof mode
    let mode = auto_mode(&simple_type);
    assert!(matches!(mode, VerifyMode::Proof | VerifyMode::Auto));
}

#[test]
fn test_proof_result() {
    let cost = VerificationCost::new("test".into(), Duration::from_millis(100), true);

    let result = ProofResult::new(cost.clone())
        .with_cached()
        .with_smt_lib("(assert true)".into());

    assert!(result.cached);
    assert!(result.smt_lib.is_some());
}

#[test]
fn test_verification_error_cost() {
    let cost = VerificationCost::new("test".into(), Duration::from_secs(1), false);

    let err = VerificationError::CannotProve {
        constraint: "x > 0".into(),
        counterexample: None,
        cost: cost.clone(),
        suggestions: List::from(vec![Text::from("Add precondition")]),
    };

    assert!(err.cost().is_some());
    assert_eq!(err.suggestions().len(), 1);
}
