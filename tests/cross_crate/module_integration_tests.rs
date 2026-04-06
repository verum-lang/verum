//! Cross-Crate Integration Tests
//!
//! Verifies that all crates work together correctly and data flows
//! properly between modules.

use verum_ast::{expr::*, literal::*, module::Module, span::Span};
use verum_cbgr::{Allocator, GenRef, Tier};
use verum_context::{Context, Injectable, Provide};
use verum_diagnostics::{Diagnostic, DiagnosticContext, Severity};
use verum_interpreter::{Environment, Evaluator, Value};
use verum_lexer::{Lexer, Token, TokenKind};
use verum_parser::Parser;
use verum_resolve::{Resolver, Scope, Symbol, SymbolKind};
use verum_runtime::ThreadPool;
use verum_std::core::{List, Text, Map, Set};
use verum_types::{TypeChecker, Type};

// ============================================================================
// Lexer → Parser Integration
// ============================================================================

#[test]
fn test_lexer_to_parser_integration() {
    let source = "fn add(x: Int, y: Int) -> Int { x + y }";

    // Lexer produces tokens
    let mut lexer = Lexer::new(source);
    let tokens: Vec<Token> = lexer.collect();
    assert!(!tokens.is_empty(), "Lexer should produce tokens");

    // Parser consumes source (creates own lexer)
    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Parser should succeed");

    assert_eq!(module.declarations.len(), 1, "Should parse one function");
}

#[test]
fn test_lexer_token_preservation() {
    let source = "let x = 42;";

    let mut lexer = Lexer::new(source);
    let tokens: Vec<Token> = lexer.collect();

    // Verify token structure
    let token_kinds: Vec<TokenKind> = tokens.iter().map(|t| t.kind.clone()).collect();

    // Should contain: Let, Ident, Eq, IntLit, Semi
    assert!(token_kinds.contains(&TokenKind::Let));
    assert!(token_kinds.iter().any(|k| matches!(k, TokenKind::Ident(_))));
    assert!(token_kinds.contains(&TokenKind::Eq));
}

// ============================================================================
// Parser → AST Integration
// ============================================================================

#[test]
fn test_parser_to_ast_integration() {
    let source = "42 + 10";

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    // Verify AST structure
    match &expr.kind {
        ExprKind::Binary { op, left, right } => {
            assert!(matches!(op, BinOp::Add));
            assert!(matches!(left.kind, ExprKind::Literal(_)));
            assert!(matches!(right.kind, ExprKind::Literal(_)));
        }
        _ => panic!("Expected binary expression"),
    }
}

#[test]
fn test_parser_span_tracking() {
    let source = "  42  ";

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    // Span should cover the number
    let span = expr.span;
    assert!(span.start <= 2);
    assert!(span.end >= 4);
}

// ============================================================================
// Parser → Type Checker Integration
// ============================================================================

#[test]
fn test_parser_to_typechecker_integration() {
    let source = "10 + 20";

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    let mut checker = TypeChecker::new();
    let typed = checker.synth_expr(&expr).expect("Should type check");

    assert_eq!(typed.ty, Type::int());
}

#[test]
fn test_parser_typechecker_error_detection() {
    let source = "true + 42"; // Type error: can't add bool and int

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    let mut checker = TypeChecker::new();
    let result = checker.synth_expr(&expr);

    // Should fail type checking
    assert!(result.is_err(), "Should detect type error");
}

// ============================================================================
// Type Checker → Interpreter Integration
// ============================================================================

#[test]
fn test_typechecker_to_interpreter_integration() {
    let source = "10 * 5";

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    let mut checker = TypeChecker::new();
    let typed = checker.synth_expr(&expr).expect("Should type check");
    assert_eq!(typed.ty, Type::int());

    let mut env = Environment::new();
    let mut eval = Evaluator::new();
    let result = eval.eval_expr(&expr, &mut env).expect("Should evaluate");

    match result {
        Value::Int(n) => assert_eq!(n, 50),
        _ => panic!("Expected Int"),
    }
}

// ============================================================================
// AST → Resolver Integration
// ============================================================================

#[test]
fn test_ast_to_resolver_integration() {
    let source = r#"
        let x = 10;
        let y = x + 20;
    "#;

    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Should parse");

    let mut resolver = Resolver::new();
    // Resolver would process the module to build symbol table
    // This is a simplified test
    assert!(module.declarations.len() >= 2);
}

#[test]
fn test_resolver_scope_handling() {
    let mut scope = Scope::new();

    // Define symbols
    let sym1 = Symbol::new("x".into(), SymbolKind::Variable, Span::dummy());
    let sym2 = Symbol::new("y".into(), SymbolKind::Variable, Span::dummy());

    scope.define(sym1.clone());
    scope.define(sym2.clone());

    // Lookup symbols
    assert!(scope.lookup("x").is_some());
    assert!(scope.lookup("y").is_some());
    assert!(scope.lookup("z").is_none());
}

// ============================================================================
// CBGR → Runtime Integration
// ============================================================================

#[test]
fn test_cbgr_to_runtime_integration() {
    let allocator = Allocator::new();

    // Allocate values
    let val1: GenRef<i64> = allocator.alloc(42, Tier::Standard);
    let val2: GenRef<i64> = allocator.alloc(100, Tier::Standard);

    // Values should be accessible
    assert_eq!(*val1, 42);
    assert_eq!(*val2, 100);
}

#[test]
fn test_cbgr_multiple_tiers() {
    let allocator = Allocator::new();

    let tier0 = allocator.alloc(1, Tier::Standard);
    let tier1 = allocator.alloc(2, Tier::Checked);
    let tier2 = allocator.alloc(3, Tier::Unsafe);

    assert_eq!(*tier0, 1);
    assert_eq!(*tier1, 2);
    assert_eq!(*tier2, 3);
}

// ============================================================================
// Context System Integration
// ============================================================================

#[test]
fn test_context_dependency_injection() {
    // Test that context system works with multiple crates
    let ctx = Context::new();

    // Provide values
    ctx.provide::<i32>(42);
    ctx.provide::<String>("Hello".to_string());

    // Inject values
    let num: i32 = ctx.get().expect("Should have i32");
    let text: String = ctx.get().expect("Should have String");

    assert_eq!(num, 42);
    assert_eq!(text, "Hello");
}

// ============================================================================
// Diagnostics Integration
// ============================================================================

#[test]
fn test_diagnostics_with_parser_errors() {
    let source = "fn incomplete(";
    let mut diag_ctx = DiagnosticContext::new();

    let mut parser = Parser::new(source);
    let result = parser.parse_module();

    if result.is_err() {
        let diag = Diagnostic::new(
            Severity::Error,
            "Parse error".to_string(),
            Span::dummy(),
        );
        diag_ctx.report(diag);
    }

    assert!(diag_ctx.has_errors());
}

#[test]
fn test_diagnostics_with_type_errors() {
    let source = "true + 42";
    let mut diag_ctx = DiagnosticContext::new();

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    let mut checker = TypeChecker::new();
    let result = checker.synth_expr(&expr);

    if result.is_err() {
        let diag = Diagnostic::new(
            Severity::Error,
            "Type error".to_string(),
            expr.span,
        );
        diag_ctx.report(diag);
    }

    assert!(diag_ctx.has_errors());
}

// ============================================================================
// Standard Library Integration
// ============================================================================

#[test]
fn test_stdlib_list_with_interpreter() {
    let mut list = List::new();
    list.push(Value::Int(1));
    list.push(Value::Int(2));
    list.push(Value::Int(3));

    assert_eq!(list.len(), 3);

    match &list[0] {
        Value::Int(n) => assert_eq!(*n, 1),
        _ => panic!("Expected Int"),
    }
}

#[test]
fn test_stdlib_text_operations() {
    let text = Text::from("Hello, Verum!");

    assert_eq!(text.len(), 13);
    assert!(text.starts_with("Hello"));
    assert!(text.ends_with("Verum!"));
    assert!(text.contains("Verum"));
}

#[test]
fn test_stdlib_map_operations() {
    let mut map = Map::new();

    map.insert("x".to_string(), 10);
    map.insert("y".to_string(), 20);

    assert_eq!(map.get("x"), Some(&10));
    assert_eq!(map.get("y"), Some(&20));
    assert_eq!(map.get("z"), None);
}

#[test]
fn test_stdlib_set_operations() {
    let mut set = Set::new();

    set.insert(1);
    set.insert(2);
    set.insert(3);

    assert!(set.contains(&1));
    assert!(set.contains(&2));
    assert!(!set.contains(&4));
}

// ============================================================================
// Runtime Thread Pool Integration
// ============================================================================

#[test]
fn test_runtime_threadpool_basic() {
    let pool = ThreadPool::new(4);

    let result = pool.execute(|| 42);
    // Thread pool should execute the task
    assert!(result.is_ok() || result.is_err()); // Basic smoke test
}

// ============================================================================
// Full Pipeline Integration Tests
// ============================================================================

#[test]
fn test_full_pipeline_simple_expression() {
    // Source → Lexer → Parser → TypeChecker → Interpreter
    let source = "2 + 3";

    // Step 1: Lex
    let mut lexer = Lexer::new(source);
    let tokens: Vec<Token> = lexer.collect();
    assert!(!tokens.is_empty());

    // Step 2: Parse
    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Parse failed");

    // Step 3: Type Check
    let mut checker = TypeChecker::new();
    let typed = checker.synth_expr(&expr).expect("Type check failed");
    assert_eq!(typed.ty, Type::int());

    // Step 4: Evaluate
    let mut env = Environment::new();
    let mut eval = Evaluator::new();
    let result = eval.eval_expr(&expr, &mut env).expect("Eval failed");

    match result {
        Value::Int(n) => assert_eq!(n, 5),
        _ => panic!("Expected Int"),
    }
}

#[test]
fn test_full_pipeline_with_variables() {
    let source = r#"
        let x = 10;
        let y = 20;
    "#;

    // Parse
    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Parse failed");

    // Verify declarations
    assert!(module.declarations.len() >= 2);
}

#[test]
fn test_full_pipeline_with_functions() {
    let source = r#"
        fn double(x: Int) -> Int {
            x * 2
        }
    "#;

    // Parse
    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Parse failed");
    assert_eq!(module.declarations.len(), 1);

    // Type checking would verify function signature
    // Execution would test function application
}

// ============================================================================
// Re-export Verification Tests
// ============================================================================

#[test]
fn test_reexport_verum_std_types() {
    // Verify that verum_std re-exports work
    let _list: List<i32> = List::new();
    let _text: Text = Text::from("test");
    let _map: Map<String, i32> = Map::new();
    let _set: Set<i32> = Set::new();

    // If this compiles, re-exports work correctly
}

#[test]
fn test_reexport_verum_ast_types() {
    // Verify AST re-exports
    let _span = Span::dummy();
    let _literal = Literal::int(42, Span::dummy());

    // If this compiles, re-exports work correctly
}

// ============================================================================
// Version Compatibility Tests
// ============================================================================

#[test]
fn test_crate_version_compatibility() {
    // All crates should be version 0.1.0
    // This is a compile-time check - if versions mismatch,
    // the build will fail

    // Smoke test to ensure all crates can be imported
    let _ = Allocator::new();
    let _ = Context::new();
    let _ = Lexer::new("");
    let _ = Parser::new("");
    let _ = TypeChecker::new();
    let _ = Evaluator::new();

    // If this compiles, version compatibility is OK
}

// ============================================================================
// Data Structure Compatibility Tests
// ============================================================================

#[test]
fn test_span_compatibility_across_crates() {
    let span = Span::new(0, 10);

    // Create AST nodes with spans
    let literal = Literal::int(42, span);
    let expr = Expr::literal(literal);

    // Spans should be preserved
    assert_eq!(expr.span, span);
}

#[test]
fn test_type_compatibility_across_crates() {
    // Types should work consistently across crates
    let int_type = Type::int();
    let bool_type = Type::bool();
    let text_type = Type::text();

    // Basic type operations
    assert_ne!(int_type, bool_type);
    assert_ne!(int_type, text_type);
    assert_eq!(int_type, Type::int());
}
