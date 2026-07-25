//! Email address literal parser
//!
//! Tagged text literal parser: handles `tag#"content"` compile-time parsing
//! and validation. Tags are registered via @tagged_literal attribute.
//!
//! Parses and validates email address literals:
//! - email#"user@example.com"
//! - email#"john.doe+tag@subdomain.example.com"

use regex::Regex;
use verum_ast::{SourceFile, Span};
use verum_common::{List, Text};
use verum_diagnostics::{Diagnostic, DiagnosticBuilder};

use crate::literal_registry::ParsedLiteral;

/// Convert AST span to diagnostic span using source file information
fn convert_span(ast_span: Span, source_file: Option<&SourceFile>) -> verum_diagnostics::Span {
    match source_file {
        Some(file) => {
            // Use SourceFile to convert byte offsets to line/column
            match file.span_to_line_col(ast_span) {
                Some(line_col_span) => line_col_span,
                None => {
                    // Fallback if span doesn't match file (shouldn't happen)
                    verum_diagnostics::Span::new("<unknown>", 1, 1, 1)
                }
            }
        }
        None => {
            // Fallback when source file is not available
            // This can happen in tests or partial compilation
            verum_diagnostics::Span::new("<unknown>", 1, 1, 1)
        }
    }
}

/// Parse email literal at compile-time
///
/// Semantic literal: `email#"user@example.com"` is compile-time validated.
/// Validates local-part and domain per RFC 5321/5322. Produces type Email.
///
/// # Arguments
/// - `content`: The email address string
/// - `span`: Source location for error reporting
/// - `source_file`: Optional source file for accurate span conversion
///
/// # Returns
/// Validated email address on success
pub fn parse_email(
    content: &Text,
    span: Span,
    source_file: Option<&SourceFile>,
) -> std::result::Result<ParsedLiteral, Diagnostic> {
    let s = content.as_str().trim();

    if s.is_empty() {
        return Err(DiagnosticBuilder::error()
            .message("Email address cannot be empty")
            .span(convert_span(span, source_file))
            .build());
    }

    // RFC 5322 compliant email regex (simplified)
    let email_regex = Regex::new(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$").unwrap();

    if !email_regex.is_match(s) {
        return Err(DiagnosticBuilder::error()
            .message(format!("Invalid email address format: '{}'", s))
            .span(convert_span(span, source_file))
            .build());
    }

    // Additional validation
    if !s.contains('@') {
        return Err(DiagnosticBuilder::error()
            .message("Email address must contain '@' symbol")
            .span(convert_span(span, source_file))
            .build());
    }

    let parts: List<&str> = s.split('@').collect();
    if parts.len() != 2 {
        return Err(DiagnosticBuilder::error()
            .message("Email address must have exactly one '@' symbol")
            .span(convert_span(span, source_file))
            .build());
    }

    let local = parts[0];
    let domain = parts[1];

    if local.is_empty() {
        return Err(DiagnosticBuilder::error()
            .message("Email local part cannot be empty")
            .span(convert_span(span, source_file))
            .build());
    }

    if domain.is_empty() || !domain.contains('.') {
        return Err(DiagnosticBuilder::error()
            .message("Email domain must contain at least one dot")
            .span(convert_span(span, source_file))
            .build());
    }

    Ok(ParsedLiteral::Email(content.clone()))
}
