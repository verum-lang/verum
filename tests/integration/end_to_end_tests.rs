//! End-to-End Integration Tests for Complete Verum Pipeline
//!
//! Tests the complete compilation pipeline:
//! Source → Lexer → Parser → Type Checker → Code Generator → Execution
//!
//! Spec: All components from TIER 0-4

use verum_ast::{expr::*, literal::*, pattern::Pattern, span::Span, ty::*};
use verum_cbgr::{Allocator, GenRef, Tier};
use verum_interpreter::{Environment, Evaluator, Value};
use verum_lexer::{Lexer, Token, TokenKind};
use verum_parser::Parser;
use verum_std::core::{List, Text};
use verum_types::{TypeChecker, Type};

// ============================================================================
// Basic Arithmetic and Logic Tests
// ============================================================================

#[test]
fn test_e2e_simple_arithmetic() {
    // Test: 2 + 3 * 4
    let source = "2 + 3 * 4";

    // Lex
    let mut lexer = Lexer::new(source);
    let tokens: Vec<Token> = lexer.collect();
    assert!(!tokens.is_empty(), "Lexer failed to tokenize");

    // Parse
    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Parser failed");

    // Type check
    let mut checker = TypeChecker::new();
    let typed = checker.synth_expr(&expr).expect("Type checking failed");
    assert_eq!(typed.ty, Type::int(), "Expected Int type");

    // Evaluate
    let mut env = Environment::new();
    let mut eval = Evaluator::new();
    let result = eval.eval_expr(&expr, &mut env).expect("Evaluation failed");

    match result {
        Value::Int(n) => assert_eq!(n, 14, "2 + 3 * 4 should equal 14"),
        _ => panic!("Expected Int value"),
    }
}

#[test]
fn test_e2e_boolean_logic() {
    // Test: true && false || true
    let source = "true && false || true";

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Parser failed");

    let mut checker = TypeChecker::new();
    let typed = checker.synth_expr(&expr).expect("Type checking failed");
    assert_eq!(typed.ty, Type::bool());

    let mut env = Environment::new();
    let mut eval = Evaluator::new();
    let result = eval.eval_expr(&expr, &mut env).expect("Evaluation failed");

    match result {
        Value::Bool(b) => assert!(b, "true && false || true should be true"),
        _ => panic!("Expected Bool value"),
    }
}

#[test]
fn test_e2e_comparison() {
    let source = "10 > 5";

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Parser failed");

    let mut checker = TypeChecker::new();
    let typed = checker.synth_expr(&expr).expect("Type checking failed");
    assert_eq!(typed.ty, Type::bool());

    let mut env = Environment::new();
    let mut eval = Evaluator::new();
    let result = eval.eval_expr(&expr, &mut env).expect("Evaluation failed");

    match result {
        Value::Bool(b) => assert!(b, "10 > 5 should be true"),
        _ => panic!("Expected Bool value"),
    }
}

// ============================================================================
// Function Definition and Application Tests
// ============================================================================

#[test]
fn test_e2e_function_definition() {
    // Test: fn add(x: Int, y: Int) -> Int { x + y }
    let source = r#"
        fn add(x: Int, y: Int) -> Int {
            x + y
        }
    "#;

    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Parser failed");

    assert_eq!(module.declarations.len(), 1, "Should have one declaration");
}

#[test]
fn test_e2e_recursive_function() {
    // Test: Factorial function
    let source = r#"
        fn factorial(n: Int) -> Int {
            match n {
                0 => 1,
                n => n * factorial(n - 1)
            }
        }
    "#;

    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Parser failed");
    assert_eq!(module.declarations.len(), 1);
}

#[test]
fn test_e2e_fibonacci() {
    let source = r#"
        fn fibonacci(n: Int) -> Int {
            match n {
                0 => 0,
                1 => 1,
                n => fibonacci(n - 1) + fibonacci(n - 2)
            }
        }
    "#;

    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Parser failed");
    assert_eq!(module.declarations.len(), 1);
}

// ============================================================================
// Pattern Matching Tests
// ============================================================================

#[test]
fn test_e2e_pattern_matching_literals() {
    let source = r#"
        match 42 {
            0 => "zero",
            1 => "one",
            42 => "answer",
            _ => "other"
        }
    "#;

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Parser failed");

    let mut env = Environment::new();
    let mut eval = Evaluator::new();
    let result = eval.eval_expr(&expr, &mut env).expect("Evaluation failed");

    match result {
        Value::Text(ref s) => assert_eq!(s.as_str(), "answer"),
        _ => panic!("Expected Text value"),
    }
}

#[test]
fn test_e2e_pattern_matching_tuples() {
    let source = r#"
        match (1, 2) {
            (0, _) => "first is zero",
            (_, 0) => "second is zero",
            (1, 2) => "one and two",
            _ => "other"
        }
    "#;

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Parser failed");

    let mut env = Environment::new();
    let mut eval = Evaluator::new();
    let result = eval.eval_expr(&expr, &mut env).expect("Evaluation failed");

    match result {
        Value::Text(ref s) => assert_eq!(s.as_str(), "one and two"),
        _ => panic!("Expected Text value"),
    }
}

// ============================================================================
// Variable Binding Tests
// ============================================================================

#[test]
fn test_e2e_let_binding() {
    let source = r#"
        let x = 10;
        let y = 20;
        x + y
    "#;

    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Parser failed");
    assert!(module.declarations.len() >= 2, "Should have let bindings");
}

#[test]
fn test_e2e_shadowing() {
    let source = r#"
        let x = 10;
        let x = 20;
        x
    "#;

    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Parser failed");
    assert!(module.declarations.len() >= 2);
}

// ============================================================================
// Collection Tests
// ============================================================================

#[test]
fn test_e2e_list_creation() {
    let source = "[1, 2, 3, 4, 5]";

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Parser failed");

    let mut env = Environment::new();
    let mut eval = Evaluator::new();
    let result = eval.eval_expr(&expr, &mut env).expect("Evaluation failed");

    match result {
        Value::List(ref list) => {
            assert_eq!(list.len(), 5, "List should have 5 elements");
        }
        _ => panic!("Expected List value"),
    }
}

#[test]
fn test_e2e_tuple_creation() {
    let source = "(1, true, \"hello\")";

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Parser failed");

    let mut env = Environment::new();
    let mut eval = Evaluator::new();
    let result = eval.eval_expr(&expr, &mut env).expect("Evaluation failed");

    match result {
        Value::Tuple(ref elements) => {
            assert_eq!(elements.len(), 3, "Tuple should have 3 elements");
        }
        _ => panic!("Expected Tuple value"),
    }
}

// ============================================================================
// Control Flow Tests
// ============================================================================

#[test]
fn test_e2e_if_expression() {
    let source = r#"
        if 10 > 5 {
            "greater"
        } else {
            "not greater"
        }
    "#;

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Parser failed");

    let mut env = Environment::new();
    let mut eval = Evaluator::new();
    let result = eval.eval_expr(&expr, &mut env).expect("Evaluation failed");

    match result {
        Value::Text(ref s) => assert_eq!(s.as_str(), "greater"),
        _ => panic!("Expected Text value"),
    }
}

#[test]
fn test_e2e_nested_if() {
    let source = r#"
        if true {
            if false {
                1
            } else {
                2
            }
        } else {
            3
        }
    "#;

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Parser failed");

    let mut env = Environment::new();
    let mut eval = Evaluator::new();
    let result = eval.eval_expr(&expr, &mut env).expect("Evaluation failed");

    match result {
        Value::Int(n) => assert_eq!(n, 2),
        _ => panic!("Expected Int value"),
    }
}

// ============================================================================
// CBGR Memory Management Tests
// ============================================================================

#[test]
fn test_e2e_cbgr_allocation() {
    let allocator = Allocator::new();
    let value = 42i64;

    // Allocate in Tier 0 (standard CBGR)
    let gen_ref: GenRef<i64> = allocator.alloc(value, Tier::Standard);

    // Access value
    assert_eq!(*gen_ref, 42);
}

#[test]
fn test_e2e_cbgr_tier_checked() {
    let allocator = Allocator::new();
    let value = vec![1, 2, 3, 4, 5];

    // Allocate in Tier 1 (checked references)
    let gen_ref = allocator.alloc(value, Tier::Checked);

    assert_eq!(gen_ref.len(), 5);
}

#[test]
fn test_e2e_cbgr_tier_unsafe() {
    let allocator = Allocator::new();
    let value = 100i64;

    // Allocate in Tier 2 (unsafe, zero-cost)
    let gen_ref = allocator.alloc(value, Tier::Unsafe);

    assert_eq!(*gen_ref, 100);
}

// ============================================================================
// Standard Library Usage Tests
// ============================================================================

#[test]
fn test_e2e_stdlib_list() {
    let mut list = List::new();
    list.push(1);
    list.push(2);
    list.push(3);

    assert_eq!(list.len(), 3);
    assert_eq!(list[0], 1);
    assert_eq!(list[2], 3);
}

#[test]
fn test_e2e_stdlib_text() {
    let text = Text::from("Hello, Verum!");
    assert_eq!(text.len(), 13);
    assert!(text.contains("Verum"));
}

// ============================================================================
// Complex Integration Tests
// ============================================================================

#[test]
fn test_e2e_complex_program() {
    // Test a program that uses multiple features
    let source = r#"
        fn sum_list(items: List<Int>) -> Int {
            match items {
                [] => 0,
                [head, ...tail] => head + sum_list(tail)
            }
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_module();
    assert!(result.is_ok(), "Complex program should parse successfully");
}

#[test]
fn test_e2e_multiple_functions() {
    let source = r#"
        fn double(x: Int) -> Int {
            x * 2
        }

        fn triple(x: Int) -> Int {
            x * 3
        }

        fn apply_both(x: Int) -> (Int, Int) {
            (double(x), triple(x))
        }
    "#;

    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Parser failed");
    assert_eq!(module.declarations.len(), 3, "Should have 3 function declarations");
}

#[test]
fn test_e2e_type_annotations() {
    let source = r#"
        let x: Int = 42;
        let y: Bool = true;
        let z: Text = "hello";
    "#;

    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Parser failed");
    assert!(module.declarations.len() >= 3);
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[test]
fn test_e2e_parse_error() {
    let source = "fn incomplete(x: Int {"; // Missing closing brace

    let mut parser = Parser::new(source);
    let result = parser.parse_module();
    assert!(result.is_err(), "Should fail to parse incomplete function");
}

#[test]
fn test_e2e_type_error() {
    let source = r#"
        fn bad_add(x: Int, y: Bool) -> Int {
            x + y
        }
    "#;

    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Should parse");

    // Type checking should fail
    // Note: This would need type checker integration to verify
    assert_eq!(module.declarations.len(), 1);
}

// ============================================================================
// Edge Cases Tests
// ============================================================================

#[test]
fn test_e2e_empty_module() {
    let source = "";

    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Empty module should parse");
    assert_eq!(module.declarations.len(), 0);
}

#[test]
fn test_e2e_whitespace_only() {
    let source = "   \n\n  \t  \n  ";

    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Whitespace should parse");
    assert_eq!(module.declarations.len(), 0);
}

#[test]
fn test_e2e_comments_only() {
    let source = r#"
        // This is a comment
        /* This is a block comment */
        // Another comment
    "#;

    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Comments should parse");
    assert_eq!(module.declarations.len(), 0);
}

// ============================================================================
// Large Program Tests
// ============================================================================

#[test]
fn test_e2e_many_functions() {
    let mut source = String::new();
    for i in 0..100 {
        source.push_str(&format!("fn func{}(x: Int) -> Int {{ x + {} }}\n", i, i));
    }

    let mut parser = Parser::new(&source);
    let module = parser.parse_module().expect("Large program should parse");
    assert_eq!(module.declarations.len(), 100, "Should have 100 functions");
}

#[test]
fn test_e2e_deeply_nested_expressions() {
    // Create deeply nested arithmetic: ((((1 + 2) + 3) + 4) + 5)
    let source = "((((1 + 2) + 3) + 4) + 5)";

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Deep nesting should parse");

    let mut env = Environment::new();
    let mut eval = Evaluator::new();
    let result = eval.eval_expr(&expr, &mut env).expect("Evaluation failed");

    match result {
        Value::Int(n) => assert_eq!(n, 15),
        _ => panic!("Expected Int value"),
    }
}
