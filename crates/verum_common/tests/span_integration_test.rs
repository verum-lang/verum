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
//! Integration test for unified span handling
//!
//! This test verifies that the span consolidation from verum_ast and
//! verum_diagnostics into verum_common works correctly.

use verum_common::span::{FileId, LineColSpan, SourceFile, Span};
use verum_common::span_utils::{offset_to_line_col, span_to_line_col_span};

#[test]
fn test_span_basic_operations() {
    let file_id = FileId::new(1);
    let span1 = Span::new(0, 10, file_id);
    let span2 = Span::new(5, 15, file_id);

    // Test basic properties
    assert_eq!(span1.len(), 10);
    assert!(!span1.is_empty());
    assert_eq!(span1.file_id, file_id);

    // Test merging
    let merged = span1.merge(span2);
    assert_eq!(merged.start, 0);
    assert_eq!(merged.end, 15);
    assert_eq!(merged.file_id, file_id);

    // Test containment
    let inner = Span::new(3, 7, file_id);
    assert!(span1.contains(inner));
    assert!(!inner.contains(span1));

    // Test overlap
    assert!(span1.overlaps(span2));
    let non_overlapping = Span::new(20, 30, file_id);
    assert!(!span1.overlaps(non_overlapping));
}

#[test]
fn test_line_col_span() {
    let span = LineColSpan::new("test.vr", 1, 5, 10);
    assert_eq!(span.file, "test.vr");
    assert_eq!(span.line, 1);
    assert_eq!(span.column, 5);
    assert_eq!(span.end_column, 10);
    assert!(!span.is_multiline());
    assert_eq!(span.length(), 5);

    let multiline = LineColSpan::new_multiline("test.vr", 1, 5, 3, 10);
    assert!(multiline.is_multiline());
    assert_eq!(multiline.end_line(), 3);
}

#[test]
fn test_source_file_line_lookup() {
    let file_id = FileId::new(0);
    let source = "line 1\nline 2\nline 3\nline 4";
    let file = SourceFile::new(file_id, "test.vr".to_string(), source.to_string());

    // Test line_col conversion
    let (line, col) = file.line_col(0);
    assert_eq!(line, 0);
    assert_eq!(col, 0);

    let (line, col) = file.line_col(7); // Start of line 2
    assert_eq!(line, 1);
    assert_eq!(col, 0);

    let (line, col) = file.line_col(10); // Middle of line 2
    assert_eq!(line, 1);
    assert_eq!(col, 3);
}

#[test]
fn test_span_to_line_col_conversion() {
    let file_id = FileId::new(0);
    let source = "line 1\nline 2\nline 3";
    let file = SourceFile::new(file_id, "test.vr".to_string(), source.to_string());

    // Single line span
    let span = Span::new(0, 6, file_id);
    let line_col_span = file.span_to_line_col(span).unwrap();
    assert_eq!(line_col_span.line, 1); // 1-indexed
    assert_eq!(line_col_span.column, 1);
    assert_eq!(line_col_span.end_column, 7);
    assert!(!line_col_span.is_multiline());

    // Multi-line span
    let span = Span::new(3, 10, file_id); // "e 1\nlin"
    let line_col_span = file.span_to_line_col(span).unwrap();
    assert_eq!(line_col_span.line, 1);
    assert_eq!(line_col_span.column, 4);
    assert_eq!(line_col_span.end_line(), 2);
    assert_eq!(line_col_span.end_column, 4);
    assert!(line_col_span.is_multiline());
}

#[test]
fn test_span_text_extraction() {
    let file_id = FileId::new(0);
    let source = "hello world\nfoo bar";
    let file = SourceFile::new(file_id, "test.vr".to_string(), source.to_string());

    let span = Span::new(0, 5, file_id); // "hello"
    assert_eq!(file.span_text(span), Some("hello"));

    let span = Span::new(6, 11, file_id); // "world"
    assert_eq!(file.span_text(span), Some("world"));

    let span = Span::new(12, 15, file_id); // "foo"
    assert_eq!(file.span_text(span), Some("foo"));
}

#[test]
fn test_span_line_extraction() {
    let file_id = FileId::new(0);
    let source = "line 1\nline 2\nline 3";
    let file = SourceFile::new(file_id, "test.vr".to_string(), source.to_string());

    let span = Span::new(3, 6, file_id); // Middle of first line
    assert_eq!(file.span_line(span), Some("line 1\n"));

    let span = Span::new(7, 10, file_id); // Middle of second line
    assert_eq!(file.span_line(span), Some("line 2\n"));

    let span = Span::new(14, 17, file_id); // Third line (no newline at end)
    assert_eq!(file.span_line(span), Some("line 3"));
}

#[test]
fn test_offset_conversions() {
    let source = "line 1\nline 2\nline 3";

    // Test offset_to_line_col
    let (line, col) = offset_to_line_col(0, source);
    assert_eq!((line, col), (0, 0));

    let (line, col) = offset_to_line_col(7, source);
    assert_eq!((line, col), (1, 0));

    let (line, col) = offset_to_line_col(10, source);
    assert_eq!((line, col), (1, 3));
}

#[test]
fn test_span_to_line_col_span_utility() {
    let source = "line 1\nline 2";
    let file_id = FileId::new(0);

    let span = Span::new(0, 6, file_id);
    let lc_span = span_to_line_col_span(span, source, "test.vr");

    assert_eq!(lc_span.file, "test.vr");
    assert_eq!(lc_span.line, 1);
    assert_eq!(lc_span.column, 1);
    assert_eq!(lc_span.end_column, 7);
}

#[test]
fn test_file_id_operations() {
    let id1 = FileId::new(0);
    let id2 = FileId::new(1);
    let dummy = FileId::dummy();

    assert_eq!(id1.raw(), 0);
    assert_eq!(id2.raw(), 1);
    assert!(dummy.is_dummy());
    assert!(!id1.is_dummy());

    // File IDs should be comparable
    assert_eq!(id1, id1);
    assert_ne!(id1, id2);
}

#[test]
fn test_span_display() {
    let file_id = FileId::new(5);
    let span = Span::new(10, 20, file_id);
    let display = format!("{}", span);
    assert!(display.contains("5")); // File ID
    assert!(display.contains("10")); // Start
    assert!(display.contains("20")); // End
}

#[test]
fn test_line_col_span_display() {
    let span = LineColSpan::new("test.vr", 10, 5, 15);
    let display = format!("{}", span);
    assert!(display.contains("test.vr"));
    assert!(display.contains("10"));
    assert!(display.contains("5"));

    let multiline = LineColSpan::new_multiline("test.vr", 1, 1, 3, 10);
    let display = format!("{}", multiline);
    assert!(display.contains("1:1"));
    assert!(display.contains("3:10"));
}
