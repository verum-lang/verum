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
//! XML literal parser tests
//! Per CLAUDE.md standards - tests in tests/ directory

use verum_ast::Span;
use verum_compiler::literal_parsers::parse_xml;
use verum_common::Text;

#[test]
fn test_parse_simple_xml() {
    let result = parse_xml(&Text::from("<root>content</root>"), Span::default(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_xml_with_attributes() {
    let result = parse_xml(
        &Text::from("<element id=\"123\" class=\"test\">value</element>"),
        Span::default(),
        None,
    );
    assert!(result.is_ok());
}

#[test]
fn test_parse_xml_nested() {
    let xml = "<root><parent><child>value</child></parent></root>";
    let result = parse_xml(&Text::from(xml), Span::default(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_xml_self_closing() {
    let result = parse_xml(&Text::from("<element />"), Span::default(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_xml_empty() {
    let result = parse_xml(&Text::from(""), Span::default(), None);
    assert!(result.is_err());
}

#[test]
fn test_parse_xml_unclosed_tag() {
    let result = parse_xml(&Text::from("<root><unclosed>"), Span::default(), None);
    assert!(result.is_err());
}

#[test]
fn test_parse_xml_mismatched_tags() {
    let result = parse_xml(&Text::from("<root></wrong>"), Span::default(), None);
    assert!(result.is_err());
}

#[test]
fn test_parse_xml_with_declaration() {
    let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><root>content</root>";
    let result = parse_xml(&Text::from(xml), Span::default(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_xml_with_namespaces() {
    let xml = "<root xmlns:ns=\"http://example.com\"><ns:element>value</ns:element></root>";
    let result = parse_xml(&Text::from(xml), Span::default(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_xml_cdata() {
    let xml = "<root><![CDATA[Some <special> content]]></root>";
    let result = parse_xml(&Text::from(xml), Span::default(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_xml_comments() {
    let xml = "<root><!-- This is a comment --><element>value</element></root>";
    let result = parse_xml(&Text::from(xml), Span::default(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_xml_entities() {
    let xml = "<root>&lt;escaped&gt; &amp; entities</root>";
    let result = parse_xml(&Text::from(xml), Span::default(), None);
    assert!(result.is_ok());
}
