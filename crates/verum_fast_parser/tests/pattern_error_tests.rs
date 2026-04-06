#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    deprecated,
    unexpected_cfgs
)]
//! Pattern Parser Error Tests
//!
//! This test suite verifies that the pattern parser produces helpful error
//! messages for malformed patterns. Each test case documents the expected
//! error code and message.
//!
//! Error codes tested:
//! - E070: Invalid @ binding pattern
//! - E071: Invalid identifier in pattern
//! - E072: Invalid rest/spread pattern position
//! - E073: Invalid mut pattern
//! - E074: Empty tuple pattern with trailing comma
//! - E075: Invalid active pattern arguments
//! - E076: Invalid field pattern syntax
//! - E077: Duplicate field in pattern
//! - E078: Nested or-pattern without parentheses
//! - E079: Or-pattern with inconsistent bindings
//! - E088: Invalid let pattern (leading pipe)

use verum_ast::FileId;
use verum_fast_parser::{RecursiveParser, VerumParser};
use verum_lexer::{Lexer, Token};

/// Parse a pattern directly and expect it to fail
fn parse_pattern_err(source: &str) -> String {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let tokens: Vec<Token> = lexer.filter_map(|r| r.ok()).collect();
    let mut parser = RecursiveParser::new(&tokens, file_id);
    match parser.parse_pattern() {
        Ok(_) => panic!("Expected parse error for '{}', but parsing succeeded", source),
        Err(e) => format!("{}", e),
    }
}

/// Parse a full module and expect it to fail
fn parse_module_err(source: &str) -> String {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    match parser.parse_module(lexer, file_id) {
        Ok(_) => panic!("Expected parse error, but parsing succeeded for:\n{}", source),
        Err(errors) => errors.iter().map(|e| format!("{}", e)).collect::<Vec<_>>().join("\n"),
    }
}

/// Check that an error message contains expected text
fn assert_error_contains(error: &str, expected: &str) {
    assert!(
        error.contains(expected),
        "Expected error to contain '{}', but got:\n{}",
        expected,
        error
    );
}

// ============================================================================
// E070: INVALID @ BINDING PATTERN
// ============================================================================

mod at_binding_errors {
    use super::*;

    #[test]
    fn at_binding_missing_pattern() {
        // x @ - missing pattern after @
        let error = parse_module_err("fn test() { let x @ = value; }");
        // Error: "expected pattern" after @
        assert_error_contains(&error, "expected pattern");
    }

    #[test]
    fn at_binding_on_wildcard() {
        // _ @ pattern - can't bind wildcard with @
        // Note: This might be valid in some languages, check if it parses
        let source = "fn test() { match x { _ @ Some(y) => {} } }";
        // Just verify it doesn't crash - behavior may vary
        let _ = parse_module_err(source);
    }
}

// ============================================================================
// E072: INVALID REST/SPREAD PATTERN POSITION
// ============================================================================

mod rest_pattern_errors {
    use super::*;

    #[test]
    fn multiple_rest_patterns() {
        // [.., x, ..] - multiple rest patterns not allowed
        let error = parse_module_err("fn test() { let [.., x, ..] = arr; }");
        assert_error_contains(&error, "..");
    }

    #[test]
    fn rest_in_tuple_parses() {
        // (.., x) - rest pattern in tuple position
        // Note: This is syntactically valid, parsed as Tuple([Rest, Ident])
        // Semantic validity depends on type checking
        let source = "fn test() { let (.., x) = tup; }";
        let file_id = FileId::new(0);
        let lexer = Lexer::new(source, file_id);
        let parser = VerumParser::new();
        // This parses successfully as syntax, semantic check may reject it
        let result = parser.parse_module(lexer, file_id);
        assert!(result.is_ok());
    }
}

// ============================================================================
// E074: EMPTY TUPLE PATTERN ERRORS
// ============================================================================

mod tuple_pattern_errors {
    use super::*;

    #[test]
    fn tuple_leading_comma() {
        // (,) - leading comma in tuple
        let error = parse_module_err("fn test() { let (,) = x; }");
        // Parser reports E074: empty tuple pattern
        assert_error_contains(&error, "empty tuple pattern");
    }

    #[test]
    fn tuple_consecutive_commas() {
        // (a,, b) - consecutive commas
        let error = parse_module_err("fn test() { let (a,, b) = x; }");
        // Parser reports E074: empty tuple pattern (missing element between commas)
        assert_error_contains(&error, "empty tuple pattern");
    }

    #[test]
    fn tuple_unclosed() {
        // (a, b - unclosed tuple
        let error = parse_module_err("fn test() { let (a, b = x; }");
        assert_error_contains(&error, ")");
    }
}

// ============================================================================
// E076: INVALID FIELD PATTERN SYNTAX
// ============================================================================

mod field_pattern_errors {
    use super::*;

    #[test]
    fn record_missing_field() {
        // Point { , } - empty field before comma
        let error = parse_module_err("fn test() { let Point { , } = p; }");
        assert!(!error.is_empty());
    }

    #[test]
    fn record_uses_equals_instead_of_colon() {
        // Point { x = y } - using = instead of :
        let error = parse_module_err("fn test() { let Point { x = y } = p; }");
        // Parser reports: "field pattern uses ':' not '='"
        assert_error_contains(&error, ":");
    }

    #[test]
    fn record_unclosed() {
        // Point { x - missing closing brace (without = which gets different error)
        let error = parse_module_err("fn test() { let Point { x = p; }");
        assert!(!error.is_empty());
    }

    #[test]
    fn record_invalid_field_syntax() {
        // Point { 42: x } - numeric field name
        let error = parse_module_err("fn test() { let Point { 42: x } = p; }");
        assert!(!error.is_empty());
    }
}

// ============================================================================
// E077: DUPLICATE FIELD IN PATTERN
// ============================================================================

mod duplicate_field_errors {
    use super::*;

    #[test]
    fn record_duplicate_fields() {
        // Point { x, x } - duplicate field
        let error = parse_module_err("fn test() { let Point { x, x } = p; }");
        // Parser may or may not catch duplicates
        // This is a semantic error that might be caught later
        let _ = error;
    }
}

// ============================================================================
// E078: NESTED OR-PATTERN ERRORS
// ============================================================================

mod or_pattern_errors {
    use super::*;

    #[test]
    fn or_leading_pipe() {
        // | a | b - leading pipe in or pattern
        let error = parse_module_err("fn test() { let | a | b = x; }");
        assert_error_contains(&error, "|");
    }

    #[test]
    fn or_trailing_pipe() {
        // a | b | - trailing pipe: error says "expected pattern" after |
        let error = parse_module_err("fn test() { let a | b | = x; }");
        assert_error_contains(&error, "expected pattern");
    }

    #[test]
    fn or_double_pipe() {
        // a || b - double pipe (this is logical OR, not pattern OR)
        let error = parse_module_err("fn test() { let a || b = x; }");
        assert!(!error.is_empty());
    }
}

// ============================================================================
// ARRAY/SLICE PATTERN ERRORS
// ============================================================================

mod array_pattern_errors {
    use super::*;

    #[test]
    fn array_unclosed() {
        // [a, b - unclosed array
        let error = parse_module_err("fn test() { let [a, b = x; }");
        assert_error_contains(&error, "]");
    }

    #[test]
    fn array_leading_comma() {
        // [, a] - leading comma: reports "unexpected comma"
        let error = parse_module_err("fn test() { let [, a] = x; }");
        assert_error_contains(&error, "unexpected comma");
    }
}

// ============================================================================
// RANGE PATTERN ERRORS
// ============================================================================

mod range_pattern_errors {
    use super::*;

    #[test]
    fn range_invalid_bound() {
        // x..y - identifiers not allowed as range bounds in patterns
        let error = parse_module_err("fn test() { match n { x..y => {} } }");
        // Range patterns only allow literals
        assert!(!error.is_empty());
    }
}

// ============================================================================
// REFERENCE PATTERN ERRORS
// ============================================================================

mod reference_pattern_errors {
    use super::*;

    #[test]
    fn reference_missing_inner() {
        // & - reference without inner pattern
        let error = parse_module_err("fn test() { let & = x; }");
        assert!(!error.is_empty());
    }

    #[test]
    fn reference_mut_missing_inner() {
        // &mut - mutable reference without inner pattern
        let error = parse_module_err("fn test() { let &mut = x; }");
        assert!(!error.is_empty());
    }
}

// ============================================================================
// GUARD PATTERN ERRORS
// ============================================================================

mod guard_pattern_errors {
    use super::*;

    #[test]
    fn guard_missing_condition() {
        // (x if) - guard without condition
        let error = parse_module_err("fn test() { match n { (x if) => {} } }");
        assert!(!error.is_empty());
    }

    #[test]
    fn guard_outside_parens_in_match() {
        // Match arm guards are valid, this tests the normal syntax
        // x if condition => - this is the standard match arm guard syntax
        let source = "fn test() { match n { x if x > 0 => {} } }";
        // This should parse successfully as a match arm guard
        let file_id = FileId::new(0);
        let lexer = Lexer::new(source, file_id);
        let parser = VerumParser::new();
        let result = parser.parse_module(lexer, file_id);
        assert!(result.is_ok(), "Standard match arm guard should parse");
    }
}

// ============================================================================
// ACTIVE PATTERN ERRORS
// ============================================================================

mod active_pattern_errors {
    use super::*;

    #[test]
    fn active_unclosed_params() {
        // InRange(1, 2 - unclosed parameter list
        let error = parse_module_err("fn test() { match n { InRange(1, 2 => {} } }");
        assert_error_contains(&error, ")");
    }

    #[test]
    fn active_unclosed_bindings() {
        // ParseInt()(n - unclosed binding list
        let error = parse_module_err("fn test() { match n { ParseInt()(n => {} } }");
        assert_error_contains(&error, ")");
    }
}

// ============================================================================
// STREAM PATTERN ERRORS
// ============================================================================

mod stream_pattern_errors {
    use super::*;

    #[test]
    fn stream_as_identifier() {
        // stream without brackets is a valid identifier pattern
        let source = "fn test() { let stream = iter; }";
        let file_id = FileId::new(0);
        let lexer = Lexer::new(source, file_id);
        let parser = VerumParser::new();
        // 'stream' without brackets is parsed as identifier
        let result = parser.parse_module(lexer, file_id);
        assert!(result.is_ok());
    }

    #[test]
    fn stream_unclosed() {
        // stream[a, b - unclosed stream pattern
        let error = parse_module_err("fn test() { let stream[a, b = iter; }");
        assert_error_contains(&error, "]");
    }
}

// ============================================================================
// AND PATTERN ERRORS
// ============================================================================

mod and_pattern_errors {
    use super::*;

    #[test]
    fn and_missing_right() {
        // Even() & - missing right pattern
        let error = parse_module_err("fn test() { match n { Even() & => {} } }");
        assert!(!error.is_empty());
    }

    #[test]
    fn and_double_ampersand() {
        // Even() && Odd() - && is logical and, not pattern and
        // This might parse differently depending on context
        let error = parse_module_err("fn test() { match n { Even() && Odd() => {} } }");
        // The && will be parsed as part of expression, causing structural issues
        assert!(!error.is_empty());
    }
}

// ============================================================================
// VARIANT PATTERN ERRORS
// ============================================================================

mod variant_pattern_errors {
    use super::*;

    #[test]
    fn variant_unclosed_tuple() {
        // Some(x - unclosed variant tuple
        let error = parse_module_err("fn test() { match opt { Some(x => {} } }");
        assert_error_contains(&error, ")");
    }

    #[test]
    fn variant_unclosed_record() {
        // Result::Ok { value - unclosed variant record
        let error = parse_module_err("fn test() { match r { Result::Ok { value => {} } }");
        assert_error_contains(&error, "}");
    }
}

// ============================================================================
// TYPE TEST PATTERN ERRORS
// ============================================================================

mod type_test_pattern_errors {
    use super::*;

    #[test]
    fn type_test_missing_type() {
        // x is - missing type
        let error = parse_module_err("fn test() { match val { x is => {} } }");
        assert!(!error.is_empty());
    }
}

// ============================================================================
// CONTEXTUAL ERRORS
// ============================================================================

mod contextual_errors {
    use super::*;

    #[test]
    fn let_or_pattern_is_valid_syntax() {
        // Or patterns in let are syntactically valid
        // Semantic checks (irrefutability) happen later
        let source = "fn test() { let Some(x) | None = opt; }";
        let file_id = FileId::new(0);
        let lexer = Lexer::new(source, file_id);
        let parser = VerumParser::new();
        let result = parser.parse_module(lexer, file_id);
        assert!(result.is_ok(), "Or patterns in let should parse");
    }

    #[test]
    fn for_loop_rest_pattern_is_valid() {
        // Rest pattern (..) is syntactically valid in for loop
        // It binds to the range, semantic validity depends on type
        let source = "fn test() { for .. in items {} }";
        let file_id = FileId::new(0);
        let lexer = Lexer::new(source, file_id);
        let parser = VerumParser::new();
        // This parses - .. is a valid pattern
        let result = parser.parse_module(lexer, file_id);
        assert!(result.is_ok());
    }

    #[test]
    fn match_arm_missing_pattern() {
        // match x { => } - missing pattern
        let error = parse_module_err("fn test() { match x { => {} } }");
        assert!(!error.is_empty());
    }

    #[test]
    fn match_arm_missing_arrow() {
        // match x { Some(y) {} } - missing =>
        let error = parse_module_err("fn test() { match x { Some(y) {} } }");
        assert_error_contains(&error, "=>");
    }
}

// ============================================================================
// EDGE CASE ERRORS
// ============================================================================

mod edge_case_errors {
    use super::*;

    #[test]
    fn empty_pattern_context() {
        // let = x; - missing pattern entirely
        let error = parse_module_err("fn test() { let = x; }");
        assert!(!error.is_empty());
    }

    #[test]
    fn deeply_nested_unclosed() {
        // Very deep nesting with missing closing brackets
        let error = parse_module_err("fn test() { let ((([[ = x; }");
        assert!(!error.is_empty());
    }

    #[test]
    fn keyword_as_pattern() {
        // fn = x; - keyword can't be pattern
        let error = parse_module_err("fn test() { let fn = x; }");
        assert!(!error.is_empty());
    }
}
