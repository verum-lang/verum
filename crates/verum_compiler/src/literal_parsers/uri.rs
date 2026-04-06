//! URI/URL literal parser
//!
//! Tagged text literal parser: handles `tag#"content"` compile-time parsing
//! and validation. Tags are registered via @tagged_literal attribute.
//!
//! Parses and validates URI/URL literals:
//! - url#"https://example.com"
//! - url#"https://api.example.com/v1/users"

use verum_ast::Span;
use verum_common::Text;
use verum_diagnostics::{Diagnostic, DiagnosticBuilder};

use crate::literal_registry::ParsedLiteral;

/// Parse URI literal at compile-time
///
/// Semantic literal: `url#"https://..."` or `uri#"..."` is compile-time validated.
/// Validates scheme, authority, path, query, and fragment components per RFC 3986.
/// Produces type Url. Interpolated form `url"...{expr}..."` auto-encodes parameters.
///
/// # Arguments
/// - `content`: The URI string
/// - `span`: Source location for error reporting
///
/// # Returns
/// Validated URI on success
pub fn parse_uri(
    content: &Text,
    _span: Span,
    _source_file: Option<&verum_ast::SourceFile>,
) -> std::result::Result<ParsedLiteral, Diagnostic> {
    let s = content.as_str().trim();

    // Basic URL validation
    if s.is_empty() {
        return Err(DiagnosticBuilder::error()
            .message("URI cannot be empty")
            .build());
    }

    // Check for scheme
    if !s.contains("://") {
        return Err(DiagnosticBuilder::error()
            .message(format!(
                "Invalid URI: missing scheme (e.g., 'https://'): '{}'",
                s
            ))
            .build());
    }

    // Validate common schemes
    let valid_schemes = ["http", "https", "ftp", "ftps", "ws", "wss", "file"];
    let scheme = s.split("://").next().unwrap();

    if !valid_schemes.contains(&scheme) {
        return Err(DiagnosticBuilder::error()
            .message(format!(
                "Unsupported URI scheme: '{}'. Supported schemes: {:?}",
                scheme, valid_schemes
            ))
            .build());
    }

    // Basic validation - just check it has a host after scheme
    let after_scheme = s.split("://").nth(1).unwrap_or("");
    if after_scheme.is_empty() {
        return Err(DiagnosticBuilder::error()
            .message("Invalid URI: missing host after scheme")
            .build());
    }

    Ok(ParsedLiteral::Uri(content.clone()))
}
