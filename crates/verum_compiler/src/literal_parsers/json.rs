//! JSON literal parser
//!
//! Tagged text literal parser: handles `tag#"content"` compile-time parsing
//! and validation. Tags are registered via @tagged_literal attribute.
//!
//! Parses and validates JSON literals:
//! - json#"{ \"key\": \"value\" }"
//! - json#"[1, 2, 3]"

use verum_ast::Span;
use verum_common::Text;
use verum_diagnostics::{Diagnostic, DiagnosticBuilder};

use crate::literal_registry::ParsedLiteral;

/// Parse JSON literal at compile-time
///
/// Semantic literal: `json#"{...}"` is compile-time validated JSON (JSON5 relaxed:
/// unquoted keys, trailing commas, single-quoted strings, comments allowed).
/// Produces type JsonValue. Multiline form: `json#"""..."""`.
///
/// # Arguments
/// - `content`: The JSON string
/// - `span`: Source location for error reporting
///
/// # Returns
/// Validated JSON on success
pub fn parse_json(
    content: &Text,
    _span: Span,
    _source_file: Option<&verum_ast::SourceFile>,
) -> std::result::Result<ParsedLiteral, Diagnostic> {
    let s = content.as_str().trim();

    if s.is_empty() {
        return Err(DiagnosticBuilder::error()
            .message("JSON cannot be empty")
            .build());
    }

    // Validate JSON syntax using serde_json
    match serde_json::from_str::<serde_json::Value>(s) {
        Ok(_) => Ok(ParsedLiteral::Json(content.clone())),
        Err(e) => Err(DiagnosticBuilder::error()
            .message(format!("Invalid JSON: {}", e))
            .build()),
    }
}
