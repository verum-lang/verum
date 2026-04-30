//! Soundness regression: an unknown rewrite rule with cross-ExprKind
//! source/target must NOT silently pass.
//!
//! Pre-fix `structurally_compatible` had a catch-all `_ => true` arm
//! that made the "unknown rewrite rule" branch in
//! `validate_apply_rewrite_rule` accept ANY source→target pair
//! under any unregistered rule name — same trust-the-user
//! soundness pattern as the catch-all arm fixed in 8429bd4e and
//! the quantifier rules in 80f43418.
//!
//! Post-fix the catch-all uses `std::mem::discriminant` so cross-kind
//! pairs (e.g. Path source, Literal target where both arms aren't
//! the explicit Literal↔Path cross-pair) reject.
//!
//! Note: the explicit Literal↔Path cross-pair stays accepted —
//! definition unfolding legitimately swaps a constant for its name.
//! This test pins the cross-kind rejection for OTHER pairs (Path
//! ↔ Binary) that have no semantic story for direct rewriting.

use verum_ast::expr::{BinOp, Expr, ExprKind};
use verum_ast::literal::Literal;
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_ast::LiteralKind;
use verum_common::{Heap, List};

use verum_smt::proof_term_unified::ProofTerm;
use verum_verification::proof_validator::ProofValidator;

fn bool_lit(value: bool) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal::new(LiteralKind::Bool(value), Span::dummy())),
        Span::dummy(),
    )
}

fn ident_expr(name: &str) -> Expr {
    Expr::new(
        ExprKind::Path(Path::single(Ident::new(name, Span::dummy()))),
        Span::dummy(),
    )
}

fn binary_and(left: Expr, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op: BinOp::And,
            left: Heap::new(left),
            right: Heap::new(right),
        },
        Span::dummy(),
    )
}

#[test]
fn unknown_rewrite_with_cross_kind_pair_is_rejected() {
    // Pin: an unregistered rewrite rule MUST NOT accept a cross-kind
    // source→target pair under the structural-compatibility check.
    // Pre-fix `_ => true` in `structurally_compatible` made the
    // "unknown rule" branch accept arbitrary pairs.
    //
    // We construct a Rewrite proof:
    //   source: Path("p")  (a name that the validator won't unfold)
    //   target: Binary(p AND p)  (a Binary expression)
    //   rule:   "totally_made_up_rewrite"  (never registered)
    //
    // The validator hits validate_standard_rewrite → unknown-rule
    // arm → structurally_compatible(Path, Binary) which now returns
    // false (different discriminants, not in the explicit
    // Literal↔Path cross-pair).
    let mut validator = ProofValidator::new();
    let source_axiom = ProofTerm::Axiom {
        name: "p_holds".into(),
        formula: ident_expr("p"),
    };
    validator.register_axiom("p_holds", ident_expr("p"));

    // Target is a Binary expression — different ExprKind than the
    // axiom's Path source.
    let target = binary_and(ident_expr("p"), ident_expr("p"));

    let proof = ProofTerm::Rewrite {
        source: Heap::new(source_axiom),
        rule: "totally_made_up_rewrite".into(),
        target: target.clone(),
    };

    // The "claimed conclusion" matches the target. With the
    // pre-fix catch-all, the rewrite would have been accepted as a
    // valid proof of the cross-kind expression.
    let result = validator.validate(&proof, &target);
    assert!(
        result.is_err(),
        "unknown rewrite rule with cross-kind source/target MUST reject; \
         pre-fix this validated trivially via structurally_compatible's \
         `_ => true` catch-all"
    );
    let msg = format!("{:?}", result.unwrap_err());
    assert!(
        msg.contains("unknown rewrite rule") || msg.contains("not compatible"),
        "error must explain the structural rejection. got: {}",
        msg
    );
}

#[test]
fn unknown_rewrite_with_same_kind_pair_still_accepted() {
    // Pin the symmetric direction: same-discriminant source/target
    // pairs (e.g. Path ↔ Path) MUST still pass under unknown rule
    // names. This preserves the sound subset of the catch-all
    // (identity-shape transformations are at least plausibly
    // valid; the SMT/expr_eq pipeline catches the actual
    // mathematical content elsewhere).
    let mut validator = ProofValidator::new();
    let source_axiom = ProofTerm::Axiom {
        name: "p_holds".into(),
        formula: ident_expr("p"),
    };
    validator.register_axiom("p_holds", ident_expr("p"));

    // Same ExprKind (Path) — a same-kind rewrite that the unknown-rule
    // path should still accept structurally.
    let target = ident_expr("p");

    let proof = ProofTerm::Rewrite {
        source: Heap::new(source_axiom),
        rule: "totally_made_up_rewrite".into(),
        target: target.clone(),
    };

    let result = validator.validate(&proof, &target);
    assert!(
        result.is_ok(),
        "same-kind unknown rewrite MUST still pass — the structural \
         catch-all applies the discriminant check, and Path == Path \
         passes. Regression here would over-tighten and reject \
         legitimate same-kind transformations: {:?}",
        result
    );
}

#[test]
fn simp_rule_rejects_cross_kind_pair() {
    // Pin: `simp` / `simplify` previously accepted any source→target
    // unconditionally. Cross-kind pairs (Path → Binary) MUST now
    // reject. Same trust-the-user pattern as the unknown-rule arm
    // closed in 7ef97a6d.
    let mut validator = ProofValidator::new();
    let source_axiom = ProofTerm::Axiom {
        name: "p_holds".into(),
        formula: ident_expr("p"),
    };
    validator.register_axiom("p_holds", ident_expr("p"));

    let target = binary_and(ident_expr("p"), ident_expr("p"));

    let proof = ProofTerm::Rewrite {
        source: Heap::new(source_axiom),
        rule: "simp".into(),
        target: target.clone(),
    };

    let result = validator.validate(&proof, &target);
    assert!(
        result.is_err(),
        "simp must reject cross-kind source/target — pre-fix accepted unconditionally"
    );
    let msg = format!("{:?}", result.unwrap_err());
    assert!(
        msg.contains("simp requires structurally-compatible") || msg.contains("not compatible"),
        "error must explain the gate. got: {}",
        msg
    );
}

#[test]
fn arith_rule_rejects_non_arithmetic_pair() {
    // Pin: `arith` / `omega` / `lia` were unconditional `Ok(())`
    // pre-fix. Now require BOTH source and target to be arithmetic
    // expressions. A Bool literal is not arithmetic, so this MUST
    // reject.
    let mut validator = ProofValidator::new();
    let source_axiom = ProofTerm::Axiom {
        name: "axiom_true".into(),
        formula: bool_lit(true),
    };
    validator.register_axiom("axiom_true", bool_lit(true));

    let target = bool_lit(false);

    let proof = ProofTerm::Rewrite {
        source: Heap::new(source_axiom),
        rule: "arith".into(),
        target: target.clone(),
    };

    let result = validator.validate(&proof, &target);
    assert!(
        result.is_err(),
        "arith must reject non-arithmetic source/target pair"
    );
    let msg = format!("{:?}", result.unwrap_err());
    assert!(
        msg.contains("arith tactic requires arithmetic"),
        "error must explain the arithmetic gate. got: {}",
        msg
    );
}

#[test]
fn unfold_rule_rejects_cross_kind_pair() {
    // Pin: any rule starting with `unfold_` was unconditional
    // `Ok(())` pre-fix. Cross-kind pairs (Path → Binary) MUST
    // now reject. Same gate as `simp`.
    let mut validator = ProofValidator::new();
    let source_axiom = ProofTerm::Axiom {
        name: "p_holds".into(),
        formula: ident_expr("p"),
    };
    validator.register_axiom("p_holds", ident_expr("p"));

    let target = binary_and(ident_expr("p"), ident_expr("p"));

    let proof = ProofTerm::Rewrite {
        source: Heap::new(source_axiom),
        rule: "unfold_p".into(),
        target: target.clone(),
    };

    let result = validator.validate(&proof, &target);
    assert!(
        result.is_err(),
        "unfold_* must reject cross-kind source/target — pre-fix accepted unconditionally"
    );
}

#[test]
fn unknown_rewrite_literal_path_cross_pair_still_accepted() {
    // Pin the explicit Literal↔Path cross-pair — preserved from the
    // pre-fix code as a legitimate definition-unfolding
    // transformation (a constant can be replaced by its name and
    // vice versa). A regression that drops this explicit arm and
    // falls through to the discriminant fallback would surface here
    // as a rejection.
    let mut validator = ProofValidator::new();
    // Source axiom: bool literal `true`.
    let source_axiom = ProofTerm::Axiom {
        name: "axiom_true".into(),
        formula: bool_lit(true),
    };
    validator.register_axiom("axiom_true", bool_lit(true));

    // Target: a Path (some named binding). Cross-kind: Literal ↔ Path.
    let target = ident_expr("p");

    let proof = ProofTerm::Rewrite {
        source: Heap::new(source_axiom),
        rule: "totally_made_up_rewrite".into(),
        target: target.clone(),
    };

    let result = validator.validate(&proof, &target);
    assert!(
        result.is_ok(),
        "Literal↔Path cross-pair MUST still pass — definition unfolding \
         is the documented reason this explicit arm exists: {:?}",
        result
    );
    // Suppress unused warning for List import since we don't construct
    // any List in this file.
    let _: List<()> = List::new();
}
