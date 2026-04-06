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
// Tests for lexical_parsing module
// Migrated from src/lexical_parsing.rs per CLAUDE.md standards

use std::path::PathBuf;
use verum_compiler::phases::lexical_parsing::LexicalParsingPhase;

#[test]
fn test_parse_simple_function() {
    let mut phase = LexicalParsingPhase::new();
    let source = r#"
            fn hello() {
                println("Hello, World!");
            }
        "#;

    let result = phase.parse_file(PathBuf::from("test.vr"), source);
    assert!(result.is_ok());
}

#[test]
fn test_parse_error_recovery() {
    let mut phase = LexicalParsingPhase::new();
    let source = r#"
            fn broken( {
                // Missing closing paren
            }
        "#;

    let result = phase.parse_file(PathBuf::from("test.vr"), source);
    assert!(result.is_err());
}
