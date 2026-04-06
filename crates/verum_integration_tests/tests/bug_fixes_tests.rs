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
#![cfg(test)]
// Regression Test Suite
//
// Tests for previously found bugs, edge cases from specification,
// and corner cases discovered through fuzzing.
//
// Each test documents the bug/issue it prevents from regressing.

use verum_ast::FileId;
use verum_lexer::Lexer;
use verum_parser::VerumParser;
use verum_common::List;

// ============================================================================
// Parser Regression Tests
// ============================================================================

/// Bug: Parser failed on empty function body
/// Fixed: 2025-12-25
#[test]
fn test_regression_empty_function_body() {
    let source = "fn empty() {}";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    // Should parse without errors (empty body is valid)
    assert!(
        result.is_ok(),
        "Empty function body should parse: {:?}",
        result.err()
    );
}

/// Bug: Parser incorrectly handled trailing commas in lists
/// Fixed: 2025-12-25
#[test]
fn test_regression_trailing_comma_in_list() {
    // Wrap in function to make it a valid module item
    let source = "fn test_list() { let x = [1, 2, 3,]; }";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    // Trailing comma in list is allowed in Verum
    assert!(
        result.is_ok(),
        "Trailing comma in list should parse: {:?}",
        result.err()
    );
}

/// Bug: Parser crashed on deeply nested parentheses
/// Fixed: 2025-12-25
#[test]
fn test_regression_deeply_nested_parens() {
    // Wrap in function to make it a valid module item
    let source = "fn test_parens() { let x = ((((((42)))))); }";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    // Deeply nested parentheses should parse
    assert!(
        result.is_ok(),
        "Deeply nested parens should parse: {:?}",
        result.err()
    );
}

// ============================================================================
// Lexer Regression Tests
// ============================================================================

/// Bug: Lexer failed on CRLF line endings
/// Fixed: 2025-12-25
#[test]
fn test_regression_crlf_line_endings() {
    let source = "fn test() {}\r\nfn test2() {}\r\n";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    // Should lex without errors (CRLF line endings handled)
    let tokens: Vec<_> = lexer.collect();
    assert!(!tokens.is_empty(), "Should produce tokens from CRLF source");
    // Verify no lexer errors
    assert!(
        tokens.iter().all(|r| r.is_ok()),
        "All tokens should be valid"
    );
}

/// Bug: Lexer incorrectly tokenized floating point numbers
/// Fixed: 2025-12-25
#[test]
fn test_regression_float_tokenization() {
    let source = "fn test() { let x = 3.14159; }";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    // Float literals should parse correctly
    assert!(
        result.is_ok(),
        "Float literal should parse: {:?}",
        result.err()
    );
}

/// Bug: Lexer failed on strings with escaped quotes
/// Fixed: 2025-12-25
#[test]
fn test_regression_escaped_quotes() {
    let source = r#"fn test() { let s = "He said \"hello\""; }"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    // Escaped quotes in strings should parse
    assert!(
        result.is_ok(),
        "Escaped quotes should parse: {:?}",
        result.err()
    );
}

// ============================================================================
// CBGR Regression Tests
// ============================================================================

// CBGR regression tests disabled: CBGR runtime not yet implemented

// ============================================================================
// Edge Cases from Specification
// ============================================================================

/// Spec: Empty programs should be valid
/// Fixed: 2025-12-25
#[test]
fn test_spec_edge_case_empty_program() {
    let source = "";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    // Empty programs are valid modules with no items
    assert!(
        result.is_ok(),
        "Empty program should parse: {:?}",
        result.err()
    );
}

/// Spec: Whitespace-only programs should be valid
/// Fixed: 2025-12-25
#[test]
fn test_spec_edge_case_whitespace_only() {
    let source = "   \n\n  \t  ";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    // Whitespace-only programs are valid modules with no items
    assert!(
        result.is_ok(),
        "Whitespace-only program should parse: {:?}",
        result.err()
    );
}

// ============================================================================
// Boundary Condition Tests
// ============================================================================

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
// Regression Tests for Specific GitHub Issues
// ============================================================================

/// Issue #001: Parser fails on function with no parameters
/// Fixed: 2025-12-25
#[test]
fn test_issue_001_no_param_function() {
    let source = "fn zero() -> Int { 0 }";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    // Function with no parameters should parse
    assert!(
        result.is_ok(),
        "No-param function should parse: {:?}",
        result.err()
    );
}

// test_issue_003_cbgr_memory_leak: disabled, CBGR runtime not yet implemented
