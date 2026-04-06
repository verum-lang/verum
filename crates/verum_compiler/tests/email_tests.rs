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
// Tests for email literal parser
// Per CLAUDE.md standards - tests in tests/ directory

use verum_ast::{FileId, Span};
use verum_compiler::literal_parsers::email::parse_email;
use verum_common::Text;

fn test_span() -> Span {
    Span::new(0, 10, FileId::new(0))
}

#[test]
fn test_parse_simple_email() {
    let result = parse_email(&Text::from("user@example.com"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_email_with_subdomain() {
    let result = parse_email(&Text::from("user@mail.example.com"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_email_with_plus() {
    let result = parse_email(&Text::from("user+tag@example.com"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_email_with_dots() {
    let result = parse_email(&Text::from("john.doe@example.com"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_email_with_numbers() {
    let result = parse_email(&Text::from("user123@example.com"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_empty_email() {
    let result = parse_email(&Text::from(""), test_span(), None);
    assert!(result.is_err());
}

#[test]
fn test_parse_missing_at() {
    let result = parse_email(&Text::from("userexample.com"), test_span(), None);
    assert!(result.is_err());
}

#[test]
fn test_parse_missing_domain() {
    let result = parse_email(&Text::from("user@"), test_span(), None);
    assert!(result.is_err());
}

#[test]
fn test_parse_missing_tld() {
    let result = parse_email(&Text::from("user@example"), test_span(), None);
    assert!(result.is_err());
}

#[test]
fn test_parse_email_with_underscores() {
    let result = parse_email(&Text::from("user_name@example.com"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_email_with_hyphens() {
    let result = parse_email(&Text::from("user-name@example.com"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_email_multiple_at() {
    let result = parse_email(&Text::from("user@@example.com"), test_span(), None);
    assert!(result.is_err());
}

#[test]
fn test_parse_email_long_tld() {
    let result = parse_email(&Text::from("user@example.museum"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_email_numeric_domain() {
    let result = parse_email(&Text::from("user@123.456.com"), test_span(), None);
    assert!(result.is_ok());
}
