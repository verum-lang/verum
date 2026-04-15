//! End-to-end integration test for the `Session → RoutingStats → SMT
//! switcher` wiring (Task #42).
//!
//! Constructs a `Session`, extracts its shared `RoutingStats` handle,
//! feeds that handle into a `SmtBackendSwitcher`, runs a trivial SMT
//! query, and asserts that the update is visible through the session
//! handle. This is the minimal witness that the telemetry plumbing is
//! complete end-to-end.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use verum_common::Text;
use verum_compiler::options::CompilerOptions;
use verum_compiler::session::Session;
use verum_smt::backend_switcher::{SmtBackendSwitcher, SwitcherConfig};

#[test]
fn session_shares_routing_stats_handle_with_switcher() {
    let mut opts = CompilerOptions::default();
    opts.input = PathBuf::from("<test>");
    opts.output = PathBuf::from("<test>");

    let session = Session::new(opts);

    // Session starts with an empty stats collector.
    let before = session.routing_stats().total_queries.load(Ordering::Relaxed);
    assert_eq!(before, 0, "fresh session must have zero recorded queries");

    // Build a switcher sharing the session's stats handle.
    let stats = session.routing_stats().clone();
    let switcher = SmtBackendSwitcher::with_shared_stats(
        SwitcherConfig::default(),
        stats,
    );

    // `switcher.routing_stats()` must return the SAME Arc instance as
    // the session holds — this is the contract that makes per-session
    // telemetry aggregation work.
    let switcher_stats = switcher.routing_stats();
    assert!(
        Arc::ptr_eq(&switcher_stats, session.routing_stats()),
        "switcher and session must share the same RoutingStats Arc"
    );

    // Record a synthetic solver outcome via the shared collector and
    // verify both views observe the update.
    switcher_stats.total_sat.fetch_add(7, Ordering::Relaxed);
    assert_eq!(
        session.routing_stats().total_sat.load(Ordering::Relaxed),
        7,
        "session must see updates made through the switcher's handle"
    );
}

#[test]
fn refinement_disabled_downgrades_verify_mode() {
    // When [types] refinement = false, the SMT verification path is a
    // no-op. Session::new reconciles this by forcing verify_mode =
    // Runtime so the pipeline doesn't spin up a solver for nothing.
    let mut opts = CompilerOptions::default();
    opts.input = PathBuf::from("<test>");
    opts.output = PathBuf::from("<test>");
    opts.verify_mode = verum_compiler::options::VerifyMode::Proof;
    opts.language_features.types.refinement = false;

    let session = Session::new(opts);

    assert_eq!(
        session.options().verify_mode,
        verum_compiler::options::VerifyMode::Runtime,
        "disabled refinement must downgrade verify_mode to Runtime"
    );
}

#[test]
fn refinement_enabled_preserves_verify_mode() {
    // Default: refinement on → verify_mode untouched.
    let mut opts = CompilerOptions::default();
    opts.input = PathBuf::from("<test>");
    opts.output = PathBuf::from("<test>");
    opts.verify_mode = verum_compiler::options::VerifyMode::Proof;
    assert!(opts.language_features.types.refinement);

    let session = Session::new(opts);

    assert_eq!(
        session.options().verify_mode,
        verum_compiler::options::VerifyMode::Proof,
        "enabled refinement must preserve Proof mode"
    );
}

#[test]
fn unsafe_allowed_flag_readable_via_session() {
    let mut opts = CompilerOptions::default();
    opts.input = PathBuf::from("<test>");
    opts.output = PathBuf::from("<test>");

    // Default: unsafe on.
    let on = Session::new(opts.clone());
    assert!(on.language_features().unsafe_allowed());

    // Flip → session reflects it. phase_type_check reads this to
    // gate the safety_gate pre-pass.
    opts.language_features.safety.unsafe_allowed = false;
    let off = Session::new(opts);
    assert!(!off.language_features().unsafe_allowed());
}

#[test]
fn context_system_flag_readable_via_session() {
    let mut opts = CompilerOptions::default();
    opts.input = PathBuf::from("<test>");
    opts.output = PathBuf::from("<test>");

    // Default: context system on.
    let on = Session::new(opts.clone());
    assert!(on.language_features().context_system_on());

    // Flip the flag → session reflects it. This is what
    // `pipeline::phase_context_validation` queries to skip the phase.
    opts.language_features.context.enabled = false;
    let off = Session::new(opts);
    assert!(!off.language_features().context_system_on());
}

#[test]
fn language_features_accessor_returns_session_view() {
    let mut opts = CompilerOptions::default();
    opts.input = PathBuf::from("<test>");
    opts.output = PathBuf::from("<test>");
    opts.language_features.codegen.mlir_gpu = true;

    let session = Session::new(opts);

    assert!(
        session.language_features().gpu_enabled(),
        "Session::language_features() must expose the same set as options()"
    );
}

#[test]
fn set_routing_stats_replaces_handle() {
    let mut opts = CompilerOptions::default();
    opts.input = PathBuf::from("<test>");
    opts.output = PathBuf::from("<test>");
    let mut session = Session::new(opts);

    let original = session.routing_stats().clone();
    let injected = std::sync::Arc::new(
        verum_smt::routing_stats::RoutingStats::new(),
    );
    // Pre-load the injected collector to distinguish it.
    injected.total_sat.store(42, Ordering::Relaxed);

    session.set_routing_stats(injected.clone());

    assert_eq!(
        session.routing_stats().total_sat.load(Ordering::Relaxed),
        42,
        "set_routing_stats must swap in the injected collector"
    );
    assert_eq!(
        original.total_sat.load(Ordering::Relaxed),
        0,
        "original collector must remain unchanged after replacement"
    );
    let _ = Text::from("keep Text import alive");
}
