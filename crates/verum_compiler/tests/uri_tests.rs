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
//! URI literal parser tests
//! Per CLAUDE.md standards - tests in tests/ directory

use verum_ast::Span;
use verum_compiler::literal_parsers::parse_uri;
use verum_common::Text;

#[test]
fn test_parse_https_url() {
    let result = parse_uri(&Text::from("https://example.com"), Span::default(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_http_url() {
    let result = parse_uri(&Text::from("http://example.com"), Span::default(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_url_with_path() {
    let result = parse_uri(
        &Text::from("https://example.com/path/to/resource"),
        Span::default(),
        None,
    );
    assert!(result.is_ok());
}

#[test]
fn test_parse_url_with_query() {
    let result = parse_uri(
        &Text::from("https://example.com/search?q=test&page=1"),
        Span::default(),
        None,
    );
    assert!(result.is_ok());
}

#[test]
fn test_parse_websocket_url() {
    let result = parse_uri(
        &Text::from("wss://example.com/socket"),
        Span::default(),
        None,
    );
    assert!(result.is_ok());
}

#[test]
fn test_parse_empty_uri() {
    let result = parse_uri(&Text::from(""), Span::default(), None);
    assert!(result.is_err());
}

#[test]
fn test_parse_missing_scheme() {
    let result = parse_uri(&Text::from("example.com"), Span::default(), None);
    assert!(result.is_err());
}

#[test]
fn test_parse_invalid_scheme() {
    let result = parse_uri(&Text::from("invalid://example.com"), Span::default(), None);
    assert!(result.is_err());
}

#[test]
fn test_parse_ftp_url() {
    let result = parse_uri(
        &Text::from("ftp://ftp.example.com/files"),
        Span::default(),
        None,
    );
    assert!(result.is_ok());
}

#[test]
fn test_parse_file_url() {
    let result = parse_uri(
        &Text::from("file:///home/user/file.txt"),
        Span::default(),
        None,
    );
    assert!(result.is_ok());
}

#[test]
fn test_parse_url_with_port() {
    let result = parse_uri(
        &Text::from("https://example.com:8080/api"),
        Span::default(),
        None,
    );
    assert!(result.is_ok());
}

#[test]
fn test_parse_url_with_fragment() {
    let result = parse_uri(
        &Text::from("https://example.com/page#section"),
        Span::default(),
        None,
    );
    assert!(result.is_ok());
}

#[test]
fn test_parse_url_with_auth() {
    let result = parse_uri(
        &Text::from("https://user:pass@example.com/"),
        Span::default(),
        None,
    );
    assert!(result.is_ok());
}
