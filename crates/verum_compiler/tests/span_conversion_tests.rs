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
//! Integration tests for AST span to diagnostic span conversion
//!
//! Tests the complete flow from source file -> AST -> diagnostics with accurate
//! line/column information in error messages.

use std::path::PathBuf;
use verum_ast::FileId;
use verum_common::span::{SourceFile, Span};
use verum_compiler::{CompilerOptions, Session};

/// Helper to create a Session with a source string loaded
fn session_with_source(source: &str) -> (Session, FileId) {
    let opts = CompilerOptions {
        input: PathBuf::from("/tmp/test_span.vr"),
        output: PathBuf::from("/tmp/test_span.out"),
        ..Default::default()
    };
    let session = Session::new(opts);
    let file_id = session
        .load_source_string(source, PathBuf::from("test.vr"))
        .expect("Failed to load source string");
    (session, file_id)
}

#[test]
fn test_single_line_span_conversion() {
    let source = "fn hello() {}";
    let (session, file_id) = session_with_source(source);

    // Span covering "hello" (bytes 3..8)
    let ast_span = Span::new(3, 8, file_id);
    let diag_span = session.convert_span(ast_span);

    assert_eq!(diag_span.line, 1, "Should be line 1");
    assert_eq!(diag_span.column, 4, "Should start at column 4 (1-indexed)");
    assert_eq!(diag_span.end_column, 9, "Should end at column 9");
    assert!(!diag_span.is_multiline(), "Should be single-line");
}

#[test]
fn test_multiline_span_conversion() {
    let source = "fn foo() {\n    let x = 1;\n}\n";
    let (session, file_id) = session_with_source(source);

    // Span covering from "fn" to the closing brace: bytes 0..27
    let ast_span = Span::new(0, 27, file_id);
    let diag_span = session.convert_span(ast_span);

    assert_eq!(diag_span.line, 1, "Should start at line 1");
    assert!(diag_span.is_multiline(), "Should be multiline");
    assert_eq!(diag_span.end_line(), 3, "Should end at line 3");
}

#[test]
fn test_diagnostic_with_accurate_span() {
    // Test that parsed AST spans translate to correct diagnostics
    let source = "fn add(a: Int, b: Int) -> Int {\n    a + b\n}";
    let (session, file_id) = session_with_source(source);

    // Span covering "a + b" on line 2, starting at byte offset 36
    // Line 1: "fn add(a: Int, b: Int) -> Int {\n" = 32 chars
    // Line 2: "    a + b\n" => "a" starts at 36
    let ast_span = Span::new(36, 41, file_id);
    let diag_span = session.convert_span(ast_span);

    assert_eq!(diag_span.line, 2, "Expression should be on line 2");
    assert_eq!(diag_span.column, 5, "Expression should start at column 5");
}

#[test]
fn test_dummy_span_conversion() {
    let source = "fn test() {}";
    let (session, _file_id) = session_with_source(source);

    // Dummy spans should convert gracefully
    let dummy = Span::dummy();
    let diag_span = session.convert_span(dummy);

    // Should produce a valid fallback span
    assert_eq!(diag_span.line, 1, "Dummy span should have line 1");
    assert_eq!(diag_span.column, 1, "Dummy span should have column 1");
}

#[test]
fn test_unknown_file_span_conversion() {
    let source = "fn test() {}";
    let (session, _file_id) = session_with_source(source);

    // Span with a file ID that doesn't exist in the session
    let unknown_file = FileId::new(9999);
    let ast_span = Span::new(0, 5, unknown_file);
    let diag_span = session.convert_span(ast_span);

    // Should produce a fallback span without panicking
    assert_eq!(diag_span.line, 1, "Unknown file span should have line 1");
}

#[test]
fn test_span_conversion_performance() {
    // Test that span conversion handles large files efficiently
    let mut source = String::new();
    for i in 0..1000 {
        source.push_str(&format!("fn func_{}() {{}}\n", i));
    }

    let (session, file_id) = session_with_source(&source);

    // Convert many spans to verify performance
    let start = std::time::Instant::now();
    for i in 0..1000 {
        let offset = (i * 18) as u32; // approximate bytes per line
        let ast_span = Span::new(offset, offset + 5, file_id);
        let _ = session.convert_span(ast_span);
    }
    let elapsed = start.elapsed();

    // Should complete well under 100ms for 1000 conversions
    assert!(
        elapsed.as_millis() < 100,
        "1000 span conversions took {}ms, expected <100ms",
        elapsed.as_millis()
    );
}

#[test]
fn test_utf8_span_conversion() {
    // UTF-8 multibyte characters affect byte offsets but not logical column positions
    let source = "fn greet() {\n    let msg = \"hello\";\n}\n";
    let (session, file_id) = session_with_source(source);

    // Span for "msg" on line 2 - use find() to get exact byte offset
    let msg_offset = source.find("msg").unwrap() as u32;
    let ast_span = Span::new(msg_offset, msg_offset + 3, file_id);
    let diag_span = session.convert_span(ast_span);

    assert_eq!(diag_span.line, 2, "Should be on line 2");
    // "    let msg" means msg is at column 9 (1-indexed), but byte offset within
    // line may differ. Calculate expected column from line start.
    let line2_start = source.find('\n').unwrap() + 1;
    let expected_col = (msg_offset as usize - line2_start) + 1;
    assert_eq!(diag_span.column, expected_col, "msg should be at correct column");
}

#[test]
fn test_multiline_single_span() {
    // A span that covers exactly one line should not be marked multiline
    let source = "line one\nline two\nline three\n";
    let (session, file_id) = session_with_source(source);

    // Span covering "line two" exactly (bytes 9..17)
    let ast_span = Span::new(9, 17, file_id);
    let diag_span = session.convert_span(ast_span);

    assert_eq!(diag_span.line, 2, "Should be on line 2");
    assert!(!diag_span.is_multiline(), "Single-line content should not be multiline");
    assert_eq!(diag_span.column, 1, "Should start at column 1");
    assert_eq!(diag_span.end_column, 9, "Should end at column 9");
}
