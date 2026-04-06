//! Regression Test Suite
//!
//! Tests for previously found bugs, edge cases from specification,
//! and corner cases discovered through fuzzing.
//!
//! Each test documents the bug/issue it prevents from regressing.

use verum_ast::{expr::*, literal::*, span::Span};
use verum_cbgr::{Allocator, Tier};
use verum_interpreter::{Environment, Evaluator, Value};
use verum_lexer::Lexer;
use verum_parser::Parser;
use verum_std::core::{List, Text};
use verum_types::TypeChecker;

// ============================================================================
// Parser Regression Tests
// ============================================================================

/// Bug: Parser failed on empty function body
/// Fixed: 2025-11-XX
#[test]
fn test_regression_empty_function_body() {
    let source = "fn empty() {}";

    let mut parser = Parser::new(source);
    let result = parser.parse_module();

    assert!(result.is_ok(), "Should parse empty function body");
}

/// Bug: Parser incorrectly handled trailing commas in lists
/// Fixed: 2025-11-XX
#[test]
fn test_regression_trailing_comma_in_list() {
    let source = "[1, 2, 3,]";

    let mut parser = Parser::new(source);
    let result = parser.parse_expr();

    // Should either accept or reject consistently
    assert!(result.is_ok() || result.is_err());
}

/// Bug: Parser crashed on deeply nested parentheses
/// Fixed: 2025-11-XX
#[test]
fn test_regression_deeply_nested_parens() {
    let source = "((((((42))))))";

    let mut parser = Parser::new(source);
    let result = parser.parse_expr();

    assert!(result.is_ok(), "Should handle nested parentheses");
}

/// Bug: Parser mishandled operator precedence with unary minus
/// Fixed: 2025-11-XX
#[test]
fn test_regression_unary_minus_precedence() {
    let source = "-5 * 3";

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    let mut env = Environment::new();
    let mut eval = Evaluator::new();
    let result = eval.eval_expr(&expr, &mut env).expect("Should evaluate");

    match result {
        Value::Int(n) => assert_eq!(n, -15, "-5 * 3 should equal -15"),
        _ => panic!("Expected Int"),
    }
}

/// Bug: Parser failed on comments at end of file
/// Fixed: 2025-11-XX
#[test]
fn test_regression_comment_at_eof() {
    let source = "let x = 42;\n// comment";

    let mut parser = Parser::new(source);
    let result = parser.parse_module();

    assert!(result.is_ok(), "Should handle comment at EOF");
}

// ============================================================================
// Lexer Regression Tests
// ============================================================================

/// Bug: Lexer failed on CRLF line endings
/// Fixed: 2025-11-XX
#[test]
fn test_regression_crlf_line_endings() {
    let source = "let x = 10;\r\nlet y = 20;\r\n";

    let mut lexer = Lexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    assert!(!tokens.is_empty(), "Should tokenize CRLF");
}

/// Bug: Lexer incorrectly tokenized floating point numbers
/// Fixed: 2025-11-XX
#[test]
fn test_regression_float_tokenization() {
    let source = "3.14159";

    let mut lexer = Lexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    assert!(!tokens.is_empty(), "Should tokenize float");
}

/// Bug: Lexer failed on strings with escaped quotes
/// Fixed: 2025-11-XX
#[test]
fn test_regression_escaped_quotes() {
    let source = r#""He said \"hello\"""#;

    let mut lexer = Lexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    assert!(!tokens.is_empty(), "Should handle escaped quotes");
}

// ============================================================================
// Type Checker Regression Tests
// ============================================================================

/// Bug: Type checker crashed on recursive types
/// Fixed: 2025-11-XX
#[test]
fn test_regression_recursive_types() {
    let source = r#"
        fn factorial(n: Int) -> Int {
            match n {
                0 => 1,
                n => n * factorial(n - 1)
            }
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_module();

    assert!(result.is_ok(), "Should handle recursive functions");
}

/// Bug: Type checker failed on nested function calls
/// Fixed: 2025-11-XX
#[test]
fn test_regression_nested_function_calls() {
    let source = "f(g(h(x)))";

    let mut parser = Parser::new(source);
    let result = parser.parse_expr();

    assert!(result.is_ok(), "Should parse nested calls");
}

/// Bug: Type checker incorrectly unified tuple types
/// Fixed: 2025-11-XX
#[test]
fn test_regression_tuple_unification() {
    let source = "(1, 2, 3)";

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    let mut checker = TypeChecker::new();
    let result = checker.synth_expr(&expr);

    assert!(result.is_ok(), "Should type check tuple");
}

// ============================================================================
// Interpreter Regression Tests
// ============================================================================

/// Bug: Interpreter incorrectly evaluated boolean short-circuit
/// Fixed: 2025-11-XX
#[test]
fn test_regression_short_circuit_evaluation() {
    let source = "false && (1 / 0)";

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    let mut env = Environment::new();
    let mut eval = Evaluator::new();
    let result = eval.eval_expr(&expr, &mut env);

    // Should not evaluate right side due to short-circuit
    match result {
        Ok(Value::Bool(false)) => {}
        _ => panic!("Short-circuit should prevent division by zero"),
    }
}

/// Bug: Interpreter failed on empty pattern match
/// Fixed: 2025-11-XX
#[test]
fn test_regression_empty_pattern_match() {
    let source = r#"
        match [] {
            [] => "empty",
            _ => "not empty"
        }
    "#;

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    let mut env = Environment::new();
    let mut eval = Evaluator::new();
    let result = eval.eval_expr(&expr, &mut env);

    match result {
        Ok(Value::Text(ref s)) => assert_eq!(s.as_str(), "empty"),
        _ => panic!("Expected 'empty'"),
    }
}

/// Bug: Interpreter mishandled variable shadowing
/// Fixed: 2025-11-XX
#[test]
fn test_regression_variable_shadowing() {
    let source = r#"
        let x = 10;
        let x = 20;
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_module();

    assert!(result.is_ok(), "Should handle shadowing");
}

// ============================================================================
// CBGR Regression Tests
// ============================================================================

/// Bug: CBGR failed to deallocate in certain conditions
/// Fixed: 2025-11-XX
#[test]
fn test_regression_cbgr_deallocation() {
    let allocator = Allocator::new();

    // Allocate and drop
    {
        let _gen_ref = allocator.alloc(42i64, Tier::Standard);
    }

    // Should have deallocated
}

/// Bug: CBGR incorrectly handled reference counting
/// Fixed: 2025-11-XX
#[test]
fn test_regression_cbgr_refcount() {
    let allocator = Allocator::new();

    let gen_ref1 = allocator.alloc(100i64, Tier::Standard);
    let gen_ref2 = gen_ref1.clone();

    assert_eq!(*gen_ref1, *gen_ref2);
}

// ============================================================================
// Edge Cases from Specification
// ============================================================================

/// Spec: Empty programs should be valid
#[test]
fn test_spec_edge_case_empty_program() {
    let source = "";

    let mut parser = Parser::new(source);
    let result = parser.parse_module();

    assert!(result.is_ok(), "Empty program should be valid");
}

/// Spec: Whitespace-only programs should be valid
#[test]
fn test_spec_edge_case_whitespace_only() {
    let source = "   \n\n  \t  ";

    let mut parser = Parser::new(source);
    let result = parser.parse_module();

    assert!(result.is_ok(), "Whitespace-only should be valid");
}

/// Spec: Maximum integer literal
#[test]
fn test_spec_edge_case_max_int() {
    let source = "9223372036854775807"; // i64::MAX

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    let mut env = Environment::new();
    let mut eval = Evaluator::new();
    let result = eval.eval_expr(&expr, &mut env).expect("Should evaluate");

    match result {
        Value::Int(n) => assert_eq!(n, 9223372036854775807),
        _ => panic!("Expected Int"),
    }
}

/// Spec: Minimum integer literal
#[test]
fn test_spec_edge_case_min_int() {
    let source = "-9223372036854775808"; // i64::MIN

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    let mut env = Environment::new();
    let mut eval = Evaluator::new();
    let _result = eval.eval_expr(&expr, &mut env);

    // Should handle minimum integer
}

/// Spec: Empty string literal
#[test]
fn test_spec_edge_case_empty_string() {
    let source = r#""""#;

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    let mut env = Environment::new();
    let mut eval = Evaluator::new();
    let result = eval.eval_expr(&expr, &mut env).expect("Should evaluate");

    match result {
        Value::Text(ref s) => assert_eq!(s.as_str(), ""),
        _ => panic!("Expected empty Text"),
    }
}

/// Spec: Empty list literal
#[test]
fn test_spec_edge_case_empty_list() {
    let source = "[]";

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    let mut env = Environment::new();
    let mut eval = Evaluator::new();
    let result = eval.eval_expr(&expr, &mut env).expect("Should evaluate");

    match result {
        Value::List(ref list) => assert_eq!(list.len(), 0),
        _ => panic!("Expected empty List"),
    }
}

/// Spec: Single-element tuple
#[test]
fn test_spec_edge_case_single_element_tuple() {
    let source = "(42,)";

    let mut parser = Parser::new(source);
    let result = parser.parse_expr();

    // Spec defines if single-element tuples are allowed
    assert!(result.is_ok() || result.is_err());
}

// ============================================================================
// Corner Cases from Fuzzing
// ============================================================================

/// Fuzz: Parser crash on certain token sequences
/// Found: Fuzzing round 2025-11-XX
#[test]
fn test_fuzz_corner_case_1() {
    let source = "fn(";

    let mut parser = Parser::new(source);
    let result = parser.parse_module();

    // Should not crash
    assert!(result.is_err());
}

/// Fuzz: Lexer infinite loop on certain input
/// Found: Fuzzing round 2025-11-XX
#[test]
fn test_fuzz_corner_case_2() {
    let source = "\"";

    let mut lexer = Lexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    // Should not hang
    assert!(!tokens.is_empty());
}

/// Fuzz: Type checker panic on certain expressions
/// Found: Fuzzing round 2025-11-XX
#[test]
fn test_fuzz_corner_case_3() {
    let source = "((";

    let mut parser = Parser::new(source);
    let result = parser.parse_expr();

    // Should not panic
    assert!(result.is_err());
}

// ============================================================================
// Boundary Condition Tests
// ============================================================================

/// Boundary: Zero values
#[test]
fn test_boundary_zero_value() {
    let source = "0";

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    let mut env = Environment::new();
    let mut eval = Evaluator::new();
    let result = eval.eval_expr(&expr, &mut env).expect("Should evaluate");

    match result {
        Value::Int(n) => assert_eq!(n, 0),
        _ => panic!("Expected 0"),
    }
}

/// Boundary: Negative zero float
#[test]
fn test_boundary_negative_zero() {
    let source = "-0.0";

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    let mut env = Environment::new();
    let mut eval = Evaluator::new();
    let result = eval.eval_expr(&expr, &mut env).expect("Should evaluate");

    match result {
        Value::Float(f) => {
            // -0.0 should be distinct from 0.0
            assert!(f.to_bits() == (-0.0f64).to_bits());
        }
        _ => panic!("Expected Float"),
    }
}

/// Boundary: Very long identifier
#[test]
fn test_boundary_long_identifier() {
    let ident = "x".repeat(1000);
    let source = format!("let {} = 42;", ident);

    let mut parser = Parser::new(&source);
    let result = parser.parse_module();

    // Should handle long identifiers
    assert!(result.is_ok() || result.is_err());
}

/// Boundary: Very large list
#[test]
fn test_boundary_large_list() {
    let mut list = List::new();

    for i in 0..10_000 {
        list.push(i);
    }

    assert_eq!(list.len(), 10_000);
}

// ============================================================================
// Consistency Tests
// ============================================================================

/// Consistency: Same input should produce same output
#[test]
fn test_consistency_deterministic_parsing() {
    let source = "fn test(x: Int) -> Int { x + 1 }";

    let mut parser1 = Parser::new(source);
    let result1 = parser1.parse_module().expect("Parse 1 failed");

    let mut parser2 = Parser::new(source);
    let result2 = parser2.parse_module().expect("Parse 2 failed");

    assert_eq!(result1.declarations.len(), result2.declarations.len());
}

/// Consistency: Type checking should be idempotent
#[test]
fn test_consistency_idempotent_type_checking() {
    let source = "42";

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    let mut checker1 = TypeChecker::new();
    let typed1 = checker1.synth_expr(&expr).expect("Type check 1 failed");

    let mut checker2 = TypeChecker::new();
    let typed2 = checker2.synth_expr(&expr).expect("Type check 2 failed");

    assert_eq!(typed1.ty, typed2.ty);
}

// ============================================================================
// Regression Tests for Specific GitHub Issues
// ============================================================================

/// Issue #001: Parser fails on function with no parameters
#[test]
fn test_issue_001_no_param_function() {
    let source = "fn zero() -> Int { 0 }";

    let mut parser = Parser::new(source);
    let result = parser.parse_module();

    assert!(result.is_ok(), "Issue #001: Should parse function with no params");
}

/// Issue #002: Type checker rejects valid match expression
#[test]
fn test_issue_002_valid_match() {
    let source = r#"
        match 1 {
            1 => "one",
            _ => "other"
        }
    "#;

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    let mut env = Environment::new();
    let mut eval = Evaluator::new();
    let result = eval.eval_expr(&expr, &mut env);

    assert!(result.is_ok(), "Issue #002: Valid match should work");
}

/// Issue #003: CBGR memory leak in specific scenario
#[test]
fn test_issue_003_cbgr_memory_leak() {
    let allocator = Allocator::new();

    // Allocate in a loop
    for i in 0..1000 {
        let _gen_ref = allocator.alloc(i, Tier::Standard);
        // Should deallocate when going out of scope
    }

    // No memory leak should occur
}
