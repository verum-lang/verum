//! Pin: `TacticConfig.allow_admits` gates the `admit` and
//! `sorry` tactics. Closes the inert-defense pattern: the field
//! was documented as "Allow admit/sorry tactics" with default
//! `true`, but no code path consulted it.
//!
//! With the wiring in place, a verification run that sets
//! `allow_admits = false` will reject any goal that the user
//! tries to discharge via `admit` or `sorry`. This is the
//! configuration that production / CI pipelines should run
//! under: an admitted goal is a hole, not a proof.

use verum_ast::decl::TacticExpr;
use verum_verification::tactic_evaluation::{
    TacticConfig, TacticError, TacticEvaluator,
};

#[test]
fn default_config_allows_admits() {
    // Pin the documented default. Drift here would silently
    // change verification behaviour for callers relying on
    // `Default::default()`.
    let cfg = TacticConfig::default();
    assert!(cfg.allow_admits);
}

#[test]
fn admit_under_default_config_does_not_fail_due_to_gate() {
    // Pin: with the default permissive config, the new
    // `allow_admits` gate must not fire. The tactic may still
    // fail for unrelated reasons (e.g. no current goal), but
    // the failure must not be the gate's diagnostic.
    let mut evaluator = TacticEvaluator::new();
    let result = evaluator.apply_tactic(&TacticExpr::Admit);
    if let Err(TacticError::Failed(msg)) = &result {
        assert!(
            !msg.as_str().contains("allow_admits"),
            "gate fired despite default allow_admits=true: {}",
            msg
        );
    }
}

#[test]
fn sorry_under_default_config_does_not_fail_due_to_gate() {
    let mut evaluator = TacticEvaluator::new();
    let result = evaluator.apply_tactic(&TacticExpr::Sorry);
    if let Err(TacticError::Failed(msg)) = &result {
        assert!(
            !msg.as_str().contains("allow_admits"),
            "gate fired despite default allow_admits=true: {}",
            msg
        );
    }
}

#[test]
fn admit_is_rejected_when_allow_admits_is_false() {
    // Pin: production-style config rejects `admit`. The error
    // message names the flag so the user can opt back in
    // explicitly if needed.
    let mut evaluator = TacticEvaluator::new();
    let mut cfg = TacticConfig::default();
    cfg.allow_admits = false;
    evaluator.set_config(cfg);

    let result = evaluator.apply_tactic(&TacticExpr::Admit);
    match result {
        Err(TacticError::Failed(msg)) => {
            assert!(
                msg.as_str().contains("allow_admits"),
                "diagnostic should name the flag, got: {}",
                msg
            );
        }
        other => panic!(
            "expected admit to fail under allow_admits=false, got: {:?}",
            other
        ),
    }
}

#[test]
fn sorry_is_rejected_when_allow_admits_is_false() {
    let mut evaluator = TacticEvaluator::new();
    let mut cfg = TacticConfig::default();
    cfg.allow_admits = false;
    evaluator.set_config(cfg);

    let result = evaluator.apply_tactic(&TacticExpr::Sorry);
    match result {
        Err(TacticError::Failed(msg)) => {
            assert!(
                msg.as_str().contains("allow_admits"),
                "diagnostic should name the flag, got: {}",
                msg
            );
        }
        other => panic!(
            "expected sorry to fail under allow_admits=false, got: {:?}",
            other
        ),
    }
}

#[test]
fn config_setter_round_trips() {
    // Pin: `set_config` / `config()` round-trip. Without this
    // accessor pair, downstream callers can't observe the
    // policy state to make their own gating decisions.
    let mut evaluator = TacticEvaluator::new();
    assert!(evaluator.config().allow_admits);

    let mut cfg = TacticConfig::default();
    cfg.allow_admits = false;
    evaluator.set_config(cfg);
    assert!(!evaluator.config().allow_admits);
}
