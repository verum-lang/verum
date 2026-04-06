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
// Tests for compile-time literal parsing: tagged text literals (d#"...", rx#"..."),
// composite literals (mat#"...", interval#"..."), semantic literals (json#"...", xml#"..."),
// and safe interpolation handlers (sql"...", html"..."). All literals are validated
// at compile time via the meta-system.

use verum_ast::{FileId, Span};
use verum_compiler::literal_parsers::*;
use verum_compiler::literal_registry::ParsedLiteral;
use verum_common::Text;

fn test_span() -> Span {
    Span::new(0, 10, FileId::new(0))
}

// DateTime Parser Tests

#[test]
fn test_datetime_rfc3339() {
    let result = parse_datetime(&Text::from("2024-01-15T10:30:00Z"), test_span(), None);
    assert!(result.is_ok());
    matches!(result.unwrap(), ParsedLiteral::DateTime(_));
}

#[test]
fn test_datetime_with_timezone() {
    let result = parse_datetime(&Text::from("2024-01-15T10:30:00+05:00"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_datetime_date_only() {
    let result = parse_datetime(&Text::from("2024-01-15"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_datetime_invalid() {
    let result = parse_datetime(&Text::from("invalid-date"), test_span(), None);
    assert!(result.is_err());
}

// Regex Parser Tests

#[test]
fn test_regex_simple() {
    let result = parse_regex(&Text::from("[a-z]+"), test_span(), None);
    assert!(result.is_ok());
    matches!(result.unwrap(), ParsedLiteral::Regex(_));
}

#[test]
fn test_regex_email_pattern() {
    let pattern = "^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\\.[a-zA-Z]{2,}$";
    let result = parse_regex(&Text::from(pattern), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_regex_invalid() {
    let result = parse_regex(&Text::from("[a-z"), test_span(), None);
    assert!(result.is_err());
}

// Interval Parser Tests

#[test]
fn test_interval_closed() {
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
    } else {
        panic!("Expected Interval");
    }
}

#[test]
fn test_interval_open() {
    let result = parse_interval(&Text::from("(0, 100)"), test_span(), None);
    assert!(result.is_ok());
    if let Ok(ParsedLiteral::Interval {
        inclusive_start,
        inclusive_end,
        ..
    }) = result
    {
        assert!(!inclusive_start);
        assert!(!inclusive_end);
    }
}

#[test]
fn test_interval_half_open() {
    let result = parse_interval(&Text::from("[0, 100)"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_interval_invalid() {
    let result = parse_interval(&Text::from("[100, 0]"), test_span(), None);
    assert!(result.is_err());
}

// Matrix Parser Tests

#[test]
fn test_matrix_2x2() {
    let result = parse_matrix(&Text::from("[[1, 2], [3, 4]]"), test_span(), None);
    assert!(result.is_ok());
    if let Ok(ParsedLiteral::Matrix { rows, cols, data }) = result {
        assert_eq!(rows, 2);
        assert_eq!(cols, 2);
        assert_eq!(data.len(), 4);
    } else {
        panic!("Expected Matrix");
    }
}

#[test]
fn test_matrix_3x3() {
    let result = parse_matrix(
        &Text::from("[[1, 2, 3], [4, 5, 6], [7, 8, 9]]"),
        test_span(),
        None,
    );
    assert!(result.is_ok());
    if let Ok(ParsedLiteral::Matrix { rows, cols, .. }) = result {
        assert_eq!(rows, 3);
        assert_eq!(cols, 3);
    }
}

#[test]
fn test_matrix_uneven_rows() {
    let result = parse_matrix(&Text::from("[[1, 2], [3, 4, 5]]"), test_span(), None);
    assert!(result.is_err());
}

// URI Parser Tests

#[test]
fn test_uri_https() {
    let result = parse_uri(&Text::from("https://example.com"), test_span(), None);
    assert!(result.is_ok());
    matches!(result.unwrap(), ParsedLiteral::Uri(_));
}

#[test]
fn test_uri_with_path() {
    let result = parse_uri(
        &Text::from("https://api.example.com/v1/users"),
        test_span(),
        None,
    );
    assert!(result.is_ok());
}

#[test]
fn test_uri_websocket() {
    let result = parse_uri(&Text::from("wss://example.com/socket"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_uri_missing_scheme() {
    let result = parse_uri(&Text::from("example.com"), test_span(), None);
    assert!(result.is_err());
}

// Email Parser Tests

#[test]
fn test_email_simple() {
    let result = parse_email(&Text::from("user@example.com"), test_span(), None);
    assert!(result.is_ok());
    matches!(result.unwrap(), ParsedLiteral::Email(_));
}

#[test]
fn test_email_with_subdomain() {
    let result = parse_email(&Text::from("user@mail.example.com"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_email_with_plus() {
    let result = parse_email(&Text::from("user+tag@example.com"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_email_invalid() {
    let result = parse_email(&Text::from("invalid"), test_span(), None);
    assert!(result.is_err());
}

// JSON Parser Tests

#[test]
fn test_json_object() {
    let result = parse_json(&Text::from(r#"{"key": "value"}"#), test_span(), None);
    assert!(result.is_ok());
    matches!(result.unwrap(), ParsedLiteral::Json(_));
}

#[test]
fn test_json_array() {
    let result = parse_json(&Text::from("[1, 2, 3]"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_json_nested() {
    let json = r#"{"name": "John", "address": {"city": "NY"}}"#;
    let result = parse_json(&Text::from(json), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_json_invalid() {
    let result = parse_json(&Text::from("{invalid}"), test_span(), None);
    assert!(result.is_err());
}

// XML Parser Tests

#[test]
fn test_xml_simple() {
    let result = parse_xml(&Text::from("<root></root>"), test_span(), None);
    assert!(result.is_ok());
    matches!(result.unwrap(), ParsedLiteral::Xml(_));
}

#[test]
fn test_xml_with_content() {
    let result = parse_xml(
        &Text::from("<root><item>value</item></root>"),
        test_span(),
        None,
    );
    assert!(result.is_ok());
}

#[test]
fn test_xml_with_attributes() {
    let result = parse_xml(
        &Text::from(r#"<root id="1"><item>value</item></root>"#),
        test_span(),
        None,
    );
    assert!(result.is_ok());
}

#[test]
fn test_xml_invalid() {
    let result = parse_xml(&Text::from("<root><item></root>"), test_span(), None);
    assert!(result.is_err());
}

// YAML Parser Tests

#[test]
fn test_yaml_object() {
    let result = parse_yaml(&Text::from("key: value"), test_span(), None);
    assert!(result.is_ok());
    matches!(result.unwrap(), ParsedLiteral::Yaml(_));
}

#[test]
fn test_yaml_array() {
    let yaml = "- item1\n- item2\n- item3";
    let result = parse_yaml(&Text::from(yaml), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_yaml_nested() {
    let yaml = r#"
        parent:
          child1: value1
          child2: value2
    "#;
    let result = parse_yaml(&Text::from(yaml), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_yaml_invalid() {
    let result = parse_yaml(&Text::from("key: : invalid"), test_span(), None);
    assert!(result.is_err());
}

// Integration Tests

#[test]
fn test_all_parsers_handle_empty_input() {
    let empty = Text::from("");
    let span = test_span();

    assert!(parse_datetime(&empty, span, None).is_err());
    // Note: Empty regex is valid - it matches empty string
    // assert!(parse_regex(&empty, span, None).is_err());
    assert!(parse_interval(&empty, span, None).is_err());
    assert!(parse_matrix(&empty, span, None).is_err());
    assert!(parse_uri(&empty, span, None).is_err());
    assert!(parse_email(&empty, span, None).is_err());
    assert!(parse_json(&empty, span, None).is_err());
    assert!(parse_xml(&empty, span, None).is_err());
    assert!(parse_yaml(&empty, span, None).is_err());
}
