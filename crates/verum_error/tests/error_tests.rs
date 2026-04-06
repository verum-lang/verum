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
// Tests for unified error hierarchy (VerumError, ErrorKind, error levels, recoverability).

use verum_error::{ErrorKind, Result, VerumError};

#[test]
fn test_error_creation() {
    let err = VerumError::new("test error", ErrorKind::IO);
    assert_eq!(err.kind(), ErrorKind::IO);
    assert_eq!(err.message().as_str(), "test error");
}

#[test]
fn test_error_kinds() {
    let errors = vec![
        VerumError::type_error("type"),
        VerumError::refinement("refinement"),
        VerumError::context("context"),
        VerumError::verification("verification"),
        VerumError::io("io"),
        VerumError::parse("parse"),
        VerumError::lex("lex"),
        VerumError::memory("memory"),
        VerumError::concurrency("concurrency"),
        VerumError::network("network"),
        VerumError::database("database"),
        VerumError::circuit_open("circuit"),
        VerumError::retry_exhausted("retry"),
        VerumError::supervision("supervision"),
        VerumError::timeout("timeout"),
        VerumError::security("security"),
        VerumError::codegen("codegen"),
        VerumError::config("config"),
        VerumError::not_implemented("feature"),
    ];

    let kinds = vec![
        ErrorKind::Type,
        ErrorKind::Refinement,
        ErrorKind::Context,
        ErrorKind::Verification,
        ErrorKind::IO,
        ErrorKind::Parse,
        ErrorKind::Lex,
        ErrorKind::Memory,
        ErrorKind::Concurrency,
        ErrorKind::Network,
        ErrorKind::Database,
        ErrorKind::CircuitOpen,
        ErrorKind::RetryExhausted,
        ErrorKind::Supervision,
        ErrorKind::Timeout,
        ErrorKind::Security,
        ErrorKind::Codegen,
        ErrorKind::Config,
        ErrorKind::NotImplemented,
    ];

    for (err, kind) in errors.iter().zip(kinds.iter()) {
        assert_eq!(err.kind(), *kind);
    }
}

#[test]
fn test_error_levels() {
    assert_eq!(ErrorKind::Type.level(), 0);
    assert_eq!(ErrorKind::Refinement.level(), 0);
    assert_eq!(ErrorKind::Verification.level(), 1);
    assert_eq!(ErrorKind::IO.level(), 2);
    assert_eq!(ErrorKind::CircuitOpen.level(), 3);
    assert_eq!(ErrorKind::Security.level(), 4);
}

#[test]
fn test_error_recoverability() {
    assert!(ErrorKind::IO.is_recoverable());
    assert!(ErrorKind::Network.is_recoverable());
    assert!(ErrorKind::Timeout.is_recoverable());
    assert!(!ErrorKind::Memory.is_recoverable());
    assert!(!ErrorKind::Type.is_recoverable());
}

#[test]
fn test_error_fatality() {
    assert!(ErrorKind::Memory.is_fatal());
    assert!(ErrorKind::Affine.is_fatal());
    assert!(ErrorKind::Sandbox.is_fatal());
    assert!(!ErrorKind::IO.is_fatal());
    assert!(!ErrorKind::Network.is_fatal());
}

#[test]
fn test_error_display() {
    let err = VerumError::io("connection failed");
    let display = format!("{}", err);
    assert!(display.contains("I/O"));
    assert!(display.contains("connection failed"));
}

#[test]
fn test_result_type() {
    fn returns_ok() -> Result<i32> {
        Ok(42)
    }

    fn returns_err() -> Result<i32> {
        Err(VerumError::io("error"))
    }

    assert_eq!(returns_ok().unwrap(), 42);
    assert!(returns_err().is_err());
}

#[test]
fn test_io_error_conversion() {
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
    let verum_err: VerumError = io_err.into();

    assert_eq!(verum_err.kind(), ErrorKind::IO);
    assert!(verum_err.message().contains("file not found"));
}
