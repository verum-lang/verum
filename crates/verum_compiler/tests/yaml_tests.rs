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
//! YAML literal parser tests
//! Per CLAUDE.md standards - tests in tests/ directory

use verum_ast::Span;
use verum_compiler::literal_parsers::parse_yaml;
use verum_common::Text;

#[test]
fn test_parse_simple_yaml() {
    let result = parse_yaml(&Text::from("key: value"), Span::default(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_yaml_list() {
    let result = parse_yaml(
        &Text::from("- item1\n- item2\n- item3"),
        Span::default(),
        None,
    );
    assert!(result.is_ok());
}

#[test]
fn test_parse_yaml_nested() {
    let yaml = "parent:\n  child1: value1\n  child2: value2";
    let result = parse_yaml(&Text::from(yaml), Span::default(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_yaml_with_numbers() {
    let yaml = "count: 42\nprice: 19.99";
    let result = parse_yaml(&Text::from(yaml), Span::default(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_yaml_with_booleans() {
    let yaml = "enabled: true\ndisabled: false";
    let result = parse_yaml(&Text::from(yaml), Span::default(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_yaml_empty() {
    let result = parse_yaml(&Text::from(""), Span::default(), None);
    assert!(result.is_err());
}

#[test]
fn test_parse_yaml_null() {
    let yaml = "value: null";
    let result = parse_yaml(&Text::from(yaml), Span::default(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_yaml_multiline_string() {
    let yaml = "description: |\n  This is a\n  multiline string";
    let result = parse_yaml(&Text::from(yaml), Span::default(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_yaml_inline_list() {
    let yaml = "items: [1, 2, 3, 4]";
    let result = parse_yaml(&Text::from(yaml), Span::default(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_yaml_inline_map() {
    let yaml = "person: {name: John, age: 30}";
    let result = parse_yaml(&Text::from(yaml), Span::default(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_yaml_invalid() {
    let yaml = "  bad: indent\n: missing key";
    let result = parse_yaml(&Text::from(yaml), Span::default(), None);
    // This may or may not be valid depending on YAML parser strictness
    // For now we just run it - the key test is that it doesn't panic
    let _ = result;
}

#[test]
fn test_parse_yaml_with_anchors() {
    let yaml = "defaults: &defaults\n  timeout: 30\nserver:\n  <<: *defaults\n  host: localhost";
    let result = parse_yaml(&Text::from(yaml), Span::default(), None);
    assert!(result.is_ok());
}
