use verum_kernel::{
    proof_checker::{Certificate, CheckError, Foundation, Term},
    proof_checker_meta::check_universe_stability,
};

#[test]
fn unsupported_foundation_cannot_count_as_stable_rejection() {
    let function_type = Term::pi(Term::universe(0), Term::universe(0));
    let function = Term::lam(Term::universe(0), Term::var(0));
    let certificate = Certificate {
        term: Term::FunExt {
            function_type: Box::new(function_type.clone()),
            lhs: Box::new(function.clone()),
            rhs: Box::new(function.clone()),
            pointwise: Box::new(Term::lam(Term::universe(0), Term::refl(Term::var(0)))),
        },
        claimed_type: Term::id_ty(function_type, function.clone(), function),
        metadata: Default::default(),
    };
    certificate
        .verify_with_foundation(Foundation::FunctionExtensionality)
        .unwrap();
    let (verdicts, stable) = check_universe_stability(&certificate, 3);
    assert!(!stable);
    assert_eq!(verdicts.len(), 4);
    for result in verdicts {
        assert!(matches!(
            result,
            Err(CheckError::FoundationRequired(
                Foundation::FunctionExtensionality
            ))
        ));
    }
}
