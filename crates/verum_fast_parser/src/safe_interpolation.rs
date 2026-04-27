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

    // Handle format specifiers: f"{expr:spec}" -> wrap expr in stdlib
    // formatter call so the codegen path actually honours the spec.
    //
    // History: this used to silently DROP the format spec (the
    // returned string was just the expression, the spec discarded).
    // Result: f"{v:x}" with v=195 emitted "195" instead of "c3" —
    // discovered while validating task #37, opened as task #38.
    //
    // Strategy: split the expression and the spec here, parse the
    // expression normally, then wrap in `expr.to_<radix>()` (or
    // future `__fmt(expr, "spec")` for richer specs).  Keeps the
    // AST shape unchanged (still a single Expr per interpolation)
    // and reuses the existing stdlib `Int.to_hex / to_octal /
    // to_binary` methods on `core/base/primitives.vr`.
    let (actual_expr_str, format_spec) = split_expr_and_spec(actual_expr_str);
    if actual_expr_str.is_empty() && format_spec.is_some() {
        // Empty expression with format spec (e.g., {:?}) - not valid in Verum
        return Err(ParseError::invalid_interpolation(
            format!("empty expression with format spec `{}`", format_spec.unwrap_or("")),
            Span::new(
                start_pos as u32,
                (start_pos + expr_str.len()) as u32,
                file_id,
            ),
        ));
    }

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

    // Apply format spec by wrapping in a method call (e.g.
    // `{v:x}` → `v.to_hex()`).  We do this BEFORE the @raw wrap so
    // that `@raw {expr:html}` (hypothetical) would honour spec then
    // mark raw — order matches HTML's typical compose pipeline.
    let expr = if let Some(spec) = format_spec {
        wrap_with_format_spec(expr, spec)?
    } else {
        expr
    };

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

/// Wrap an interpolation expression with stdlib formatter calls
/// based on the format spec — Python-style grammar:
///
/// `[[fill]align][sign][#][0][width][.precision][type]`
///
/// Supported specs (compose using existing stdlib methods on
/// `core/base/primitives.vr` and `core/text/text.vr`; no ad-hoc
/// opcodes — every formatter call is a regular method dispatch):
///
///   - `x` → `expr.to_hex()`
///   - `X` → `expr.to_hex().to_uppercase()`  (case via Text.to_uppercase)
///   - `o` → `expr.to_octal()`
///   - `b` → `expr.to_binary()`
///   - `?` / `s` / nothing → `expr.to_string()` (canonical default)
///   - bare width N (digits only): `expr.to_string().pad_left(N, ' ')`
///   - `0N`  zero-padded width: `... .pad_left(N, '0')`
///   - `>N`  right-align (default for numerics): `... .pad_left(N, ' ')`
///   - `<N`  left-align: `... .pad_right(N, ' ')`
///   - `0Nx` zero-padded hex of width N
///   - `Nx`  hex padded with spaces to width N
///
/// Anything not matching falls through to plain ToString — better
/// than rejecting valid programs while a future native FormatValue
/// opcode (richer specs: precision / sign / alternate / fill char /
/// center align) is staged separately.
fn wrap_with_format_spec(expr: Expr, spec: &str) -> ParseResult<Expr> {
    let parsed = parse_format_spec(spec);

    let span = expr.span;
    // Stage 1 — radix/type conversion.
    let converted = match parsed.type_char {
        Some('x') | Some('X') => method_call_no_args(expr, "to_hex", span),
        Some('o') => method_call_no_args(expr, "to_octal", span),
        Some('b') => method_call_no_args(expr, "to_binary", span),
        // 's' / '?' / None all default to the canonical to_string path,
        // which is what the InterpolatedString codegen does anyway via
        // Instruction::ToString.  Skip the redundant method call when
        // there is no width/align spec either, so the common
        // `f"{x}"` and `f"{x:?}"` paths remain a single ToString op.
        _ => {
            if parsed.width == 0 && !parsed.upper {
                return Ok(expr);
            }
            method_call_no_args(expr, "to_string", span)
        }
    };

    // Stage 2 — uppercase if `X` was requested.
    let cased = if parsed.upper {
        method_call_no_args(converted, "to_uppercase", span)
    } else {
        converted
    };

    // Stage 3 — width / alignment / fill.
    if parsed.width > 0 {
        let fill_char = parsed.fill;
        let width_lit = int_literal(parsed.width as i64, span);
        let fill_lit = char_literal(fill_char, span);
        let pad_method = if parsed.left_align {
            "pad_right"
        } else {
            "pad_left"
        };
        let padded = Expr::new(
            ExprKind::MethodCall {
                receiver: verum_common::Heap::new(cased),
                method: verum_ast::Ident::new(pad_method, span),
                type_args: List::new(),
                args: List::from(vec![width_lit, fill_lit]),
            },
            span,
        );
        Ok(padded)
    } else {
        Ok(cased)
    }
}

/// Compact Python-style format-spec descriptor.
struct ParsedSpec {
    /// Fill character used by pad_left / pad_right (default ' ').
    fill: char,
    /// `true` when the spec used `<` alignment (left-pad).
    /// `false` for default (right-pad) and `>`.
    /// Centre-align (`^`) is currently treated as right-pad —
    /// proper centre composition needs a `pad_centre` stdlib method.
    left_align: bool,
    /// Minimum field width (0 = no padding).
    width: u32,
    /// Conversion-type character (`x`/`o`/`b`/`?`/`s`/`X`/None).
    type_char: Option<char>,
    /// True when type was `X` (uppercase hex); drives the
    /// `.to_uppercase()` post-processing step.
    upper: bool,
}

fn parse_format_spec(spec: &str) -> ParsedSpec {
    let mut chars = spec.chars().peekable();

    // Default state.
    let mut fill: char = ' ';
    let mut left_align: bool = false;
    let mut width: u32 = 0;

    // [[fill]align] — fill is a single char immediately followed
    // by an alignment char.  Spec like `*<5` means fill='*',
    // align='<', width=5.  Plain `<5` means fill=' ', align='<',
    // width=5 (no fill specified).
    let snapshot: Vec<char> = chars.clone().collect();
    if snapshot.len() >= 2 && matches!(snapshot[1], '<' | '>' | '^' | '=') {
        fill = snapshot[0];
        left_align = snapshot[1] == '<';
        // consume both
        chars.next();
        chars.next();
    } else if let Some(&first) = chars.peek()
        && matches!(first, '<' | '>' | '^' | '=')
    {
        left_align = first == '<';
        chars.next();
    }

    // [0] — zero-pad shortcut: '0' before width digits.  Sets
    // fill='0' unless an explicit fill was already provided.
    if chars.peek() == Some(&'0') {
        // Peek ahead: only treat as zero-pad if next char is a digit
        let mut probe = chars.clone();
        probe.next();
        if probe.peek().is_some_and(|c| c.is_ascii_digit()) {
            if fill == ' ' {
                fill = '0';
            }
            chars.next(); // consume the '0'
        }
    }

    // [width] — leading digits.
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            width = width * 10 + (c as u32 - '0' as u32);
            chars.next();
        } else {
            break;
        }
    }

    // Skip optional '.precision' for now (not yet wired through;
    // it would compose via a stdlib `format_float_precision(n)`
    // helper which doesn't exist yet — punted).
    if chars.peek() == Some(&'.') {
        chars.next();
        while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
            chars.next();
        }
    }

    // [type]
    let type_char = chars.next();
    let upper = matches!(type_char, Some('X'));

    ParsedSpec {
        fill,
        left_align,
        width,
        type_char,
        upper,
    }
}

fn method_call_no_args(receiver: Expr, name: &str, span: verum_ast::Span) -> Expr {
    Expr::new(
        ExprKind::MethodCall {
            receiver: verum_common::Heap::new(receiver),
            method: verum_ast::Ident::new(name, span),
            type_args: List::new(),
            args: List::new(),
        },
        span,
    )
}

fn int_literal(value: i64, span: verum_ast::Span) -> Expr {
    use verum_ast::literal::{IntLit, Literal, LiteralKind};
    Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Int(IntLit {
                value: value as i128,
                suffix: None,
            }),
            span,
        }),
        span,
    )
}

fn char_literal(value: char, span: verum_ast::Span) -> Expr {
    use verum_ast::literal::{Literal, LiteralKind};
    Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Char(value),
            span,
        }),
        span,
    )
}

/// Split an interpolation expression into (expr_str, format_spec).
///
/// Mirrors the bracket-aware logic of `strip_format_spec` but
/// returns the spec instead of dropping it.
fn split_expr_and_spec(expr_str: &str) -> (&str, Option<&str>) {
    let mut depth = 0i32;
    for (i, ch) in expr_str.char_indices() {
        match ch {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth -= 1,
            ':' if depth == 0 => {
                let spec = &expr_str[i + 1..];
                return (&expr_str[..i], Some(spec));
            }
            _ => {}
        }
    }
    (expr_str, None)
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
