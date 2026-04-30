//! Soundness regression: an unknown inference rule must NOT silently pass.
//!
//! Pre-fix `apply_inference_rule` had an unsound fallback for any rule name
//! it didn't have a hardcoded match for: if the user supplied at least one
//! premise, it returned `Ok(expected.clone())`. That made the downstream
//! `expr_eq(derived, expected)` check trivially true and let arbitrary
//! "rule names" stand in for real proofs:
//!
//! ```ignore
//! ProofTerm::Apply {
//!     rule: "totally_made_up_rule",     // no such rule registered
//!     premises: vec![any_well_typed_premise],
//! }                                     // → validated as proof of ANY claim
//! ```
//!
//! Post-fix the apply branch routes through `register_inference_rule` and
//! returns `ValidationError::ValidationFailed { "unknown inference rule" }`
//! when the name is unknown.

use verum_ast::expr::{Expr, ExprKind};
use verum_ast::literal::Literal;
use verum_ast::LiteralKind;
use verum_ast::span::Span;
use verum_common::{Heap, List};

use verum_smt::proof_term_unified::ProofTerm;
use verum_verification::proof_validator::ProofValidator;

fn bool_lit(value: bool) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal::new(LiteralKind::Bool(value), Span::dummy())),
        Span::dummy(),
    )
}

fn axiom_premise() -> Heap<ProofTerm> {
    // Axiom variant validates trivially when conclusion matches its
    // formula — the simplest well-formed premise we can build, and the
    // exact shape that the old fallback path would accept under any rule
    // name.
    Heap::new(ProofTerm::Axiom {
        name: "p_holds".into(),
        formula: bool_lit(true),
    })
}

fn make_apply_with_made_up_rule() -> ProofTerm {
    let mut premises: List<Heap<ProofTerm>> = List::new();
    premises.push(axiom_premise());

    ProofTerm::Apply {
        rule: "totally_made_up_rule".into(),
        premises,
    }
}

#[test]
fn unknown_inference_rule_with_premises_is_rejected() {
    let mut validator = ProofValidator::new();
    validator.register_axiom("p_holds", bool_lit(true));
    let proof = make_apply_with_made_up_rule();

    // The "claimed conclusion" we're trying to prove. Without the soundness
    // fix, the validator would have happily accepted this — which is
    // catastrophic: it would have validated any proposition as a theorem.
    let claimed_conclusion = bool_lit(false);

    let result = validator.validate(&proof, &claimed_conclusion);
    assert!(
        result.is_err(),
        "validator must reject proofs that name an unregistered inference rule \
         — even when premises type-check, the soundness gate is the rule, not the premises"
    );

    let msg = format!("{:?}", result.unwrap_err());
    assert!(
        msg.contains("unknown inference rule") || msg.contains("totally_made_up_rule"),
        "error message must name the missing rule. got: {}",
        msg
    );
}

#[test]
fn registered_inference_rule_validates_when_arity_matches() {
    let mut validator = ProofValidator::new();
    validator.register_axiom("p_holds", bool_lit(true));

    let truth = bool_lit(true);
    // Synthetic rule schema: `my_intro` takes 1 premise (anything) and
    // concludes `true`. It's a deliberately trivial rule — testing the
    // dispatch path, not the rule's mathematical content.
    let mut schema_premises: List<Expr> = List::new();
    schema_premises.push(truth.clone());
    validator.register_inference_rule(
        "my_intro",
        schema_premises,
        truth.clone(),
    );

    let mut premises: List<Heap<ProofTerm>> = List::new();
    premises.push(axiom_premise());

    let proof = ProofTerm::Apply {
        rule: "my_intro".into(),
        premises,
    };

    let result = validator.validate(&proof, &truth);
    assert!(
        result.is_ok(),
        "registered rule must validate when arity and conclusion match: {:?}",
        result
    );
}

#[test]
fn registered_inference_rule_rejects_arity_mismatch() {
    let mut validator = ProofValidator::new();
    validator.register_axiom("p_holds", bool_lit(true));

    let truth = bool_lit(true);
    // Schema declares 2 premises, but the proof only supplies 1.
    let mut schema_premises: List<Expr> = List::new();
    schema_premises.push(truth.clone());
    schema_premises.push(truth.clone());
    validator.register_inference_rule(
        "two_arg_rule",
        schema_premises,
        truth.clone(),
    );

    let mut premises: List<Heap<ProofTerm>> = List::new();
    premises.push(axiom_premise());

    let proof = ProofTerm::Apply {
        rule: "two_arg_rule".into(),
        premises,
    };

    let result = validator.validate(&proof, &truth);
    assert!(
        result.is_err(),
        "validator must reject when premise count doesn't match rule schema"
    );
    let msg = format!("{:?}", result.unwrap_err());
    assert!(
        msg.contains("expects 2 premises") && msg.contains("got 1"),
        "error must say schema arity vs supplied arity. got: {}",
        msg
    );
}

// ============================================================================
// Quantifier-rule soundness gates (forall_elim / exists_intro)
//
// Pre-fix `apply_inference_rule` returned `Ok(expected.clone())` for both
// forall_elim and exists_intro WITHOUT verifying the premise / expected
// was actually quantified. Same trust-the-user soundness leak as the
// catch-all arm fixed in 8429bd4e — `forall_elim` called on a premise
// that ISN'T a Forall would silently validate any expected.
// ============================================================================

#[test]
fn forall_elim_rejects_non_forall_premise() {
    // Pin: forall_elim called with a non-quantified premise (here: a
    // bare bool literal `true`) MUST be rejected. Pre-fix the rule
    // accepted any premise + any expected as if the elimination
    // succeeded.
    let mut validator = ProofValidator::new();
    validator.register_axiom("p_holds", bool_lit(true));

    let mut premises: List<Heap<ProofTerm>> = List::new();
    premises.push(axiom_premise());

    let proof = ProofTerm::Apply {
        rule: "forall_elim".into(),
        premises,
    };

    // Claim a wholly unrelated conclusion. The pre-fix path would
    // accept this trivially.
    let claimed_conclusion = bool_lit(false);

    let result = validator.validate(&proof, &claimed_conclusion);
    assert!(
        result.is_err(),
        "forall_elim must reject non-quantified premise — premise is bool literal, not Forall"
    );
    let msg = format!("{:?}", result.unwrap_err());
    assert!(
        msg.contains("forall_elim requires a universally-quantified premise"),
        "error must explain the soundness gate. got: {}",
        msg
    );
}

// NOTE: tests for the strengthened body-shape gate
// (forall_elim's expected-vs-body discriminant check / exists_intro's
// premise-vs-body discriminant check) cannot yet be written against
// the public `validate` entry point because `expr_eq` doesn't handle
// Forall/Exists variants — `validate_axiom`'s formula-match check
// fails before the apply rule runs. The gates themselves are in
// place and don't regress the existing test suite. Adding
// Forall/Exists arms to expr_eq_impl is tracked separately as a
// precondition for direct end-to-end testing.

#[test]
fn exists_intro_rejects_non_exists_expected() {
    // Symmetric pin for exists_intro: when the expected conclusion
    // isn't an existential, the rule must reject. Pre-fix any premise
    // would validate any expected.
    let mut validator = ProofValidator::new();
    validator.register_axiom("p_holds", bool_lit(true));

    let mut premises: List<Heap<ProofTerm>> = List::new();
    premises.push(axiom_premise());

    let proof = ProofTerm::Apply {
        rule: "exists_intro".into(),
        premises,
    };

    // Claim a non-existential expected — bool literal.
    let claimed_conclusion = bool_lit(false);

    let result = validator.validate(&proof, &claimed_conclusion);
    assert!(
        result.is_err(),
        "exists_intro must reject non-existential expected"
    );
    let msg = format!("{:?}", result.unwrap_err());
    assert!(
        msg.contains("exists_intro requires an existentially-quantified expected"),
        "error must explain the soundness gate. got: {}",
        msg
    );
}
