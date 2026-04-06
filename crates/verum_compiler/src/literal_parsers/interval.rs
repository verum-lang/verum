//! Interval literal parser
//!
//! Tagged text literal parser: handles `tag#"content"` compile-time parsing
//! and validation. Tags are registered via @tagged_literal attribute.
//!
//! Parses mathematical interval literals:
//! - interval#"[0, 100]"  // Closed interval
//! - interval#"[0, 100)"  // Half-open interval
//! - interval#"(0, 100]"  // Half-open interval
//! - interval#"(0, 100)"  // Open interval

use verum_ast::{SourceFile, Span};
use verum_common::{List, Text};
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

/// Parse interval literal at compile-time
///
/// Composite literal: `interval#"[0, 100)"` is compile-time validated interval
/// notation. Uses standard mathematical bracket notation: [ for inclusive, ( for
/// exclusive. Produces type Interval<f64> or DateRange.
///
/// # Arguments
/// - `content`: The interval string (e.g., "[0, 100)")
/// - `span`: Source location for error reporting
/// - `source_file`: Optional source file for accurate span conversion
///
/// # Returns
/// Parsed interval with bounds and inclusivity flags
///
/// # Examples
/// ```ignore
/// use verum_compiler::literal_parsers::parse_interval;
/// use verum_ast::Span;
/// use verum_common::Text;
///
/// let span = Span::new(0, 10, verum_ast::FileId::new(0));
/// let result = parse_interval(&Text::from("[0, 100)"), span, None);
/// assert!(result.is_ok());
/// ```
pub fn parse_interval(
    content: &Text,
    span: Span,
    source_file: Option<&SourceFile>,
) -> std::result::Result<ParsedLiteral, Diagnostic> {
    let s = content.as_str().trim();

    if s.len() < 5 {
        return Err(DiagnosticBuilder::new(Severity::Error)
            .message(format!(
                "Invalid interval format: '{}'. Expected format like '[0, 100)'",
                s
            ))
            .span(convert_span(span, source_file))
            .build());
    }

    // Parse opening bracket
    let inclusive_start = match s.chars().next() {
        Some('[') => true,
        Some('(') => false,
        _ => {
            return Err(DiagnosticBuilder::new(Severity::Error)
                .message(format!(
                    "Invalid interval: must start with '[' or '(', got '{}'",
                    s
                ))
                .span(convert_span(span, source_file))
                .build());
        }
    };

    // Parse closing bracket
    let inclusive_end = match s.chars().last() {
        Some(']') => true,
        Some(')') => false,
        _ => {
            return Err(DiagnosticBuilder::new(Severity::Error)
                .message(format!(
                    "Invalid interval: must end with ']' or ')', got '{}'",
                    s
                ))
                .span(convert_span(span, source_file))
                .build());
        }
    };

    // Extract middle content
    let middle = &s[1..s.len() - 1];

    // Split by comma
    let parts: List<&str> = middle.split(',').collect();
    if parts.len() != 2 {
        return Err(DiagnosticBuilder::new(Severity::Error)
            .message(format!(
                "Invalid interval: expected two values separated by comma, got '{}'",
                middle
            ))
            .span(convert_span(span, source_file))
            .build());
    }

    // Parse start and end values
    let start_str = parts[0].trim();
    let end_str = parts[1].trim();

    let start = start_str.parse::<f64>().map_err(|_| {
        DiagnosticBuilder::new(Severity::Error)
            .message(format!("Invalid interval start value: '{}'", start_str))
            .span(convert_span(span, source_file))
            .build()
    })?;

    let end = end_str.parse::<f64>().map_err(|_| {
        DiagnosticBuilder::new(Severity::Error)
            .message(format!("Invalid interval end value: '{}'", end_str))
            .span(convert_span(span, source_file))
            .build()
    })?;

    // Validate interval
    if start > end {
        return Err(DiagnosticBuilder::new(Severity::Error)
            .message(format!(
                "Invalid interval: start {} is greater than end {}",
                start, end
            ))
            .span(convert_span(span, source_file))
            .build());
    }

    Ok(ParsedLiteral::Interval {
        start,
        end,
        inclusive_start,
        inclusive_end,
    })
}
