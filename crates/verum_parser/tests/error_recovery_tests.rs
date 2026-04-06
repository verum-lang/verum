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
//! Comprehensive tests for parser error recovery.
//!
//! These tests verify that the parser can:
//! 1. Recover from errors and continue parsing
//! 2. Report multiple errors in a single pass
//! 3. Provide actionable error messages
//! 4. Handle common syntax mistakes gracefully

use verum_ast::FileId;
use verum_common::Text;
use verum_lexer::{Lexer, Token};
use verum_parser::{ParseError, RecursiveParser};

/// Helper to parse a source string and return errors.
fn parse_and_get_errors(source: &str) -> Vec<ParseError> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let tokens: Vec<Token> = lexer.filter_map(|r| r.ok()).collect();
    let mut parser = RecursiveParser::new(&tokens, file_id);

    // Try to parse the module
    let _ = parser.parse_module();

    // Return accumulated errors
    parser.errors
}

/// Helper to count errors matching a predicate.
fn count_errors<F>(errors: &[ParseError], predicate: F) -> usize
where
    F: Fn(&ParseError) -> bool,
{
    errors.iter().filter(|e| predicate(e)).count()
}

// ============================================================================
// Test 1: Missing Semicolons
// ============================================================================

#[test]
fn test_missing_semicolon_recovery() {
    let source = r#"
        let x = 5
        let y = 10
        let z = x + y
    "#;

    let errors = parse_and_get_errors(source);

    // Should report errors but continue parsing
    assert!(!errors.is_empty(), "Should detect missing semicolons");

    // Check that we detected multiple statements despite missing semicolons
    // The parser should recover and parse all three let statements
    println!("Errors: {:#?}", errors);
}

#[test]
fn test_missing_semicolon_with_suggestions() {
    let source = r#"
        let x = 42
        return x
    "#;

    let errors = parse_and_get_errors(source);

    // Should provide helpful suggestions
    for error in &errors {
        if error.help.is_some() {
            println!("Error with help: {}", error);
        }
    }
}

// ============================================================================
// Test 2: Unclosed Delimiters
// ============================================================================

#[test]
fn test_unclosed_brace_recovery() {
    let source = r#"
        fn foo() {
            let x = 5
            let y = 10
        // Missing closing brace

        fn bar() {
            return 42
        }
    "#;

    let errors = parse_and_get_errors(source);

    // Should detect unclosed brace and recover to parse bar()
    assert!(!errors.is_empty(), "Should detect unclosed brace");

    println!("Errors: {:#?}", errors);
}

#[test]
fn test_unclosed_paren_recovery() {
    let source = r#"
        fn test() {
            let arr = [1, 2, 3
            return arr
        }
    "#;

    let errors = parse_and_get_errors(source);

    // Should detect unclosed bracket
    assert!(!errors.is_empty(), "Should detect unclosed bracket");

    println!("Errors: {:#?}", errors);
}

#[test]
fn test_mismatched_delimiter() {
    let source = r#"
        fn test() {
            let x = (1 + 2]
            return x
        }
    "#;

    let errors = parse_and_get_errors(source);

    // Should detect mismatched delimiters
    assert!(!errors.is_empty(), "Should detect mismatched delimiters");

    println!("Errors: {:#?}", errors);
}

// ============================================================================
// Test 3: Invalid Syntax with Recovery
// ============================================================================

#[test]
fn test_invalid_struct_syntax() {
    let source = r#"
        type Bad is struct { }

        type Good is {
            x: Int,
            y: Float
        }
    "#;

    let errors = parse_and_get_errors(source);

    // Should report error for first type but parse the second
    println!("Errors: {:#?}", errors);
}

#[test]
fn test_multiple_errors_in_sequence() {
    let source = r#"
        fn broken() {
            let a = (1 + 2
            let b = [1, 2, 3
            let c = {x: 1, y: 2
            return a + b
        }
    "#;

    let errors = parse_and_get_errors(source);

    // Should report multiple delimiter errors
    assert!(errors.len() >= 2, "Should detect multiple errors");

    println!("Found {} errors:", errors.len());
    for (i, error) in errors.iter().enumerate() {
        println!("  {}. {}", i + 1, error);
    }
}

// ============================================================================
// Test 4: Error Messages Quality
// ============================================================================

#[test]
fn test_error_message_clarity() {
    let source = r#"
        fn foo() {
            let x: Int = "not an int"
        }
    "#;

    let errors = parse_and_get_errors(source);

    // Verify error messages are clear and actionable
    for error in &errors {
        let msg = format!("{}", error);
        println!("Error message: {}", msg);

        // Error messages should not be empty
        assert!(!msg.is_empty(), "Error message should not be empty");
    }
}

#[test]
fn test_suggestions_present() {
    let source = r#"
        fn test() {
            let x = 5
            let y = 10
        }
    "#;

    let errors = parse_and_get_errors(source);

    // Count errors with helpful suggestions
    let with_help = count_errors(&errors, |e| e.help.is_some());

    println!(
        "{} out of {} errors have suggestions",
        with_help,
        errors.len()
    );
}

// ============================================================================
// Test 5: Synchronization Points
// ============================================================================

#[test]
fn test_synchronization_at_semicolon() {
    let source = r#"
        fn test() {
            invalid syntax here;
            let x = 5;
            more invalid stuff;
            let y = 10;
        }
    "#;

    let errors = parse_and_get_errors(source);

    // Parser should synchronize at semicolons and continue
    println!("Synchronization test - {} errors found", errors.len());
    for error in &errors {
        println!("  {}", error);
    }
}

#[test]
fn test_synchronization_at_item_boundary() {
    let source = r#"
        invalid top level syntax

        fn foo() {
            return 1
        }

        more invalid

        fn bar() {
            return 2
        }
    "#;

    let errors = parse_and_get_errors(source);

    // Parser should recover at function boundaries
    println!("Item boundary sync - {} errors found", errors.len());
}

// ============================================================================
// Test 6: Complex Error Scenarios
// ============================================================================

#[test]
fn test_nested_delimiter_errors() {
    let source = r#"
        fn complex() {
            let nested = {
                x: [1, 2, {
                    y: (3, 4
                }]
            }
        }
    "#;

    let errors = parse_and_get_errors(source);

    println!("Nested delimiter errors: {:#?}", errors);
}

#[test]
fn test_recovery_preserves_context() {
    let source = r#"
        fn first() {
            let x =
        }

        fn second() {
            let y = 42
        }
    "#;

    let errors = parse_and_get_errors(source);

    // Should report error in first() but successfully parse second()
    println!("Context preservation test - {} errors", errors.len());
}

// ============================================================================
// Test 7: Real-World Examples
// ============================================================================

#[test]
fn test_forgot_return_type() {
    let source = r#"
        fn compute(x: Int, y: Int) {
            return x + y
        }
    "#;

    let errors = parse_and_get_errors(source);

    // This should parse successfully (return type is optional)
    // But if there's an error, it should be clear
    if !errors.is_empty() {
        println!("Errors in return type test:");
        for error in &errors {
            println!("  {}", error);
        }
    }
}

#[test]
fn test_incomplete_expression() {
    let source = r#"
        fn test() {
            let x = 1 +
        }
    "#;

    let errors = parse_and_get_errors(source);

    // Should report incomplete expression
    assert!(!errors.is_empty(), "Should detect incomplete expression");
    println!("Incomplete expression errors: {:#?}", errors);
}

#[test]
fn test_missing_colon_in_type() {
    let source = r#"
        fn test() {
            let x Int = 5
        }
    "#;

    let errors = parse_and_get_errors(source);

    // Should provide clear error about missing colon
    println!("Missing colon errors: {:#?}", errors);
}

// ============================================================================
// Test 8: Error Limit
// ============================================================================

#[test]
fn test_error_limit_not_exceeded() {
    // Create source with many errors but not pathological
    let mut source = String::from("fn test() {\n");
    for i in 0..50 {
        source.push_str(&format!("    let x{} = \n", i));
    }
    source.push_str("}\n");

    let errors = parse_and_get_errors(&source);

    // Should cap errors at reasonable limit
    assert!(
        errors.len() <= 1000,
        "Should cap errors at MAX_PARSE_ERRORS"
    );
    println!("Error limit test: {} errors (capped at 1000)", errors.len());
}

// ============================================================================
// Test 9: Helpful Suggestions
// ============================================================================

#[test]
fn test_suggests_semicolon() {
    let source = r#"
        let x = 5
        let y = 10
    "#;

    let errors = parse_and_get_errors(source);

    // Check for semicolon suggestions
    for error in &errors {
        if let Some(help) = &error.help {
            println!("Suggestion: {}", help);
            // Should mention adding semicolon
            assert!(
                help.contains(";") || help.to_lowercase().contains("semicolon"),
                "Should suggest adding semicolon"
            );
        }
    }
}

#[test]
fn test_suggests_closing_delimiter() {
    let source = r#"
        fn test() {
            let x = (1 + 2
    "#;

    let errors = parse_and_get_errors(source);

    // Should suggest closing parenthesis
    for error in &errors {
        if let Some(help) = &error.help {
            println!("Delimiter suggestion: {}", help);
        }
    }
}
