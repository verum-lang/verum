//! Safe interpolation handlers for semantic literals
//!
//! Interpolation system: tagged literal parsing (§1.4.4) and safe interpolation
//! handlers (§1.4.5), all desugaring to meta-system operations.
//!
//! This module provides safe interpolation handlers that prevent injection attacks:
//! - SQL interpolation with parameterized queries (prevents SQL injection)
//! - HTML interpolation with auto-escaping (prevents XSS)
//! - URL interpolation with proper encoding (prevents URL injection)
//!
//! ## Security Guarantees
//!
//! All interpolation handlers follow the principle of "secure by default":
//! - All user input is automatically escaped/encoded
//! - Dangerous contexts (script tags, event handlers) are forbidden
//! - Scheme injection is prevented
//! - Compile-time validation of structure
//!
//! ## Usage
//!
//! ```verum
//! // SQL - parameterized queries
//! let user_id = 42;
//! let query = sql"SELECT * FROM users WHERE id = {user_id}";
//!
//! // HTML - auto-escaping
//! let name = "<script>alert('xss')</script>";
//! let html = html"<div>Hello, {name}!</div>";
//! // Output: <div>Hello, &lt;script&gt;...&lt;/script&gt;!</div>
//!
//! // URL - percent-encoding
//! let search = "hello world";
//! let url = url"https://api.example.com/search?q={search}";
//! // Output: https://api.example.com/search?q=hello%20world
//! ```

pub mod html;
pub mod sql;
pub mod url;

// Re-export handler functions and types
pub use html::{HtmlFragment, HtmlInterpolationHandler, HtmlTag};
pub use sql::{SqlInterpolationHandler, SqlQuery};
pub use url::{SafeUrl, UrlComponent, UrlInterpolationHandler};

use verum_ast::{Expr, Span};
use verum_diagnostics::Diagnostic;
use verum_common::Text;

/// Unified interpolation handler that dispatches to the correct handler
/// based on the tag prefix.
pub struct InterpolationDispatcher;

impl InterpolationDispatcher {
    /// Dispatch interpolation to the appropriate handler
    ///
    /// # Arguments
    /// - `tag`: The interpolation tag (sql, html, url, etc.)
    /// - `template`: The template string
    /// - `interpolations`: The expressions to interpolate
    /// - `span`: Source location for error reporting
    ///
    /// # Returns
    /// The result of the interpolation as a string representation
    pub fn dispatch(
        tag: &str,
        template: &Text,
        interpolations: &[Expr],
        span: Span,
    ) -> Result<InterpolationResult, Diagnostic> {
        match tag.to_lowercase().as_str() {
            "sql" => {
                let query = SqlInterpolationHandler::handle(template, interpolations, span)?;
                Ok(InterpolationResult::Sql(query))
            }
            "html" => {
                // Validate security context first
                HtmlInterpolationHandler::validate_interpolation_context(template, span)?;
                let fragment = HtmlInterpolationHandler::handle(template, interpolations, span)?;
                Ok(InterpolationResult::Html(fragment))
            }
            "url" => {
                // Validate no scheme interpolation
                UrlInterpolationHandler::validate_no_scheme_interpolation(template, span)?;
                let url = UrlInterpolationHandler::handle(template, interpolations, span)?;
                Ok(InterpolationResult::Url(url))
            }
            _ => Err(verum_diagnostics::DiagnosticBuilder::error()
                .message(format!("Unknown interpolation handler: '{}'", tag))
                .help("Available handlers: sql, html, url")
                .build()),
        }
    }
}

/// Result of interpolation processing
#[derive(Debug, Clone)]
pub enum InterpolationResult {
    /// SQL query with parameters
    Sql(SqlQuery),
    /// HTML fragment with escaped content
    Html(HtmlFragment),
    /// URL with encoded components
    Url(SafeUrl),
}

impl InterpolationResult {
    /// Get the resulting string content
    pub fn content(&self) -> &Text {
        match self {
            InterpolationResult::Sql(q) => &q.template,
            InterpolationResult::Html(h) => &h.content,
            InterpolationResult::Url(u) => &u.url,
        }
    }

    /// Check if the result contains interpolations
    pub fn has_interpolations(&self) -> bool {
        match self {
            InterpolationResult::Sql(q) => !q.params.is_empty(),
            InterpolationResult::Html(h) => h.has_interpolations,
            InterpolationResult::Url(u) => u.has_interpolations,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dispatch_sql() {
        let template = Text::from("SELECT * FROM users");
        let result = InterpolationDispatcher::dispatch("sql", &template, &[], Span::default());
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), InterpolationResult::Sql(_)));
    }

    #[test]
    fn test_dispatch_html() {
        let template = Text::from("<div>Hello</div>");
        let result = InterpolationDispatcher::dispatch("html", &template, &[], Span::default());
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), InterpolationResult::Html(_)));
    }

    #[test]
    fn test_dispatch_url() {
        let template = Text::from("https://example.com");
        let result = InterpolationDispatcher::dispatch("url", &template, &[], Span::default());
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), InterpolationResult::Url(_)));
    }

    #[test]
    fn test_dispatch_unknown() {
        let template = Text::from("test");
        let result = InterpolationDispatcher::dispatch("unknown", &template, &[], Span::default());
        assert!(result.is_err());
    }

    #[test]
    fn test_dispatch_case_insensitive() {
        let template = Text::from("SELECT 1");
        let result1 = InterpolationDispatcher::dispatch("SQL", &template, &[], Span::default());
        let result2 = InterpolationDispatcher::dispatch("Sql", &template, &[], Span::default());
        assert!(result1.is_ok());
        assert!(result2.is_ok());
    }
}
