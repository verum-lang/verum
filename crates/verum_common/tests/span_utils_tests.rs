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
// Tests for span_utils module
// Migrated from src/span_utils.rs per CLAUDE.md standards

use verum_common::span::{FileId, Span};
use verum_common::span_utils::*;

#[test]
fn test_offset_to_line_col_start() {
    let text = "line 1\nline 2\nline 3";
    let (line, col) = offset_to_line_col(0, text);
    assert_eq!(line, 0);
    assert_eq!(col, 0);
}

#[test]
fn test_offset_to_line_col_end_of_first_line() {
    let text = "line 1\nline 2\nline 3";
    let (line, col) = offset_to_line_col(6, text);
    assert_eq!(line, 0);
    assert_eq!(col, 6);
}

#[test]
fn test_offset_to_line_col_second_line() {
    let text = "line 1\nline 2\nline 3";
    let (line, col) = offset_to_line_col(7, text);
    assert_eq!(line, 1);
    assert_eq!(col, 0);
}

#[test]
fn test_offset_to_line_col_middle() {
    let text = "line 1\nline 2\nline 3";
    let (line, col) = offset_to_line_col(10, text);
    assert_eq!(line, 1);
    assert_eq!(col, 3);
}

#[test]
fn test_line_col_to_offset_start() {
    let text = "line 1\nline 2\nline 3";
    let offset = line_col_to_offset(0, 0, text).unwrap();
    assert_eq!(offset, 0);
}

#[test]
fn test_line_col_to_offset_second_line() {
    let text = "line 1\nline 2\nline 3";
    let offset = line_col_to_offset(1, 0, text).unwrap();
    assert_eq!(offset, 7);
}

#[test]
fn test_line_col_to_offset_middle() {
    let text = "line 1\nline 2\nline 3";
    let offset = line_col_to_offset(1, 3, text).unwrap();
    assert_eq!(offset, 10);
}

#[test]
fn test_line_col_to_offset_out_of_bounds() {
    let text = "line 1\nline 2";
    let offset = line_col_to_offset(10, 0, text);
    assert_eq!(offset, None);
}

#[test]
fn test_span_to_line_col_span_single_line() {
    let text = "line 1\nline 2";
    let span = Span::new(0, 6, FileId::new(0));
    let lc_span = span_to_line_col_span(span, text, "test.vr");

    assert_eq!(lc_span.file, "test.vr");
    assert_eq!(lc_span.line, 1);
    assert_eq!(lc_span.column, 1);
    assert_eq!(lc_span.end_column, 7);
    assert_eq!(lc_span.end_line, None);
    assert!(!lc_span.is_multiline());
}

#[test]
fn test_span_to_line_col_span_multiline() {
    let text = "line 1\nline 2";
    let span = Span::new(3, 10, FileId::new(0)); // "e 1\nlin"
    let lc_span = span_to_line_col_span(span, text, "test.vr");

    assert_eq!(lc_span.line, 1);
    assert_eq!(lc_span.column, 4);
    assert_eq!(lc_span.end_line, Some(2));
    assert_eq!(lc_span.end_column, 4);
    assert!(lc_span.is_multiline());
}

#[test]
fn test_round_trip_offset_line_col() {
    let text = "line 1\nline 2\nline 3";
    let original_offset = 10;

    let (line, col) = offset_to_line_col(original_offset, text);
    let offset = line_col_to_offset(line, col, text).unwrap();

    assert_eq!(offset, original_offset);
}
