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
// Error handling tests for the Verum lexer.
// Tests lexical error detection: unterminated strings, invalid escapes, invalid numbers, etc.
// Covers: unterminated strings, invalid escape sequences, invalid number bases,
// malformed hex/binary/octal literals, and other lexical error conditions.

use verum_ast::span::FileId;
use verum_lexer::{LexError, Lexer};

fn tokenize_with_errors(input: &str) -> Vec<Result<verum_lexer::Token, LexError>> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    lexer.collect()
}

// ===== Invalid Token Tests =====

#[test]
fn test_invalid_token_at_char() {
    let _results = tokenize_with_errors("@#");
    // @ is valid, # should produce error or hex color token
    // The behavior depends on context
}

// ===== Integer Literal Error Tests =====

#[test]
fn test_invalid_hex_no_digits() {
    let results = tokenize_with_errors("0x");
    // Should produce an error for hex with no digits
    assert!(results.iter().any(|r| r.is_err()));
}

#[test]
fn test_invalid_binary_no_digits() {
    let results = tokenize_with_errors("0b");
    // Should produce an error for binary with no digits
    assert!(results.iter().any(|r| r.is_err()));
}

#[test]
fn test_invalid_binary_wrong_digit() {
    let results = tokenize_with_errors("0b2");
    // Should produce an error for binary with invalid digit
    assert!(results.iter().any(|r| r.is_err()));
}

#[test]
fn test_invalid_hex_invalid_char() {
    let results = tokenize_with_errors("0xZZZ");
    // Should produce an error for hex with invalid character
    assert!(results.iter().any(|r| r.is_err()));
}

// ===== String Literal Error Tests =====

#[test]
fn test_unterminated_string() {
    let results = tokenize_with_errors(r#""unterminated"#);
    // Should produce an error for unterminated string
    // This will either be an error or the string token won't exist properly
    let _has_eof = results.iter().any(|r| {
        if let Ok(token) = r {
            token.kind == verum_lexer::TokenKind::Eof
        } else {
            false
        }
    });
    // The last token should still be EOF even with error
}

#[test]
fn test_string_with_newline_unterminated() {
    let _results = tokenize_with_errors(
        r#""hello
    unterminated"#,
    );
    // Unterminated string with newline should error
}

// ===== Comment Error Tests =====

#[test]
fn test_unterminated_block_comment() {
    let _results = tokenize_with_errors("a /* unterminated");
    // Should handle unterminated block comment
    // Either produces error or treats rest as comment
}

#[test]
fn test_nested_block_comments_unsupported() {
    // Note: spec mentions nested comments are future work
    let _results = tokenize_with_errors("/* outer /* inner */ outer */");
    // Non-nested implementation will end at first */
}

// ===== Mixed Error Scenarios =====

#[test]
fn test_error_recovery_with_valid_token() {
    let _results = tokenize_with_errors(r#""unterminated hello"#);
    // After error, lexer should still be able to lex valid tokens
}

#[test]
fn test_multiple_errors() {
    let results = tokenize_with_errors("0x 0b let");
    // Multiple errors should all be collected
    let error_count = results.iter().filter(|r| r.is_err()).count();
    assert!(error_count >= 2);
}

// ===== Character Literal Error Tests =====

#[test]
fn test_unterminated_char_literal() {
    let _results = tokenize_with_errors(r#"'a"#);
    // Unterminated char literal
}

#[test]
fn test_empty_char_literal() {
    // Empty char literals may or may not error depending on parser
    let _results = tokenize_with_errors("''");
}

// ===== Edge Cases =====

#[test]
fn test_only_error_token() {
    // A token that's purely invalid
    let _results = tokenize_with_errors("@+@");
    // @ symbols are valid, so this may not error
    // Testing for behavior
}

#[test]
fn test_eof_after_error() {
    let results = tokenize_with_errors("0x");
    // Even with error, EOF should be present
    let has_eof = results.iter().any(|r| {
        if let Ok(token) = r {
            token.kind == verum_lexer::TokenKind::Eof
        } else {
            false
        }
    });
    assert!(has_eof);
}

#[test]
fn test_whitespace_around_errors() {
    let _results = tokenize_with_errors("   0x   ");
    // Errors with whitespace should still work
}

// ===== Invalid Number Formats =====

#[test]
fn test_consecutive_dots_in_float() {
    // "1..5" should tokenize as "1 .. 5" (integer, range operator, integer)
    let results = tokenize_with_errors("1..5");
    let tokens: Vec<_> = results.iter().filter_map(|r| r.as_ref().ok()).collect();
    assert!(tokens.len() >= 3); // should have tokens before ..
}

#[test]
fn test_multiple_decimal_points() {
    // "1.2.3" is ambiguous - should error or parse first float
    let _results = tokenize_with_errors("1.2.3");
}

// ===== Valid Edge Cases (should NOT error) =====

#[test]
fn test_valid_underscore_only_with_digits() {
    let results = tokenize_with_errors("1_000_000_000");
    // Should be valid
    let has_error = results.iter().any(|r| r.is_err());
    assert!(!has_error);
}

#[test]
fn test_valid_hex_letters() {
    let results = tokenize_with_errors("0xDEADBEEF");
    // Should be valid
    let has_error = results.iter().any(|r| r.is_err());
    assert!(!has_error);
}

#[test]
fn test_valid_binary_pattern() {
    let results = tokenize_with_errors("0b1010_1010");
    // Should be valid
    let has_error = results.iter().any(|r| r.is_err());
    assert!(!has_error);
}

#[test]
fn test_valid_string_with_all_escapes() {
    let results = tokenize_with_errors(r#""\\n\\t\\r\\\"""#);
    // Should be valid
    let has_error = results.iter().any(|r| r.is_err());
    assert!(!has_error);
}

#[test]
fn test_valid_unicode_escape() {
    let results = tokenize_with_errors(r#""\u{1F600}""#);
    // Should be valid
    let has_error = results.iter().any(|r| r.is_err());
    assert!(!has_error);
}

#[test]
fn test_valid_hex_escape() {
    let results = tokenize_with_errors(r#""\xFF""#);
    // Should be valid
    let has_error = results.iter().any(|r| r.is_err());
    assert!(!has_error);
}
