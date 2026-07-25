//! Matrix literal parser
//!
//! Tagged text literal parser: handles `tag#"content"` compile-time parsing
//! and validation. Tags are registered via @tagged_literal attribute.
//!
//! Parses matrix literals in row-major order:
//! - mat#"[[1, 2], [3, 4]]"
//! - mat#"[[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]]"

use verum_ast::Span;
use verum_common::{List, Text};
use verum_diagnostics::{Diagnostic, DiagnosticBuilder};

use crate::literal_registry::ParsedLiteral;

/// Parse matrix literal at compile-time
///
/// Composite literal: `mat#"[[1, 2], [3, 4]]"` is compile-time validated matrix.
/// Validates consistent row dimensions. Produces type Matrix<R, C, T> with
/// compile-time known dimensions. Data stored in row-major order.
///
/// # Arguments
/// - `content`: The matrix string (e.g., "[[1, 2], [3, 4]]")
/// - `span`: Source location for error reporting
///
/// # Returns
/// Parsed matrix with dimensions and data in row-major order
///
/// # Examples
/// ```ignore
/// use verum_compiler::literal_parsers::parse_matrix;
/// use verum_ast::Span;
/// use verum_common::Text;
///
/// let span = Span::new(0, 10, verum_ast::FileId::new(0));
/// let result = parse_matrix(&Text::from("[[1, 2], [3, 4]]"), span);
/// assert!(result.is_ok());
/// ```
pub fn parse_matrix(
    content: &Text,
    _span: Span,
    _source_file: Option<&verum_ast::SourceFile>,
) -> std::result::Result<ParsedLiteral, Diagnostic> {
    let s = content.as_str().trim();

    // Must start and end with double brackets
    if !s.starts_with("[[") || !s.ends_with("]]") {
        return Err(DiagnosticBuilder::error()
            .message(format!(
                "Invalid matrix format: '{}'. Expected format like '[[1, 2], [3, 4]]'",
                s
            ))
            .build());
    }

    // Remove outer brackets
    let inner = &s[1..s.len() - 1];

    // Parse rows
    let mut rows: List<List<f64>> = List::new();
    let mut current_row = Text::new();
    let mut depth = 0;

    for ch in inner.chars() {
        match ch {
            '[' => {
                depth += 1;
                if depth > 1 {
                    current_row.push(ch);
                }
            }
            ']' => {
                depth -= 1;
                if depth == 0 {
                    // End of row
                    let row = parse_row(&current_row)?;
                    rows.push(row);
                    current_row.clear();
                } else {
                    current_row.push(ch);
                }
            }
            ',' if depth == 0 => {
                // Skip commas between rows
            }
            _ => {
                if depth > 0 {
                    current_row.push(ch);
                }
            }
        }
    }

    if rows.is_empty() {
        return Err(DiagnosticBuilder::error()
            .message("Matrix must have at least one row")
            .build());
    }

    // Validate all rows have same length
    let cols = rows[0].len();
    for (i, row) in rows.iter().enumerate() {
        if row.len() != cols {
            return Err(DiagnosticBuilder::error()
                .message(format!(
                    "Matrix row {} has {} columns, expected {}",
                    i,
                    row.len(),
                    cols
                ))
                .build());
        }
    }

    // Flatten to row-major order
    let mut data = List::new();
    for row in &rows {
        for &val in row {
            data.push(val);
        }
    }

    Ok(ParsedLiteral::Matrix {
        rows: rows.len(),
        cols,
        data,
    })
}

fn parse_row(s: &str) -> Result<List<f64>, Diagnostic> {
    let s = s.trim();
    if s.is_empty() {
        return Err(DiagnosticBuilder::error()
            .message("Matrix row cannot be empty")
            .build());
    }

    let parts: List<&str> = s.split(',').collect();
    let mut row = List::new();

    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        let val = part.parse::<f64>().map_err(|_| {
            DiagnosticBuilder::error()
                .message(format!("Invalid matrix element: '{}'", part))
                .build()
        })?;

        row.push(val);
    }

    if row.is_empty() {
        return Err(DiagnosticBuilder::error()
            .message("Matrix row cannot be empty")
            .build());
    }

    Ok(row)
}
