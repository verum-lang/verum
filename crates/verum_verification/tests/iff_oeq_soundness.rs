//! Soundness regression: `validate_iff_oeq` must verify that the
//! iff_proof actually concludes `left <=> right` for the claimed
//! pair (left, right).
//!
//! Pre-fix the validator only checked:
//! 1. iff_proof validates internally against its own conclusion.
//! 2. `(left = right) == expected` syntactically.
//!
//! It did NOT check that iff_proof's conclusion is `left <=> right`.
//! A user could pair a proof of `P <=> Q` with claim `A = B` for
//! unrelated A, B — pre-fix this silently passed.

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
fn iff_oeq_rejects_proof_unrelated_to_claimed_pair() {
    // iff_proof concludes `P = Q` (axiom), but claim is `A = B`.
    // Pre-fix this passed because iff_proof is internally valid
    // and `(A = B) == expected` syntactically.
    let mut validator = ProofValidator::new();
    let p = ident_expr("P");
    let q = ident_expr("Q");
    let a = ident_expr("A");
    let b = ident_expr("B");

    // Axiom: P <=> Q (using Iff op).
    let iff_pq = binary(BinOp::Iff, p, q);
    validator.register_axiom("pq_iff", iff_pq.clone());
    let iff_proof = ProofTerm::Axiom {
        name: "pq_iff".into(),
        formula: iff_pq,
    };

    let claimed_eq = binary(BinOp::Eq, a.clone(), b.clone());
    let proof = ProofTerm::IffOEq {
        iff_proof: Heap::new(iff_proof),
        left: a,
        right: b,
    };

    let result = validator.validate(&proof, &claimed_eq);
    assert!(
        result.is_err(),
        "iff_proof concludes P<=>Q but claim is A=B — must reject"
    );
}

#[test]
fn iff_oeq_accepts_proof_for_matching_pair() {
    // iff_proof concludes `A <=> B`, claim is `A = B`. Sound case.
    let mut validator = ProofValidator::new();
    let a = ident_expr("A");
    let b = ident_expr("B");
    let iff_ab = binary(BinOp::Iff, a.clone(), b.clone());
    validator.register_axiom("ab_iff", iff_ab.clone());
    let iff_proof = ProofTerm::Axiom {
        name: "ab_iff".into(),
        formula: iff_ab,
    };

    let claimed_eq = binary(BinOp::Eq, a.clone(), b.clone());
    let proof = ProofTerm::IffOEq {
        iff_proof: Heap::new(iff_proof),
        left: a,
        right: b,
    };

    let result = validator.validate(&proof, &claimed_eq);
    assert!(
        result.is_ok(),
        "matching iff_proof must validate: {:?}",
        result
    );
}

#[test]
fn iff_oeq_rejects_iff_proof_with_swapped_sides() {
    // iff_proof concludes `B <=> A`, claim is `A = B`. The two
    // pairs share variables but in opposite order — the structural
    // gate must catch this since iff/eq are commutative in
    // semantics but not in pattern shape.
    let mut validator = ProofValidator::new();
    let a = ident_expr("A");
    let b = ident_expr("B");
    let iff_ba = binary(BinOp::Iff, b.clone(), a.clone());
    validator.register_axiom("ba_iff", iff_ba.clone());
    let iff_proof = ProofTerm::Axiom {
        name: "ba_iff".into(),
        formula: iff_ba,
    };

    let claimed_eq = binary(BinOp::Eq, a.clone(), b.clone());
    let proof = ProofTerm::IffOEq {
        iff_proof: Heap::new(iff_proof),
        left: a,
        right: b,
    };

    let result = validator.validate(&proof, &claimed_eq);
    assert!(
        result.is_err(),
        "swapped iff sides must require explicit symmetry — pre-fix this silently passed"
    );
}

#[test]
fn iff_oeq_rejects_non_iff_premise() {
    // iff_proof concludes a non-iff expression (literal `true`).
    // Must reject because `true` doesn't carry the `<=> ` shape.
    let mut validator = ProofValidator::new();
    let a = ident_expr("A");
    let b = ident_expr("B");
    validator.register_axiom("trivial", bool_lit(true));
    let bad_proof = ProofTerm::Axiom {
        name: "trivial".into(),
        formula: bool_lit(true),
    };

    let claimed_eq = binary(BinOp::Eq, a.clone(), b.clone());
    let proof = ProofTerm::IffOEq {
        iff_proof: Heap::new(bad_proof),
        left: a,
        right: b,
    };

    let result = validator.validate(&proof, &claimed_eq);
    assert!(
        result.is_err(),
        "non-iff premise must be rejected — `true` is not (A <=> B)"
    );
}
