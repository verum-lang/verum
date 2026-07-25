//! YAML literal parser
//!
//! Tagged text literal parser: handles `tag#"content"` compile-time parsing
//! and validation. Tags are registered via @tagged_literal attribute.
//!
//! Parses and validates YAML literals:
//! - yaml#"key: value"
//! - yaml#"- item1\n- item2"

use verum_ast::Span;
use verum_common::Text;
use verum_diagnostics::{Diagnostic, DiagnosticBuilder};

use crate::literal_registry::ParsedLiteral;

/// Parse YAML literal at compile-time
///
/// Semantic literal: `yaml#"..."` is compile-time validated YAML. Supports native
/// YAML syntax including multiline strings (| and >), anchors, and aliases.
/// Produces type YamlValue. Multiline form: `yaml#"""..."""`.
///
/// # Arguments
/// - `content`: The YAML string
/// - `span`: Source location for error reporting
///
/// # Returns
/// Validated YAML on success
pub fn parse_yaml(
    content: &Text,
    _span: Span,
    _source_file: Option<&verum_ast::SourceFile>,
) -> std::result::Result<ParsedLiteral, Diagnostic> {
    let s = content.as_str().trim();

    if s.is_empty() {
        return Err(DiagnosticBuilder::error()
            .message("YAML cannot be empty")
            .build());
    }

    // Validate YAML syntax using serde_yaml
    match serde_yaml::from_str::<serde_yaml::Value>(s) {
        Ok(_) => Ok(ParsedLiteral::Yaml(content.clone())),
        Err(e) => Err(DiagnosticBuilder::error()
            .message(format!("Invalid YAML: {}", e))
            .build()),
    }
}
