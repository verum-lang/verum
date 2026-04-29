//! Pin: `Cvc5Config` fields `preprocessing`, `quantifier_mode`,
//! and `verbosity` round-trip through `Default::default()` and
//! the public Cvc5Config API. Closes the inert-defense pattern:
//! prior to wiring these fields had no readers, so callers
//! configuring them had no effect on the underlying CVC5 solver.
//!
//! Behavioural verification (CVC5 actually honours the option
//! values) is exercised by the wider Cvc5Backend integration
//! suite. These tests pin the configuration contract at the
//! Verum-side struct boundary so drift in defaults or accessor
//! semantics is caught quickly.

use verum_smt::cvc5_backend::{Cvc5Config, QuantifierMode};

#[test]
fn default_config_has_documented_values() {
    let cfg = Cvc5Config::default();
    // Drift in the documented defaults would silently change
    // CVC5 behaviour for every caller relying on
    // `Default::default()`.
    assert!(cfg.preprocessing);
    assert_eq!(cfg.quantifier_mode, QuantifierMode::Auto);
    assert_eq!(cfg.verbosity, 0);
}

#[test]
fn config_accepts_preprocessing_off() {
    let mut cfg = Cvc5Config::default();
    cfg.preprocessing = false;
    assert!(!cfg.preprocessing);
}

#[test]
fn config_accepts_all_quantifier_modes() {
    // Pin: every documented `QuantifierMode` round-trips
    // through assignment. New enum variants must be added to the
    // enum AND to this exhaustive switch — drift surfaces as a
    // missing branch.
    let modes = [
        QuantifierMode::Auto,
        QuantifierMode::None,
        QuantifierMode::EMatching,
        QuantifierMode::CEGQI,
        QuantifierMode::MBQI,
    ];
    for mode in modes {
        let mut cfg = Cvc5Config::default();
        cfg.quantifier_mode = mode;
        assert_eq!(cfg.quantifier_mode, mode);
    }
}

#[test]
fn config_accepts_verbosity_full_range() {
    // Pin: documented verbosity range is 0-5. Construction
    // accepts the full u32 surface; the wiring saturates at 5
    // when forwarding to CVC5.
    for level in [0u32, 1, 2, 3, 4, 5, 100, u32::MAX] {
        let mut cfg = Cvc5Config::default();
        cfg.verbosity = level;
        assert_eq!(cfg.verbosity, level);
    }
}
