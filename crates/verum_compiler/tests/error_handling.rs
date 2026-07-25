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
//!
//! Tests that the compiler properly reports errors for invalid code

use std::path::PathBuf;
use verum_compiler::{CompilerOptions, Session};

#[test]
fn test_session_creation() {
    // Basic test that Session can be created with default options
    let options = CompilerOptions {
        input: PathBuf::from("/tmp/test.vr"),
        output: PathBuf::from("/tmp/test.out"),
        ..Default::default()
    };

    let session = Session::new(options);

    // Should start with no errors
    assert!(!session.has_errors());
    assert_eq!(session.error_count(), 0);
    assert_eq!(session.warning_count(), 0);
}

#[test]
fn test_load_source_string() {
    let options = CompilerOptions {
        input: PathBuf::from("/tmp/test.vr"),
        output: PathBuf::from("/tmp/test.out"),
        ..Default::default()
    };

    let session = Session::new(options);

    // Load a simple valid source string
    let source = "// This is a comment";
    let result = session.load_source_string(source, PathBuf::from("<test>"));

    // Should successfully load the source
    assert!(result.is_ok());
    let file_id = result.unwrap();

    // Should be able to retrieve it
    let source_file = session.get_source(file_id);
    assert!(source_file.is_some());
}

#[test]
fn test_session_stats() {
    let options = CompilerOptions {
        input: PathBuf::from("/tmp/test.vr"),
        output: PathBuf::from("/tmp/test.out"),
        ..Default::default()
    };

    let session = Session::new(options);

    // Load a source file
    let _ = session.load_source_string("// test", PathBuf::from("<test>"));

    // Check stats
    let stats = session.stats();
    assert_eq!(stats.num_files, 1);
    assert_eq!(stats.num_errors, 0);
    assert_eq!(stats.num_warnings, 0);
}
