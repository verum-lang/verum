//! Regex literal parser
//!
//! Tagged text literal parser: handles `tag#"content"` compile-time parsing
//! and validation. Tags are registered via @tagged_literal attribute.
//!
//! Parses and validates regex literals:
//! - rx#"[a-z]+"
//! - rx#"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$"

use regex::Regex;
use verum_ast::{SourceFile, Span};
use verum_common::Text;
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

/// Parse regex literal at compile-time
///
/// Tagged text literal: `rx#"pattern"` is compile-time validated regex.
/// Produces type Regex. Catches syntax errors at compile time.
///
/// Validates regex patterns at compile-time, catching syntax errors early.
/// Supports:
/// - Basic patterns: `[a-z]+`, `\d{3}-\d{4}`
/// - Anchors: `^pattern$`
/// - Named groups: `(?<name>pattern)`
/// - Character classes: `[a-zA-Z0-9]`, `\w`, `\d`, `\s`
/// - Quantifiers: `+`, `*`, `?`, `{n,m}`
/// - Alternation: `pattern1|pattern2`
///
/// # Arguments
/// - `content`: The regex pattern string
/// - `span`: Source location for error reporting
/// - `source_file`: Optional source file for accurate span conversion
///
/// # Returns
/// Validated regex pattern wrapped in `ParsedLiteral::Regex` on success,
/// or a diagnostic error with precise location on failure
///
/// # Examples
/// ```
/// use verum_compiler::literal_parsers::parse_regex;
/// use verum_ast::{Span, FileId};
/// use verum_common::Text;
///
/// let span = Span::new(0, 10, FileId::new(0));
/// let result = parse_regex(&Text::from("[a-z]+"), span, None);
/// assert!(result.is_ok());
/// ```
pub fn parse_regex(
    content: &Text,
    span: Span,
    source_file: Option<&SourceFile>,
) -> std::result::Result<ParsedLiteral, Diagnostic> {
    let pattern = content.as_str();

    // Validate regex at compile-time using the regex crate
    // This catches:
    // - Unclosed brackets: `[a-z`
    // - Unmatched parentheses: `(abc`
    // - Invalid escape sequences: `\q`
    // - Invalid quantifiers: `{,5}`, `{5,3}`
    // - Invalid character class ranges: `[z-a]`
    // - Dangling meta-characters: `*pattern`, `+pattern`
    match Regex::new(pattern) {
        Ok(_regex) => {
            // Regex is valid, return the pattern
            // Note: We store the pattern as Text, not the compiled Regex,
            // because Regex is not Clone/Send/Sync-compatible with our AST
            Ok(ParsedLiteral::Regex(content.clone()))
        }
        Err(e) => {
            // Regex compilation failed - provide detailed error
            let error_msg = format!("Invalid regex pattern: {}", e);

            Err(DiagnosticBuilder::error()
                .message(error_msg)
                .span(convert_span(span, source_file))
                .build())
        }
    }
}
