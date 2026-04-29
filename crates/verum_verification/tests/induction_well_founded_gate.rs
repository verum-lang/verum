//! Pin: `ValidationConfig.check_well_founded` rejects vacuous
//! induction (the property template doesn't reference the
//! induction variable, so the IH and the step obligation are
//! syntactically identical and the IH discharges the step
//! trivially regardless of soundness).
//!
//! Closes the inert-defense pattern: the field was documented as
//! "Check that induction is well-founded" with default `true`,
//! but no code path consulted it.

use verum_ast::expr::{Expr, ExprKind, QuantifierBinding};
use verum_ast::literal::{Literal, LiteralKind};
use verum_ast::pattern::{Pattern, PatternKind};
use verum_ast::span::Span;
use verum_ast::Ident;
use verum_common::{List, Maybe, Text};
use verum_smt::proof_term_unified::ProofTerm;
use verum_verification::proof_validator::{
    ProofValidator, ValidationConfig, ValidationError,
};

fn span() -> Span {
    Span::dummy()
}

fn bool_literal(value: bool) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Bool(value),
            span: span(),
        }),
        span(),
    )
}

/// Build `forall (n) => true` — a vacuous universal whose body
/// does not reference the induction variable.
fn vacuous_forall(var_name: &str) -> Expr {
    let binding = QuantifierBinding {
        pattern: Pattern {
            kind: PatternKind::Ident {
                by_ref: false,
                mutable: false,
                name: Ident::new(var_name, span()),
                subpattern: Maybe::None,
            },
            span: span(),
        },
        ty: Maybe::None,
        domain: Maybe::None,
        guard: Maybe::None,
        span: span(),
    };
    Expr::new(
        ExprKind::Forall {
            bindings: List::from_iter([binding]),
            body: Box::new(bool_literal(true)),
        },
        span(),
    )
}

/// Build a trivially-valid induction proof whose base/step legs
/// both prove `true` by reflexivity. The proof is NOT what's
/// being validated here — we want the well-founded gate to
/// reject before the leg-validation runs.
fn induction_of_true(var_name: &str) -> ProofTerm {
    let true_expr = bool_literal(true);
    ProofTerm::Induction {
        var: Text::from(var_name),
        base_case: Box::new(ProofTerm::Reflexivity {
            term: true_expr.clone(),
        }),
        inductive_case: Box::new(ProofTerm::Reflexivity { term: true_expr }),
    }
}

#[test]
fn vacuous_induction_is_rejected_when_well_founded_check_enabled() {
    // Pin: default `check_well_founded = true` rejects vacuous
    // induction with `InductionError`. The ValidationConfig
    // documented contract is now load-bearing.
    let mut validator = ProofValidator::new();
    let proof = induction_of_true("n");
    let target = vacuous_forall("n");

    let result = validator.validate(&proof, &target);
    match result {
        Err(ValidationError::InductionError { message }) => {
            // Diagnostic mentions the variable so the user can
            // identify which binding is unused.
            assert!(
                message.as_str().contains("not well-founded")
                    || message.as_str().contains("vacuous"),
                "expected well-founded diagnostic, got: {}",
                message
            );
        }
        other => panic!(
            "expected InductionError on vacuous induction, got: {:?}",
            other
        ),
    }
}

#[test]
fn vacuous_induction_is_accepted_when_well_founded_check_disabled() {
    // Pin: with `check_well_founded = false` the gate is bypassed
    // — the older induction logic runs. We don't assert success
    // here (the proof legs may still trip other validation
    // checks); we only assert that the response is NOT the new
    // well-founded gate's diagnostic. This proves the toggle
    // actually controls the new gate.
    let mut config = ValidationConfig::default();
    config.check_well_founded = false;
    let mut validator = ProofValidator::with_config(config);

    let proof = induction_of_true("n");
    let target = vacuous_forall("n");

    let result = validator.validate(&proof, &target);
    match result {
        Err(ValidationError::InductionError { message }) => {
            // The diagnostic must NOT be the well-founded one.
            // Other induction errors are fine — this test only
            // pins the gate's reachability under the toggle.
            assert!(
                !(message.as_str().contains("not well-founded")
                    || message.as_str().contains("vacuous")),
                "well-founded gate fired despite check_well_founded=false: {}",
                message
            );
        }
        // Any non-Err or other Err is also fine: the point is
        // that the well-founded gate did not fire.
        _ => {}
    }
}

#[test]
fn well_founded_check_default_is_true() {
    // Pin: the documented default ("// Check that induction is
    // well-founded ... pub check_well_founded: bool ...
    // check_well_founded: true") matches the runtime default. If
    // this drifts, callers that rely on the default's safety
    // contract silently lose protection.
    let config = ValidationConfig::default();
    assert!(config.check_well_founded);
}
