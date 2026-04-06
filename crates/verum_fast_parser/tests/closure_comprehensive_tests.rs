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
//! Comprehensive closure and lambda tests for Verum parser
//!
//! Tests cover:
//! - Basic closure syntax (|params| body)
//! - Typed closure parameters
//! - Move closures
//! - Closures with return types
//! - Closures with context requirements
//! - Nested closures
//! - Higher-order function calls with closures
//! - Closure as function arguments
//! - Closure coercion patterns

use verum_ast::{Expr, ExprKind, FileId, ItemKind, Module};
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

fn parse_expr(source: &str) -> Expr {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser
        .parse_expr_str(source, file_id)
        .unwrap_or_else(|e| panic!("Failed to parse expr '{}': {:?}", source, e))
}

fn parse_module(source: &str) -> Module {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser
        .parse_module(lexer, file_id)
        .unwrap_or_else(|e| panic!("Failed to parse module: {:?}", e))
}

fn assert_parses(source: &str) {
    parse_module(source);
}

// ============================================================================
// BASIC CLOSURE SYNTAX
// ============================================================================

#[test]
fn test_closure_no_params() {
    let expr = parse_expr("|| 42");
    match &expr.kind {
        ExprKind::Closure { params, .. } => {
            assert_eq!(params.len(), 0, "Expected zero parameters");
        }
        _ => panic!("Expected Closure expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_closure_single_param() {
    let expr = parse_expr("|x| x + 1");
    match &expr.kind {
        ExprKind::Closure { params, .. } => {
            assert_eq!(params.len(), 1);
        }
        _ => panic!("Expected Closure"),
    }
}

#[test]
fn test_closure_multiple_params() {
    let expr = parse_expr("|x, y, z| x + y + z");
    match &expr.kind {
        ExprKind::Closure { params, .. } => {
            assert_eq!(params.len(), 3);
        }
        _ => panic!("Expected Closure"),
    }
}

#[test]
fn test_closure_with_block_body() {
    let expr = parse_expr("|x| { let y = x * 2; y + 1 }");
    match &expr.kind {
        ExprKind::Closure { params, .. } => {
            assert_eq!(params.len(), 1);
        }
        _ => panic!("Expected Closure"),
    }
}

#[test]
fn test_closure_with_typed_params() {
    let expr = parse_expr("|x: Int, y: Float| x as Float + y");
    match &expr.kind {
        ExprKind::Closure { params, .. } => {
            assert_eq!(params.len(), 2);
            // Params should have type annotations
            assert!(params[0].ty.is_some(), "First param should have type");
            assert!(params[1].ty.is_some(), "Second param should have type");
        }
        _ => panic!("Expected Closure"),
    }
}

#[test]
fn test_closure_with_return_type() {
    let expr = parse_expr("|x: Int| -> Bool { x > 0 }");
    match &expr.kind {
        ExprKind::Closure { return_type, .. } => {
            assert!(return_type.is_some(), "Should have return type annotation");
        }
        _ => panic!("Expected Closure"),
    }
}

// ============================================================================
// MOVE CLOSURES
// ============================================================================

#[test]
fn test_move_closure_basic() {
    let expr = parse_expr("move |x| x + captured");
    match &expr.kind {
        ExprKind::Closure { move_, .. } => {
            assert!(*move_, "Should be a move closure");
        }
        _ => panic!("Expected Closure"),
    }
}

#[test]
fn test_move_closure_no_params() {
    let expr = parse_expr("move || value");
    match &expr.kind {
        ExprKind::Closure { move_, params, .. } => {
            assert!(*move_);
            assert_eq!(params.len(), 0);
        }
        _ => panic!("Expected Closure"),
    }
}

// ============================================================================
// CLOSURES WITH CONTEXT REQUIREMENTS
// ============================================================================

#[test]
fn test_closure_with_using_clause() {
    let expr = parse_expr("|x| using [Logger] -> Unit { Logger.log(x) }");
    match &expr.kind {
        ExprKind::Closure { contexts, .. } => {
            assert_eq!(contexts.len(), 1, "Should have 1 context requirement");
        }
        _ => panic!("Expected Closure"),
    }
}

// ============================================================================
// NESTED CLOSURES
// ============================================================================

#[test]
fn test_nested_closure_simple() {
    let expr = parse_expr("|x| |y| x + y");
    match &expr.kind {
        ExprKind::Closure { body, .. } => {
            assert!(
                matches!(body.kind, ExprKind::Closure { .. }),
                "Body should be a closure, got {:?}",
                body.kind
            );
        }
        _ => panic!("Expected outer Closure"),
    }
}

#[test]
fn test_nested_closure_three_levels() {
    let expr = parse_expr("|x| |y| |z| x + y + z");
    match &expr.kind {
        ExprKind::Closure { body, .. } => match &body.kind {
            ExprKind::Closure { body: inner, .. } => {
                assert!(
                    matches!(inner.kind, ExprKind::Closure { .. }),
                    "Innermost should be closure"
                );
            }
            _ => panic!("Second level should be closure"),
        },
        _ => panic!("Expected outer Closure"),
    }
}

// ============================================================================
// HIGHER-ORDER FUNCTION CALLS WITH CLOSURES
// ============================================================================

#[test]
fn test_closure_as_function_arg() {
    let source = r#"
        fn main() {
            let doubled = list.map(|x| x * 2);
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_closure_in_filter() {
    let source = r#"
        fn main() {
            let evens = numbers.filter(|x| x % 2 == 0);
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_closure_in_fold() {
    let source = r#"
        fn main() {
            let sum = numbers.fold(0, |acc, x| acc + x);
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_chained_higher_order_calls() {
    let source = r#"
        fn main() {
            let result = items
                .filter(|x| x.is_valid())
                .map(|x| x.value)
                .fold(0, |acc, v| acc + v);
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_closure_in_sort_by() {
    let source = r#"
        fn main() {
            let sorted = items.sort_by(|a, b| a.name.compare(b.name));
        }
    "#;
    assert_parses(source);
}

// ============================================================================
// CLOSURES AS RETURN VALUES
// ============================================================================

#[test]
fn test_function_returning_closure() {
    let source = r#"
        fn make_adder(n: Int) -> fn(Int) -> Int {
            |x| x + n
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_function_returning_closure_with_block() {
    let source = r#"
        fn make_multiplier(factor: Int) -> fn(Int) -> Int {
            move |x| {
                let result = x * factor;
                result
            }
        }
    "#;
    assert_parses(source);
}

// ============================================================================
// CLOSURES IN COMPLEX EXPRESSIONS
// ============================================================================

#[test]
fn test_closure_in_let_binding() {
    let source = r#"
        fn main() {
            let f = |x: Int| -> Int { x * x };
            let result = f(5);
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_closure_immediately_invoked() {
    let source = r#"
        fn main() {
            let result = (|x| x + 1)(41);
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_closure_in_match_arm() {
    let source = r#"
        fn main() {
            let handler = match mode {
                Mode.Fast => |x| x * 2,
                Mode.Safe => |x| validate(x),
            };
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_closure_with_pattern_param() {
    let source = r#"
        fn main() {
            let extract = |(x, y)| x + y;
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_closure_capturing_multiple_vars() {
    let source = r#"
        fn main() {
            let a = 1;
            let b = 2;
            let f = move |x| a + b + x;
        }
    "#;
    assert_parses(source);
}
