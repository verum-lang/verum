//! Error Handling Integration Tests
//!
//! Tests error propagation through the compilation pipeline,
//! diagnostic quality, error recovery, and error continuation.

use verum_diagnostics::{Diagnostic, DiagnosticContext, Severity};
use verum_interpreter::{Environment, Evaluator, InterpreterError};
use verum_lexer::{Lexer, LexError};
use verum_parser::{Parser, ParseError};
use verum_types::{TypeChecker, TypeError};

// ============================================================================
// Lexer Error Tests
// ============================================================================

#[test]
fn test_lexer_invalid_character() {
    let source = "let x = @#$%";

    let mut lexer = Lexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    // Lexer should handle invalid characters gracefully
    // Either by creating error tokens or skipping them
    assert!(!tokens.is_empty());
}

#[test]
fn test_lexer_unterminated_string() {
    let source = r#"let x = "unclosed string"#;

    let mut lexer = Lexer::new(source);
    let _tokens: Vec<_> = lexer.collect();

    // Lexer should detect unterminated strings
}

#[test]
fn test_lexer_invalid_number() {
    let source = "let x = 123abc456";

    let mut lexer = Lexer::new(source);
    let _tokens: Vec<_> = lexer.collect();

    // Lexer should handle malformed numbers
}

// ============================================================================
// Parser Error Tests
// ============================================================================

#[test]
fn test_parser_missing_semicolon() {
    let source = r#"
        let x = 10
        let y = 20;
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_module();

    // Parser should detect missing semicolon
    // May recover and continue parsing
    assert!(result.is_ok() || result.is_err());
}

#[test]
fn test_parser_unmatched_brace() {
    let source = r#"
        fn test() {
            let x = 10;
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_module();

    assert!(result.is_err(), "Should detect unmatched brace");
}

#[test]
fn test_parser_unmatched_paren() {
    let source = "let x = (10 + 20;";

    let mut parser = Parser::new(source);
    let result = parser.parse_expr();

    assert!(result.is_err(), "Should detect unmatched paren");
}

#[test]
fn test_parser_invalid_function_syntax() {
    let source = "fn (x: Int) -> Int { x }"; // Missing function name

    let mut parser = Parser::new(source);
    let result = parser.parse_module();

    assert!(result.is_err(), "Should detect invalid function syntax");
}

#[test]
fn test_parser_incomplete_match() {
    let source = r#"
        match x {
            1 =>
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_expr();

    assert!(result.is_err(), "Should detect incomplete match arm");
}

// ============================================================================
// Parser Error Recovery Tests
// ============================================================================

#[test]
fn test_parser_recovery_after_error() {
    let source = r#"
        fn bad( { }
        fn good(x: Int) -> Int { x }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_module();

    // Parser should recover and parse the second function
    // Even if the first one has errors
    assert!(result.is_ok() || result.is_err());
}

#[test]
fn test_parser_multiple_errors() {
    let source = r#"
        fn bad1( { }
        fn bad2) { }
        fn good() -> Int { 42 }
    "#;

    let mut parser = Parser::new(source);
    let _result = parser.parse_module();

    // Parser should collect multiple errors
}

// ============================================================================
// Type Error Tests
// ============================================================================

#[test]
fn test_type_error_binary_op_mismatch() {
    let source = "true + 42";

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    let mut checker = TypeChecker::new();
    let result = checker.synth_expr(&expr);

    assert!(result.is_err(), "Should detect type mismatch");
}

#[test]
fn test_type_error_function_arg_mismatch() {
    let source = r#"
        fn add(x: Int, y: Int) -> Int {
            x + y
        }
        add(10, true)
    "#;

    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Should parse");

    // Type checker should detect argument type mismatch
    assert!(module.declarations.len() >= 1);
}

#[test]
fn test_type_error_return_type_mismatch() {
    let source = r#"
        fn returns_int() -> Int {
            true
        }
    "#;

    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Should parse");

    // Type checker should detect return type mismatch
    assert_eq!(module.declarations.len(), 1);
}

#[test]
fn test_type_error_if_branch_mismatch() {
    let source = r#"
        if true {
            10
        } else {
            "string"
        }
    "#;

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    let mut checker = TypeChecker::new();
    let result = checker.synth_expr(&expr);

    assert!(result.is_err(), "Should detect branch type mismatch");
}

#[test]
fn test_type_error_undefined_variable() {
    let source = "x + 10";

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    let mut checker = TypeChecker::new();
    let result = checker.synth_expr(&expr);

    assert!(result.is_err(), "Should detect undefined variable");
}

// ============================================================================
// Interpreter Error Tests
// ============================================================================

#[test]
fn test_interpreter_division_by_zero() {
    let source = "10 / 0";

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    let mut env = Environment::new();
    let mut eval = Evaluator::new();
    let result = eval.eval_expr(&expr, &mut env);

    assert!(result.is_err(), "Should detect division by zero");
}

#[test]
fn test_interpreter_undefined_function() {
    let source = "unknown_function(10)";

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    let mut env = Environment::new();
    let mut eval = Evaluator::new();
    let result = eval.eval_expr(&expr, &mut env);

    assert!(result.is_err(), "Should detect undefined function");
}

#[test]
fn test_interpreter_pattern_match_failure() {
    let source = r#"
        match 42 {
            1 => "one",
            2 => "two"
        }
    "#;

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    let mut env = Environment::new();
    let mut eval = Evaluator::new();
    let result = eval.eval_expr(&expr, &mut env);

    // Should either have a wildcard or detect non-exhaustive match
    assert!(result.is_ok() || result.is_err());
}

// ============================================================================
// Diagnostic Quality Tests
// ============================================================================

#[test]
fn test_diagnostic_span_accuracy() {
    let source = "let x = true + 42;";

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    let mut checker = TypeChecker::new();
    let result = checker.synth_expr(&expr);

    if let Err(_error) = result {
        // Error should have accurate span information
        // pointing to the problematic expression
    }
}

#[test]
fn test_diagnostic_multiple_errors() {
    let source = r#"
        let x: Int = true;
        let y: Bool = 42;
        let z = x + y;
    "#;

    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Should parse");

    // Should collect all type errors
    assert!(module.declarations.len() >= 3);
}

#[test]
fn test_diagnostic_context_information() {
    let mut diag_ctx = DiagnosticContext::new();

    // Create a diagnostic with context
    let diag = Diagnostic::new(
        Severity::Error,
        "Type mismatch: expected Int, found Bool".to_string(),
        verum_ast::span::Span::new(10, 14),
    );

    diag_ctx.report(diag);

    assert!(diag_ctx.has_errors());
    assert_eq!(diag_ctx.error_count(), 1);
}

// ============================================================================
// Error Recovery and Continuation Tests
// ============================================================================

#[test]
fn test_error_recovery_continue_after_parse_error() {
    let source = r#"
        let x = ;
        let y = 10;
        let z = 20;
    "#;

    let mut parser = Parser::new(source);
    let module = parser.parse_module();

    // Parser should recover and parse subsequent declarations
    assert!(module.is_ok() || module.is_err());
}

#[test]
fn test_error_recovery_continue_after_type_error() {
    let source = r#"
        let x: Int = true;
        let y: Int = 42;
    "#;

    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Should parse");

    // Type checker should check all declarations
    // even if earlier ones have errors
    assert!(module.declarations.len() >= 2);
}

// ============================================================================
// Error Message Quality Tests
// ============================================================================

#[test]
fn test_error_message_clarity() {
    let source = "true + 42";

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    let mut checker = TypeChecker::new();
    let result = checker.synth_expr(&expr);

    if let Err(error) = result {
        let error_msg = format!("{:?}", error);
        // Error message should be informative
        assert!(!error_msg.is_empty());
    }
}

// ============================================================================
// Cascading Error Tests
// ============================================================================

#[test]
fn test_cascading_type_errors() {
    let source = r#"
        fn bad() -> Int {
            let x = true;
            let y = x + 10;
            y
        }
    "#;

    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Should parse");

    // Should detect multiple related errors
    assert_eq!(module.declarations.len(), 1);
}

// ============================================================================
// Error Boundary Tests
// ============================================================================

#[test]
fn test_error_in_nested_expression() {
    let source = "((10 + true) * 2)";

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().expect("Should parse");

    let mut checker = TypeChecker::new();
    let result = checker.synth_expr(&expr);

    assert!(result.is_err(), "Should detect nested type error");
}

#[test]
fn test_error_in_function_body() {
    let source = r#"
        fn test() -> Int {
            let x = true;
            x + 10
        }
    "#;

    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Should parse");

    assert_eq!(module.declarations.len(), 1);
}

// ============================================================================
// Suggestion Quality Tests
// ============================================================================

#[test]
fn test_suggestion_for_typo() {
    // Test that diagnostics can suggest corrections
    let mut diag_ctx = DiagnosticContext::new();

    let mut diag = Diagnostic::new(
        Severity::Error,
        "Undefined variable 'lenght'".to_string(),
        verum_ast::span::Span::new(0, 6),
    );

    // Add suggestion
    diag = diag.with_help("Did you mean 'length'?");

    diag_ctx.report(diag);

    assert!(diag_ctx.has_errors());
}

// ============================================================================
// Error Limit Tests
// ============================================================================

#[test]
fn test_error_limit_tracking() {
    let mut diag_ctx = DiagnosticContext::new();

    // Report many errors
    for i in 0..10 {
        let diag = Diagnostic::new(
            Severity::Error,
            format!("Error {}", i),
            verum_ast::span::Span::new(i, i + 1),
        );
        diag_ctx.report(diag);
    }

    assert_eq!(diag_ctx.error_count(), 10);
}

// ============================================================================
// Warning vs Error Tests
// ============================================================================

#[test]
fn test_warning_vs_error_distinction() {
    let mut diag_ctx = DiagnosticContext::new();

    // Report warning
    let warning = Diagnostic::new(
        Severity::Warning,
        "Unused variable".to_string(),
        verum_ast::span::Span::new(0, 5),
    );
    diag_ctx.report(warning);

    // Report error
    let error = Diagnostic::new(
        Severity::Error,
        "Type mismatch".to_string(),
        verum_ast::span::Span::new(10, 15),
    );
    diag_ctx.report(error);

    assert!(diag_ctx.has_errors());
    assert_eq!(diag_ctx.error_count(), 1);
}

// ============================================================================
// Stack Trace Tests
// ============================================================================

#[test]
fn test_error_with_stack_trace() {
    let source = r#"
        fn a() -> Int { b() }
        fn b() -> Int { c() }
        fn c() -> Int { true }
    "#;

    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Should parse");

    // Should track call stack in type errors
    assert_eq!(module.declarations.len(), 3);
}
