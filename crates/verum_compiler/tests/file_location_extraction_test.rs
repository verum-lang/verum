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
//! Test for FileLocation extraction from function spans
//!
//! This test verifies that the verification profiler correctly extracts
//! file location information (path, line, column) from function spans.

use std::path::PathBuf;
use verum_compiler::options::CompilerOptions;
use verum_compiler::session::Session;
use verum_common::span::{FileId, SourceFile, Span};

#[test]
fn test_file_location_extraction_from_span() {
    // Create a test source file
    let source = r#"
fn test_function(x: Int) -> Int
    requires x > 0
    ensures result > 0
{
    x + 1
}
"#;

    // Create a temporary test file
    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir.join("test_file_location.vr");
    std::fs::write(&test_file, source).expect("Failed to write test file");

    // Create session and load the file
    let options = CompilerOptions::new(test_file.clone(), PathBuf::from("/dev/null"));
    let session = Session::new(options);

    // Load the file
    let file_id = session
        .load_file(&test_file)
        .expect("Failed to load test file");

    // Get the source file
    let source_file = session.get_source(file_id).expect("Source file not found");

    // Create a span pointing to the function declaration (line 2, roughly column 0)
    // The newline at the start means line 2 is at byte offset ~1 (after the first \n)
    let span = Span::new(1, 10, file_id);

    // Convert span to line/column using SourceFile
    let line_col_span = source_file
        .span_to_line_col(span)
        .expect("Failed to convert span");

    // Verify the location is correct
    // Line 2 in source (1-indexed) should be where "fn test_function" is
    assert_eq!(
        line_col_span.line, 2,
        "Expected line 2 for function declaration"
    );
    assert!(line_col_span.column > 0, "Expected non-zero column");

    // Verify we can extract the file path
    assert!(source_file.path.is_some(), "Source file should have a path");
    if let Some(ref path) = source_file.path {
        assert_eq!(path, &test_file, "File path should match");
    }

    // Clean up
    let _ = std::fs::remove_file(&test_file);
}

#[test]
fn test_file_location_with_multiple_lines() {
    // Create a source file with multiple functions
    let source = r#"fn first_function() -> Int { 1 }

fn second_function(x: Int) -> Int
    requires x > 0
{
    x * 2
}

fn third_function() { }
"#;

    // Create SourceFile directly
    let file_id = FileId::new(0);
    let source_file = SourceFile::new(file_id, "test.vr".to_string(), source.to_string());

    // Test span for first function (line 1, column 0)
    let span1 = Span::new(0, 2, file_id);
    let loc1 = source_file.span_to_line_col(span1).unwrap();
    assert_eq!(loc1.line, 1);
    assert_eq!(loc1.file.as_str(), "test.vr");

    // Test span for second function (line 3, somewhere in the middle)
    // Line 3 starts after "fn first_function() -> Int { 1 }\n\n"
    let line3_offset = "fn first_function() -> Int { 1 }\n\n".len() as u32;
    let span2 = Span::new(line3_offset, line3_offset + 10, file_id);
    let loc2 = source_file.span_to_line_col(span2).unwrap();
    assert_eq!(loc2.line, 3);

    // Test span for third function (line 8)
    let line8_offset = source.lines().take(7).map(|l| l.len() + 1).sum::<usize>() as u32;
    let span3 = Span::new(line8_offset, line8_offset + 10, file_id);
    let loc3 = source_file.span_to_line_col(span3).unwrap();
    assert_eq!(loc3.line, 8);
}

#[test]
fn test_unknown_file_location() {
    use verum_compiler::verification_profiler::FileLocation;

    // Test FileLocation::unknown()
    let unknown = FileLocation::unknown();

    assert_eq!(unknown.line, 0);
    assert_eq!(unknown.column, 0);
    assert_eq!(unknown.file.display().to_string(), "<unknown>");
}
