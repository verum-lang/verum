//! Soundness regression: `validate_apply_def` requires `def_proof`
//! to conclude an equality.
//!
//! Pre-fix the validator only checked
//! `def_proof.conclusion() == expected`, with `_original` and
//! `_name` ignored. A user could pair a proof of any expression
//! with a claim about a "definition unfolding" — pre-fix passed.
//!
//! Post-fix `def_proof` must structurally conclude
//! `Binary { op: Eq, .. }` (a definitional equality). Anything
//! else fails the shape gate before the rule can fire.

use verum_ast::expr::{BinOp, Expr, ExprKind};
use verum_ast::literal::Literal;
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_ast::LiteralKind;
use verum_common::Heap;
use verum_smt::proof_term_unified::ProofTerm;
use verum_verification::proof_validator::ProofValidator;

fn ident_expr(name: &str) -> Expr {
    Expr::path(Path::single(Ident::new(name, Span::dummy())))
}

fn bool_lit(value: bool) -> Expr {
    Expr::literal(Literal::new(LiteralKind::Bool(value), Span::dummy()))
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
fn apply_def_rejects_non_equality_def_proof() {
    // def_proof concludes `true` (not an equality). Pre-fix this
    // silently passed because formula-shape was not checked.
    let mut validator = ProofValidator::new();
    let bool_true = bool_lit(true);
    validator.register_axiom("trivial", bool_true.clone());
    let bad_proof = ProofTerm::Axiom {
        name: "trivial".into(),
        formula: bool_true.clone(),
    };

    let proof = ProofTerm::ApplyDef {
        def_proof: Heap::new(bad_proof),
        original: ident_expr("foo"),
        name: "foo".into(),
    };

    let result = validator.validate(&proof, &bool_true);
    assert!(
        result.is_err(),
        "non-equality def_proof must be rejected — pre-fix this silently passed"
    );
}

#[test]
fn apply_def_accepts_equality_def_proof() {
    // def_proof concludes `foo = body` (an equality). Sound case.
    let mut validator = ProofValidator::new();
    let foo = ident_expr("foo");
    let body = ident_expr("body");
    let eq_foo_body = binary(BinOp::Eq, foo.clone(), body.clone());
    validator.register_axiom("foo_def", eq_foo_body.clone());
    let def_proof = ProofTerm::Axiom {
        name: "foo_def".into(),
        formula: eq_foo_body.clone(),
    };

    let proof = ProofTerm::ApplyDef {
        def_proof: Heap::new(def_proof),
        original: foo.clone(),
        name: "foo".into(),
    };

    let result = validator.validate(&proof, &eq_foo_body);
    assert!(
        result.is_ok(),
        "equality-shaped def_proof must validate: {:?}",
        result
    );
}
