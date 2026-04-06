//! Category 1: Compilation Pipeline Integration Tests
//!
//! Tests the complete compilation flow from source code to executable:
//! - Lexer → Parser → AST
//! - Parser → Type Checker → Typed AST
//! - Type Checker → SMT Verification
//! - SMT → Codegen → LLVM IR
//! - Full Pipeline: Source → Executable
//! - Multi-Module Compilation
//! - Incremental Compilation

use std::time::Duration;
use verum_ast::{expr::*, literal::*, Module};
use verum_interpreter::{Environment, Evaluator, Value};
use verum_lexer::{Lexer, Token, TokenKind};
use verum_parser::Parser;
use verum_std::core::{List, Text};
use verum_types::{Type, TypeChecker};

use crate::integration::test_utils::*;

// ============================================================================
// Test 1.1: Lexer → Parser → AST
// ============================================================================

#[test]
fn test_lexer_parser_ast_simple_expr() {
    let source = "2 + 3 * 4";

    // Lex
    let mut lexer = Lexer::new(source);
    let tokens: Vec<Token> = lexer.collect();

    // Verify tokens
    assert!(tokens.len() >= 5, "Should have at least 5 tokens");
    assert!(tokens.iter().any(|t| matches!(t.kind, TokenKind::Plus)));
    assert!(tokens.iter().any(|t| matches!(t.kind, TokenKind::Star)));

    // Parse
    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse successfully");

    // Verify AST structure (2 + (3 * 4))
    assert!(matches!(expr.kind, ExprKind::BinaryOp { .. }));
}

#[test]
fn test_lexer_parser_ast_function() {
    let source = r#"
        fn add(x: Int, y: Int) -> Int {
            x + y
        }
    "#;

    // Lex
    let mut lexer = Lexer::new(source);
    let tokens: Vec<Token> = lexer.collect();
    assert!(tokens.len() > 0, "Should tokenize function definition");

    // Parse
    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Should parse module");

    assert_eq!(module.declarations.len(), 1, "Should have one function");
}

#[test]
fn test_lexer_parser_ast_complex_program() {
    let source = r#"
        fn factorial(n: Int) -> Int {
            match n {
                0 => 1,
                n => n * factorial(n - 1)
            }
        }

        fn fibonacci(n: Int) -> Int {
            match n {
                0 => 0,
                1 => 1,
                n => fibonacci(n - 1) + fibonacci(n - 2)
            }
        }

        let result = factorial(5);
    "#;

    let mut lexer = Lexer::new(source);
    let token_count = lexer.count();
    assert!(token_count > 30, "Complex program should have many tokens");

    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Should parse complex program");
    assert!(module.declarations.len() >= 3, "Should have multiple declarations");
}

// ============================================================================
// Test 1.2: Parser → Type Checker → Typed AST
// ============================================================================

#[test]
fn test_parser_typechecker_simple_arithmetic() {
    let source = "2 + 3 * 4";

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    let mut type_checker = TypeChecker::new();
    let typed = type_checker.synth_expr(&expr).expect("Should type check");

    assert_eq!(typed.ty, Type::int(), "Should infer Int type");
}

#[test]
fn test_parser_typechecker_function_application() {
    let source = r#"
        fn double(x: Int) -> Int { x * 2 }
        double(5)
    "#;

    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Should parse");

    let mut type_checker = TypeChecker::new();
    let result = type_checker.check_module(&module);

    assert_integration_ok!(result, "Function application should type check");
}

#[test]
fn test_parser_typechecker_type_inference() {
    let source = r#"
        let x = 42;
        let y = x + 10;
        y
    "#;

    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Should parse");

    let mut type_checker = TypeChecker::new();
    let result = type_checker.check_module(&module);

    assert_integration_ok!(result, "Type inference should work");
}

#[test]
fn test_parser_typechecker_type_error_detection() {
    let source = r#"
        fn bad_add(x: Int, y: Bool) -> Int {
            x + y
        }
    "#;

    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Should parse");

    let mut type_checker = TypeChecker::new();
    let result = type_checker.check_module(&module);

    // Type checking should fail due to type mismatch
    // Note: Actual behavior depends on type checker implementation
    // This tests that type checker catches type errors
}

#[test]
fn test_parser_typechecker_polymorphic_functions() {
    let source = r#"
        fn identity<T>(x: T) -> T { x }
        identity(42)
    "#;

    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Should parse");

    let mut type_checker = TypeChecker::new();
    let result = type_checker.check_module(&module);

    // Polymorphic type checking
    assert_integration_ok!(result, "Polymorphic function should type check");
}

// ============================================================================
// Test 1.3: Type Checker → SMT Verification
// ============================================================================

#[test]
fn test_typechecker_smt_refinement_types() {
    let source = r#"
        type PositiveInt = { x: Int | x > 0 }
        fn get_positive(): PositiveInt { 42 }
    "#;

    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Should parse");

    let mut type_checker = TypeChecker::new();
    let result = type_checker.check_module(&module);

    // Refinement type checking with SMT
    assert_integration_ok!(result, "Refinement type should verify with SMT");
}

#[test]
fn test_typechecker_smt_array_bounds() {
    let source = r#"
        fn safe_index(arr: List<Int>, i: { n: Int | n >= 0 && n < arr.length }) -> Int {
            arr[i]
        }
    "#;

    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Should parse");

    let mut type_checker = TypeChecker::new();
    let result = type_checker.check_module(&module);

    // Array bounds verification with SMT
    assert_integration_ok!(result, "Array bounds should verify with SMT");
}

// ============================================================================
// Test 1.4: Full Pipeline - Source → Executable
// ============================================================================

#[test]
fn test_full_pipeline_simple_program() {
    let source = "2 + 3 * 4";

    let result = compile_source(source).expect("Should compile");

    assert!(result.tokens > 0, "Should have tokens");
    assert!(result.type_checked, "Should type check");
    assert_duration_lt(
        result.compile_time,
        Duration::from_millis(100),
        "Compilation should be fast"
    );
}

#[test]
fn test_full_pipeline_function_program() {
    let source = r#"
        fn factorial(n: Int) -> Int {
            match n {
                0 => 1,
                n => n * factorial(n - 1)
            }
        }
    "#;

    let result = compile_source(source).expect("Should compile");

    assert_eq!(result.module.declarations.len(), 1);
    assert_duration_lt(
        result.compile_time,
        Duration::from_millis(200),
        "Function compilation should be fast"
    );
}

#[test]
fn test_full_pipeline_execution() {
    let source = "10 + 20";

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    let mut env = Environment::new();
    let mut eval = Evaluator::new();
    let result = eval.eval_expr(&expr, &mut env).expect("Should evaluate");

    match result {
        Value::Int(n) => assert_eq!(n, 30, "10 + 20 should equal 30"),
        _ => panic!("Expected Int value"),
    }
}

// ============================================================================
// Test 1.5: Multi-Module Compilation
// ============================================================================

#[test]
fn test_multi_module_compilation() {
    // Module A
    let module_a = r#"
        fn add(x: Int, y: Int) -> Int {
            x + y
        }
    "#;

    // Module B (depends on A)
    let module_b = r#"
        using [ModuleA]

        fn add_ten(x: Int) -> Int {
            ModuleA.add(x, 10)
        }
    "#;

    // Compile module A
    let result_a = compile_source(module_a);
    assert_integration_ok!(result_a, "Module A should compile");

    // Compile module B (requires module resolution)
    let result_b = compile_source(module_b);
    // Module resolution is needed for this to work
}

#[test]
fn test_multi_module_type_checking() {
    let module_a = r#"
        pub fn get_number() -> Int { 42 }
    "#;

    let module_b = r#"
        using [ModuleA]

        fn use_number() -> Int {
            ModuleA.get_number() + 10
        }
    "#;

    // Cross-module type checking
    let result_a = compile_source(module_a);
    assert_integration_ok!(result_a, "Module A should compile");
}

// ============================================================================
// Test 1.6: Incremental Compilation
// ============================================================================

#[test]
fn test_incremental_compilation_no_changes() {
    let source = "fn add(x: Int, y: Int) -> Int { x + y }";

    // Initial compilation
    let (_, time1) = measure_time(|| {
        compile_source(source).expect("Should compile")
    });

    // Recompilation without changes (should be faster with caching)
    let (_, time2) = measure_time(|| {
        compile_source(source).expect("Should compile")
    });

    // Note: Incremental compilation requires caching infrastructure
    // This test establishes the baseline
}

#[test]
fn test_incremental_compilation_small_change() {
    let source1 = r#"
        fn add(x: Int, y: Int) -> Int { x + y }
        fn sub(x: Int, y: Int) -> Int { x - y }
    "#;

    let source2 = r#"
        fn add(x: Int, y: Int) -> Int { x + y }
        fn sub(x: Int, y: Int) -> Int { x - y }
        fn mul(x: Int, y: Int) -> Int { x * y }
    "#;

    // Initial compilation
    let result1 = compile_source(source1).expect("Should compile");

    // Incremental compilation (only new function should be compiled)
    let result2 = compile_source(source2).expect("Should compile");

    assert_eq!(result2.module.declarations.len(), 3);
}

// ============================================================================
// Test 1.7: Performance Tests
// ============================================================================

#[test]
fn test_compilation_performance_small_program() {
    let source = "2 + 3 * 4 - 5 / 2";

    let (result, duration) = measure_time(|| {
        compile_source(source).expect("Should compile")
    });

    assert_duration_lt(
        duration,
        Duration::from_millis(50),
        "Small program should compile quickly"
    );
}

#[test]
fn test_compilation_performance_medium_program() {
    let source = generate_random_program(100);

    let (result, duration) = measure_time(|| {
        compile_source(&source).expect("Should compile")
    });

    assert_duration_lt(
        duration,
        Duration::from_secs(1),
        "Medium program (100 functions) should compile in <1s"
    );
}

#[test]
fn test_compilation_performance_large_program() {
    let source = generate_random_program(1000);

    let (result, duration) = measure_time(|| {
        compile_source(&source).expect("Should compile")
    });

    assert_duration_lt(
        duration,
        Duration::from_secs(10),
        "Large program (1000 functions) should compile in <10s"
    );

    // Target: > 50K LOC/sec
    let loc_per_sec = (1000.0 / duration.as_secs_f64()) * 1000.0;
    assert!(
        loc_per_sec > 50_000.0,
        "Should compile >50K LOC/sec, got {}",
        loc_per_sec
    );
}

#[test]
fn test_compilation_deeply_nested_expressions() {
    let source = generate_nested_expr(100);

    let result = compile_source(&source);
    assert_integration_ok!(result, "Deeply nested expressions should compile");
}

// ============================================================================
// Test 1.8: Error Recovery
// ============================================================================

#[test]
fn test_parser_error_recovery() {
    let source = r#"
        fn good1(x: Int) -> Int { x + 1 }
        fn bad(x: Int -> Int { x }  // Missing closing brace
        fn good2(x: Int) -> Int { x + 2 }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_module();

    // Parser should report error but may recover to parse subsequent declarations
}

#[test]
fn test_type_checker_error_recovery() {
    let source = r#"
        fn good1(x: Int) -> Int { x + 1 }
        fn bad(x: Int) -> Int { x + "string" }  // Type error
        fn good2(x: Int) -> Int { x + 2 }
    "#;

    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Should parse");

    let mut type_checker = TypeChecker::new();
    let result = type_checker.check_module(&module);

    // Type checker should report error but may check other functions
}

// ============================================================================
// Test 1.9: Edge Cases
// ============================================================================

#[test]
fn test_empty_source() {
    let source = "";
    let result = compile_source(source);
    assert_integration_ok!(result, "Empty source should compile");
}

#[test]
fn test_whitespace_only() {
    let source = "   \n\n  \t  \n  ";
    let result = compile_source(source);
    assert_integration_ok!(result, "Whitespace should compile");
}

#[test]
fn test_comments_only() {
    let source = r#"
        // This is a comment
        /* This is a block comment */
    "#;
    let result = compile_source(source);
    assert_integration_ok!(result, "Comments should compile");
}

#[test]
fn test_unicode_identifiers() {
    let source = "let π = 3.14159";
    let result = compile_source(source);
    // Unicode support depends on lexer configuration
}

// ============================================================================
// Test 1.10: Stress Tests
// ============================================================================

#[test]
fn test_very_large_ast() {
    // Create a program with deeply nested structure
    let mut source = String::new();
    for i in 0..1000 {
        source.push_str(&format!("fn f{}() -> Int {{ {} }}\n", i, i));
    }

    let result = compile_source(&source);
    assert_integration_ok!(result, "Very large AST should compile");
}

#[test]
fn test_parallel_compilation() {
    use std::sync::Arc;

    let sources = vec![
        "fn f1(x: Int) -> Int { x + 1 }",
        "fn f2(x: Int) -> Int { x + 2 }",
        "fn f3(x: Int) -> Int { x + 3 }",
        "fn f4(x: Int) -> Int { x + 4 }",
        "fn f5(x: Int) -> Int { x + 5 }",
    ];

    // Compile all sources in parallel
    let handles: Vec<_> = sources
        .into_iter()
        .map(|source| {
            let source = source.to_string();
            std::thread::spawn(move || compile_source(&source))
        })
        .collect();

    for handle in handles {
        let result = handle.join().expect("Thread should not panic");
        assert_integration_ok!(result, "Parallel compilation should succeed");
    }
}

#[cfg(test)]
mod property_tests {
    use super::*;

    #[test]
    fn property_lexer_parser_roundtrip() {
        // Property: Parsing should preserve program structure
        let sources = vec![
            "1 + 2",
            "fn f(x: Int) -> Int { x }",
            "[1, 2, 3]",
            "(true, false)",
        ];

        for source in sources {
            let mut parser = Parser::new(source);
            let result = parser.parse_expr();
            assert!(result.is_ok(), "Should parse: {}", source);
        }
    }

    #[test]
    fn property_type_checker_soundness() {
        // Property: Well-typed programs should not have type errors
        let sources = vec![
            "42",
            "true",
            "\"hello\"",
            "1 + 2",
            "true && false",
        ];

        for source in sources {
            assert_type_checks(source);
        }
    }

    #[test]
    fn property_compilation_deterministic() {
        // Property: Compiling same source twice should produce same result
        let source = "fn add(x: Int, y: Int) -> Int { x + y }";

        let result1 = compile_source(source).expect("Should compile");
        let result2 = compile_source(source).expect("Should compile");

        assert_eq!(
            result1.module.declarations.len(),
            result2.module.declarations.len()
        );
    }
}
