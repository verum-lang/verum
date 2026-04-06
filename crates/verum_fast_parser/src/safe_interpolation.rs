//! Safe interpolated string parsing for Verum
//!
//! This module implements parsing for safe interpolated strings with semantic tags.
//! These strings provide compile-time safety for different domains:
//! - `sql"{user_id}"` - SQL-safe parameterization
//! - `html"{content}"` - HTML auto-escaping
//! - `json"{data}"` - JSON-safe encoding
//! - `url"{path}"` - URL encoding
//! - `gql"{id}"` - GraphQL safe queries
//!
//! Grammar: interpolated_string = identifier , '"' , { string_char | interpolation } , '"' ;
//!          interpolation = '{' , expression , '}' ;
//! Tags: sql (SQL-safe params), html (auto-escaping), json (encoding),
//!        url (URL encoding), gql (GraphQL safe). Format: f"..." for general.

use crate::error::ParseError;
use crate::parser::{ParseResult, RecursiveParser};
use verum_ast::{Expr, ExprKind, Span};
use verum_common::{List, Text};

/// Parse the content of an interpolated string into parts and expressions.
///
/// Given a string like "Hello {name}, you are {age} years old",
/// returns:
/// - parts: ["Hello ", ", you are ", " years old"]
/// - exprs: [name_expr, age_expr]
///
/// The parts array always has one more element than exprs:
/// parts[0] {exprs[0]} parts[1] {exprs[1]} ... parts[n]
pub fn parse_interpolated_content(
    parser: &mut RecursiveParser,
    content: &str,
    file_id: verum_ast::FileId,
) -> ParseResult<(Vec<Text>, Vec<Expr>)> {
    let mut parts = Vec::new();
    let mut exprs = Vec::new();

    let mut current_part = String::new();
    let mut chars = content.chars().peekable();
    let mut pos = 0; // Track position for error reporting

    while let Some(ch) = chars.next() {
        pos += 1;

        match ch {
            '{' => {
                // Check for escaped brace {{ -> {
                if chars.peek() == Some(&'{') {
                    chars.next();
                    pos += 1;
                    current_part.push('{');
                    continue;
                }

                // Start of interpolation
                // Save the current part
                parts.push(Text::from(current_part.as_str()));
                current_part.clear();

                // Extract the expression between { and }
                let mut expr_str = String::new();
                let mut brace_depth = 1;
                let expr_start_pos = pos;

                for inner_ch in chars.by_ref() {
                    pos += 1;

                    match inner_ch {
                        '{' => {
                            brace_depth += 1;
                            expr_str.push(inner_ch);
                        }
                        '}' => {
                            brace_depth -= 1;
                            if brace_depth == 0 {
                                break;
                            }
                            expr_str.push(inner_ch);
                        }
                        _ => {
                            expr_str.push(inner_ch);
                        }
                    }
                }

                if brace_depth != 0 {
                    return Err(ParseError::invalid_interpolation(
                        "unclosed interpolation brace in string",
                        Span::new(expr_start_pos as u32, pos as u32, file_id),
                    ));
                }

                // Parse the expression
                // Note: We need to unescape the expression string because escape sequences like \"
                // inside interpolations represent actual quotes in the Verum code to be parsed
                let expr =
                    parse_interpolation_expr(parser, expr_str.trim(), file_id, expr_start_pos)?;
                exprs.push(expr);
            }
            '}' => {
                // Check for escaped brace }} -> }
                if chars.peek() == Some(&'}') {
                    chars.next();
                    pos += 1;
                    current_part.push('}');
                } else {
                    // Unmatched closing brace
                    return Err(ParseError::invalid_interpolation(
                        "unmatched closing brace in interpolated string",
                        Span::new(pos as u32, pos as u32, file_id),
                    ));
                }
            }
            _ => {
                current_part.push(ch);
            }
        }
    }

    // Add the final part
    parts.push(Text::from(current_part.as_str()));

    Ok((parts, exprs))
}

/// Parse an expression from a string (used for interpolation content)
///
/// MEMORY SAFETY FIX: This function now properly manages token lifetime
/// without using Box::leak, preventing memory accumulation during parsing.
///
/// Special directives supported:
/// - `@raw expr`: Marks the expression as raw (no escaping) for HTML-safe strings
fn parse_interpolation_expr(
    _parser: &mut RecursiveParser,
    expr_str: &str,
    file_id: verum_ast::FileId,
    start_pos: usize,
) -> ParseResult<Expr> {
    use verum_ast::{Ident, Path};
    use verum_lexer::Lexer;

    // Check for @raw directive
    let (is_raw, actual_expr_str) = if let Some(stripped) = expr_str.strip_prefix("@raw ") {
        (true, stripped.trim())
    } else if let Some(stripped) = expr_str.strip_prefix("@raw\t") {
        (true, stripped.trim())
    } else {
        (false, expr_str)
    };

    // Handle format specifiers: f"{expr:spec}" -> strip ":spec" before parsing
    // A format specifier starts with ':' at the top level (not inside brackets/parens)
    // Examples: f"{x:02}", f"{val:.2f}", f"{name:>20}", f"{:?}"
    let actual_expr_str = {
        let stripped = strip_format_spec(actual_expr_str);
        if stripped.is_empty() && actual_expr_str.contains(':') {
            // Empty expression with format spec (e.g., {:?}) - not valid in Verum
            // Return the original to produce a clear error
            actual_expr_str
        } else {
            stripped
        }
    };

    // Unescape the expression string so that \" becomes " and \\ becomes \, etc.
    // This is necessary because inside interpolated strings, escape sequences like \"
    // represent literal characters in the Verum code to be parsed.
    let unescaped_expr = unescape_interpolation_expr(actual_expr_str);

    // Create a new lexer for this expression
    let lexer = Lexer::new(&unescaped_expr, file_id);

    // Collect tokens (Result types need to be handled)
    let tokens_result: Result<Vec<_>, _> = lexer.collect();
    let tokens = tokens_result.map_err(|e| {
        ParseError::invalid_interpolation(
            format!("lexer error in interpolation: {:?}", e),
            Span::new(
                start_pos as u32,
                (start_pos + unescaped_expr.len()) as u32,
                file_id,
            ),
        )
    })?;

    // CRITICAL FIX: Parse within the scope where tokens are valid
    // instead of leaking memory with Box::leak
    // The tokens Vec lives on the stack and the parser borrows from it
    let expr = {
        let mut temp_parser = RecursiveParser::new(&tokens, file_id);
        temp_parser.parse_expr().map_err(|e| {
            ParseError::invalid_interpolation(
                format!("invalid expression in interpolation: {}", e),
                Span::new(
                    start_pos as u32,
                    (start_pos + unescaped_expr.len()) as u32,
                    file_id,
                ),
            )
        })?
    };
    // tokens is dropped here, no memory leak

    // If @raw directive was used, wrap the expression in a call to `__raw_interpolation`
    // This is a compiler intrinsic that marks the value as not needing escaping
    if is_raw {
        let span = expr.span;
        let ident = Ident::new("__raw_interpolation", span);
        let raw_fn = Expr::new(
            ExprKind::Path(Path::from_ident(ident)),
            span,
        );
        Ok(Expr::new(
            ExprKind::Call {
                func: verum_common::Heap::new(raw_fn),
                type_args: List::new(),
                args: List::from(vec![expr]),
            },
            span,
        ))
    } else {
        Ok(expr)
    }
}

/// Strip format specifier from an interpolation expression.
///
/// Given `x:02` returns `x`, given `val:.2f` returns `val`.
/// Given `name` returns `name` (no specifier).
/// Given `:?` returns `` (empty expression, for debug format).
/// Handles nested brackets/parens/braces to avoid stripping colons inside expressions.
fn strip_format_spec(expr_str: &str) -> &str {
    let mut depth = 0i32;
    for (i, ch) in expr_str.char_indices() {
        match ch {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth -= 1,
            ':' if depth == 0 => {
                // Found a top-level colon - everything before is the expression
                return &expr_str[..i];
            }
            _ => {}
        }
    }
    // No format specifier found
    expr_str
}

/// Unescape an expression string extracted from an interpolation.
///
/// This handles escape sequences that appear in the interpolated string content
/// and converts them to the actual characters they represent for Verum parsing.
///
/// For example: `\"hello\"` becomes `"hello"`, `\\n` becomes `\n`, etc.
fn unescape_interpolation_expr(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('r') => result.push('\r'),
                Some('t') => result.push('\t'),
                Some('\\') => result.push('\\'),
                Some('"') => result.push('"'),
                Some('\'') => result.push('\''),
                Some('{') => result.push('{'),
                Some('}') => result.push('}'),
                Some(other) => {
                    // Unknown escape sequence - preserve as-is
                    result.push('\\');
                    result.push(other);
                }
                None => {
                    // Trailing backslash - preserve it
                    result.push('\\');
                }
            }
        } else {
            result.push(ch);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::RecursiveParser;
    use verum_ast::FileId;
    use verum_lexer::Lexer;

    /// MEMORY SAFETY FIX: Test helper that creates parser without Box::leak
    /// Uses a closure pattern to ensure tokens outlive the parser
    fn with_parser<F, R>(source: &str, f: F) -> R
    where
        F: FnOnce(&mut RecursiveParser) -> R,
    {
        let file_id = FileId::new(0);
        let lexer = Lexer::new(source, file_id);
        let tokens: Vec<_> = lexer.filter_map(Result::ok).collect();
        let mut parser = RecursiveParser::new(&tokens, file_id);
        f(&mut parser)
    }

    #[test]
    fn test_parse_simple_interpolation() {
        let content = "Hello {name}!";
        let file_id = FileId::new(0);

        with_parser("", |parser| {
            let (parts, exprs) =
                parse_interpolated_content(parser, content, file_id).expect("should parse");

            assert_eq!(parts.len(), 2);
            assert_eq!(exprs.len(), 1);
            assert_eq!(parts[0].as_str(), "Hello ");
            assert_eq!(parts[1].as_str(), "!");
        });
    }

    #[test]
    fn test_parse_multiple_interpolations() {
        let content = "x={x}, y={y}, sum={x + y}";
        let file_id = FileId::new(0);

        with_parser("", |parser| {
            let (parts, exprs) =
                parse_interpolated_content(parser, content, file_id).expect("should parse");

            assert_eq!(parts.len(), 4);
            assert_eq!(exprs.len(), 3);
            assert_eq!(parts[0].as_str(), "x=");
            assert_eq!(parts[1].as_str(), ", y=");
            assert_eq!(parts[2].as_str(), ", sum=");
            assert_eq!(parts[3].as_str(), "");
        });
    }

    #[test]
    fn test_parse_escaped_braces() {
        let content = "{{literal}} {var}";
        let file_id = FileId::new(0);

        with_parser("", |parser| {
            let (parts, exprs) =
                parse_interpolated_content(parser, content, file_id).expect("should parse");

            assert_eq!(parts.len(), 2);
            assert_eq!(exprs.len(), 1);
            assert_eq!(parts[0].as_str(), "{literal} ");
            assert_eq!(parts[1].as_str(), "");
        });
    }

    #[test]
    fn test_parse_nested_braces() {
        let content = r#"Data: {data.get("key").unwrap_or("default")}"#;
        let file_id = FileId::new(0);

        with_parser("", |parser| {
            let (parts, exprs) =
                parse_interpolated_content(parser, content, file_id).expect("should parse");

            assert_eq!(parts.len(), 2);
            assert_eq!(exprs.len(), 1);
            assert_eq!(parts[0].as_str(), "Data: ");
            assert_eq!(parts[1].as_str(), "");
        });
    }

    #[test]
    fn test_parse_with_escaped_quotes() {
        // Test that escaped quotes inside interpolations are handled correctly
        let content = r#"Result: {\"hello\" + name}"#;
        let file_id = FileId::new(0);

        with_parser("", |parser| {
            let (parts, exprs) =
                parse_interpolated_content(parser, content, file_id).expect("should parse");

            assert_eq!(parts.len(), 2);
            assert_eq!(exprs.len(), 1);
            assert_eq!(parts[0].as_str(), "Result: ");
            assert_eq!(parts[1].as_str(), "");
        });
    }

    #[test]
    fn test_parse_with_method_and_escaped_string() {
        // Test that method calls with escaped string arguments work
        let content = r#"Numbers: {nums.join(\", \")}"#;
        let file_id = FileId::new(0);

        with_parser("", |parser| {
            let (parts, exprs) =
                parse_interpolated_content(parser, content, file_id).expect("should parse");

            assert_eq!(parts.len(), 2);
            assert_eq!(exprs.len(), 1);
            assert_eq!(parts[0].as_str(), "Numbers: ");
            assert_eq!(parts[1].as_str(), "");
        });
    }

    #[test]
    fn test_parse_empty_interpolation() {
        let content = "No interpolations here";
        let file_id = FileId::new(0);

        with_parser("", |parser| {
            let (parts, exprs) =
                parse_interpolated_content(parser, content, file_id).expect("should parse");

            assert_eq!(parts.len(), 1);
            assert_eq!(exprs.len(), 0);
            assert_eq!(parts[0].as_str(), "No interpolations here");
        });
    }

    #[test]
    fn test_parse_unclosed_brace_error() {
        let content = "Hello {name";
        let file_id = FileId::new(0);

        with_parser("", |parser| {
            let result = parse_interpolated_content(parser, content, file_id);
            assert!(result.is_err());
        });
    }

    #[test]
    fn test_parse_unmatched_closing_brace_error() {
        let content = "Hello }name";
        let file_id = FileId::new(0);

        with_parser("", |parser| {
            let result = parse_interpolated_content(parser, content, file_id);
            assert!(result.is_err());
        });
    }
}
