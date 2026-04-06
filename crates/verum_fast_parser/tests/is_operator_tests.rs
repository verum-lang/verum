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
//! Tests for the `is` operator in if/while conditions
//!
//! The `is` operator is used for pattern testing in Verum.
//! Tests for is operator: pattern testing via x is Pattern syntax

use verum_ast::{FileId, Module, Expr, ExprKind};
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

fn parse_module(source: &str) -> Result<Module, String> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser.parse_module(lexer, file_id).map_err(|errors| {
        errors
            .into_iter()
            .map(|e| format!("{:?}", e))
            .collect::<Vec<_>>()
            .join(", ")
    })
}

fn parse_expr(source: &str) -> Result<Expr, String> {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser.parse_expr_str(source, file_id).map_err(|e| format!("{:?}", e))
}

fn assert_parses(source: &str) {
    parse_module(source).unwrap_or_else(|_| panic!("Failed to parse:\n{}", source));
}

#[test]
fn test_basic_is_expression() {
    assert_parses(r#"
fn test() {
    let x = Some(1);
    let b = x is Some(_);
}
"#);
}

#[test]
fn test_is_in_if_condition_no_parens() {
    assert_parses(r#"
fn test() {
    let value = Some(42);
    if value is Some(x) {
        x + 1
    }
}
"#);
}

#[test]
fn test_is_in_if_condition_with_parens() {
    assert_parses(r#"
fn test() {
    let value = Some(42);
    if (value is Some(x)) {
        x + 1
    }
}
"#);
}

#[test]
fn test_is_in_while_condition_no_parens() {
    assert_parses(r#"
fn test() {
    let mut result = Some(10);
    while result is Some(n) {
        result = None;
    }
}
"#);
}

#[test]
fn test_is_in_while_condition_with_parens() {
    assert_parses(r#"
fn test() {
    let mut result = Some(10);
    while (result is Some(n)) {
        result = None;
    }
}
"#);
}

#[test]
fn test_is_with_qualified_path_if() {
    assert_parses(r#"
type Maybe<T> is None | Some(T);
fn test() {
    let value: Maybe<Int> = Maybe.Some(42);
    if value is Maybe.Some(x) {
        x
    }
}
"#);
}

#[test]
fn test_is_with_qualified_path_while() {
    assert_parses(r#"
type Maybe<T> is None | Some(T);
fn test() {
    let mut result: Maybe<Int> = Maybe.Some(10);
    while result is Maybe.Some(n) {
        if n <= 0 {
            result = Maybe.None;
        } else {
            result = Maybe.Some(n - 1);
        }
    }
}
"#);
}

#[test]
fn test_is_not_expression_only() {
    // Test expression parsing directly
    let result = parse_expr("value is not None");
    match result {
        Ok(expr) => {
            if let ExprKind::Is { negated, .. } = expr.kind {
                assert!(negated, "is not None should be negated");
            } else {
                panic!("Expected Is expression, got {:?}", expr.kind);
            }
        }
        Err(e) => panic!("Failed to parse 'value is not None': {}", e),
    }
}

#[test]
fn test_is_not_some_in_if() {
    // Minimal test case with Some instead of None
    assert_parses(r#"
fn test() {
    if value is not Some(_) { }
}
"#);
}

#[test]
fn test_is_none_in_if_without_not() {
    // Test is None without negation
    assert_parses(r#"
fn test() {
    if value is None { }
}
"#);
}

#[test]
fn test_is_not_in_simple_if() {
    // Minimal test case
    assert_parses(r#"
fn test() {
    if value is not None { }
}
"#);
}

#[test]
fn test_is_not_in_if_with_parens() {
    // This should work with parentheses
    assert_parses(r#"
fn test() {
    if (value is not None) { }
}
"#);
}

#[test]
fn test_is_not_pattern() {
    assert_parses(r#"
fn test() {
    let value = Some(42);
    if value is not None {
        42
    }
}
"#);
}

/// Test the complete VCS is_operator.vr spec file
#[test]
fn test_vcs_is_operator_spec() {
    // This mirrors the content of vcs/specs/L0-critical/builtin-syntax/is_operator.vr
    assert_parses(r#"
type Maybe<T> is None | Some(T);
type Result<T, E> is Ok(T) | Err(E);

fn test_is_operator() -> Int {
    let value: Maybe<Int> = Maybe.Some(42);

    // Basic pattern test with is
    if value is Maybe.Some(x) {
        assert_eq(x, 42);
    }

    // Negated pattern test with is not
    if value is not Maybe.None {
        print("Value is not None");
    }

    // While loop with is
    let mut result: Maybe<Int> = Maybe.Some(10);
    while result is Maybe.Some(n) {
        if n <= 0 {
            result = Maybe.None;
        } else {
            result = Maybe.Some(n - 1);
        }
    }

    // Guard patterns with is
    let data: Result<Int, Text> = Result.Ok(100);
    let output = match data {
        Result.Ok(x) if x is Int => x * 2,
        Result.Err(e) => 0,
        _ => -1,
    };
    assert_eq(output, 200);

    0
}

fn main() -> Int {
    test_is_operator()
}
"#);
}
