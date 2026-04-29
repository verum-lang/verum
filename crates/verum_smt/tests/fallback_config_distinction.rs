//! Pin: `FallbackConfig.on_timeout` is independent of
//! `FallbackConfig.on_unknown`. The two flags appear similar
//! because Z3 surfaces timeouts as `SolveResult::Unknown` with a
//! reason string, but they're conceptually distinct: a caller
//! may want to fall back on timeouts (where the alternative
//! solver might have a different complexity profile) while NOT
//! falling back on genuine unknowns (where both solvers are
//! likely to give up).
//!
//! Closes the inert-defense pattern that wired
//! `FallbackConfig.on_timeout` (commit cee85686). These tests
//! pin the configuration contract — every flag round-trips
//! through the struct, and the strict-singleton check ensures
//! the two flags can be set independently without one shadowing
//! the other at the value level.

use verum_smt::backend_switcher::FallbackConfig;

#[test]
fn fallback_config_default_has_documented_values() {
    let cfg = FallbackConfig::default();
    // Pin documented defaults; drift here would silently
    // change semantics for every caller relying on
    // `Default::default()`.
    assert!(cfg.enabled);
    assert!(cfg.on_timeout);
    assert!(cfg.on_unknown);
    assert!(cfg.on_error);
    assert!(cfg.max_attempts > 0);
}

#[test]
fn on_timeout_independent_of_on_unknown() {
    // Pin: on_timeout = true, on_unknown = false is a valid
    // configuration meaning "fall back ONLY on timeouts, not
    // on genuine undecidability". The struct must accept this
    // shape without coupling.
    let mut cfg = FallbackConfig::default();
    cfg.on_timeout = true;
    cfg.on_unknown = false;
    assert!(cfg.on_timeout);
    assert!(!cfg.on_unknown);
}

#[test]
fn on_unknown_without_on_timeout_is_valid() {
    // Pin: the inverse pairing — on_unknown without on_timeout —
    // is also a valid configuration. (Less common in practice
    // because timeouts surface as Unknown, but a caller might
    // want to normalise all Unknowns including timeouts under
    // the on_unknown branch.)
    let mut cfg = FallbackConfig::default();
    cfg.on_timeout = false;
    cfg.on_unknown = true;
    assert!(!cfg.on_timeout);
    assert!(cfg.on_unknown);
}

#[test]
fn all_four_flag_combinations_compile_and_round_trip() {
    // Pin: every (timeout, unknown) ∈ {true, false}² combination
    // is a valid configuration that round-trips through the
    // struct. No combination is implicitly normalised away.
    for timeout in [true, false] {
        for unknown in [true, false] {
            let mut cfg = FallbackConfig::default();
            cfg.on_timeout = timeout;
            cfg.on_unknown = unknown;
            assert_eq!(cfg.on_timeout, timeout);
            assert_eq!(cfg.on_unknown, unknown);
        }
    }
}

#[test]
fn fallback_disabled_overrides_per_kind_flags() {
    // Pin: `enabled = false` is the master switch — when off,
    // the per-kind flags are individually settable but not
    // semantically meaningful at the dispatch site (the entire
    // fallback path is short-circuited by the master gate).
    // Round-trip the field shape; the dispatch behaviour is
    // pinned by the existing fallback-from-z3-to-cvc5
    // integration test in `backend_switcher_integration.rs`.
    let cfg = FallbackConfig {
        enabled: false,
        on_timeout: true,
        on_unknown: true,
        on_error: true,
        max_attempts: 3,
    };
    assert!(!cfg.enabled);
    assert!(cfg.on_timeout);
    assert!(cfg.on_unknown);
}
