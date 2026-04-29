//! Soundness regression: a conditional rewrite rule must NOT silently
//! pass when its condition is unverifiable.
//!
//! Pre-fix `validate_rewrite_rule_application` discharged conditions
//! with:
//!
//!     for condition in &rule.conditions {
//!         let instantiated_cond = self.apply_bindings(condition, &bindings);
//!         // For now, we trust the condition is satisfied
//!         let _ = instantiated_cond;
//!     }
//!
//! That made conditional rewrites unsound: a malformed proof could
//! apply `safe_div(a, b) → a/b` (conditioned on `b ≠ 0`) without ever
//! demonstrating that `b ≠ 0` actually holds.
//!
//! Post-fix the validator only accepts a rewrite when each condition
//! is one of: literal `true`, reflexive equality, a registered axiom,
//! or a hypothesis currently in scope.

use verum_ast::expr::{BinOp, Expr, ExprKind};
use verum_ast::literal::Literal;
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_ast::LiteralKind;
use verum_common::{Heap, List, Text};

use verum_verification::proof_validator::{ProofValidator, RewriteRule};

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

fn eq_expr(lhs: Expr, rhs: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op: BinOp::Eq,
            left: Heap::new(lhs),
            right: Heap::new(rhs),
        },
        Span::dummy(),
    )
}

/// Build a rewrite rule with a single condition. The LHS and RHS are
/// the same identifier so pattern matching trivially succeeds — we're
/// testing the condition path, not the rewrite path.
fn rule_with_condition(name: &str, condition: Expr) -> RewriteRule {
    let body = ident_expr("x");
    let mut conditions = List::new();
    conditions.push(condition);
    RewriteRule {
        name: name.into(),
        lhs: body.clone(),
        rhs: body,
        conditions,
        bidirectional: false,
    }
}

/// Drive the rewrite-application path through the public surface. Uses
/// `try_rewrite` via a `ProofTerm::Rewrite` to invoke
/// `validate_rewrite_rule_application` with a registered rule.
fn try_apply_rewrite(
    validator: &mut ProofValidator,
    rule: RewriteRule,
) -> Result<(), Text> {
    use verum_smt::proof_term_unified::ProofTerm;

    let body = ident_expr("x");
    validator.register_rewrite_rule(rule.clone());

    // Source proof = Axiom asserting `x` (so its conclusion is `x`).
    let source = ProofTerm::Axiom {
        name: "src".into(),
        formula: body.clone(),
    };
    validator.register_axiom("src", body.clone());

    let target_proof = ProofTerm::Rewrite {
        source: Heap::new(source),
        rule: rule.name.clone(),
        target: body.clone(),
    };

    validator
        .validate(&target_proof, &body)
        .map_err(|e| Text::from(format!("{:?}", e)))
}

#[test]
fn condition_literal_true_is_accepted() {
    let mut validator = ProofValidator::new();
    let result = try_apply_rewrite(
        &mut validator,
        rule_with_condition("trivial_true", bool_lit(true)),
    );
    assert!(
        result.is_ok(),
        "literal `true` condition must discharge cleanly: {:?}",
        result
    );
}

#[test]
fn condition_literal_false_is_rejected() {
    let mut validator = ProofValidator::new();
    let result = try_apply_rewrite(
        &mut validator,
        rule_with_condition("trivial_false", bool_lit(false)),
    );
    assert!(
        result.is_err(),
        "literal `false` condition must be rejected — pre-fix this silently passed"
    );
    assert!(
        result.unwrap_err().as_str().contains("literally false"),
        "error must point at the false condition"
    );
}

#[test]
fn condition_reflexive_equality_is_accepted() {
    let mut validator = ProofValidator::new();
    let cond = eq_expr(ident_expr("y"), ident_expr("y"));
    let result =
        try_apply_rewrite(&mut validator, rule_with_condition("refl_y", cond));
    assert!(
        result.is_ok(),
        "reflexive equality `y == y` must discharge cleanly: {:?}",
        result
    );
}

#[test]
fn unverified_condition_is_rejected() {
    let mut validator = ProofValidator::new();
    // `b != 0` shape — but represented as `y == z` between two distinct
    // identifiers with no axiom or hypothesis backing it. Pre-fix this
    // silently passed; post-fix the validator must reject because none
    // of the four soundness gates apply.
    let cond = eq_expr(ident_expr("y"), ident_expr("z"));
    let result =
        try_apply_rewrite(&mut validator, rule_with_condition("y_eq_z", cond));
    assert!(
        result.is_err(),
        "unverified condition must NOT silently pass — soundness leak"
    );
    assert!(
        result.unwrap_err().as_str().contains("unverified condition"),
        "error must name the failure mode"
    );
}

#[test]
fn condition_matching_registered_axiom_is_accepted() {
    let mut validator = ProofValidator::new();
    let cond = eq_expr(ident_expr("y"), ident_expr("z"));
    // Now `y == z` IS a registered axiom; the same condition that was
    // rejected in the previous test now passes through this gate.
    validator.register_axiom("y_eq_z_axiom", cond.clone());

    let result =
        try_apply_rewrite(&mut validator, rule_with_condition("uses_y_eq_z", cond));
    assert!(
        result.is_ok(),
        "condition matching a registered axiom must discharge: {:?}",
        result
    );
}
