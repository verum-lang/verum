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
// Unit tests for repl.rs
//
// Migrated from src/repl.rs to comply with CLAUDE.md test organization.

use std::path::PathBuf;
use verum_compiler::{CompilerOptions, Repl, Session};

#[test]
fn test_repl_creation() {
    let options = CompilerOptions::new(PathBuf::from("repl"), PathBuf::from("repl"));
    let session = Session::new(options);
    let repl = Repl::new(session);

    // REPL should be created successfully
    // This is a basic smoke test
    drop(repl);
}

#[test]
fn test_is_complete() {
    let options = CompilerOptions::new(PathBuf::from("repl"), PathBuf::from("repl"));
    let session = Session::new(options);
    let repl = Repl::new(session);

    // Test balanced braces
    assert!(repl.is_complete("let x = 42"));
    assert!(repl.is_complete("fn test() { }"));
    assert!(!repl.is_complete("fn test() { "));
    assert!(repl.is_complete("{ { } }"));
    assert!(!repl.is_complete("{ { }"));
}
