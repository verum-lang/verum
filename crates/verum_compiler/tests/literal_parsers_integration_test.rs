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
//! Integration test demonstrating all literal parsers working together
//!
//! This test validates that all 6 literal parsers (email, uri, xml, yaml, interval, matrix)
//! are fully functional and integrated into the compilation pipeline.
//!
//! Per CLAUDE.md: Tests in tests/ directory, not in src/ with #[cfg(test)]

use verum_ast::{FileId, Span};
use verum_compiler::literal_parsers::{
    parse_email, parse_interval, parse_matrix, parse_uri, parse_xml, parse_yaml,
};
use verum_compiler::literal_registry::{LiteralRegistry, ParsedLiteral};
use verum_common::{Maybe, Text};

fn test_span() -> Span {
    Span::new(0, 100, FileId::new(0))
}

#[test]
fn test_all_parsers_integration() {
    // Test Email Parser
    let email_result = parse_email(&Text::from("user@example.com"), test_span(), None);
    assert!(
        email_result.is_ok(),
        "Email parser should accept valid email"
    );

    // Test URI Parser
    let uri_result = parse_uri(&Text::from("https://api.example.com/v1"), test_span(), None);
    assert!(uri_result.is_ok(), "URI parser should accept valid URI");

    // Test XML Parser
    let xml_result = parse_xml(
        &Text::from("<root><item>test</item></root>"),
        test_span(),
        None,
    );
    assert!(xml_result.is_ok(), "XML parser should accept valid XML");

    // Test YAML Parser
    let yaml_result = parse_yaml(
        &Text::from("key: value\nlist:\n  - item1\n  - item2"),
        test_span(),
        None,
    );
    assert!(yaml_result.is_ok(), "YAML parser should accept valid YAML");

    // Test Interval Parser
    let interval_result = parse_interval(&Text::from("[0, 100]"), test_span(), None);
    assert!(
        interval_result.is_ok(),
        "Interval parser should accept valid interval"
    );
    if let Ok(ParsedLiteral::Interval {
        start,
        end,
        inclusive_start,
        inclusive_end,
    }) = interval_result
    {
        assert_eq!(start, 0.0);
        assert_eq!(end, 100.0);
        assert!(inclusive_start);
        assert!(inclusive_end);
    }

    // Test Matrix Parser
    let matrix_result = parse_matrix(&Text::from("[[1, 2, 3], [4, 5, 6]]"), test_span(), None);
    assert!(
        matrix_result.is_ok(),
        "Matrix parser should accept valid matrix"
    );
    if let Ok(ParsedLiteral::Matrix { rows, cols, data }) = matrix_result {
        assert_eq!(rows, 2);
        assert_eq!(cols, 3);
        assert_eq!(data.len(), 6);
    }
}

#[test]
fn test_literal_registry_integration() {
    let registry = LiteralRegistry::new();
    registry.register_builtin_handlers();

    // Test that all tags are registered
    let email_tag = Text::from("email");
    let handler = registry.get_handler(&email_tag);
    assert!(handler.is_some(), "Email handler should be registered");

    let url_tag = Text::from("url");
    let handler = registry.get_handler(&url_tag);
    assert!(handler.is_some(), "URL handler should be registered");

    let xml_tag = Text::from("xml");
    let handler = registry.get_handler(&xml_tag);
    assert!(handler.is_some(), "XML handler should be registered");

    let yaml_tag = Text::from("yaml");
    let handler = registry.get_handler(&yaml_tag);
    assert!(handler.is_some(), "YAML handler should be registered");

    let interval_tag = Text::from("interval");
    let handler = registry.get_handler(&interval_tag);
    assert!(handler.is_some(), "Interval handler should be registered");

    let mat_tag = Text::from("mat");
    let handler = registry.get_handler(&mat_tag);
    assert!(handler.is_some(), "Matrix handler should be registered");
}

#[test]
fn test_parse_literal_via_registry() {
    let registry = LiteralRegistry::new();
    registry.register_builtin_handlers();

    // Test parsing via registry dispatch
    let email = Text::from("email");
    let content = Text::from("admin@example.com");
    let result = registry.parse_literal(&email, &content, test_span(), None);
    assert!(result.is_ok(), "Registry should parse email via dispatch");

    let url = Text::from("url");
    let content = Text::from("https://example.com");
    let result = registry.parse_literal(&url, &content, test_span(), None);
    assert!(result.is_ok(), "Registry should parse URI via dispatch");
}

#[test]
fn test_error_handling_all_parsers() {
    // Email: invalid format
    let result = parse_email(&Text::from("not-an-email"), test_span(), None);
    assert!(result.is_err(), "Should reject invalid email");

    // URI: missing scheme
    let result = parse_uri(&Text::from("example.com"), test_span(), None);
    assert!(result.is_err(), "Should reject URI without scheme");

    // XML: unclosed tag
    let result = parse_xml(&Text::from("<root><item>"), test_span(), None);
    assert!(result.is_err(), "Should reject unclosed XML tag");

    // YAML: invalid syntax
    let result = parse_yaml(&Text::from("key: : invalid"), test_span(), None);
    assert!(result.is_err(), "Should reject invalid YAML");

    // Interval: start > end
    let result = parse_interval(&Text::from("[100, 0]"), test_span(), None);
    assert!(result.is_err(), "Should reject invalid interval");

    // Matrix: uneven rows
    let result = parse_matrix(&Text::from("[[1, 2], [3, 4, 5]]"), test_span(), None);
    assert!(result.is_err(), "Should reject non-rectangular matrix");
}

#[test]
fn test_complex_real_world_examples() {
    // Complex email with subdomain and plus addressing
    let result = parse_email(
        &Text::from("john.doe+tag@mail.company.com"),
        test_span(),
        None,
    );
    assert!(result.is_ok());

    // Complex URI with query and fragment
    let result = parse_uri(
        &Text::from("https://api.example.com:8080/v1/users?page=1&limit=10#results"),
        test_span(),
        None,
    );
    assert!(result.is_ok());

    // Complex nested XML with attributes
    let xml = r#"
        <config version="1.0">
            <database>
                <host>localhost</host>
                <port>5432</port>
                <credentials user="admin" encrypted="true"/>
            </database>
            <features>
                <feature name="auth" enabled="true"/>
                <feature name="logging" enabled="false"/>
            </features>
        </config>
    "#;
    let result = parse_xml(&Text::from(xml), test_span(), None);
    assert!(result.is_ok());

    // Complex YAML with nested structures
    let yaml = r#"
production:
  timeout: 30
  retries: 3
  debug: false
  servers:
    - host: server1.example.com
      port: 8080
    - host: server2.example.com
      port: 8080
development:
  timeout: 30
  retries: 3
  debug: true
  servers:
    - host: localhost
      port: 3000
"#;
    let result = parse_yaml(&Text::from(yaml), test_span(), None);
    assert!(result.is_ok(), "YAML parse failed: {:?}", result.err());

    // Scientific notation interval
    let result = parse_interval(&Text::from("(1e-10, 1e10)"), test_span(), None);
    assert!(result.is_ok());

    // Large matrix with floating point
    let result = parse_matrix(
        &Text::from("[[1.5, 2.5, 3.5, 4.5], [5.5, 6.5, 7.5, 8.5], [9.5, 10.5, 11.5, 12.5]]"),
        test_span(),
        None,
    );
    assert!(result.is_ok());
    if let Ok(ParsedLiteral::Matrix { rows, cols, .. }) = result {
        assert_eq!(rows, 3);
        assert_eq!(cols, 4);
    }
}

#[test]
fn test_edge_cases_all_parsers() {
    // Email: Maximum length TLD
    let result = parse_email(&Text::from("user@example.international"), test_span(), None);
    assert!(result.is_ok());

    // URI: File protocol with triple slash
    let result = parse_uri(
        &Text::from("file:///absolute/path/to/file"),
        test_span(),
        None,
    );
    assert!(result.is_ok());

    // XML: Empty self-closing tag
    let result = parse_xml(&Text::from("<empty/>"), test_span(), None);
    assert!(result.is_ok());

    // YAML: Null value
    let result = parse_yaml(&Text::from("value: null"), test_span(), None);
    assert!(result.is_ok());

    // Interval: Single point (start == end)
    let result = parse_interval(&Text::from("[42, 42]"), test_span(), None);
    assert!(result.is_ok());

    // Matrix: 1x1 matrix (single element)
    let result = parse_matrix(&Text::from("[[3.14159]]"), test_span(), None);
    assert!(result.is_ok());
    if let Ok(ParsedLiteral::Matrix { rows, cols, data }) = result {
        assert_eq!(rows, 1);
        assert_eq!(cols, 1);
        assert_eq!(data.len(), 1);
    }
}

#[test]
fn test_whitespace_handling() {
    // Email: with surrounding whitespace
    let result = parse_email(&Text::from("  user@example.com  "), test_span(), None);
    assert!(result.is_ok());

    // URI: with surrounding whitespace
    let result = parse_uri(&Text::from("  https://example.com  "), test_span(), None);
    assert!(result.is_ok());

    // XML: with whitespace and newlines
    let result = parse_xml(
        &Text::from("\n  <root>\n    <item/>\n  </root>\n  "),
        test_span(),
        None,
    );
    assert!(result.is_ok());

    // Matrix: with extra spaces
    let result = parse_matrix(&Text::from("[[ 1 , 2 , 3 ]]"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_spec_compliance() {
    // Verify all parsers follow literal syntax rules: compile-time validation,
    // proper type inference, and meaningful error diagnostics on invalid input

    // Each parser should:
    // 1. Accept valid input
    // 2. Reject invalid input with clear error messages
    // 3. Execute at compile-time
    // 4. Return ParsedLiteral enum variant

    let registry = LiteralRegistry::new();
    registry.register_builtin_handlers();

    // Verify handler registration matches spec
    let expected_tags = vec![
        "d", "rx", "interval", "mat", "url", "email", "json", "xml", "yaml",
    ];

    for tag_str in expected_tags {
        let tag = Text::from(tag_str);
        let handler = registry.get_handler(&tag);
        assert!(
            handler.is_some(),
            "Tag '{}' should be registered per spec",
            tag_str
        );

        if handler.is_some() {
            let h = handler.unwrap();
            assert!(
                h.compile_time,
                "Tag '{}' should support compile-time parsing per spec",
                tag_str
            );
        }
    }
}
