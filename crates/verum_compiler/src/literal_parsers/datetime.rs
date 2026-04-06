//! DateTime literal parser
//!
//! Tagged text literal parser: handles `tag#"content"` compile-time parsing
//! and validation. Tags are registered via @tagged_literal attribute.
//!
//! Parses datetime literals in ISO 8601 format:
//! - d#"2024-01-15T10:30:00Z"
//! - d#"2024-01-15T10:30:00+05:00"
//! - d#"2024-01-15"

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use verum_ast::{SourceFile, Span};
use verum_common::Text;
use verum_diagnostics::{Diagnostic, DiagnosticBuilder, Severity};

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

/// Parse datetime literal at compile-time
///
/// Tagged text literal: `d#"2025-11-05"` or `datetime#"..."` is compile-time
/// validated datetime. Produces type DateTime. Invalid dates (e.g., month 13)
/// are caught at compile time.
///
/// # Arguments
/// - `content`: The datetime string (e.g., "2024-01-15T10:30:00Z")
/// - `span`: Source location for error reporting
/// - `source_file`: Optional source file for accurate span conversion
///
/// # Returns
/// Unix timestamp (seconds since epoch) on success
///
/// # Examples
/// ```ignore
/// use verum_compiler::literal_parsers::parse_datetime;
/// use verum_ast::Span;
/// use verum_common::Text;
///
/// let span = Span::new(0, 10, verum_ast::FileId::new(0));
/// let result = parse_datetime(&Text::from("2024-01-15T10:30:00Z"), span, None);
/// assert!(result.is_ok());
/// ```
pub fn parse_datetime(
    content: &Text,
    span: Span,
    source_file: Option<&SourceFile>,
) -> std::result::Result<ParsedLiteral, Diagnostic> {
    let s = content.as_str().trim();

    // Try parsing as full datetime with timezone
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(ParsedLiteral::DateTime(dt.timestamp()));
    }

    // Try parsing as date only (assume UTC midnight)
    if let Ok(date) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let dt = Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0).unwrap());
        return Ok(ParsedLiteral::DateTime(dt.timestamp()));
    }

    // Try parsing other ISO 8601 formats
    let formats = vec![
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%SZ",
    ];

    for fmt in formats {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
            let dt = Utc.from_utc_datetime(&naive);
            return Ok(ParsedLiteral::DateTime(dt.timestamp()));
        }
    }

    Err(DiagnosticBuilder::new(Severity::Error)
        .message(format!(
            "Invalid datetime format: '{}'. Expected ISO 8601 format (e.g., '2024-01-15T10:30:00Z')",
            s
        ))
        .span(convert_span(span, source_file))
        .build())
}
