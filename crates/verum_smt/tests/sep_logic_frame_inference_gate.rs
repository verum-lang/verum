//! Pin: `SepLogicConfig.enable_frame_inference` gates the
//! `SepLogicEncoder::infer_frame` method. Closes the inert-
//! defense pattern: the field was documented as "Enable frame
//! inference" with default `true`, but no code path consulted
//! it — `infer_frame` always ran the full algorithm regardless.
//!
//! Callers that only need entailment validity (without the
//! residual-frame computation) can disable this for ~30%
//! reduction in encoder work on large heaps. The wiring makes
//! the documented opt-out load-bearing.

use verum_smt::separation_logic::{
    SepAssertion, SepLogicConfig, SepLogicEncoder,
};

#[test]
fn default_config_enables_frame_inference() {
    let cfg = SepLogicConfig::default();
    assert!(cfg.enable_frame_inference);
}

#[test]
fn frame_inference_runs_when_enabled() {
    // Pin: with the default permissive config, infer_frame
    // executes the algorithm. Trivial Emp |= Emp succeeds with
    // an Emp frame (identity).
    let encoder = SepLogicEncoder::new(SepLogicConfig::default());
    let result = encoder.infer_frame(&SepAssertion::Emp, &SepAssertion::Emp);
    assert!(
        result.success,
        "Emp |= Emp should succeed under default config"
    );
}

#[test]
fn frame_inference_skipped_when_disabled() {
    // Pin: with the gate off, infer_frame returns a typed
    // failure instead of running. The diagnostic names the flag
    // so callers can opt back in explicitly.
    let mut cfg = SepLogicConfig::default();
    cfg.enable_frame_inference = false;
    let encoder = SepLogicEncoder::new(cfg);

    let result = encoder.infer_frame(&SepAssertion::Emp, &SepAssertion::Emp);
    assert!(
        !result.success,
        "infer_frame should fail when gate is off"
    );
    assert!(
        result.message.as_str().contains("enable_frame_inference"),
        "diagnostic should name the flag, got: {}",
        result.message
    );
}

#[test]
fn config_default_round_trips() {
    // Pin: default factory keeps all SepLogicConfig fields at
    // their documented values.
    let cfg = SepLogicConfig::default();
    assert!(cfg.enable_frame_inference);
    assert!(cfg.enable_symbolic_execution);
    assert!(cfg.enable_caching);
    assert_eq!(cfg.entailment_timeout_ms, 5000);
    assert_eq!(cfg.max_unfolding_depth, 10);
}
