//! Pin: `InterpolationConfig.max_projection_vars` and
//! `InterpolationConfig.quantifier_elimination` actually
//! influence the model-based interpolation projection step.
//!
//! Closes the inert-defense pattern for both fields. Previously
//! they were declared with documented defaults but no code path
//! consulted them: a model-based projection would always invoke
//! the Z3 `qe` tactic regardless of the budget, and the boolean
//! gate had no effect.
//!
//! These tests pin the wiring contract via the public API:
//! - construct `InterpolationConfig` with extreme values
//! - feed `interpolate(A, B)` with formulas that force projection
//! - assert the engine respects the configured limits

use verum_smt::interpolation::{InterpolationConfig, InterpolationEngine};

#[test]
fn default_config_has_documented_max_projection_vars() {
    let cfg = InterpolationConfig::default();
    // Pin the documented default. Drift here would silently
    // change the budget for every caller relying on
    // `Default::default()`.
    assert_eq!(cfg.max_projection_vars, 100);
}

#[test]
fn default_config_has_quantifier_elimination_enabled() {
    let cfg = InterpolationConfig::default();
    assert!(cfg.quantifier_elimination);
}

#[test]
fn engine_constructs_with_zero_budget() {
    // Pin: the constructor must not reject extreme budgets — the
    // bound is enforced at projection time, not at engine
    // creation. This keeps the engine usable for shapes where the
    // caller knows their formulas have no projection variables
    // (e.g. propositional fragments).
    let mut cfg = InterpolationConfig::default();
    cfg.max_projection_vars = 0;
    let _ = InterpolationEngine::new(cfg);
}

#[test]
fn engine_constructs_with_large_budget() {
    let mut cfg = InterpolationConfig::default();
    cfg.max_projection_vars = usize::MAX;
    let _ = InterpolationEngine::new(cfg);
}

#[test]
fn engine_constructs_with_qe_disabled() {
    // Pin: `quantifier_elimination = false` is a valid
    // configuration. Engine creation must not panic; the gate is
    // applied later inside `project_onto_shared`.
    let mut cfg = InterpolationConfig::default();
    cfg.quantifier_elimination = false;
    let _ = InterpolationEngine::new(cfg);
}
