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
//! Uses user-defined macro names (not Rust built-ins, which are correctly rejected
//! by the parser with "did you mean?" suggestions).

use verum_ast::{Expr, ExprKind, FileId};
use verum_lexer::Lexer;
use verum_parser::VerumParser;

fn parse_expr(input: &str) -> Expr {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser
        .parse_expr_str(input, file_id)
        .expect("Failed to parse expression")
}

#[test]
fn test_macro_call_with_parens() {
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
    let expr = parse_expr("check_macro!{ x > 0 }");

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
    let expr = parse_expr("debug_custom!()");

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
    let expr = parse_expr("std.my_macro!(\"test\")");

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

    assert!(!program.items.is_empty());
}

#[test]
fn test_macro_call_nested_delimiters() {
    let expr = parse_expr("check_eq!(make_list![1, 2], make_list![1, 2])");

    match expr.kind {
        ExprKind::MacroCall { path, args } => {
            assert_eq!(path.segments.len(), 1);
            assert_eq!(args.delimiter, verum_ast::expr::MacroDelimiter::Paren);
            assert!(!args.tokens.is_empty());
        }
        _ => panic!("Expected MacroCall, got {:?}", expr.kind),
    }
}

#[test]
fn test_macro_call_with_format_string() {
    let expr = parse_expr(r#"fmt_macro!("Value: {}", x)"#);

    match expr.kind {
        ExprKind::MacroCall { path, .. } => {
            assert_eq!(path.segments.len(), 1);
        }
        _ => panic!("Expected MacroCall, got {:?}", expr.kind),
    }
}

#[test]
fn test_bang_not_macro() {
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
    let expr = parse_expr("x != y");

    match expr.kind {
        ExprKind::Binary { .. } => {
            // Expected: binary != operator
        }
        _ => panic!("Expected Binary (!=), got {:?}", expr.kind),
    }
}

// Tests that Rust macros are properly rejected with helpful messages
#[test]
fn test_rust_println_macro_rejected() {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    let result = parser.parse_expr_str("println!(\"Hello\")", file_id);
    assert!(result.is_err(), "Rust println! macro should be rejected");
}

#[test]
fn test_rust_vec_macro_rejected() {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    let result = parser.parse_expr_str("vec![1, 2, 3]", file_id);
    assert!(result.is_err(), "Rust vec! macro should be rejected");
}
