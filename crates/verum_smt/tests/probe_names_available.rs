//! Pins every z3 probe name the strategy selector requests against the
//! linked z3 build.
//!
//! `z3::Probe::new` unwraps the raw `Z3_mk_probe` result, so an unknown name
//! does not surface as an error — it aborts whoever called the selector,
//! which in production is the compiler partway through a solve. Which probes
//! exist is a property of the z3 build rather than of the goal, so it cannot
//! be validated from the input and has to be pinned here.
//!
//! This is not hypothetical: `is-qfuf` does not exist in z3 0.20, and asking
//! for it made every `select_tactic` call with a non-empty goal abort.

use verum_smt::strategy_selection::PROBE_NAMES;

#[test]
fn every_probe_name_exists_in_this_z3_build() {
    for name in PROBE_NAMES {
        // Panics (aborting this test, loudly and by name) if z3 does not
        // know the probe — which is the whole point.
        let _ = z3::Probe::new(name);
    }
}

#[test]
fn probe_names_are_unique() {
    let mut seen: Vec<&str> = PROBE_NAMES.to_vec();
    seen.sort_unstable();
    let before = seen.len();
    seen.dedup();
    assert_eq!(before, seen.len(), "duplicate probe name in PROBE_NAMES");
}
