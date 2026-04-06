//! Safe URL interpolation handler
//!
//! Safe interpolation handlers (compile-time literal protocol):
//! Interpolation handlers registered via @interpolation_handler receive template
//! strings and expression lists, returning injection-safe parameterized output.
//! URL interpolation URL-encodes all interpolated values to prevent injection.
//!
//! Provides URL interpolation that prevents injection attacks by URL-encoding
//! all interpolated values.
//!
//! # Example
//! ```verum
//! let query = "hello world";
//! let url = url"https://api.example.com/search?q={query}";
//! // Desugars to: Url("https://api.example.com/search?q=hello%20world")
//! ```

use verum_ast::{Expr, Span};
use verum_diagnostics::{Diagnostic, DiagnosticBuilder};
use verum_common::Text;

/// URL interpolation handler
///
/// # Security
/// This handler PREVENTS URL injection attacks by:
/// 1. URL-encoding all interpolated values
/// 2. Validating URL structure at compile-time
/// 3. Preventing protocol injection
pub struct UrlInterpolationHandler;

/// Represents a safe URL
#[derive(Debug, Clone, PartialEq)]
pub struct SafeUrl {
    /// The URL string (with encoded interpolations)
    pub url: Text,
    /// The URL scheme (http, https, etc.)
    pub scheme: Text,
    /// Whether this URL contains any interpolations
    pub has_interpolations: bool,
}

/// URL component types for proper encoding
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UrlComponent {
    /// Path segment (e.g., /users/{id})
    Path,
    /// Query parameter value (e.g., ?q={query})
    QueryValue,
    /// Query parameter name
    QueryName,
    /// Fragment (e.g., #{section})
    Fragment,
}

impl UrlInterpolationHandler {
    /// Handle URL interpolation at compile-time
    ///
    /// Safe interpolation: all interpolated values are URL-encoded before insertion
    /// into the template. The handler validates URL structure at compile-time and
    /// prevents protocol injection by checking scheme allowlists.
    ///
    /// # Arguments
    /// - `template`: The URL template string with {expr} placeholders
    /// - `interpolations`: The expressions to interpolate
    /// - `span`: Source location for error reporting
    ///
    /// # Returns
    /// A SafeUrl with properly encoded content
    ///
    /// # Safety
    /// This function generates SAFE URLs that prevent injection attacks.
    /// All interpolated values are URL-encoded before insertion.
    pub fn handle(
        template: &Text,
        interpolations: &[Expr],
        span: Span,
    ) -> Result<SafeUrl, Diagnostic> {
        let mut result = Text::new();
        let mut chars = template.as_str().chars().peekable();
        let mut param_index = 0;
        let mut has_interpolations = false;
        let mut scheme = Text::new();
        let mut in_scheme = true;
        let mut current_component = UrlComponent::Path;

        while let Some(ch) = chars.next() {
            if ch == '{' {
                // Check if this is an interpolation (not escaped)
                if chars.peek() == Some(&'{') {
                    result.push('{');
                    chars.next();
                } else {
                    // Interpolation
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
                            .message("Unclosed interpolation in URL template")
                            .build());
                    }

                    if param_index >= interpolations.len() {
                        return Err(DiagnosticBuilder::error()
                            .message(format!(
                                "Too few interpolation expressions: expected at least {}",
                                param_index + 1
                            ))
                            .build());
                    }

                    // Insert a placeholder that will be encoded at runtime
                    // The encoding type depends on where we are in the URL
                    result.push_str(&format!(
                        "{{__url_encoded_{}_{}__}}",
                        param_index,
                        match current_component {
                            UrlComponent::Path => "path",
                            UrlComponent::QueryValue => "query",
                            UrlComponent::QueryName => "qname",
                            UrlComponent::Fragment => "frag",
                        }
                    ));
                    param_index += 1;
                    has_interpolations = true;
                }
            } else if ch == '}' {
                if chars.peek() == Some(&'}') {
                    result.push('}');
                    chars.next();
                } else {
                    return Err(DiagnosticBuilder::error()
                        .message("Unexpected '}' in URL template (use '}}' for literal brace)")
                        .build());
                }
            } else {
                // Track URL component for proper encoding
                if in_scheme && ch == ':' {
                    in_scheme = false;
                    current_component = UrlComponent::Path;
                } else if in_scheme {
                    scheme.push(ch);
                } else if ch == '?' {
                    current_component = UrlComponent::QueryValue;
                } else if ch == '#' {
                    current_component = UrlComponent::Fragment;
                } else if ch == '&'
                    && matches!(
                        current_component,
                        UrlComponent::QueryValue | UrlComponent::QueryName
                    )
                {
                    current_component = UrlComponent::QueryName;
                } else if ch == '=' && current_component == UrlComponent::QueryName {
                    current_component = UrlComponent::QueryValue;
                }

                result.push(ch);
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

        // Validate the URL structure
        Self::validate_url(&template, &scheme, span)?;

        Ok(SafeUrl {
            url: result,
            scheme,
            has_interpolations,
        })
    }

    /// Validate URL structure and security
    fn validate_url(url: &Text, scheme: &Text, _span: Span) -> Result<(), Diagnostic> {
        let scheme_lower = scheme.as_str().to_lowercase();

        // Check for allowed schemes
        let allowed_schemes = ["http", "https", "ftp", "ftps", "mailto", "tel", "data"];
        if !scheme.is_empty() && !allowed_schemes.contains(&scheme_lower.as_str()) {
            return Err(DiagnosticBuilder::error()
                .message(format!(
                    "URL scheme '{}' is not allowed. Allowed schemes: {}",
                    scheme.as_str(),
                    allowed_schemes.join(", ")
                ))
                .build());
        }

        // Check for javascript: scheme (XSS vector)
        if scheme_lower == "javascript" {
            return Err(DiagnosticBuilder::error()
                .message("javascript: URLs are forbidden for security reasons")
                .build());
        }

        // Check for data: URLs with script content
        if scheme_lower == "data" && url.as_str().to_lowercase().contains("text/html") {
            return Err(DiagnosticBuilder::error()
                .message("data: URLs with HTML content are forbidden for security reasons")
                .help("Use data: URLs only for images or other non-executable content")
                .build());
        }

        Ok(())
    }

    /// URL-encode a string for safe inclusion in a URL path
    ///
    /// # Security
    /// This function encodes all characters that have special meaning in URLs.
    pub fn encode_path(input: &str) -> Text {
        let mut result = Text::new();
        for ch in input.chars() {
            if Self::is_unreserved(ch) {
                result.push(ch);
            } else {
                // Percent-encode the character
                for byte in ch.to_string().as_bytes() {
                    result.push_str(&format!("%{:02X}", byte));
                }
            }
        }
        result
    }

    /// URL-encode a string for safe inclusion in a query parameter
    ///
    /// # Security
    /// This function encodes all characters that have special meaning in query strings.
    pub fn encode_query(input: &str) -> Text {
        let mut result = Text::new();
        for ch in input.chars() {
            if Self::is_unreserved(ch) {
                result.push(ch);
            } else if ch == ' ' {
                result.push('+');
            } else {
                for byte in ch.to_string().as_bytes() {
                    result.push_str(&format!("%{:02X}", byte));
                }
            }
        }
        result
    }

    /// URL-encode a string for safe inclusion in a fragment
    pub fn encode_fragment(input: &str) -> Text {
        Self::encode_path(input)
    }

    /// Check if a character is unreserved per RFC 3986
    fn is_unreserved(ch: char) -> bool {
        ch.is_ascii_alphanumeric() || ch == '-' || ch == '.' || ch == '_' || ch == '~'
    }

    /// Validate that interpolations don't occur in the scheme
    ///
    /// # Security
    /// Prevents protocol injection attacks.
    pub fn validate_no_scheme_interpolation(
        template: &Text,
        _span: Span,
    ) -> Result<(), Diagnostic> {
        let s = template.as_str();

        // Find the scheme part (before ://)
        if let Some(scheme_end) = s.find("://") {
            let scheme_part = &s[..scheme_end];
            if scheme_part.contains('{') {
                return Err(DiagnosticBuilder::error()
                    .message("Interpolation in URL scheme is forbidden for security reasons")
                    .help("The URL scheme (http, https, etc.) must be a constant value")
                    .build());
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_encode_path() {
        assert_eq!(
            UrlInterpolationHandler::encode_path("hello world").as_str(),
            "hello%20world"
        );
    }

    #[test]
    fn test_url_encode_path_special_chars() {
        assert_eq!(
            UrlInterpolationHandler::encode_path("a/b?c=d&e").as_str(),
            "a%2Fb%3Fc%3Dd%26e"
        );
    }

    #[test]
    fn test_url_encode_query() {
        assert_eq!(
            UrlInterpolationHandler::encode_query("hello world").as_str(),
            "hello+world"
        );
    }

    #[test]
    fn test_url_encode_unicode() {
        let encoded = UrlInterpolationHandler::encode_path("日本語");
        assert!(encoded.as_str().contains('%'));
    }

    #[test]
    fn test_simple_url() {
        let template = Text::from("https://example.com/api");
        let result = UrlInterpolationHandler::handle(&template, &[], Span::default());
        assert!(result.is_ok());
        let url = result.unwrap();
        assert_eq!(url.scheme.as_str(), "https");
        assert!(!url.has_interpolations);
    }

    #[test]
    fn test_url_with_interpolation() {
        use verum_ast::expr::{Expr, ExprKind};
        use verum_ast::ty::{Ident, Path};

        let template = Text::from("https://example.com/{path}");
        let path_expr = Expr::new(
            ExprKind::Path(Path::single(Ident::new("path", Span::default()))),
            Span::default(),
        );
        let result = UrlInterpolationHandler::handle(&template, &[path_expr], Span::default());
        assert!(result.is_ok());
        let url = result.unwrap();
        assert!(url.has_interpolations);
    }

    #[test]
    fn test_javascript_url_forbidden() {
        let template = Text::from("javascript:alert(1)");
        let result = UrlInterpolationHandler::handle(&template, &[], Span::default());
        assert!(result.is_err());
    }

    #[test]
    fn test_data_html_url_forbidden() {
        let template = Text::from("data:text/html,<script>alert(1)</script>");
        let result = UrlInterpolationHandler::handle(&template, &[], Span::default());
        assert!(result.is_err());
    }

    #[test]
    fn test_scheme_interpolation_forbidden() {
        let template = Text::from("{scheme}://example.com");
        let result =
            UrlInterpolationHandler::validate_no_scheme_interpolation(&template, Span::default());
        assert!(result.is_err());
    }

    #[test]
    fn test_unreserved_chars() {
        assert!(UrlInterpolationHandler::is_unreserved('a'));
        assert!(UrlInterpolationHandler::is_unreserved('Z'));
        assert!(UrlInterpolationHandler::is_unreserved('0'));
        assert!(UrlInterpolationHandler::is_unreserved('-'));
        assert!(UrlInterpolationHandler::is_unreserved('.'));
        assert!(UrlInterpolationHandler::is_unreserved('_'));
        assert!(UrlInterpolationHandler::is_unreserved('~'));
        assert!(!UrlInterpolationHandler::is_unreserved('/'));
        assert!(!UrlInterpolationHandler::is_unreserved('?'));
        assert!(!UrlInterpolationHandler::is_unreserved(' '));
    }
}
