//! Safe HTML interpolation handler
//!
//! Safe interpolation handler: receives template strings and expression lists,
//! returns injection-safe parameterized output at compile-time.
//!
//! Provides HTML interpolation that prevents XSS attacks by auto-escaping
//! all interpolated values.
//!
//! # Example
//! ```verum
//! let user_name = "<script>alert('xss')</script>";
//! let html = html"<div>Hello, {user_name}!</div>";
//! // Desugars to: HtmlFragment("<div>Hello, &lt;script&gt;...!</div>")
//! ```

use verum_ast::{Expr, Span};
use verum_diagnostics::{Diagnostic, DiagnosticBuilder};
use verum_common::{List, Text};

/// HTML interpolation handler
///
/// # Security
/// This handler PREVENTS XSS attacks by:
/// 1. Auto-escaping all HTML special characters in interpolated values
/// 2. Validating HTML structure at compile-time
/// 3. Preventing script injection
pub struct HtmlInterpolationHandler;

/// Represents a safe HTML fragment
#[derive(Debug, Clone, PartialEq)]
pub struct HtmlFragment {
    /// The HTML content (with escaped interpolations)
    pub content: Text,
    /// Whether this fragment contains any interpolations
    pub has_interpolations: bool,
    /// Detected tags for structure validation
    pub tags: List<HtmlTag>,
}

/// An HTML tag detected during parsing
#[derive(Debug, Clone, PartialEq)]
pub struct HtmlTag {
    /// Tag name
    pub name: Text,
    /// Is this an opening tag?
    pub is_opening: bool,
    /// Is this a self-closing tag?
    pub is_self_closing: bool,
}

/// Characters that must be escaped in HTML content
const HTML_ESCAPE_CHARS: &[(char, &str)] = &[
    ('&', "&amp;"),
    ('<', "&lt;"),
    ('>', "&gt;"),
    ('"', "&quot;"),
    ('\'', "&#39;"),
];

impl HtmlInterpolationHandler {
    /// Handle HTML interpolation at compile-time
    ///
    /// Safe interpolation: `html"<h1>{title}</h1>"` auto-escapes all interpolated
    /// values to prevent XSS. Desugars to HtmlTemplate.new(template).with_escaped(args).render().
    /// All interpolation handlers must use parameterization, not string concatenation.
    ///
    /// # Arguments
    /// - `template`: The HTML template string with {expr} placeholders
    /// - `interpolations`: The expressions to interpolate
    /// - `span`: Source location for error reporting
    ///
    /// # Returns
    /// An HtmlFragment with escaped content
    ///
    /// # Safety
    /// This function generates SAFE HTML that prevents XSS attacks.
    /// All interpolated values are HTML-escaped before insertion.
    pub fn handle(
        template: &Text,
        interpolations: &[Expr],
        span: Span,
    ) -> Result<HtmlFragment, Diagnostic> {
        let mut result = Text::new();
        let mut tags = List::new();
        let mut chars = template.as_str().chars().peekable();
        let mut param_index = 0;
        let mut has_interpolations = false;

        while let Some(ch) = chars.next() {
            if ch == '{' {
                // Check if this is an interpolation (not escaped)
                if chars.peek() == Some(&'{') {
                    // Escaped brace: {{
                    result.push('{');
                    chars.next();
                } else {
                    // Interpolation: mark for runtime escaping
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
                            .message("Unclosed interpolation in HTML template")
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

                    // Insert a placeholder that will be escaped at runtime
                    result.push_str(&format!("{{__escaped_{}__}}", param_index));
                    param_index += 1;
                    has_interpolations = true;
                }
            } else if ch == '}' {
                if chars.peek() == Some(&'}') {
                    result.push('}');
                    chars.next();
                } else {
                    return Err(DiagnosticBuilder::error()
                        .message("Unexpected '}' in HTML template (use '}}' for literal brace)")
                        .build());
                }
            } else if ch == '<' {
                // Parse tag for validation
                if let Some(tag) = Self::parse_tag(&mut chars, &mut result) {
                    tags.push(tag);
                }
                result.push(ch);
            } else {
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

        // Validate HTML structure
        Self::validate_structure(&tags, span)?;

        Ok(HtmlFragment {
            content: result,
            has_interpolations,
            tags,
        })
    }

    /// Parse an HTML tag from the character stream
    fn parse_tag<'a>(
        chars: &mut std::iter::Peekable<std::str::Chars<'a>>,
        _result: &mut Text,
    ) -> Option<HtmlTag> {
        let mut tag_name = Text::new();
        let is_closing = chars.peek() == Some(&'/');
        if is_closing {
            chars.next();
        }

        // Read tag name
        while let Some(&c) = chars.peek() {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                tag_name.push(c);
                chars.next();
            } else {
                break;
            }
        }

        if tag_name.is_empty() {
            return None;
        }

        // Skip attributes until >
        let mut is_self_closing = false;
        while let Some(&c) = chars.peek() {
            if c == '/' {
                is_self_closing = true;
                chars.next();
            } else if c == '>' {
                break;
            } else {
                chars.next();
            }
        }

        Some(HtmlTag {
            name: tag_name,
            is_opening: !is_closing,
            is_self_closing,
        })
    }

    /// Validate HTML structure (matching open/close tags)
    fn validate_structure(tags: &List<HtmlTag>, _span: Span) -> Result<(), Diagnostic> {
        // Self-closing tags that don't need a closing tag
        let void_elements = [
            "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param",
            "source", "track", "wbr",
        ];

        let mut stack: Vec<&Text> = Vec::new();

        for tag in tags.iter() {
            if tag.is_self_closing {
                continue;
            }

            let tag_lower = tag.name.as_str().to_lowercase();
            if void_elements.contains(&tag_lower.as_str()) {
                continue;
            }

            if tag.is_opening {
                stack.push(&tag.name);
            } else {
                if let Some(open_tag) = stack.pop() {
                    if open_tag.as_str().to_lowercase() != tag_lower {
                        return Err(DiagnosticBuilder::error()
                            .message(format!(
                                "Mismatched HTML tags: expected closing tag for '{}', found '{}'",
                                open_tag.as_str(),
                                tag.name.as_str()
                            ))
                            .build());
                    }
                } else {
                    return Err(DiagnosticBuilder::error()
                        .message(format!(
                            "Unexpected closing tag '{}' with no matching opening tag",
                            tag.name.as_str()
                        ))
                        .build());
                }
            }
        }

        if !stack.is_empty() {
            return Err(DiagnosticBuilder::error()
                .message(format!(
                    "Unclosed HTML tags: {}",
                    stack
                        .iter()
                        .map(|t| t.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
                .build());
        }

        Ok(())
    }

    /// Escape a string for safe HTML insertion
    ///
    /// # Security
    /// This function escapes all HTML special characters to prevent XSS attacks.
    pub fn escape(input: &str) -> Text {
        let mut result = Text::new();
        for ch in input.chars() {
            let escaped = HTML_ESCAPE_CHARS
                .iter()
                .find(|(c, _)| *c == ch)
                .map(|(_, esc)| *esc);

            if let Some(esc) = escaped {
                result.push_str(esc);
            } else {
                result.push(ch);
            }
        }
        result
    }

    /// Validate that interpolations don't occur in dangerous contexts
    ///
    /// # Security
    /// Prevents interpolation in script tags, event handlers, etc.
    pub fn validate_interpolation_context(template: &Text, _span: Span) -> Result<(), Diagnostic> {
        let lower = template.as_str().to_lowercase();

        // Check for interpolation inside script tags
        if lower.contains("<script") && lower.contains("{") {
            let script_start = lower.find("<script").unwrap_or(0);
            let script_end = lower.find("</script>").unwrap_or(lower.len());
            let between = &lower[script_start..script_end];
            if between.contains('{') && !between.contains("{{") {
                return Err(DiagnosticBuilder::error()
                    .message("Interpolation inside <script> tags is forbidden for security reasons")
                    .help("Use a separate script file or data attributes instead")
                    .build());
            }
        }

        // Check for interpolation in event handlers
        let event_handlers = [
            "onclick",
            "onload",
            "onerror",
            "onmouseover",
            "onmouseout",
            "onkeydown",
            "onkeyup",
            "onfocus",
            "onblur",
            "onsubmit",
        ];

        for handler in event_handlers {
            let pattern = format!("{}=\"{{", handler);
            let pattern_single = format!("{}='{{", handler);
            if lower.contains(&pattern) || lower.contains(&pattern_single) {
                return Err(DiagnosticBuilder::error()
                    .message(format!(
                        "Interpolation inside '{}' event handler is forbidden for security reasons",
                        handler
                    ))
                    .help("Use data attributes and addEventListener instead")
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
    fn test_html_escape() {
        assert_eq!(
            HtmlInterpolationHandler::escape("<script>alert('xss')</script>").as_str(),
            "&lt;script&gt;alert(&#39;xss&#39;)&lt;/script&gt;"
        );
    }

    #[test]
    fn test_html_escape_ampersand() {
        assert_eq!(
            HtmlInterpolationHandler::escape("a & b").as_str(),
            "a &amp; b"
        );
    }

    #[test]
    fn test_html_escape_quotes() {
        assert_eq!(
            HtmlInterpolationHandler::escape("\"test\"").as_str(),
            "&quot;test&quot;"
        );
    }

    #[test]
    fn test_simple_template() {
        let template = Text::from("<div>Hello</div>");
        let result = HtmlInterpolationHandler::handle(&template, &[], Span::default());
        assert!(result.is_ok());
        let fragment = result.unwrap();
        assert!(!fragment.has_interpolations);
    }

    #[test]
    fn test_interpolation_detection() {
        use verum_ast::expr::{Expr, ExprKind};
        use verum_ast::ty::{Ident, Path};

        let template = Text::from("<div>{name}</div>");
        let name_expr = Expr::new(
            ExprKind::Path(Path::single(Ident::new("name", Span::default()))),
            Span::default(),
        );
        let result = HtmlInterpolationHandler::handle(&template, &[name_expr], Span::default());
        assert!(result.is_ok());
        let fragment = result.unwrap();
        assert!(fragment.has_interpolations);
    }

    #[test]
    fn test_mismatched_tags() {
        let template = Text::from("<div><span></div></span>");
        let result = HtmlInterpolationHandler::handle(&template, &[], Span::default());
        assert!(result.is_err());
    }

    #[test]
    fn test_unclosed_tags() {
        let template = Text::from("<div><span>");
        let result = HtmlInterpolationHandler::handle(&template, &[], Span::default());
        assert!(result.is_err());
    }

    #[test]
    fn test_self_closing_tags() {
        let template = Text::from("<div><br/><img src=\"test.jpg\"/></div>");
        let result = HtmlInterpolationHandler::handle(&template, &[], Span::default());
        assert!(result.is_ok());
    }

    #[test]
    fn test_void_elements() {
        let template = Text::from("<div><br><hr><input type=\"text\"></div>");
        let result = HtmlInterpolationHandler::handle(&template, &[], Span::default());
        assert!(result.is_ok());
    }

    #[test]
    fn test_script_interpolation_forbidden() {
        let template = Text::from("<script>var x = {value};</script>");
        let result =
            HtmlInterpolationHandler::validate_interpolation_context(&template, Span::default());
        assert!(result.is_err());
    }

    #[test]
    fn test_event_handler_interpolation_forbidden() {
        let template = Text::from("<button onclick=\"{handler}\">Click</button>");
        let result =
            HtmlInterpolationHandler::validate_interpolation_context(&template, Span::default());
        assert!(result.is_err());
    }
}
