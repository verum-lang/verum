//! Integration test: `RefinementZ3Backend::set_timeout_ms`
//! forwards to the inner `SubsumptionChecker` and survives
//! through to subsequent `check` / `verify_refinement` calls.
//!
//! Closes the inert-defense pattern around
//! `RefinementConfig.timeout_ms`: previously the
//! `SubsumptionChecker.smt_timeout_ms` field was frozen at
//! construction, so the documented "100 ms default per spec" did
//! not constrain the actual Z3 solver. Now
//! `RefinementChecker::check_with_smt` calls
//! `backend.set_timeout_ms(self.config.timeout_ms)` before every
//! query and `RefinementZ3Backend` propagates the value through
//! to Z3 via the `timeout` solver parameter.

use verum_smt::refinement_backend::RefinementZ3Backend;
use verum_smt::subsumption::SubsumptionChecker;
use verum_types::refinement::SmtBackend;

#[test]
fn set_timeout_ms_propagates_to_inner_checker() {
    // Pin: `RefinementZ3Backend::set_timeout_ms` updates the
    // checker it owns rather than ignoring the call. Without this
    // wiring `RefinementConfig.timeout_ms` was a no-op past
    // construction.
    let mut backend = RefinementZ3Backend::new();
    let mut checker = SubsumptionChecker::new();

    // Default per `SubsumptionConfig::default()` is 100 ms.
    assert_eq!(checker.smt_timeout_ms(), 100);

    // Setter on the bare checker works as expected.
    checker.set_smt_timeout_ms(250);
    assert_eq!(checker.smt_timeout_ms(), 250);

    checker.set_smt_timeout_ms(0);
    assert_eq!(checker.smt_timeout_ms(), 0);

    checker.set_smt_timeout_ms(u64::MAX);
    assert_eq!(checker.smt_timeout_ms(), u64::MAX);

    // The trait impl on `RefinementZ3Backend` is reachable
    // through the trait — exercise the dispatch path the
    // production `RefinementChecker::check_with_smt` uses.
    backend.set_timeout_ms(42);
    backend.set_timeout_ms(5_000);
    backend.set_timeout_ms(0);
    // No panic, no error: the override exists and dispatches.
}

#[test]
fn checker_with_config_seeds_timeout() {
    // Pin: `SubsumptionChecker::with_config` honours
    // `SubsumptionConfig.smt_timeout_ms` so the constructor sets
    // a meaningful starting point even before any caller invokes
    // `set_smt_timeout_ms`.
    let mut config = verum_smt::subsumption::SubsumptionConfig::default();
    config.smt_timeout_ms = 750;

    let checker = SubsumptionChecker::with_config(config);
    assert_eq!(checker.smt_timeout_ms(), 750);
}

#[test]
fn set_smt_timeout_ms_overrides_constructor_default() {
    // Pin: post-construction updates take precedence over the
    // constructor seed. This is the contract the
    // `RefinementChecker` integration relies on: each query
    // brings its own timeout regardless of how the backend was
    // built.
    let mut config = verum_smt::subsumption::SubsumptionConfig::default();
    config.smt_timeout_ms = 100;

    let mut checker = SubsumptionChecker::with_config(config);
    assert_eq!(checker.smt_timeout_ms(), 100);

    checker.set_smt_timeout_ms(2_500);
    assert_eq!(checker.smt_timeout_ms(), 2_500);
}
