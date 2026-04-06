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
// Tests for regex literal parser
// Per CLAUDE.md standards - tests in tests/ directory

use verum_ast::{FileId, Span};
use verum_compiler::literal_parsers::regex::parse_regex;
use verum_compiler::literal_registry::ParsedLiteral;
use verum_common::Text;

fn test_span() -> Span {
    Span::new(0, 10, FileId::new(0))
}

// Basic pattern tests

#[test]
fn test_parse_simple_regex() {
    let result = parse_regex(&Text::from("[a-z]+"), test_span(), None);
    assert!(result.is_ok());
    match result.unwrap() {
        ParsedLiteral::Regex(pattern) => {
            assert_eq!(pattern.as_str(), "[a-z]+");
        }
        _ => panic!("Expected Regex variant"),
    }
}

#[test]
fn test_parse_email_regex() {
    let pattern = "^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\\.[a-zA-Z]{2,}$";
    let result = parse_regex(&Text::from(pattern), test_span(), None);
    assert!(result.is_ok());
    match result.unwrap() {
        ParsedLiteral::Regex(p) => {
            assert_eq!(p.as_str(), pattern);
        }
        _ => panic!("Expected Regex variant"),
    }
}

#[test]
fn test_parse_digit_regex() {
    let result = parse_regex(&Text::from("\\d{3}-\\d{3}-\\d{4}"), test_span(), None);
    assert!(result.is_ok());
    match result.unwrap() {
        ParsedLiteral::Regex(pattern) => {
            assert_eq!(pattern.as_str(), "\\d{3}-\\d{3}-\\d{4}");
        }
        _ => panic!("Expected Regex variant"),
    }
}

#[test]
fn test_parse_invalid_regex() {
    // Unclosed bracket
    let result = parse_regex(&Text::from("[a-z"), test_span(), None);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.message().contains("Invalid regex pattern"));
}

#[test]
fn test_parse_invalid_group() {
    // Unclosed parenthesis
    let result = parse_regex(&Text::from("(abc"), test_span(), None);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.message().contains("Invalid regex pattern"));
}

// Character class tests

#[test]
fn test_parse_character_classes() {
    let patterns = vec!["[a-zA-Z0-9]", "[0-9]+", "[^a-z]", "\\w+", "\\d+", "\\s*"];

    for pattern in patterns {
        let result = parse_regex(&Text::from(pattern), test_span(), None);
        assert!(result.is_ok(), "Pattern '{}' should be valid", pattern);
    }
}

// Anchor tests

#[test]
fn test_parse_anchors() {
    let patterns = vec!["^start", "end$", "^exact$", "\\bword\\b"];

    for pattern in patterns {
        let result = parse_regex(&Text::from(pattern), test_span(), None);
        assert!(result.is_ok(), "Pattern '{}' should be valid", pattern);
    }
}

// Quantifier tests

#[test]
fn test_parse_quantifiers() {
    let patterns = vec!["a+", "b*", "c?", "d{3}", "e{2,5}", "f{3,}"];

    for pattern in patterns {
        let result = parse_regex(&Text::from(pattern), test_span(), None);
        assert!(result.is_ok(), "Pattern '{}' should be valid", pattern);
    }
}

// Named groups test

#[test]
fn test_parse_named_groups() {
    let pattern = "(?<year>\\d{4})-(?<month>\\d{2})-(?<day>\\d{2})";
    let result = parse_regex(&Text::from(pattern), test_span(), None);
    assert!(result.is_ok());
    match result.unwrap() {
        ParsedLiteral::Regex(p) => {
            assert_eq!(p.as_str(), pattern);
        }
        _ => panic!("Expected Regex variant"),
    }
}

// Alternation test

#[test]
fn test_parse_alternation() {
    let patterns = vec!["cat|dog", "red|green|blue", "(jpg|png|gif)"];

    for pattern in patterns {
        let result = parse_regex(&Text::from(pattern), test_span(), None);
        assert!(result.is_ok(), "Pattern '{}' should be valid", pattern);
    }
}

// Complex patterns from spec

#[test]
fn test_parse_complex_email() {
    let pattern = "^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\\.[a-zA-Z]{2,}$";
    let result = parse_regex(&Text::from(pattern), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_phone_pattern() {
    let pattern = "^\\+?1?\\d{10,14}$";
    let result = parse_regex(&Text::from(pattern), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_url_pattern() {
    let pattern = "https?://[^\\s]+";
    let result = parse_regex(&Text::from(pattern), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_ipv4_pattern() {
    let pattern = "^(?:(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\\.){3}(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)$";
    let result = parse_regex(&Text::from(pattern), test_span(), None);
    assert!(result.is_ok());
}

// Error cases

#[test]
fn test_parse_unclosed_bracket() {
    let result = parse_regex(&Text::from("[a-z"), test_span(), None);
    assert!(result.is_err());
}

#[test]
fn test_parse_unclosed_paren() {
    let result = parse_regex(&Text::from("(abc"), test_span(), None);
    assert!(result.is_err());
}

#[test]
fn test_parse_invalid_quantifier() {
    let result = parse_regex(&Text::from("a{5,3}"), test_span(), None);
    assert!(result.is_err());
}

#[test]
fn test_parse_invalid_escape() {
    // \q is not a valid escape in regex
    let result = parse_regex(&Text::from("\\q"), test_span(), None);
    assert!(result.is_err());
}

#[test]
fn test_parse_invalid_range() {
    let result = parse_regex(&Text::from("[z-a]"), test_span(), None);
    assert!(result.is_err());
}

// Edge cases

#[test]
fn test_parse_empty_regex() {
    // Empty regex is valid - matches empty string
    let result = parse_regex(&Text::from(""), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_whitespace_pattern() {
    let result = parse_regex(&Text::from("\\s+"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_unicode_pattern() {
    // Rust regex crate supports Unicode
    let result = parse_regex(&Text::from("\\p{L}+"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_literal_dot() {
    let result = parse_regex(&Text::from("\\."), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_escaped_backslash() {
    let result = parse_regex(&Text::from("\\\\"), test_span(), None);
    assert!(result.is_ok());
}
