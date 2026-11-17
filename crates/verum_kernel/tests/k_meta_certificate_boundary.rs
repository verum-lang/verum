//! T1770: universe translation must preserve the certificate validation boundary.
use verum_kernel::proof_checker::{Certificate, CheckError, Level, Term};
use verum_kernel::proof_checker_meta::{shift_universes, verify_certificate_with_lift};
use verum_kernel::proof_checker_nbe;
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
    for lift in [0, 1, 2, 64, u32::MAX] {
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
    for lift in [0, 1, 2, 64, 4096, u32::MAX] {
        verify_certificate_with_lift(&cert, lift).unwrap();
        let lifted = Certificate {
            term: shift_universes(&cert.term, lift),
            claimed_type: shift_universes(&cert.claimed_type, lift),
            metadata: Default::default(),
        };
        let domain = Term::universe_at(Level::Concrete(lift));
        assert_eq!(lifted.term, Term::lam(domain.clone(), Term::var(0)));
        assert_eq!(lifted.claimed_type, Term::pi(domain.clone(), domain));
        proof_checker_nbe::verify_certificate(&lifted).unwrap();
        let wire = serde_json::to_vec(&lifted).unwrap();
        assert!(wire.len() < 512, "numeric lifts must remain compact");
        let restored: Certificate = serde_json::from_slice(&wire).unwrap();
        restored.verify().unwrap();
        proof_checker_nbe::verify_certificate(&restored).unwrap();

        // Both halves must be translated by the same amount. Merely admitting
        // a large offset must not identify adjacent universes at the boundary.
        let forged = Certificate {
            term: lifted.term,
            claimed_type: shift_universes(&lifted.claimed_type, 1),
            metadata: Default::default(),
        };
        for result in [
            forged.verify(),
            proof_checker_nbe::verify_certificate(&forged),
        ] {
            assert!(
                matches!(result, Err(CheckError::TypeMismatch { .. })),
                "{result:?}"
            );
        }
    }
}

#[test]
fn maximum_universe_argument_cannot_bypass_claimed_type_formation() {
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
    for lift in [0, 1, 2, u32::MAX] {
        assert!(verify_certificate_with_lift(&cert, lift).is_err());
    }
}
