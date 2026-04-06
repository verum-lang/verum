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
// Tests for unified module
// Migrated from src/unified.rs per CLAUDE.md standards

use verum_common::List;
use verum_error::unified::*;

#[test]
fn test_error_creation() {
    let err = VerumError::UseAfterFree {
        expected: 5,
        actual: 10,
    };
    assert!(err.to_string().contains("use after free"));
    assert!(err.is_fatal());
    assert!(!err.is_recoverable());
}

#[test]
fn test_type_mismatch() {
    let err = VerumError::TypeMismatch {
        expected: "Int".into(),
        actual: "String".into(),
    };
    assert!(err.to_string().contains("type mismatch"));
    assert!(!err.is_fatal());
}

#[test]
fn test_from_string() {
    let err: VerumError = "test error".into();
    assert!(matches!(err, VerumError::Other { .. }));
}

#[test]
fn test_from_io_error() {
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
    let err: VerumError = io_err.into();
    assert!(matches!(err, VerumError::IoError { .. }));
    assert!(err.is_recoverable());
}

#[test]
fn test_parse_errors_empty() {
    let err = VerumError::ParseErrors(List::new());
    let msg = err.to_string();
    assert!(msg.contains("parse errors"));
}

#[test]
fn test_parse_errors_multiple() {
    let mut errors = List::new();
    errors.push("error 1".into());
    errors.push("error 2".into());
    let err = VerumError::ParseErrors(errors);
    let msg = err.to_string();
    assert!(msg.contains("error 1"));
    assert!(msg.contains("error 2"));
}

#[test]
fn test_verification_timeout() {
    let err = VerumError::VerificationTimeout { timeout_ms: 5000 };
    assert!(err.to_string().contains("5000ms"));
    assert!(err.is_recoverable());
}

#[test]
fn test_message_constructor() {
    let err = VerumError::message("custom error");
    assert!(err.to_string().contains("custom error"));
}
