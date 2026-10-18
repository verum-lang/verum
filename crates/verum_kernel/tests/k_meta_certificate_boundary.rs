//! T1770: universe translation must preserve the certificate validation boundary.
use verum_kernel::proof_checker::{Certificate, Term};
use verum_kernel::proof_checker_meta::verify_certificate_with_lift;
#[test]
fn malformed_claimed_type_cannot_hide_behind_beta_conversion_in_meta_mode() {
    let cert = Certificate {
        term: Term::universe(0),
        claimed_type: Term::app(
            Term::lam(Term::universe(0), Term::var(0)),
            Term::universe(1),
        ),
        metadata: Default::default(),
    };
    assert!(cert.verify().is_err());
    for lift in [0, 1, 2, 64] {
        assert!(verify_certificate_with_lift(&cert, lift).is_err());
    }
}
#[test]
fn valid_identity_remains_valid_under_supported_lifts() {
    let cert = Certificate {
        term: Term::lam(Term::universe(0), Term::var(0)),
        claimed_type: Term::pi(Term::universe(0), Term::universe(0)),
        metadata: Default::default(),
    };
    for lift in [0, 1, 2, 64, 4096] {
        verify_certificate_with_lift(&cert, lift).unwrap();
    }
    // The concrete inference ceiling is a refusal, not silent saturation.
    assert!(verify_certificate_with_lift(&cert, u32::MAX).is_err());
}

#[test]
fn nested_universe_overflow_is_not_evidence_that_a_claimed_type_is_well_formed() {
    let cert = Certificate {
        term: Term::universe(0),
        claimed_type: Term::app(
            Term::lam(Term::universe(u32::MAX), Term::universe(1)),
            Term::universe(u32::MAX),
        ),
        metadata: Default::default(),
    };
    assert!(cert.verify().is_err());
    assert!(verum_kernel::proof_checker_nbe::verify_certificate(&cert).is_err());
    for lift in [0, 1, 2] {
        assert!(verify_certificate_with_lift(&cert, lift).is_err());
    }
}
