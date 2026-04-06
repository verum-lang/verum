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
// Unit tests for error.rs
//
// Migrated from src/error.rs to comply with test organization guidelines.

use verum_ast::span::{FileId, Span};
use verum_common::List;
use verum_lexer::TokenKind;
use verum_fast_parser::error::*;

#[test]
fn test_error_display() {
    let error = ParseError::new(
        ParseErrorKind::UnexpectedToken {
            expected: List::new(),
            found: TokenKind::Eof,
        },
        Span::new(10, 15, FileId::new(0)),
    );
    let display = format!("{}", error);
    assert!(display.contains("unexpected"));
}

#[test]
fn test_error_with_help() {
    let error = ParseError::new(
        ParseErrorKind::MissingSemicolon,
        Span::new(10, 15, FileId::new(0)),
    )
    .with_help("add a semicolon at the end of the statement");

    assert!(error.help.is_some());
    let display = format!("{}", error);
    assert!(display.contains("help:"));
}

// =============================================================================
// Error Message Formatting Tests
// =============================================================================
//
// These tests verify that error messages display human-readable file:line:column
// locations instead of internal representations like FileId(0):1367-1386.

#[test]
fn test_error_display_uses_line_column_format() {
    // Register a source file in the global registry
    let file_id = FileId::new(100);
    let source = "line 1\nline 2\nline 3\nlet x = 42;";
    verum_common::register_source_file(file_id, "test_file.vr", source);

    // Create a span pointing to "let" on line 4, column 1 (byte offset 21)
    let span = Span::new(21, 24, file_id);
    let error = ParseError::new(ParseErrorKind::MissingSemicolon, span);

    let display = format!("{}", error);

    // Should show file:line:column format
    assert!(
        display.contains("test_file.vr:4:1"),
        "Error message should contain 'test_file.vr:4:1', got: {}",
        display
    );

    // Should NOT contain raw FileId format
    assert!(
        !display.contains("FileId("),
        "Error message should NOT contain 'FileId(', got: {}",
        display
    );
}

#[test]
fn test_verum_error_conversion_uses_line_column() {
    // Register a source file
    let file_id = FileId::new(101);
    let source = "fn main() {\n    let x = 5\n}"; // Missing semicolon on line 2
    verum_common::register_source_file(file_id, "main.vr", source);

    // Create error at "let" position (byte 16 = line 2, col 5)
    let span = Span::new(16, 19, file_id);
    let parse_error = ParseError::new(
        ParseErrorKind::UnexpectedToken {
            expected: List::new(),
            found: TokenKind::Let,
        },
        span,
    );

    // Convert to VerumError
    let verum_error: VerumError = parse_error.into();
    let display = format!("{}", verum_error);

    // Should contain line:column information
    assert!(
        display.contains("main.vr") || display.contains(":2:"),
        "VerumError should contain file path or line number, got: {}",
        display
    );
}

#[test]
fn test_error_with_unregistered_file_shows_fallback() {
    // Use an unregistered FileId
    let file_id = FileId::new(999);
    let span = Span::new(100, 110, file_id);
    let error = ParseError::new(ParseErrorKind::MissingSemicolon, span);

    let display = format!("{}", error);

    // Should show fallback format like "<file:999>" not crash
    assert!(
        display.contains("<file:999>") || display.contains("999"),
        "Unregistered file should show fallback, got: {}",
        display
    );
}

#[test]
fn test_multiline_error_span() {
    let file_id = FileId::new(102);
    let source = "fn foo() {\n    invalid\n    syntax\n}";
    verum_common::register_source_file(file_id, "multiline.vr", source);

    // Span covering "invalid\n    syntax" (bytes 15-35)
    let span = Span::new(15, 35, file_id);
    let error = ParseError::new(
        ParseErrorKind::InvalidSyntax {
            message: "test error".into(),
        },
        span,
    );

    let display = format!("{}", error);

    // Should show starting position
    assert!(
        display.contains("multiline.vr:2:"),
        "Multiline span should show starting line, got: {}",
        display
    );
}
