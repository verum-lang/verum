//! A known rule name and a self-consistent hash cannot prove a false conclusion.
use verum_verification::cert_replay::{
    CertFormat, CertReplayEngine, KernelOnlyReplayEngine, ReplayVerdict, SmtCertificate,
    cross_check,
};
#[test]
fn forged_reflexivity_with_recomputed_hash_has_no_proof_authority() {
    for conclusion in ["(= 0 1)", "false", "(forall ((P Bool)) P)"] {
        let cert = SmtCertificate::new(
            CertFormat::VerumCanonical,
            "QF_UF",
            conclusion,
            format!("step s1 refl : {conclusion}"),
        );
        assert!(cert.body_hash_valid());
        let verdict = KernelOnlyReplayEngine::new().replay(&cert).unwrap();
        assert!(matches!(verdict, ReplayVerdict::StructuralOnly { .. }));
        assert!(!verdict.is_accepted());
        let consensus = cross_check(&cert, &[]);
        assert!(!consensus.all_available_accept());
        assert_eq!(consensus.accept_count(), 0);
    }
}
