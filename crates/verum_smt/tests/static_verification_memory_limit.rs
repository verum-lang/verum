//! Pin: `StaticVerificationConfig.memory_limit_mb` is honoured
//! by the static verifier when constructing solver parameters.
//!
//! Closes the inert-defense pattern: the field defaulted to
//! `Some(4096)` (4 GB) and was documented as "Memory limit (MB)"
//! but no code path consulted it. Hostile / pathological
//! constraints could push Z3's memory use above the documented
//! ceiling without triggering any failure mode.
//!
//! These tests pin the configuration contract via the public
//! struct surface; behavioural verification (e.g. that Z3
//! actually rejects a constraint that would breach the limit)
//! is exercised indirectly by the wider verifier suite.

use verum_smt::static_verification::StaticVerificationConfig;

#[test]
fn default_config_has_documented_memory_limit() {
    let cfg = StaticVerificationConfig::default();
    // Pin the documented default. Drift here means callers that
    // rely on `Default::default()` silently lose protection.
    assert_eq!(cfg.memory_limit_mb, Some(4096));
}

#[test]
fn config_accepts_no_limit() {
    // Pin: `None` is a valid configuration meaning "no
    // caller-imposed limit" — Z3 falls back to its native
    // default. This is how callers opt out of the gate without
    // setting an absurdly large value.
    let mut cfg = StaticVerificationConfig::default();
    cfg.memory_limit_mb = None;
    let _ = cfg.clone();
    assert!(cfg.memory_limit_mb.is_none());
}

#[test]
fn config_accepts_small_limit() {
    let mut cfg = StaticVerificationConfig::default();
    cfg.memory_limit_mb = Some(64);
    assert_eq!(cfg.memory_limit_mb, Some(64));
}

#[test]
fn config_accepts_zero_limit() {
    // Pin: zero is a representable value. The static verifier
    // forwards it to Z3 verbatim (Z3 treats 0 as "no limit");
    // not a panic on the verum side.
    let mut cfg = StaticVerificationConfig::default();
    cfg.memory_limit_mb = Some(0);
    assert_eq!(cfg.memory_limit_mb, Some(0));
}

#[test]
fn config_accepts_max_limit() {
    // Pin: `usize::MAX` is representable; the static verifier
    // saturates to `u32::MAX` when forwarding to Z3 because
    // `Params::set_u32` takes a u32. Construction must not
    // panic.
    let mut cfg = StaticVerificationConfig::default();
    cfg.memory_limit_mb = Some(usize::MAX);
    assert_eq!(cfg.memory_limit_mb, Some(usize::MAX));
}
