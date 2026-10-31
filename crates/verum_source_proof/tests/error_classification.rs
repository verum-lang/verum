use verum_proof::proof_checker::{CheckError, Foundation, Term};
use verum_source_proof::scoped::Error;

#[test]
fn operational_failure_never_becomes_invalid_source() {
    assert!(matches!(
        Error::from(CheckError::FuelExhausted),
        Error::Inconclusive {
            code: "normalization_limit",
            ..
        }
    ));
    assert!(matches!(
        Error::from(CheckError::FoundationRequired(
            Foundation::FunctionExtensionality
        )),
        Error::Unsupported(_)
    ));
    assert!(matches!(
        Error::from(CheckError::TypeMismatch {
            expected: Term::universe(1),
            actual: Term::universe(2)
        }),
        Error::Rejected(_)
    ));
}
