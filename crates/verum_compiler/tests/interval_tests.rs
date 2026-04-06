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
// Per CLAUDE.md standards - tests in tests/ directory

use verum_ast::{FileId, Span};
use verum_compiler::literal_parsers::interval::parse_interval;
use verum_compiler::literal_registry::ParsedLiteral;
use verum_common::Text;

fn test_span() -> Span {
    Span::new(0, 10, FileId::new(0))
}

#[test]
fn test_parse_closed_interval() {
    let result = parse_interval(&Text::from("[0, 100]"), test_span(), None);
    assert!(result.is_ok());
    if let Ok(ParsedLiteral::Interval {
        start,
        end,
        inclusive_start,
        inclusive_end,
    }) = result
    {
        assert_eq!(start, 0.0);
        assert_eq!(end, 100.0);
        assert!(inclusive_start);
        assert!(inclusive_end);
    }
}

#[test]
fn test_parse_open_interval() {
    let result = parse_interval(&Text::from("(0, 100)"), test_span(), None);
    assert!(result.is_ok());
    if let Ok(ParsedLiteral::Interval {
        start,
        end,
        inclusive_start,
        inclusive_end,
    }) = result
    {
        assert_eq!(start, 0.0);
        assert_eq!(end, 100.0);
        assert!(!inclusive_start);
        assert!(!inclusive_end);
    }
}

#[test]
fn test_parse_half_open_left() {
    let result = parse_interval(&Text::from("[0, 100)"), test_span(), None);
    assert!(result.is_ok());
    if let Ok(ParsedLiteral::Interval {
        inclusive_start,
        inclusive_end,
        ..
    }) = result
    {
        assert!(inclusive_start);
        assert!(!inclusive_end);
    }
}

#[test]
fn test_parse_half_open_right() {
    let result = parse_interval(&Text::from("(0, 100]"), test_span(), None);
    assert!(result.is_ok());
    if let Ok(ParsedLiteral::Interval {
        inclusive_start,
        inclusive_end,
        ..
    }) = result
    {
        assert!(!inclusive_start);
        assert!(inclusive_end);
    }
}

#[test]
fn test_parse_negative_values() {
    let result = parse_interval(&Text::from("[-10, 10]"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_floating_point() {
    let result = parse_interval(&Text::from("[0.5, 99.9]"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_invalid_start_greater_than_end() {
    let result = parse_interval(&Text::from("[100, 0]"), test_span(), None);
    assert!(result.is_err());
}

#[test]
fn test_parse_invalid_format() {
    let result = parse_interval(&Text::from("[0 100]"), test_span(), None);
    assert!(result.is_err());
}

#[test]
fn test_parse_single_point_interval() {
    let result = parse_interval(&Text::from("[42, 42]"), test_span(), None);
    assert!(result.is_ok());
    if let Ok(ParsedLiteral::Interval { start, end, .. }) = result {
        assert_eq!(start, 42.0);
        assert_eq!(end, 42.0);
    }
}

#[test]
fn test_parse_large_values() {
    let result = parse_interval(&Text::from("[0, 1000000]"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_scientific_notation() {
    let result = parse_interval(&Text::from("[1e-5, 1e5]"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_invalid_bracket_mismatch() {
    let result = parse_interval(&Text::from("[0, 100}"), test_span(), None);
    assert!(result.is_err());
}
