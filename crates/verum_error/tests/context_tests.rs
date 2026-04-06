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
// Tests for context module
// Migrated from src/context.rs per CLAUDE.md standards

use verum_error::context::*;
use verum_error::unified::VerumError;

#[test]
fn test_context_error_creation() {
    let err = VerumError::message("connection refused");
    let ctx_err = ContextError::new(err, "Failed to connect");

    assert_eq!(ctx_err.context().as_str(), "Failed to connect");
}

#[test]
fn test_context_chaining() {
    let err = VerumError::message("connection refused");
    let ctx1 = ContextError::new(err, "Database connection failed");
    let ctx2 = ctx1.with_context("Failed to initialize app");

    let contexts: Vec<_> = ctx2.contexts().map(|c| c.context().as_str()).collect();
    assert_eq!(contexts.len(), 2);
    assert_eq!(contexts[0], "Failed to initialize app");
    assert_eq!(contexts[1], "Database connection failed");
}

#[test]
fn test_result_context() {
    let result: Result<i32, VerumError> = Err(VerumError::message("error"));
    let ctx_result = result.context("Operation failed");

    assert!(ctx_result.is_err());
    if let Err(e) = ctx_result {
        assert_eq!(e.context().as_str(), "Operation failed");
    }
}

#[test]
fn test_with_context_not_called_on_success() {
    let mut called = false;
    let result: Result<i32, VerumError> = Ok(42);

    let _ = result.with_context(|| {
        called = true;
        "Should not be called".into()
    });

    assert!(!called, "Context closure should not be called on success");
}

#[test]
fn test_with_context_called_on_error() {
    let mut called = false;
    let result: Result<i32, VerumError> = Err(VerumError::message("error"));

    let _ = result.with_context(|| {
        called = true;
        "Called on error".into()
    });

    assert!(called, "Context closure should be called on error");
}

#[test]
fn test_context_chain_display() {
    let err = VerumError::message("connection refused");
    let ctx1 = ContextError::new(err, "Database error");
    let ctx2 = ctx1.with_context("App initialization failed");

    let display = format!("{}", ctx2);
    assert!(display.contains("App initialization failed"));
    assert!(display.contains("Database error"));
    assert!(display.contains("connection refused"));
}
