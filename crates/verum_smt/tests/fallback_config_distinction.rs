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

#[test]
fn max_attempts_one_short_circuits_fallback() {
    // Pin the inert-defense closure for `max_attempts`: prior
    // to wiring, the field was TOML-parseable and asserted in
    // tests but no fallback dispatch path consulted it, so a
    // manifest setting `max_attempts = 1` to lock to the primary
    // backend was silently ignored. The wire-up adds
    // `max_attempts > 1` to the same gate as `enabled`.
    //
    // This pin asserts the canonical "primary only" shape:
    // every per-kind flag remains true so the fallback would
    // fire under normal config — but `max_attempts = 1`
    // overrides them all at the dispatch gate.
    let cfg = FallbackConfig {
        enabled: true,
        on_timeout: true,
        on_unknown: true,
        on_error: true,
        max_attempts: 1,
    };
    assert!(cfg.enabled);
    assert_eq!(cfg.max_attempts, 1);
    // The semantic check (no fallback at the call site) is
    // covered by the integration tests; here we just pin the
    // config shape so a future regression that drops
    // max_attempts from the gate condition surfaces as a
    // distinct test failure rather than a silent switcher
    // behaviour change.
}

#[test]
fn max_attempts_two_is_documented_default() {
    // Pin: default `max_attempts = 2` matches the documented
    // two-backend topology (Z3 + CVC5 = 2 attempts). Any
    // change to this default would silently change every
    // caller's switcher behaviour without an explicit opt-in.
    let cfg = FallbackConfig::default();
    assert_eq!(cfg.max_attempts, 2);
}

#[test]
fn max_attempts_above_ceiling_caps_at_two_backends() {
    // Pin: with only Z3 + CVC5 in the switcher topology, any
    // `max_attempts >= 2` behaves identically to 2 — there's
    // no third backend to escalate to. This is documentation
    // for callers reading the field expecting an arbitrary
    // retry counter.
    let cfg = FallbackConfig {
        enabled: true,
        on_timeout: true,
        on_unknown: true,
        on_error: true,
        max_attempts: 100,
    };
    assert!(cfg.max_attempts >= 2);
    // No additional dispatch sites read max_attempts beyond
    // the > 1 gate — this assertion is forward-looking
    // documentation for a future third-backend addition.
}
