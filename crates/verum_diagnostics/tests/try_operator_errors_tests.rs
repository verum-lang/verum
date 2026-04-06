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
//! Comprehensive tests for try operator (?) error diagnostics.
//!
//! Tests for E0203 (Result type mismatch), E0204 (ambiguous multiple conversion paths),
//! and E0205 (try operator in non-Result context). The '?' operator desugars to
//! match expr { Ok(v) => v, Err(e) => return Err(e.into()) } and requires compatible
//! error types with From implementations for automatic conversion.

use verum_common::span::LineColSpan as Span;
use verum_common::{List, Text};
use verum_diagnostics::{
    Severity,
    codes::{E0203, E0204, E0205},
    e0203_result_type_mismatch, e0204_multiple_conversion_paths, e0205_nested_try_operator,
    e0205_try_in_non_result_context,
};

/// Helper to create a dummy span
fn span(line: usize, col: usize) -> Span {
    Span::new("test.vr", line, col, col + 10)
}

#[test]
fn test_e0203_basic_structure() {
    let diag = e0203_result_type_mismatch(
        span(10, 5),
        &Text::from("IoError"),
        &Text::from("AppError"),
        span(10, 1),
        Some(span(5, 1)),
    );

    // Verify basic structure
    assert_eq!(diag.code(), Some(E0203));
    assert_eq!(diag.severity(), Severity::Error);
    assert!(diag.message().contains("type mismatch"));
    assert!(diag.message().contains("IoError"));
    assert!(diag.message().contains("AppError"));

    // Verify we have multiple spans
    assert!(!diag.primary_labels().is_empty());
    assert!(!diag.secondary_labels().is_empty());
}

#[test]
fn test_e0203_has_from_implementation_suggestion() {
    let diag = e0203_result_type_mismatch(
        span(10, 5),
        &Text::from("ParseError"),
        &Text::from("AppError"),
        span(10, 1),
        None,
    );

    let helps_text: Vec<String> = diag.helps().iter().map(|h| h.message.to_string()).collect();

    // Must have From implementation suggestion
    assert!(
        helps_text.iter().any(|h| h.contains("implement From")),
        "Missing From implementation suggestion"
    );

    // Verify it shows the correct types
    let from_help = helps_text
        .iter()
        .find(|h| h.contains("implement From"))
        .unwrap();
    assert!(from_help.contains("ParseError"));
    assert!(from_help.contains("AppError"));
}

#[test]
fn test_e0203_has_map_err_suggestion() {
    let diag = e0203_result_type_mismatch(
        span(10, 5),
        &Text::from("DbError"),
        &Text::from("AppError"),
        span(10, 1),
        None,
    );

    let helps_text: Vec<String> = diag.helps().iter().map(|h| h.message.to_string()).collect();

    // Must have map_err suggestion
    assert!(
        helps_text.iter().any(|h| h.contains("map_err")),
        "Missing map_err suggestion"
    );
}

#[test]
fn test_e0203_has_match_suggestion() {
    let diag = e0203_result_type_mismatch(
        span(10, 5),
        &Text::from("NetworkError"),
        &Text::from("AppError"),
        span(10, 1),
        None,
    );

    let helps_text: Vec<String> = diag.helps().iter().map(|h| h.message.to_string()).collect();

    // Must have match suggestion
    assert!(
        helps_text.iter().any(|h| h.contains("match")),
        "Missing match suggestion"
    );
}

#[test]
fn test_e0203_has_universal_conversion_suggestion() {
    let diag = e0203_result_type_mismatch(
        span(10, 5),
        &Text::from("IoError"),
        &Text::from("AppError"),
        span(10, 1),
        None,
    );

    let helps_text: Vec<String> = diag.helps().iter().map(|h| h.message.to_string()).collect();

    // Must have universal conversion pattern suggestion
    assert!(
        helps_text
            .iter()
            .any(|h| h.contains("universal") || h.contains("<E: Error>")),
        "Missing universal error conversion suggestion"
    );
}

#[test]
fn test_e0203_includes_notes_with_context() {
    let diag = e0203_result_type_mismatch(
        span(10, 5),
        &Text::from("IoError"),
        &Text::from("AppError"),
        span(10, 1),
        None,
    );

    let notes_text: Vec<String> = diag.notes().iter().map(|n| n.message.to_string()).collect();

    // Must explain the requirement
    assert!(
        notes_text.iter().any(|n| n.contains("From")),
        "Missing explanation of From requirement"
    );
}

#[test]
fn test_e0203_includes_inline_guidance() {
    let diag = e0203_result_type_mismatch(
        span(10, 5),
        &Text::from("IoError"),
        &Text::from("AppError"),
        span(10, 1),
        None,
    );

    let notes_text: Vec<String> = diag.notes().iter().map(|n| n.message.to_string()).collect();

    // Must include self-contained implementation guidance about From conversion
    assert!(
        notes_text.iter().any(|n| n.contains("From")),
        "Missing inline guidance about From conversion requirement"
    );
}

#[test]
fn test_e0204_multiple_paths_structure() {
    let paths = List::from(vec![
        Text::from("ErrorA -> AppError (direct)"),
        Text::from("ErrorA -> ErrorB -> AppError (indirect)"),
    ]);

    let diag = e0204_multiple_conversion_paths(
        span(15, 10),
        &Text::from("ErrorA"),
        &Text::from("AppError"),
        &paths,
    );

    // Verify basic structure
    assert_eq!(diag.code(), Some(E0204));
    assert_eq!(diag.severity(), Severity::Error);
    assert!(diag.message().contains("multiple conversion paths"));
    assert!(diag.message().contains("ErrorA"));
    assert!(diag.message().contains("AppError"));
}

#[test]
fn test_e0204_lists_all_paths() {
    let paths = List::from(vec![
        Text::from("Path 1"),
        Text::from("Path 2"),
        Text::from("Path 3"),
    ]);

    let diag = e0204_multiple_conversion_paths(
        span(15, 10),
        &Text::from("SourceError"),
        &Text::from("TargetError"),
        &paths,
    );

    let notes_text = diag
        .notes()
        .iter()
        .map(|n| n.message.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    // All paths should be listed
    assert!(notes_text.contains("Path 1"));
    assert!(notes_text.contains("Path 2"));
    assert!(notes_text.contains("Path 3"));
}

#[test]
fn test_e0204_suggests_explicit_conversion() {
    let paths = List::from(vec![Text::from("ErrorA -> AppError (direct)")]);

    let diag = e0204_multiple_conversion_paths(
        span(15, 10),
        &Text::from("ErrorA"),
        &Text::from("AppError"),
        &paths,
    );

    let helps_text: Vec<String> = diag.helps().iter().map(|h| h.message.to_string()).collect();

    // Must suggest explicit conversion
    assert!(
        helps_text.iter().any(|h| h.contains("explicit")),
        "Missing explicit conversion suggestion"
    );
}

#[test]
fn test_e0205_non_result_context_structure() {
    let diag = e0205_try_in_non_result_context(
        span(20, 15),
        &Text::from("Result<Text, IoError>"),
        &Text::from("Int"),
        Some(&Text::from("compute")),
        Some(span(15, 1)),
    );

    // Verify basic structure
    assert_eq!(diag.code(), Some(E0205));
    assert_eq!(diag.severity(), Severity::Error);
    assert!(diag.message().contains("cannot use '?' operator"));
    assert!(diag.message().contains("Int"));
}

#[test]
fn test_e0205_suggests_changing_return_type() {
    let diag = e0205_try_in_non_result_context(
        span(20, 15),
        &Text::from("Result<Text, IoError>"),
        &Text::from("Text"),
        Some(&Text::from("process_data")),
        None,
    );

    let helps_text: Vec<String> = diag.helps().iter().map(|h| h.message.to_string()).collect();

    // Must suggest changing return type
    assert!(
        helps_text
            .iter()
            .any(|h| h.contains("return type") && h.contains("Result")),
        "Missing return type change suggestion"
    );

    // Should include the actual function name
    let return_type_help = helps_text
        .iter()
        .find(|h| h.contains("return type"))
        .unwrap();
    assert!(return_type_help.contains("process_data"));
}

#[test]
fn test_e0205_suggests_explicit_handling() {
    let diag = e0205_try_in_non_result_context(
        span(20, 15),
        &Text::from("Result<Int, Error>"),
        &Text::from("Int"),
        None,
        None,
    );

    let helps_text: Vec<String> = diag.helps().iter().map(|h| h.message.to_string()).collect();

    // Must suggest match handling
    assert!(
        helps_text.iter().any(|h| h.contains("match")),
        "Missing match handling suggestion"
    );
}

#[test]
fn test_e0205_suggests_unwrap_alternatives() {
    let diag = e0205_try_in_non_result_context(
        span(20, 15),
        &Text::from("Result<Int, Error>"),
        &Text::from("Int"),
        None,
        None,
    );

    let helps_text: Vec<String> = diag.helps().iter().map(|h| h.message.to_string()).collect();

    // Must suggest unwrap (even though not recommended)
    assert!(
        helps_text.iter().any(|h| h.contains("unwrap")),
        "Missing unwrap suggestion"
    );

    // Must suggest unwrap_or as a better alternative
    assert!(
        helps_text.iter().any(|h| h.contains("unwrap_or")),
        "Missing unwrap_or suggestion"
    );
}

#[test]
fn test_e0205_mentions_function_context() {
    let diag = e0205_try_in_non_result_context(
        span(20, 15),
        &Text::from("Result<Text, IoError>"),
        &Text::from("Unit"),
        Some(&Text::from("main")),
        Some(span(1, 1)),
    );

    // Should reference function name in notes
    let notes_text = diag
        .notes()
        .iter()
        .map(|n| n.message.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(notes_text.contains("main") || diag.message().contains("main"));
}

#[test]
fn test_e0205_nested_try_structure() {
    let diag = e0205_nested_try_operator(
        span(25, 20),
        &Text::from("IoError"),
        &Text::from("ParseError"),
    );

    // Verify basic structure
    assert_eq!(diag.code(), Some(E0205));
    assert_eq!(diag.severity(), Severity::Error);
    assert!(diag.message().contains("nested"));
}

#[test]
fn test_e0205_nested_suggests_flatten() {
    let diag = e0205_nested_try_operator(
        span(25, 20),
        &Text::from("IoError"),
        &Text::from("ParseError"),
    );

    let helps_text: Vec<String> = diag.helps().iter().map(|h| h.message.to_string()).collect();

    // Must suggest flattening
    assert!(
        helps_text
            .iter()
            .any(|h| h.contains("flatten") || h.contains("and_then")),
        "Missing flatten suggestion"
    );
}

#[test]
fn test_e0205_nested_suggests_restructuring() {
    let diag = e0205_nested_try_operator(
        span(25, 20),
        &Text::from("IoError"),
        &Text::from("ParseError"),
    );

    let helps_text: Vec<String> = diag.helps().iter().map(|h| h.message.to_string()).collect();

    // Must suggest restructuring
    assert!(
        helps_text
            .iter()
            .any(|h| h.contains("restructure") || h.contains("avoid nested")),
        "Missing restructure suggestion"
    );
}

#[test]
fn test_all_errors_have_minimum_suggestions() {
    let diag1 = e0203_result_type_mismatch(
        span(1, 1),
        &Text::from("E1"),
        &Text::from("E2"),
        span(1, 1),
        None,
    );

    let diag2 = e0204_multiple_conversion_paths(
        span(1, 1),
        &Text::from("E1"),
        &Text::from("E2"),
        &List::from(vec![Text::from("path1"), Text::from("path2")]),
    );

    let diag3 = e0205_try_in_non_result_context(
        span(1, 1),
        &Text::from("Result<T, E>"),
        &Text::from("T"),
        None,
        None,
    );

    // All diagnostics must have at least 2 help messages
    assert!(diag1.helps().len() >= 2, "E0203 has too few suggestions");
    assert!(diag2.helps().len() >= 2, "E0204 has too few suggestions");
    assert!(diag3.helps().len() >= 2, "E0205 has too few suggestions");
}

#[test]
fn test_all_errors_are_actionable() {
    // Verify all suggestions include code examples

    let diag1 = e0203_result_type_mismatch(
        span(1, 1),
        &Text::from("IoError"),
        &Text::from("AppError"),
        span(1, 1),
        None,
    );

    // E0203 suggestions should include actual code
    for help in diag1.helps() {
        let msg = help.message.to_string();
        // Should have either code keywords or structural markers
        assert!(
            msg.contains("implement")
                || msg.contains("map_err")
                || msg.contains("match")
                || msg.contains("{"),
            "E0203 suggestion lacks concrete code example: {}",
            msg
        );
    }
}

#[test]
fn test_error_messages_use_correct_terminology() {
    let diag = e0203_result_type_mismatch(
        span(1, 1),
        &Text::from("IoError"),
        &Text::from("AppError"),
        span(1, 1),
        None,
    );

    // Verify correct terminology
    let all_text = format!(
        "{} {} {}",
        diag.message(),
        diag.notes()
            .iter()
            .map(|n| n.message.to_string())
            .collect::<Vec<_>>()
            .join(" "),
        diag.helps()
            .iter()
            .map(|h| h.message.to_string())
            .collect::<Vec<_>>()
            .join(" ")
    );

    // Must use "From" protocol name (not "Into" or other variations)
    assert!(all_text.contains("From"));

    // Must refer to "?" operator consistently
    assert!(all_text.contains("'?'") || all_text.contains("? operator"));
}

#[test]
fn test_diagnostics_preserve_type_information() {
    // Verify diagnostics show exact type names, not generic placeholders

    let diag = e0203_result_type_mismatch(
        span(1, 1),
        &Text::from("std::io::Error"),
        &Text::from("MyApp::AppError"),
        span(1, 1),
        None,
    );

    let message = diag.message();

    // Must show full type paths
    assert!(message.contains("std::io::Error"));
    assert!(message.contains("MyApp::AppError"));
}
