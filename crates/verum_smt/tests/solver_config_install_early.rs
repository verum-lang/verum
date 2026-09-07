//! The other half of the install contract — T1233.
//!
//! A SEPARATE test binary, because it needs a fresh process: this one
//! installs BEFORE anything reads, which is the ordering `verum build`
//! and `verum verify` actually use. Its sibling
//! (`solver_config_install.rs`) walks the refused-late-install path,
//! and the two cannot share a process — the first `effective` call in
//! either one decides the answer for the whole binary.

use verum_smt::config::{self, SolverConfig};

#[test]
fn an_early_install_is_what_every_later_reader_sees() {
    let mut cfg = SolverConfig::default();
    cfg.qe.simplify_level = 0;
    cfg.sep_logic.max_unfolding_depth = 1;
    cfg.sep_logic.entailment_timeout_ms = 250;

    config::install(cfg).expect("installing before any read must succeed");
    assert!(config::is_installed());

    let eff = config::effective();
    assert_eq!(eff.qe.simplify_level, 0, "the set value must arrive");
    assert_eq!(eff.sep_logic.max_unfolding_depth, 1);
    assert_eq!(eff.sep_logic.entailment_timeout_ms, 250);

    // Untouched siblings keep their defaults — installing a partial
    // configuration must not zero the rest.
    let d = SolverConfig::default();
    assert_eq!(
        eff.sep_logic.enable_frame_inference, d.sep_logic.enable_frame_inference,
        "an untouched field keeps its default"
    );
    assert_eq!(eff.unsat_core.max_iterations, d.unsat_core.max_iterations);

    // And the constructors whose contract is "give me the configured
    // object" read it, rather than each rebuilding `Default`.
    let sep = verum_smt::separation_logic::SeparationLogic::new();
    assert_eq!(
        sep.config().max_unfolding_depth,
        1,
        "SeparationLogic::new must read the installed configuration"
    );

    // A second install is refused — the configuration is decided once.
    assert!(
        config::install(SolverConfig::default()).is_err(),
        "a second install must be refused"
    );
}
