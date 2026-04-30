//! Soundness regression: `validate_hypothesis` must check that the
//! hypothesis at `h{id}` actually has the claimed proposition, not
//! just that the user-supplied `formula` happens to match `expected`.
//!
//! Pre-fix the validator checked only:
//! 1. `formula == expected` (user-provided formula matches the claim)
//! 2. Hypothesis name `h{id}` exists in scope
//!
//! It DID NOT check that `h{id}` carries `formula` as its actual
//! content. So a hypothesis `h0 : P` could be re-labeled by the user
//! as `Hypothesis { id: 0, formula: Q }` whenever `Q == expected`
//! syntactically, and the validator silently accepted — claiming
//! `h0` proves `Q` even though it actually proves `P`.

use verum_ast::expr::{BinOp, Expr, ExprKind};
use verum_ast::literal::Literal;
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_ast::{IntLit, LiteralKind};
use verum_common::Heap;
use verum_smt::proof_term_unified::ProofTerm;
use verum_verification::proof_validator::ProofValidator;

fn ident_expr(name: &str) -> Expr {
    Expr::path(Path::single(Ident::new(name, Span::dummy())))
}

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

#[test]
fn hypothesis_with_matching_proposition_validates() {
    // h0 : (x > 0). Proof: Hypothesis { id: 0, formula: (x > 0) },
    // expected: (x > 0). All three soundness gates pass.
    let mut validator = ProofValidator::new();
    let prop = binary(BinOp::Gt, ident_expr("x"), int_lit(0));
    validator.register_hypothesis("h0", prop.clone());

    let proof = ProofTerm::Hypothesis {
        id: 0,
        formula: prop.clone(),
    };
    let result = validator.validate(&proof, &prop);
    assert!(
        result.is_ok(),
        "matching proposition must validate: {:?}",
        result
    );
}

#[test]
fn hypothesis_with_mismatched_proposition_is_rejected() {
    // h0 : (x > 0) is in scope. User claims Hypothesis { id: 0,
    // formula: (x < 0) } and asks to prove (x < 0). Pre-fix this
    // silently passed because formula == expected and h0 exists —
    // never checking that h0 actually proves (x > 0), not (x < 0).
    let mut validator = ProofValidator::new();
    let actual_prop = binary(BinOp::Gt, ident_expr("x"), int_lit(0));
    let claimed_prop = binary(BinOp::Lt, ident_expr("x"), int_lit(0));
    validator.register_hypothesis("h0", actual_prop);

    let proof = ProofTerm::Hypothesis {
        id: 0,
        formula: claimed_prop.clone(),
    };
    let result = validator.validate(&proof, &claimed_prop);
    assert!(
        result.is_err(),
        "mismatched hypothesis content must be rejected — \
         pre-fix this silently passed because formula == expected"
    );
}

#[test]
fn hypothesis_with_missing_id_is_rejected() {
    // No h0 in scope at all. Pre- and post-fix both reject —
    // pin the existing behavior so we don't regress it while
    // tightening the content check.
    let mut validator = ProofValidator::new();
    let prop = binary(BinOp::Gt, ident_expr("x"), int_lit(0));
    let proof = ProofTerm::Hypothesis {
        id: 0,
        formula: prop.clone(),
    };
    let result = validator.validate(&proof, &prop);
    assert!(
        result.is_err(),
        "dangling hypothesis reference must be rejected"
    );
}

#[test]
fn formula_must_match_expected_even_when_hypothesis_exists() {
    // h0 : (x > 0) is in scope. User writes Hypothesis { id: 0,
    // formula: (x > 0) } but claims expected = (x < 0). The
    // formula-vs-expected sanity check (gate 1) catches this.
    let mut validator = ProofValidator::new();
    let prop = binary(BinOp::Gt, ident_expr("x"), int_lit(0));
    validator.register_hypothesis("h0", prop.clone());

    let other = binary(BinOp::Lt, ident_expr("x"), int_lit(0));
    let proof = ProofTerm::Hypothesis {
        id: 0,
        formula: prop.clone(),
    };
    let result = validator.validate(&proof, &other);
    assert!(
        result.is_err(),
        "claim mismatch must be rejected by the formula-vs-expected gate"
    );
}
