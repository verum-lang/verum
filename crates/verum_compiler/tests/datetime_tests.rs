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
// Tests for datetime literal parser
// Migrated from src/datetime.rs per CLAUDE.md standards

use verum_ast::{FileId, Span};
use verum_compiler::literal_parsers::datetime::parse_datetime;
use verum_compiler::literal_registry::ParsedLiteral;
use verum_common::Text;

fn test_span() -> Span {
    Span::new(0, 10, FileId::new(0))
}

#[test]
fn test_parse_rfc3339() {
    let result = parse_datetime(&Text::from("2024-01-15T10:30:00Z"), test_span(), None);
    assert!(result.is_ok());
    if let Ok(ParsedLiteral::DateTime(ts)) = result {
        assert!(ts > 0);
    }
}

#[test]
fn test_parse_date_only() {
    let result = parse_datetime(&Text::from("2024-01-15"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_with_timezone() {
    let result = parse_datetime(&Text::from("2024-01-15T10:30:00+05:00"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_invalid() {
    let result = parse_datetime(&Text::from("invalid-date"), test_span(), None);
    assert!(result.is_err());
}

#[test]
fn test_parse_invalid_month() {
    let result = parse_datetime(&Text::from("2024-13-15T10:30:00Z"), test_span(), None);
    assert!(result.is_err());
}
