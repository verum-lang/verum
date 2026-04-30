//! Soundness regression: `is_commutative_pair` must restrict to
//! actually commutative operators.
//!
//! Pre-fix the function returned true whenever two expressions shared
//! the SAME `BinOp` and had swapped operands — without checking that
//! the operator is actually commutative. Result: a "commutativity"
//! proof for `5 - 3 = 3 - 5` (FALSE — subtraction isn't commutative)
//! was accepted. Same gap for Div, Imply, Lt/Le/Gt/Ge, Concat,
//! Shl/Shr, In, and the Call arm trusted any 2-arg function.
//!
//! Post-fix: only `Add`, `Mul`, `And`, `Or`, `Eq`, `Ne`, `BitAnd`,
//! `BitOr`, `BitXor` flow through. Call arm now returns false (no
//! special-cased trust).

use verum_ast::expr::{BinOp, Expr, ExprKind};
use verum_ast::literal::Literal;
use verum_ast::span::Span;
use verum_ast::{IntLit, LiteralKind};
use verum_common::Heap;
use verum_smt::proof_term_unified::ProofTerm;
use verum_verification::proof_validator::ProofValidator;

fn int_lit(value: i64) -> Expr {
    Expr::literal(Literal::new(
        LiteralKind::Int(IntLit::new(value as i128)),
        Span::dummy(),
    ))
}

fn binary(op: BinOp, lhs: Expr, rhs: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Heap::new(lhs),
            right: Heap::new(rhs),
        },
        Span::dummy(),
    )
}

fn commutativity_proof(left: Expr, right: Expr) -> ProofTerm {
    ProofTerm::Commutativity { left, right }
}

#[test]
fn commutativity_of_subtraction_is_rejected() {
    // 5 - 3 = 3 - 5 is FALSE. The pre-fix `is_commutative_pair`
    // accepted this because both sides share BinOp::Sub. Post-fix
    // must reject — Sub isn't on the commutative whitelist.
    let mut validator = ProofValidator::new();
    let left = binary(BinOp::Sub, int_lit(5), int_lit(3));
    let right = binary(BinOp::Sub, int_lit(3), int_lit(5));
    let claimed = binary(BinOp::Eq, left.clone(), right.clone());

    let proof = commutativity_proof(left, right);
    let result = validator.validate(&proof, &claimed);
    assert!(
        result.is_err(),
        "5 - 3 = 3 - 5 must NOT validate as commutativity — pre-fix this silently passed"
    );
}

#[test]
fn commutativity_of_division_is_rejected() {
    let mut validator = ProofValidator::new();
    let left = binary(BinOp::Div, int_lit(6), int_lit(2));
    let right = binary(BinOp::Div, int_lit(2), int_lit(6));
    let claimed = binary(BinOp::Eq, left.clone(), right.clone());

    let proof = commutativity_proof(left, right);
    let result = validator.validate(&proof, &claimed);
    assert!(result.is_err(), "Div is not commutative — must reject");
}

#[test]
fn commutativity_of_implication_is_rejected() {
    let mut validator = ProofValidator::new();
    let p = int_lit(1);
    let q = int_lit(0);
    let left = binary(BinOp::Imply, p.clone(), q.clone());
    let right = binary(BinOp::Imply, q, p);
    let claimed = binary(BinOp::Eq, left.clone(), right.clone());

    let proof = commutativity_proof(left, right);
    let result = validator.validate(&proof, &claimed);
    assert!(result.is_err(), "Imply is not commutative — must reject");
}

#[test]
fn commutativity_of_addition_is_accepted() {
    // Positive control: 2 + 3 = 3 + 2 IS valid commutativity.
    let mut validator = ProofValidator::new();
    let left = binary(BinOp::Add, int_lit(2), int_lit(3));
    let right = binary(BinOp::Add, int_lit(3), int_lit(2));
    let claimed = binary(BinOp::Eq, left.clone(), right.clone());

    let proof = commutativity_proof(left, right);
    let result = validator.validate(&proof, &claimed);
    assert!(
        result.is_ok(),
        "Add IS commutative — 2 + 3 = 3 + 2 must validate: {:?}",
        result
    );
}

#[test]
fn commutativity_of_multiplication_is_accepted() {
    let mut validator = ProofValidator::new();
    let left = binary(BinOp::Mul, int_lit(4), int_lit(7));
    let right = binary(BinOp::Mul, int_lit(7), int_lit(4));
    let claimed = binary(BinOp::Eq, left.clone(), right.clone());

    let proof = commutativity_proof(left, right);
    let result = validator.validate(&proof, &claimed);
    assert!(result.is_ok(), "Mul IS commutative: {:?}", result);
}

#[test]
fn commutativity_of_logical_and_is_accepted() {
    let mut validator = ProofValidator::new();
    let p = int_lit(1);
    let q = int_lit(0);
    let left = binary(BinOp::And, p.clone(), q.clone());
    let right = binary(BinOp::And, q, p);
    let claimed = binary(BinOp::Eq, left.clone(), right.clone());

    let proof = commutativity_proof(left, right);
    let result = validator.validate(&proof, &claimed);
    assert!(result.is_ok(), "And IS commutative: {:?}", result);
}
