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
//! Comprehensive tests for script parsing modes
//!
//! These tests verify the script parser, incremental parsing, recovery strategies,
//! and type integration work correctly in REPL and interactive contexts.

use verum_ast::FileId;
use verum_common::{List, Text};
use verum_lsp::{
    CachedLine,
    IncrementalScriptParser,
    IncrementalStats,
    ParseMode,
    RecoveryResult,
    ScriptContext,
    ScriptParseResult,
    ScriptParser,
    ScriptRecovery,
    // NOTE: ScriptTypeChecker, TypeCheckResult, TypeInfo are disabled due to circular dependency
    // They should be moved to a higher-level crate that can depend on both verum_parser and verum_types
    explain_error,
    needs_continuation,
    suggest_autocompletion,
    suggest_completion,
};
use verum_parser::ParseError;

// ====================
// Basic Script Parsing
// ====================

#[test]
fn test_parse_simple_expression() {
    let parser = ScriptParser::new();
    let mut ctx = ScriptContext::new();
    let file_id = FileId::new(1);

    let result = parser.parse_line("42 + 10", file_id, &mut ctx);
    assert!(result.is_ok());
    assert!(matches!(result.unwrap(), ScriptParseResult::Expression(_)));
}

#[test]
fn test_parse_let_binding() {
    let parser = ScriptParser::new();
    let mut ctx = ScriptContext::new();
    let file_id = FileId::new(1);

    let result = parser.parse_line("let x = 42", file_id, &mut ctx);
    assert!(result.is_ok());

    // x should be tracked in context
    assert!(ctx.bindings.contains_key(&Text::from("x")));
}

#[test]
fn test_parse_function_definition() {
    let parser = ScriptParser::new();
    let mut ctx = ScriptContext::new();
    let file_id = FileId::new(1);

    let result = parser.parse_line("fn add(a: Int, b: Int) -> Int { a + b }", file_id, &mut ctx);
    assert!(result.is_ok());
    assert!(matches!(result.unwrap(), ScriptParseResult::Item(_)));

    // Function should be tracked
    assert!(ctx.bindings.contains_key(&Text::from("add")));
}

#[test]
fn test_empty_input() {
    let parser = ScriptParser::new();
    let mut ctx = ScriptContext::new();
    let file_id = FileId::new(1);

    let result = parser.parse_line("   ", file_id, &mut ctx);
    assert!(result.is_ok());
    assert!(matches!(result.unwrap(), ScriptParseResult::Empty));
}

// ====================
// Multiline Input
// ====================

#[test]
fn test_incomplete_function() {
    let parser = ScriptParser::new();
    let mut ctx = ScriptContext::new();
    let file_id = FileId::new(1);

    let result = parser.parse_line("fn test() {", file_id, &mut ctx);
    assert!(result.is_ok());
    assert!(matches!(result.unwrap(), ScriptParseResult::Incomplete(_)));

    // Context should track incomplete state
    assert!(!ctx.is_complete());
}

#[test]
fn test_multiline_function() {
    let parser = ScriptParser::new();
    let mut ctx = ScriptContext::new();
    let file_id = FileId::new(1);

    // Line 1: function start
    let r1 = parser.parse_line("fn factorial(n: Int) -> Int {", file_id, &mut ctx);
    assert!(matches!(r1.unwrap(), ScriptParseResult::Incomplete(_)));

    // Line 2: body
    let r2 = parser.parse_line("    n * factorial(n - 1)", file_id, &mut ctx);
    assert!(matches!(r2.unwrap(), ScriptParseResult::Incomplete(_)));

    // Line 3: closing brace
    let r3 = parser.parse_line("}", file_id, &mut ctx);
    assert!(r3.is_ok());
    // Should parse as complete function
    if let Ok(ScriptParseResult::Item(_)) = r3 {
        // Success
    } else {
        panic!("Expected complete item");
    }
}

#[test]
fn test_nested_braces() {
    let parser = ScriptParser::new();
    let mut ctx = ScriptContext::new();
    let file_id = FileId::new(1);

    ctx.add_line("let map = {");
    assert!(!ctx.is_complete());
    assert_eq!(ctx.get_brace_depth(), 1);

    ctx.add_line("    'key': {");
    assert_eq!(ctx.get_brace_depth(), 2);

    ctx.add_line("        'value': 42");
    ctx.add_line("    }");
    assert_eq!(ctx.get_brace_depth(), 1);

    ctx.add_line("}");
    assert!(ctx.is_complete());
}

// ====================
// Parse Modes
// ====================

#[test]
fn test_expression_mode() {
    let parser = ScriptParser::new();
    let mut ctx = ScriptContext::new();
    let file_id = FileId::new(1);

    let result = parser.parse_with_mode("1 + 2", file_id, ParseMode::Expression, &mut ctx);
    assert!(result.is_ok());
    assert!(matches!(result.unwrap(), ScriptParseResult::Expression(_)));
}

#[test]
fn test_auto_mode_precedence() {
    let parser = ScriptParser::new();
    let mut ctx = ScriptContext::new();
    let file_id = FileId::new(1);

    // Should parse as expression first
    let result = parser.parse_with_mode("42", file_id, ParseMode::Auto, &mut ctx);
    assert!(matches!(result.unwrap(), ScriptParseResult::Expression(_)));
}

// ====================
// Incremental Parsing
// ====================

#[test]
fn test_incremental_cache_hit() {
    let mut parser = IncrementalScriptParser::new();
    let file_id = FileId::new(1);

    // First parse
    parser.parse_line("let x = 42", 1, file_id).unwrap();
    let _initial_misses = parser.stats().cache_misses;

    // Second parse of same line - should hit cache
    parser.parse_line("let x = 42", 1, file_id).unwrap();
    assert_eq!(parser.stats().cache_hits, 1);
}

#[test]
fn test_incremental_cache_invalidation() {
    let mut parser = IncrementalScriptParser::new();
    let file_id = FileId::new(1);

    parser.parse_line("let x = 1", 1, file_id).unwrap();
    parser.parse_line("let y = 2", 2, file_id).unwrap();
    parser.parse_line("let z = 3", 3, file_id).unwrap();

    // Update line 2
    parser.update_line("let y = 100", 2, file_id).unwrap();

    // Lines 2 and 3 should be invalidated
    // Line 1 should still be cached
    assert!(parser.is_cached(1));
}

#[test]
fn test_incremental_prewarm() {
    let mut parser = IncrementalScriptParser::new();
    let file_id = FileId::new(1);

    let lines = vec!["let a = 1", "let b = 2", "let c = 3"];
    parser.prewarm(&lines, file_id).unwrap();

    assert_eq!(parser.stats().cached_lines, 3);
    assert!(parser.is_cached(1));
    assert!(parser.is_cached(2));
    assert!(parser.is_cached(3));
}

#[test]
fn test_incremental_cache_limit() {
    let mut parser = IncrementalScriptParser::with_cache_limit(2);
    let file_id = FileId::new(1);

    parser.parse_line("let a = 1", 1, file_id).unwrap();
    parser.parse_line("let b = 2", 2, file_id).unwrap();
    parser.parse_line("let c = 3", 3, file_id).unwrap();

    // Should have evicted oldest
    assert_eq!(parser.stats().cached_lines, 2);
}

#[test]
fn test_incremental_session_persistence() {
    let mut parser1 = IncrementalScriptParser::new();
    let file_id = FileId::new(1);

    parser1.parse_line("let value = 42", 1, file_id).unwrap();

    // Export context
    let ctx = parser1.export_context();

    // Import into new parser
    let mut parser2 = IncrementalScriptParser::new();
    parser2.import_context(ctx);

    // Should have the binding
    assert!(
        parser2
            .context()
            .bindings
            .contains_key(&Text::from("value"))
    );
}

// ====================
// Error Recovery
// ====================

#[test]
fn test_recovery_incomplete_detection() {
    let recovery = ScriptRecovery::new();
    let mut ctx = ScriptContext::new();
    ctx.add_line("if x > 0 {");

    let error =
        ParseError::unexpected_eof(&[verum_lexer::TokenKind::RBrace], verum_ast::Span::dummy());
    let result = recovery.recover(&error, &ctx);

    assert!(matches!(result, RecoveryResult::Incomplete { .. }));
}

#[test]
fn test_recovery_similar_identifier() {
    let recovery = ScriptRecovery::new();
    let mut ctx = ScriptContext::new();
    ctx.add_binding(Text::from("value"), Text::from("Int"));

    // Test through the autocompletion API which uses identifier matching
    let suggestions = suggest_autocompletion("valu", &ctx);
    assert!(!suggestions.is_empty());
    assert_eq!(suggestions[0].0.as_str(), "value");
}

#[test]
fn test_recovery_missing_delimiters() {
    let recovery = ScriptRecovery::new();
    let mut ctx = ScriptContext::new();
    ctx.add_line("fn test() {");

    let suggestions = recovery.suggest_missing_delimiters(&ctx);
    assert!(!suggestions.is_empty());
    assert!(suggestions[0].as_str().contains("brace"));
}

#[test]
fn test_explain_error_helpful_message() {
    let mut ctx = ScriptContext::new();
    ctx.add_line("fn incomplete() {");

    let error = ParseError::unexpected_eof(&[], verum_ast::Span::dummy());
    let explanation = explain_error(&error, &ctx);

    assert!(
        explanation.as_str().contains("incomplete") || explanation.as_str().contains("expected")
    );
}

// ====================
// Autocompletion
// ====================

#[test]
fn test_suggest_completion_bindings() {
    let mut ctx = ScriptContext::new();
    ctx.add_binding(Text::from("value"), Text::from("Int"));
    ctx.add_binding(Text::from("variable"), Text::from("Text"));

    let suggestions = suggest_completion("val", &ctx);
    assert!(!suggestions.is_empty());

    let names: Vec<&str> = suggestions.iter().map(|t| t.as_str()).collect();
    assert!(names.contains(&"value"));
}

#[test]
fn test_suggest_completion_keywords() {
    let ctx = ScriptContext::new();

    let suggestions = suggest_completion("le", &ctx);
    let names: Vec<&str> = suggestions.iter().map(|t| t.as_str()).collect();
    assert!(names.contains(&"let"));
}

#[test]
fn test_suggest_autocompletion_with_types() {
    let mut ctx = ScriptContext::new();
    ctx.add_binding(Text::from("count"), Text::from("Int"));

    let suggestions = suggest_autocompletion("co", &ctx);
    assert!(!suggestions.is_empty());

    // Should include type information
    let (name, _ty) = &suggestions[0];
    assert_eq!(name.as_str(), "count");
}

// ====================
// Helper Functions
// ====================

#[test]
fn test_needs_continuation_function() {
    assert!(needs_continuation("fn test() {"));
    assert!(needs_continuation("let arr = ["));
    assert!(needs_continuation("match x {"));
    assert!(!needs_continuation("let x = 42"));
}

#[test]
fn test_is_complete() {
    let parser = ScriptParser::new();

    assert!(parser.is_complete("let x = 42"));
    assert!(parser.is_complete("fn test() { }"));
    assert!(!parser.is_complete("fn test() {"));
    assert!(!parser.is_complete("let arr = ["));
}

// ====================
// Type Integration
// ====================
// NOTE: Type integration tests are disabled because ScriptTypeChecker is in the disabled
// script_type_integration module (to break circular dependency with verum_types).
// These tests should be re-enabled when the module is moved to a higher-level crate.

/*
#[test]
fn test_type_checker_literal_inference() {
    let mut checker = ScriptTypeChecker::new();
    let file_id = FileId::new(1);

    let result = checker.check_line("42", 1, file_id).unwrap();
    assert!(matches!(result.type_info, TypeInfo::Concrete(_)));
    assert_eq!(result.type_info.display().as_str(), "Int");
}

#[test]
fn test_type_checker_binary_op() {
    let mut checker = ScriptTypeChecker::new();
    let file_id = FileId::new(1);

    let result = checker.check_line("10 + 20", 1, file_id).unwrap();
    assert_eq!(result.type_info.display().as_str(), "Int");

    let result = checker.check_line("5 > 3", 2, file_id).unwrap();
    assert_eq!(result.type_info.display().as_str(), "Bool");
}

#[test]
fn test_type_checker_array_inference() {
    let mut checker = ScriptTypeChecker::new();
    let file_id = FileId::new(1);

    let result = checker.check_line("[1, 2, 3]", 1, file_id).unwrap();
    match result.type_info {
        TypeInfo::Generic { ref base, .. } => {
            assert_eq!(base.as_str(), "List");
        }
        _ => panic!("Expected generic List type"),
    }
}

#[test]
fn test_type_checker_function_signature() {
    let mut checker = ScriptTypeChecker::new();
    let file_id = FileId::new(1);

    checker
        .check_line("fn add(a: Int, b: Int) -> Int { a + b }", 1, file_id)
        .unwrap();

    // Function should be in type environment
    let ty = checker.get_type("add");
    assert!(ty.is_some());

    if let verum_common::Maybe::Some(TypeInfo::Function { .. }) = ty {
        // Expected
    } else {
        panic!("Expected function type");
    }
}

#[test]
fn test_type_checker_disabled() {
    let mut checker = ScriptTypeChecker::parse_only();
    let file_id = FileId::new(1);

    let result = checker.check_line("42", 1, file_id).unwrap();
    assert!(matches!(result.type_info, TypeInfo::Unknown));
}

#[test]
fn test_type_checker_tuple() {
    let mut checker = ScriptTypeChecker::new();
    let file_id = FileId::new(1);

    let result = checker.check_line("(42, true)", 1, file_id).unwrap();
    match result.type_info {
        TypeInfo::Generic { ref base, ref params } => {
            assert_eq!(base.as_str(), "Tuple");
            assert_eq!(params.len(), 2);
        }
        _ => panic!("Expected tuple type"),
    }
}

#[test]
fn test_type_environment_persistence() {
    let mut checker = ScriptTypeChecker::new();
    let file_id = FileId::new(1);

    // Define multiple bindings
    checker.check_line("let x = 42", 1, file_id).unwrap();
    checker
        .check_line("fn double(n: Int) -> Int { n * 2 }", 2, file_id)
        .unwrap();

    // Both should be in environment
    assert!(checker.get_type("x").is_some());
    assert!(checker.get_type("double").is_some());

    let all_types = checker.get_all_types();
    assert!(all_types.len() >= 2);
}
*/

// ====================
// Integration Tests
// ====================

/*
#[test]
fn test_repl_session_simulation() {
    let mut checker = ScriptTypeChecker::new();
    let file_id = FileId::new(1);

    // Simulate a REPL session
    checker.check_line("let x = 42", 1, file_id).unwrap();
    checker.check_line("let y = x + 10", 2, file_id).unwrap();
    checker.check_line("y * 2", 3, file_id).unwrap();

    // All should have been parsed and type-checked
    let stats = checker.stats();
    assert!(stats.as_str().contains("bindings"));
}
*/

#[test]
fn test_incremental_repl_with_updates() {
    let mut parser = IncrementalScriptParser::new();
    let file_id = FileId::new(1);

    // Initial commands
    parser.parse_line("let x = 1", 1, file_id).unwrap();
    parser.parse_line("let y = 2", 2, file_id).unwrap();
    parser.parse_line("x + y", 3, file_id).unwrap();

    // Update middle line
    parser.update_line("let y = 100", 2, file_id).unwrap();

    // Cache should be partially invalidated
    assert!(parser.is_cached(1)); // Line 1 unchanged
}

#[test]
fn test_error_recovery_in_session() {
    let mut ctx = ScriptContext::new();
    ctx.add_binding(Text::from("count"), Text::from("Int"));

    // Autocompletion should suggest similar identifier
    let suggestions = suggest_autocompletion("cou", &ctx);
    assert!(!suggestions.is_empty());
    assert_eq!(suggestions[0].0.as_str(), "count");
}

#[test]
fn test_completion_context_aware() {
    let mut ctx = ScriptContext::new();
    ctx.add_binding(Text::from("customer_id"), Text::from("Int"));
    ctx.add_binding(Text::from("customer_name"), Text::from("Text"));

    let suggestions = suggest_completion("cust", &ctx);
    assert!(suggestions.len() >= 2);

    let names: Vec<&str> = suggestions.iter().map(|t| t.as_str()).collect();
    assert!(names.contains(&"customer_id"));
    assert!(names.contains(&"customer_name"));
}
