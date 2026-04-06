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
//! Tests for span tracking and source location.
//!
//! This module ensures that spans correctly track source locations,
//! can be merged, and provide accurate position information.

use proptest::prelude::*;
use verum_ast::span::*;
use verum_common::{List, Maybe};

#[test]
fn test_span_creation() {
    let file_id = FileId::new(0);
    let span = Span::new(10, 20, file_id);

    assert_eq!(span.start, 10);
    assert_eq!(span.end, 20);
    assert_eq!(span.file_id, file_id);
    assert_eq!(span.len(), 10);
}

#[test]
fn test_dummy_span() {
    let span = Span::dummy();
    assert_eq!(span.start, 0);
    assert_eq!(span.end, 0);
    assert_eq!(span.file_id, FileId::dummy());
    assert!(span.is_empty());
}

#[test]
fn test_span_length() {
    let file_id = FileId::new(0);

    // Normal span
    let span1 = Span::new(10, 20, file_id);
    assert_eq!(span1.len(), 10);

    // Empty span
    let span2 = Span::new(10, 10, file_id);
    assert_eq!(span2.len(), 0);
    assert!(span2.is_empty());

    // Single character span
    let span3 = Span::new(10, 11, file_id);
    assert_eq!(span3.len(), 1);
    assert!(!span3.is_empty());
}

#[test]
fn test_span_is_empty() {
    let file_id = FileId::new(0);

    // Empty spans
    assert!(Span::new(10, 10, file_id).is_empty());
    assert!(Span::new(0, 0, file_id).is_empty());

    // Invalid span (start > end) should also be considered empty
    assert!(Span::new(20, 10, file_id).is_empty());

    // Non-empty spans
    assert!(!Span::new(10, 20, file_id).is_empty());
    assert!(!Span::new(0, 1, file_id).is_empty());
}

#[test]
fn test_span_merge() {
    let file_id = FileId::new(0);

    // Adjacent spans
    let span1 = Span::new(10, 20, file_id);
    let span2 = Span::new(20, 30, file_id);
    let merged = span1.merge(span2);
    assert_eq!(merged.start, 10);
    assert_eq!(merged.end, 30);

    // Overlapping spans
    let span3 = Span::new(10, 25, file_id);
    let span4 = Span::new(15, 30, file_id);
    let merged2 = span3.merge(span4);
    assert_eq!(merged2.start, 10);
    assert_eq!(merged2.end, 30);

    // Disjoint spans
    let span5 = Span::new(10, 20, file_id);
    let span6 = Span::new(30, 40, file_id);
    let merged3 = span5.merge(span6);
    assert_eq!(merged3.start, 10);
    assert_eq!(merged3.end, 40);

    // Nested spans
    let span7 = Span::new(10, 40, file_id);
    let span8 = Span::new(20, 30, file_id);
    let merged4 = span7.merge(span8);
    assert_eq!(merged4.start, 10);
    assert_eq!(merged4.end, 40);
}

#[test]
#[should_panic(expected = "Cannot merge spans from different files")]
fn test_span_merge_different_files() {
    let span1 = Span::new(10, 20, FileId::new(0));
    let span2 = Span::new(20, 30, FileId::new(1));
    let _ = span1.merge(span2);
}

#[test]
fn test_span_contains() {
    let file_id = FileId::new(0);

    let outer = Span::new(10, 30, file_id);
    let inner = Span::new(15, 25, file_id);
    let partial = Span::new(20, 35, file_id);
    let disjoint = Span::new(40, 50, file_id);
    let different_file = Span::new(15, 25, FileId::new(1));

    // Outer contains inner
    assert!(outer.contains(inner));

    // Span contains itself
    assert!(outer.contains(outer));

    // Outer doesn't contain partial overlap
    assert!(!outer.contains(partial));

    // Outer doesn't contain disjoint
    assert!(!outer.contains(disjoint));

    // Different files
    assert!(!outer.contains(different_file));

    // Edge cases
    let edge1 = Span::new(10, 30, file_id);
    let edge2 = Span::new(10, 20, file_id);
    let edge3 = Span::new(20, 30, file_id);
    assert!(edge1.contains(edge2));
    assert!(edge1.contains(edge3));
}

#[test]
fn test_span_overlaps() {
    let file_id = FileId::new(0);

    let span1 = Span::new(10, 30, file_id);
    let span2 = Span::new(20, 40, file_id);
    let span3 = Span::new(5, 15, file_id);
    let span4 = Span::new(30, 40, file_id); // Adjacent, not overlapping
    let span5 = Span::new(40, 50, file_id); // Disjoint
    let span6 = Span::new(15, 25, file_id); // Contained

    // Overlapping spans
    assert!(span1.overlaps(span2));
    assert!(span2.overlaps(span1)); // Symmetric
    assert!(span1.overlaps(span3));
    assert!(span3.overlaps(span1));

    // Adjacent spans don't overlap
    assert!(!span1.overlaps(span4));
    assert!(!span4.overlaps(span1));

    // Disjoint spans don't overlap
    assert!(!span1.overlaps(span5));
    assert!(!span5.overlaps(span1));

    // Contained spans do overlap
    assert!(span1.overlaps(span6));
    assert!(span6.overlaps(span1));

    // Span overlaps with itself
    assert!(span1.overlaps(span1));
}

#[test]
fn test_file_id() {
    let id1 = FileId::new(0);
    let id2 = FileId::new(1);
    let dummy = FileId::dummy();

    // We can't access the internal field directly anymore, just test equality
    // Test equality
    assert_eq!(id1, FileId::new(0));
    assert_ne!(id1, id2);
    assert_ne!(id1, dummy);
    assert_eq!(dummy, FileId::dummy());
}

#[test]
fn test_source_file_creation() {
    let file_id = FileId::new(0);
    let name = "file.vr".to_string();
    let content = "let x = 42;\nlet y = x + 1;".to_string();

    let source = SourceFile::new(file_id, name.clone(), content.clone());

    assert_eq!(source.id, file_id);
    assert_eq!(source.name.as_str(), name);
    assert_eq!(source.source.as_str(), content);
}

#[test]
fn test_source_file_line_positions() {
    let file_id = FileId::new(0);
    let name = "file.vr".to_string();

    // Test with multiple lines
    let content = "line 1\nline 2\r\nline 3\rline 4".to_string();
    let source = SourceFile::new(file_id, name, content);

    let lines = &source.line_starts;
    assert_eq!(lines.len(), 4);
    assert_eq!(lines.first().copied(), Maybe::Some(0)); // Start of "line 1"
    assert_eq!(lines.get(1).copied(), Maybe::Some(7)); // Start of "line 2" (after \n)
    assert_eq!(lines.get(2).copied(), Maybe::Some(15)); // Start of "line 3" (after \r\n)
    assert_eq!(lines.get(3).copied(), Maybe::Some(22)); // Start of "line 4" (after \r)
}

#[test]
fn test_source_file_empty() {
    let file_id = FileId::new(0);
    let name = "empty.vr".to_string();
    let content = String::new();

    let source = SourceFile::new(file_id, name, content);
    let lines = &source.line_starts;
    assert_eq!(lines.len(), 1); // Even empty files have one line
    assert_eq!(lines.first().copied(), Maybe::Some(0));
}

#[test]
fn test_source_file_single_line() {
    let file_id = FileId::new(0);
    let name = "single.vr".to_string();
    let content = "single line no newline".to_string();

    let source = SourceFile::new(file_id, name, content);
    let lines = &source.line_starts;
    assert_eq!(lines.len(), 1);
    assert_eq!(lines.first().copied(), Maybe::Some(0));
}

#[test]
fn test_source_line_col() {
    let file_id = FileId::new(0);
    let name = "file.vr".to_string();
    let content = "first line\nsecond line\nthird line".to_string();
    let source = SourceFile::new(file_id, name, content);

    // Test location at start of file
    let (line1, col1) = source.line_col(0);
    assert_eq!(line1, 0); // 0-indexed
    assert_eq!(col1, 0);

    // Test location in middle of first line
    let (line2, col2) = source.line_col(5);
    assert_eq!(line2, 0); // 0-indexed
    assert_eq!(col2, 5);

    // Test location at start of second line
    let (line3, col3) = source.line_col(11);
    assert_eq!(line3, 1); // 0-indexed
    assert_eq!(col3, 0);

    // Test location in third line
    let (line4, col4) = source.line_col(24);
    assert_eq!(line4, 2); // 0-indexed
    assert_eq!(col4, 1);
}

#[test]
fn test_spanned_trait() {
    use verum_ast::*;

    let span = Span::new(10, 20, FileId::new(0));

    // Test Module implements Spanned
    let module = Module::new(List::new(), FileId::new(0), span);
    assert_eq!(module.span(), span);

    // Test Expr implements Spanned
    let expr = Expr::literal(Literal::int(42, span));
    assert_eq!(expr.span(), span);

    // Test Type implements Spanned
    let ty = Type::int(span);
    assert_eq!(ty.span(), span);

    // Test Pattern implements Spanned
    let pattern = Pattern::wildcard(span);
    assert_eq!(pattern.span(), span);

    // Test Stmt implements Spanned
    use verum_ast::stmt::{Stmt, StmtKind};
    let stmt = Stmt::new(
        StmtKind::Expr {
            expr,
            has_semi: true,
        },
        span,
    );
    assert_eq!(stmt.span(), span);
}

// Property-based tests using proptest
proptest! {
    #[test]
    fn test_span_len_is_non_negative(start in 0u32..1000, end in 0u32..1000) {
        // Guard: ensure start <= end to test only valid spans
        prop_assume!(start <= end);

        let file_id = FileId::new(0);
        let span = Span::new(start, end, file_id);
        assert_eq!(span.len(), end - start);
    }

    #[test]
    fn test_span_merge_is_commutative(
        start1 in 0u32..100,
        end1 in 0u32..100,
        start2 in 0u32..100,
        end2 in 0u32..100,
    ) {
        let file_id = FileId::new(0);
        let span1 = Span::new(start1, end1, file_id);
        let span2 = Span::new(start2, end2, file_id);

        let merged1 = span1.merge(span2);
        let merged2 = span2.merge(span1);

        assert_eq!(merged1.start, merged2.start);
        assert_eq!(merged1.end, merged2.end);
    }

    #[test]
    fn test_span_merge_covers_both(
        start1 in 0u32..100,
        end1 in 0u32..100,
        start2 in 0u32..100,
        end2 in 0u32..100,
    ) {
        let file_id = FileId::new(0);
        let span1 = Span::new(start1, end1, file_id);
        let span2 = Span::new(start2, end2, file_id);

        let merged = span1.merge(span2);

        assert!(merged.start <= start1.min(start2));
        assert!(merged.end >= end1.max(end2));
    }

    #[test]
    fn test_span_contains_is_transitive(
        start1 in 0u32..100,
        end1 in 100u32..200,
        start2 in 20u32..80,
        end2 in 80u32..100,
        start3 in 40u32..60,
        end3 in 60u32..80,
    ) {
        let file_id = FileId::new(0);
        let span1 = Span::new(start1, end1, file_id);
        let span2 = Span::new(start2, end2, file_id);
        let span3 = Span::new(start3, end3, file_id);

        // If span1 contains span2 and span2 contains span3,
        // then span1 should contain span3
        if span1.contains(span2) && span2.contains(span3) {
            assert!(span1.contains(span3));
        }
    }

    #[test]
    fn test_overlaps_is_symmetric(
        start1 in 0u32..100,
        end1 in 0u32..100,
        start2 in 0u32..100,
        end2 in 0u32..100,
    ) {
        let file_id = FileId::new(0);
        let span1 = Span::new(start1, end1, file_id);
        let span2 = Span::new(start2, end2, file_id);

        assert_eq!(span1.overlaps(span2), span2.overlaps(span1));
    }
}

#[test]
fn test_complex_span_scenarios() {
    let file_id = FileId::new(0);

    // Test zero-width spans
    let zero_span = Span::new(10, 10, file_id);
    assert!(zero_span.is_empty());
    assert_eq!(zero_span.len(), 0);

    // Zero-width span doesn't contain anything except itself
    let other_zero = Span::new(10, 10, file_id);
    assert!(zero_span.contains(other_zero));

    let non_zero = Span::new(10, 11, file_id);
    assert!(!zero_span.contains(non_zero));

    // But a non-zero span can contain a zero-width span at its boundaries
    let container = Span::new(5, 15, file_id);
    assert!(container.contains(zero_span));

    // Test spans at file boundaries
    let start_span = Span::new(0, 10, file_id);
    assert_eq!(start_span.start, 0);

    let max_span = Span::new(u32::MAX - 10, u32::MAX, file_id);
    assert_eq!(max_span.end, u32::MAX);
    assert_eq!(max_span.len(), 10);
}

#[test]
fn test_source_file_with_unicode() {
    let file_id = FileId::new(0);
    let name = "unicode.vr".to_string();

    // Test with unicode content
    let content = "let 你好 = 42;\nlet мир = \"world\";\nlet 🦀 = true;".to_string();
    let source = SourceFile::new(file_id, name, content.clone());

    // Note: byte positions, not character positions
    let lines = &source.line_starts;
    assert_eq!(lines.len(), 3);

    // Verify we can still get line/col correctly
    let (line1, col1) = source.line_col(0);
    assert_eq!(line1, 0); // 0-indexed
    assert_eq!(col1, 0);
}

#[test]
fn test_source_file_windows_line_endings() {
    let file_id = FileId::new(0);
    let name = "windows.vr".to_string();

    // Test with Windows line endings
    let content = "line 1\r\nline 2\r\nline 3".to_string();
    let source = SourceFile::new(file_id, name, content);

    let lines = &source.line_starts;
    assert_eq!(lines.len(), 3);
    assert_eq!(lines.first().copied(), Maybe::Some(0)); // Start of "line 1"
    assert_eq!(lines.get(1).copied(), Maybe::Some(8)); // Start of "line 2" (after \r\n)
    assert_eq!(lines.get(2).copied(), Maybe::Some(16)); // Start of "line 3" (after \r\n)
}

#[test]
fn test_source_file_mixed_line_endings() {
    let file_id = FileId::new(0);
    let name = "mixed.vr".to_string();

    // Test with mixed line endings (not recommended but should handle)
    // SourceFile now recognizes \n, \r, and \r\n as line endings
    let content = "unix\nmac\rwindows\r\nlast".to_string();
    let source = SourceFile::new(file_id, name, content);

    let lines = &source.line_starts;
    // All line endings (\n, \r, \r\n) create new lines
    assert_eq!(lines.len(), 4);
    assert_eq!(lines.first().copied(), Maybe::Some(0)); // Start of "unix"
    assert_eq!(lines.get(1).copied(), Maybe::Some(5)); // Start of "mac" (after \n)
    assert_eq!(lines.get(2).copied(), Maybe::Some(9)); // Start of "windows" (after \r)
    assert_eq!(lines.get(3).copied(), Maybe::Some(18)); // Start of "last" (after \r\n)
}
