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
// Migrated from src/json.rs per CLAUDE.md standards

use verum_compiler::literal_parsers::json::parse_json;

use verum_ast::{FileId, Span};
use verum_common::Text;

fn test_span() -> Span {
    Span::new(0, 10, FileId::new(0))
}

#[test]
fn test_parse_json_object() {
    let result = parse_json(&Text::from(r#"{"key": "value"}"#), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_json_array() {
    let result = parse_json(&Text::from("[1, 2, 3]"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_json_string() {
    let result = parse_json(&Text::from(r#""hello""#), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_json_number() {
    let result = parse_json(&Text::from("42"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_json_boolean() {
    let result = parse_json(&Text::from("true"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_json_null() {
    let result = parse_json(&Text::from("null"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_complex_json() {
    let json = r#"
        {
            "name": "John",
            "age": 30,
            "active": true,
            "address": {
                "street": "123 Main St",
                "city": "Anytown"
            },
            "hobbies": ["reading", "gaming"]
        }
        "#;
    let result = parse_json(&Text::from(json), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_empty_json() {
    let result = parse_json(&Text::from(""), test_span(), None);
    assert!(result.is_err());
}

#[test]
fn test_parse_invalid_json() {
    let result = parse_json(&Text::from("{invalid}"), test_span(), None);
    assert!(result.is_err());
}

#[test]
fn test_parse_unclosed_object() {
    let result = parse_json(&Text::from(r#"{"key": "value""#), test_span(), None);
    assert!(result.is_err());
}
