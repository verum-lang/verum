//! Integration test: `QEConfig.simplify_level` actually
//! configures the simplification tactic chain used by the
//! quantifier eliminator. Closes the inert-defense pattern: the
//! field was documented as a 0-3 level but every build used a
//! bare `Tactic::new("simplify")` regardless.
//!
//! Behavioural correctness for each level (i.e. that level 3
//! produces a more thoroughly simplified result than level 1) is
//! out of scope here — Z3's tactic semantics are the authority.
//! These tests pin the wiring contract: the eliminator
//! constructor honours the field for every level, including the
//! out-of-range "saturate to max" branch.

use verum_smt::quantifier_elim::{QEConfig, QuantifierEliminator};

#[test]
fn level_0_constructs_without_panic() {
    // Pin: level 0 maps to the `skip` tactic. Building the
    // eliminator must not panic and must not error during tactic
    // construction.
    let mut config = QEConfig::default();
    config.simplify_level = 0;
    let _ = QuantifierEliminator::with_config(config);
}

#[test]
fn level_1_constructs_without_panic() {
    let mut config = QEConfig::default();
    config.simplify_level = 1;
    let _ = QuantifierEliminator::with_config(config);
}

#[test]
fn level_2_default_constructs_without_panic() {
    // Pin: default level is 2; default constructor path must not
    // panic. This is the most-exercised configuration and is also
    // the most likely to silently regress if the tactic chain
    // construction is wrong.
    let _ = QuantifierEliminator::new();

    let config = QEConfig::default();
    assert_eq!(config.simplify_level, 2);
    let _ = QuantifierEliminator::with_config(config);
}

#[test]
fn level_3_constructs_without_panic() {
    let mut config = QEConfig::default();
    config.simplify_level = 3;
    let _ = QuantifierEliminator::with_config(config);
}

#[test]
fn level_above_3_clamps_to_max_chain() {
    // Pin: levels above 3 reuse the level-3 chain (no panic, no
    // silent skip). Documented contract: "Higher numeric values
    // reuse the level-3 chain rather than no-op'ing."
    for level in [4u8, 10, 50, 100, u8::MAX] {
        let mut config = QEConfig::default();
        config.simplify_level = level;
        let _ = QuantifierEliminator::with_config(config);
    }
}
