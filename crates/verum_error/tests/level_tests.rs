#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
// Tests for level-specific error types across the 5-Level Error Defense Architecture:
// Level 0 (type prevention), Level 1 (static verification),
// Level 2 (explicit handling), Level 4 (security containment).

use verum_error::levels::{level0, level1, level2, level4};
use verum_error::{ErrorKind, VerumError};

// Level 0 Tests

#[test]
fn test_refinement_error() {
    let err = level0::RefinementError::new("x > 0")
        .with_value("-5")
        .with_expected("positive");

    assert_eq!(err.predicate.as_str(), "x > 0");
    assert_eq!(err.value.as_ref().unwrap().as_str(), "-5");
    assert_eq!(err.expected.as_ref().unwrap().as_str(), "positive");
}

#[test]
fn test_refinement_error_to_verum_error() {
    let err = level0::RefinementError::new("x > 0").with_value("-5");
    let verum_err: VerumError = err.into();

    assert_eq!(verum_err.kind(), ErrorKind::Refinement);
    assert!(verum_err.message().contains("x > 0"));
    assert!(verum_err.message().contains("-5"));
}

#[test]
fn test_affine_error_use_after_move() {
    let err = level0::AffineError::use_after_move("file");

    assert!(err.message.contains("after move"));
    assert_eq!(err.variable.as_ref().unwrap().as_str(), "file");
}

#[test]
fn test_affine_error_double_free() {
    let err = level0::AffineError::double_free("buffer");

    assert!(err.message.contains("freed twice"));
    assert_eq!(err.variable.as_ref().unwrap().as_str(), "buffer");
}

#[test]
fn test_context_requirement_error() {
    let err = level0::ContextRequirementError::new("FileIO").with_function("read_file");

    assert_eq!(err.context.as_str(), "FileIO");
    assert_eq!(err.function.as_ref().unwrap().as_str(), "read_file");
}

// Level 1 Tests

#[test]
fn test_verification_error() {
    let err = level1::VerificationError::new("x < 100").with_counterexample("x = 150");

    assert_eq!(err.property.as_str(), "x < 100");
    assert_eq!(err.counterexample.as_ref().unwrap().as_str(), "x = 150");
}

#[test]
fn test_verification_error_to_verum_error() {
    let err = level1::VerificationError::new("x >= 0").with_counterexample("x = -1");

    let verum_err: VerumError = err.into();
    assert_eq!(verum_err.kind(), ErrorKind::Verification);
    assert!(verum_err.message().contains("x >= 0"));
    assert!(verum_err.message().contains("x = -1"));
}

#[test]
fn test_proof_error() {
    let err = level1::ProofError::new("divisor != 0").with_reason("cannot prove non-zero");

    assert_eq!(err.obligation.as_str(), "divisor != 0");
    assert_eq!(
        err.reason.as_ref().unwrap().as_str(),
        "cannot prove non-zero"
    );
}

// Level 2 Tests

#[test]
fn test_result_ext_map_err_into() {
    use level2::ResultExt;

    let result: Result<i32, &str> = Err("error");
    let mapped = result.map_err_into(|s| s.to_uppercase());

    assert_eq!(mapped.unwrap_err(), "ERROR");
}

#[test]
fn test_result_ext_ok_or_none() {
    use level2::ResultExt;
    use verum_common::Maybe;

    let ok_result: Result<i32, &str> = Ok(42);
    let err_result: Result<i32, &str> = Err("error");

    assert_eq!(ok_result.ok_or_none(), Maybe::Some(42));
    assert_eq!(err_result.ok_or_none(), Maybe::None);
}

#[test]
fn test_result_ext_into_verum_error() {
    use level2::ResultExt;

    let result: Result<i32, std::io::Error> = Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "not found",
    ));

    let verum_result = result.into_verum_error();
    assert!(verum_result.is_err());
    assert_eq!(verum_result.unwrap_err().kind(), ErrorKind::IO);
}

// Level 4 Tests

#[test]
fn test_security_error() {
    let err = level4::SecurityError::new("Unauthorized access").with_policy("admin-only");

    assert_eq!(err.message.as_str(), "Unauthorized access");
    assert_eq!(err.policy.as_ref().unwrap().as_str(), "admin-only");
}

#[test]
fn test_capability_error() {
    let err = level4::CapabilityError::new("FileSystem").with_operation("write");

    assert_eq!(err.capability.as_str(), "FileSystem");
    assert_eq!(err.operation.as_ref().unwrap().as_str(), "write");
}

#[test]
fn test_sandbox_error() {
    let err = level4::SandboxError::new("Attempted to escape sandbox").with_operation("exec");

    assert!(err.message.contains("escape"));
    assert_eq!(err.operation.as_ref().unwrap().as_str(), "exec");
}

#[test]
fn test_security_error_to_verum_error() {
    let err = level4::SecurityError::new("Access denied");
    let verum_err: VerumError = err.into();

    assert_eq!(verum_err.kind(), ErrorKind::Security);
    assert!(verum_err.message().contains("Access denied"));
}
