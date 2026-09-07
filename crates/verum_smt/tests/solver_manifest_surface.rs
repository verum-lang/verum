//! `[verify.solver.*]` reaches the solver configs — T1233.
//!
//! Before this surface existed the sub-tables were documented in two
//! places (47 keys, ten sections) and read in none: `VerifyConfig` had
//! no `solver` field, so serde dropped the whole subtree in silence.
//! These tests pin the three properties that make the difference
//! observable, and each one FAILED before the change:
//!
//!   1. a key set in TOML arrives in the struct the solver runs on;
//!   2. a PARTIAL table leaves its siblings at their defaults, so a
//!      manifest may set one key of forty-seven;
//!   3. a MISSPELLED key is refused rather than ignored — silence was
//!      the defect, and a fix that stays silent about typos only moves
//!      it one level down.

use verum_smt::config::SolverConfig;

/// Property 1 — the value arrives, in every sub-table.
///
/// One key per section rather than one section: a surface that wires
/// some tables and drops others fails in exactly the way the original
/// defect did, and a single-section test would not see it.
#[test]
fn every_documented_subtable_reaches_its_struct() {
    let cfg: SolverConfig = toml::from_str(
        r#"
[bisimulation]
max_depth = 7

[interpolation]
max_projection_vars = 11

[optimizer]
max_solutions = 13

[parallel]
num_workers = 17

[qe]
simplify_level = 3

[sep_logic]
max_unfolding_depth = 19

[unsat_core]
max_iterations = 23

[static]
max_cache_size = 29

[z3]
num_workers = 31

[cvc5]
verbosity = 5
"#,
    )
    .expect("every documented sub-table must deserialize");

    assert_eq!(cfg.bisimulation.max_depth, 7);
    assert_eq!(cfg.interpolation.max_projection_vars, 11);
    assert_eq!(cfg.optimizer.max_solutions, Some(13));
    assert_eq!(cfg.parallel.num_workers, 17);
    assert_eq!(cfg.qe.simplify_level, 3);
    assert_eq!(cfg.sep_logic.max_unfolding_depth, 19);
    assert_eq!(cfg.unsat_core.max_iterations, 23);
    assert_eq!(cfg.static_verification.max_cache_size, 29);
    assert_eq!(cfg.z3.num_workers, 31);
    assert_eq!(cfg.cvc5.verbosity, 5);
}

/// Property 2 — one key does not reset its forty-six siblings.
///
/// This is what `#[serde(default)]` on every config buys, and it is the
/// difference between a usable manifest section and one where setting a
/// timeout silently disables proofs.
#[test]
fn a_partial_table_leaves_its_siblings_alone() {
    let cfg: SolverConfig = toml::from_str("[sep_logic]\nmax_unfolding_depth = 1\n")
        .expect("a one-key table must deserialize");
    let d = SolverConfig::default();

    assert_eq!(cfg.sep_logic.max_unfolding_depth, 1, "the set key changed");
    assert_eq!(
        cfg.sep_logic.entailment_timeout_ms, d.sep_logic.entailment_timeout_ms,
        "a sibling key in the SAME table must keep its default"
    );
    assert_eq!(
        cfg.sep_logic.enable_frame_inference, d.sep_logic.enable_frame_inference,
        "a sibling key in the SAME table must keep its default"
    );
    assert_eq!(
        cfg.qe.simplify_level, d.qe.simplify_level,
        "an untouched SIBLING TABLE must keep its defaults"
    );
}

/// Property 3 — a typo is an error, not a shrug.
#[test]
fn a_misspelled_key_is_refused() {
    let e = toml::from_str::<SolverConfig>("[sep_logic]\nmax_unfoldng_depth = 1\n")
        .expect_err("a misspelled KEY must be refused, not ignored");
    let msg = e.to_string();
    assert!(
        msg.contains("max_unfoldng_depth"),
        "the message must name the offending key, got: {msg}"
    );

    let e = toml::from_str::<SolverConfig>("[sep-logic]\nmax_unfolding_depth = 1\n")
        .expect_err("a misspelled TABLE must be refused, not ignored");
    assert!(
        e.to_string().contains("sep-logic"),
        "the message must name the offending table, got: {e}"
    );
}

/// `is_default` distinguishes "the manifest said nothing" from "the
/// manifest asked for the defaults" — the caller logs only the second.
#[test]
fn is_default_tracks_whether_anything_was_set() {
    assert!(SolverConfig::default().is_default());

    let untouched: SolverConfig = toml::from_str("").expect("an empty table is a valid config");
    assert!(untouched.is_default(), "an empty manifest section is default");

    let touched: SolverConfig =
        toml::from_str("[qe]\nsimplify_level = 0\n").expect("must deserialize");
    assert!(!touched.is_default(), "a set key is not the default");
}
