#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
//! Test macro invocation parsing.
//!
//! This test verifies that the parser correctly handles macro invocation syntax:
//! - macro!(args)   (for user-defined macros, NOT Rust built-ins)
//! - macro![args]
//! - macro!{args}
//!
//! Note: Rust built-in macros (println!, vec!, assert!, etc.) are intentionally
//! rejected with helpful error messages suggesting the Verum equivalents.

use verum_ast::{Expr, ExprKind, FileId};
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

fn parse_expr(input: &str) -> Expr {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser
        .parse_expr_str(input, file_id)
        .expect("Failed to parse expression")
}

fn parse_expr_fails(input: &str) -> bool {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser.parse_expr_str(input, file_id).is_err()
}

#[test]
fn test_macro_call_with_parens() {
    // Use a user-defined macro name (not a Rust built-in)
    let expr = parse_expr("my_macro!(\"Hello, world!\")");

    match expr.kind {
        ExprKind::MacroCall { path, args } => {
            assert_eq!(path.segments.len(), 1);
            assert_eq!(args.delimiter, verum_ast::expr::MacroDelimiter::Paren);
        }
        _ => panic!("Expected MacroCall, got {:?}", expr.kind),
    }
}

#[test]
fn test_macro_call_with_brackets() {
    let expr = parse_expr("list_macro![1, 2, 3]");

    match expr.kind {
        ExprKind::MacroCall { path, args } => {
            assert_eq!(path.segments.len(), 1);
            assert_eq!(args.delimiter, verum_ast::expr::MacroDelimiter::Bracket);
        }
        _ => panic!("Expected MacroCall, got {:?}", expr.kind),
    }
}

#[test]
fn test_macro_call_with_braces() {
    let expr = parse_expr("check!{ x > 0 }");

    match expr.kind {
        ExprKind::MacroCall { path, args } => {
            assert_eq!(path.segments.len(), 1);
            assert_eq!(args.delimiter, verum_ast::expr::MacroDelimiter::Brace);
        }
        _ => panic!("Expected MacroCall, got {:?}", expr.kind),
    }
}

#[test]
fn test_macro_call_empty_args() {
    let expr = parse_expr("custom_debug!()");

    match expr.kind {
        ExprKind::MacroCall { path, args } => {
            assert_eq!(path.segments.len(), 1);
            assert_eq!(args.delimiter, verum_ast::expr::MacroDelimiter::Paren);
        }
        _ => panic!("Expected MacroCall, got {:?}", expr.kind),
    }
}

#[test]
fn test_macro_call_qualified_path() {
    let expr = parse_expr("std.my_println!(\"test\")");

    match expr.kind {
        ExprKind::MacroCall { path, .. } => {
            assert!(!path.segments.is_empty());
        }
        _ => panic!("Expected MacroCall, got {:?}", expr.kind),
    }
}

#[test]
fn test_macro_call_in_statement() {
    let input = r#"
fn main() {
    let result = custom_log!("Hello macro!");
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    let parser = VerumParser::new();
    let program = parser
        .parse_module(lexer, file_id)
        .expect("Failed to parse program");

    // The program should parse successfully with the macro call
    assert!(!program.items.is_empty());
}

#[test]
fn test_macro_call_nested_delimiters() {
    let expr = parse_expr("check_eq!(list_of![1, 2], list_of![1, 2])");

    match expr.kind {
        ExprKind::MacroCall { path, args } => {
            assert_eq!(path.segments.len(), 1);
            assert_eq!(args.delimiter, verum_ast::expr::MacroDelimiter::Paren);
            // The nested brackets should be captured in the token tree
            assert!(!args.tokens.is_empty());
        }
        _ => panic!("Expected MacroCall, got {:?}", expr.kind),
    }
}

#[test]
fn test_macro_call_with_format_string() {
    let expr = parse_expr(r#"custom_format!("Value: {}", x)"#);

    match expr.kind {
        ExprKind::MacroCall { path, .. } => {
            assert_eq!(path.segments.len(), 1);
        }
        _ => panic!("Expected MacroCall, got {:?}", expr.kind),
    }
}

// Rust built-in macros should be rejected with helpful errors
#[test]
fn test_rust_println_macro_rejected() {
    assert!(parse_expr_fails("println!(\"Hello\")"));
}

#[test]
fn test_rust_vec_macro_rejected() {
    assert!(parse_expr_fails("vec![1, 2, 3]"));
}

#[test]
fn test_rust_assert_macro_rejected() {
    assert!(parse_expr_fails("assert!{ x > 0 }"));
}

#[test]
fn test_rust_format_macro_rejected() {
    assert!(parse_expr_fails(r#"format!("Value: {}", x)"#));
}

#[test]
fn test_bang_not_macro() {
    // This should parse as unary NOT, not a macro
    let expr = parse_expr("!x");

    match expr.kind {
        ExprKind::Unary { .. } => {
            // Expected: unary NOT operator
        }
        _ => panic!("Expected Unary (NOT), got {:?}", expr.kind),
    }
}

#[test]
fn test_not_equals_not_macro() {
    // This should parse as not-equals, not a macro
    let expr = parse_expr("x != y");

    match expr.kind {
        ExprKind::Binary { .. } => {
            // Expected: binary != operator
        }
        _ => panic!("Expected Binary (!=), got {:?}", expr.kind),
    }
}
