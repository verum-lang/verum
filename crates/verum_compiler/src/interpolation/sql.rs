//! Safe SQL interpolation handler
//!
//! Safe interpolation handler: receives template strings and expression lists,
//! returns injection-safe parameterized output at compile-time.
//!
//! Provides SQL interpolation that prevents injection attacks by using
//! parameterized queries.
//!
//! # Example
//! ```verum
//! let user_id = 42;
//! let query = sql"SELECT * FROM users WHERE id = {user_id}";
//! // Desugars to: SqlQuery.with_params("SELECT * FROM users WHERE id = ?", vec![user_id])
//! ```

use verum_ast::{Expr, Span};
use verum_diagnostics::{Diagnostic, DiagnosticBuilder};
use verum_common::{List, Text};

/// SQL interpolation handler
///
/// # Security
/// This handler PREVENTS SQL injection by:
/// 1. Replacing all interpolations with placeholders (?)
/// 2. Collecting interpolated values as parameters
/// 3. Generating parameterized query code
pub struct SqlInterpolationHandler;

/// Represents a safe SQL query with parameters
#[derive(Debug, Clone, PartialEq)]
pub struct SqlQuery {
    /// The SQL template with placeholders
    pub template: Text,
    /// The parameters to bind
    pub params: List<Text>,
}

impl SqlInterpolationHandler {
    /// Handle SQL interpolation at compile-time
    ///
    /// Safe interpolation: `sql"SELECT * FROM users WHERE id = {user_id}"` converts
    /// {expr} placeholders to parameterized $1, $2, ... to prevent SQL injection.
    /// Desugars to SqlQuery.with_params(template, [args...]). Never uses string concat.
    ///
    /// # Arguments
    /// - `template`: The SQL template string with {expr} placeholders
    /// - `interpolations`: The expressions to interpolate
    /// - `span`: Source location for error reporting
    ///
    /// # Returns
    /// A SqlQuery with parameterized template and parameters
    ///
    /// # Safety
    /// This function generates SAFE parameterized queries that prevent SQL injection.
    /// All user input is treated as parameters, never as SQL code.
    pub fn handle(
        template: &Text,
        interpolations: &[Expr],
        _span: Span,
    ) -> Result<SqlQuery, Diagnostic> {
        let mut safe_template = Text::new();
        let mut params = List::new();
        let mut chars = template.as_str().chars().peekable();
        let mut param_index = 0;

        while let Some(ch) = chars.next() {
            if ch == '{' {
                // Check if this is an interpolation (not escaped)
                if chars.peek() == Some(&'{') {
                    // Escaped brace: {{
                    safe_template.push('{');
                    chars.next(); // Skip second {
                } else {
                    // Interpolation: replace with placeholder
                    // Find the closing }
                    let mut expr_content = Text::new();
                    let mut found_close = false;
                    while let Some(c) = chars.next() {
                        if c == '}' {
                            found_close = true;
                            break;
                        }
                        expr_content.push(c);
                    }

                    if !found_close {
                        return Err(DiagnosticBuilder::error()
                            .message("Unclosed interpolation in SQL template")
                            .build());
                    }

                    // Add placeholder
                    safe_template.push('?');

                    // Record parameter
                    if param_index < interpolations.len() {
                        // Store expression as text (will be evaluated at runtime)
                        params.push(Text::from(format!("param_{}", param_index)));
                        param_index += 1;
                    } else {
                        return Err(DiagnosticBuilder::error()
                            .message(format!(
                                "Too few interpolation expressions: expected at least {}",
                                param_index + 1
                            ))
                            .build());
                    }
                }
            } else if ch == '}' {
                // Check if this is an escaped brace
                if chars.peek() == Some(&'}') {
                    safe_template.push('}');
                    chars.next(); // Skip second }
                } else {
                    return Err(DiagnosticBuilder::error()
                        .message("Unexpected '}' in SQL template (use '}}' for literal brace)")
                        .build());
                }
            } else {
                safe_template.push(ch);
            }
        }

        if param_index != interpolations.len() {
            return Err(DiagnosticBuilder::error()
                .message(format!(
                    "Wrong number of interpolation expressions: expected {}, got {}",
                    param_index,
                    interpolations.len()
                ))
                .build());
        }

        Ok(SqlQuery {
            template: Text::from(safe_template),
            params,
        })
    }

    /// Validate that a SQL template doesn't contain dangerous patterns
    ///
    /// # Security
    /// This is an additional layer of defense that catches common SQL injection patterns
    /// even before parameter substitution.
    pub fn validate_template(template: &Text, _span: Span) -> Result<(), Diagnostic> {
        let s = template.as_str();

        // Check for common SQL injection patterns in the template itself
        // (not in interpolations, which are safe)
        let dangerous_patterns = vec![
            "--",       // SQL comment
            "/*",       // Block comment start
            ";",        // Statement separator (might be legitimate in some cases)
            "EXEC",     // Execute command
            "EXECUTE",  // Execute command
            "DROP",     // Drop statement (if not in quotes)
            "TRUNCATE", // Truncate statement
            "ALTER",    // Alter statement
        ];

        for pattern in dangerous_patterns {
            if s.to_uppercase().contains(pattern) {
                // This is a warning, not an error - some patterns might be legitimate
                // The actual safety comes from parameterization
                tracing::warn!(
                    "SQL template contains potentially dangerous pattern '{}': {}",
                    pattern,
                    s
                );
            }
        }

        Ok(())
    }
}
