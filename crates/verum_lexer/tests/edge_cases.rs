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
//! Edge case tests for verum_lexer
//!
//! Tests boundary conditions and unusual inputs for lexer robustness.

use verum_ast::span::FileId;
use verum_lexer::{Lexer, Token, TokenKind};

// ============================================================================
// Numeric Edge Cases
// ============================================================================

#[test]
fn test_zero_in_all_bases() {
    let test_cases = vec![("0", 0i64), ("0x0", 0), ("0o0", 0), ("0b0", 0)];

    for (input, expected) in test_cases {
        let lexer = Lexer::new(input, FileId::new(0));
        let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

        if let TokenKind::Integer(lit) = &tokens[0].kind {
            assert_eq!(lit.as_i64().unwrap(), expected, "Failed for input: {}", input);
        }
    }
}

#[test]
fn test_float_edge_cases() {
    let test_cases = vec![
        "0.0", "1.0", ".5",   // Leading decimal point
        "5.",   // Trailing decimal point
        "1e0",  // Scientific notation with zero exponent
        "1e+0", // Explicit positive exponent
        "1e-0", // Negative zero exponent
        "inf",  // Infinity (if supported)
        "nan",  // NaN (if supported)
    ];

    for input in test_cases {
        let lexer = Lexer::new(input, FileId::new(0));
        let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

        // Should parse without error
        assert!(!tokens.is_empty(), "Failed to parse: {}", input);
    }
}

#[test]
fn test_integer_with_many_underscores() {
    let input = "1_2_3_4_5_6_7_8_9_0";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    assert!(matches!(tokens[0].kind, TokenKind::Integer(_)));
}

#[test]
fn test_leading_zeros() {
    let test_cases = vec!["00", "007", "0000042"];

    for input in test_cases {
        let lexer = Lexer::new(input, FileId::new(0));
        let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

        // Should parse (might be octal or error depending on spec)
        assert!(!tokens.is_empty(), "Failed for: {}", input);
    }
}

// ============================================================================
// String Edge Cases
// ============================================================================

#[test]
fn test_empty_string() {
    let input = r#""""#;
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    assert!(matches!(tokens[0].kind, TokenKind::Text(_)));
}

#[test]
fn test_string_with_only_escapes() {
    let input = r#""\n\t\r\0""#;
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    assert!(matches!(tokens[0].kind, TokenKind::Text(_)));
}

#[test]
fn test_string_with_escaped_quotes() {
    let input = r#""\"quoted\"""#;
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    assert!(matches!(tokens[0].kind, TokenKind::Text(_)));
}

#[test]
fn test_raw_multiline_literals() {
    // Test raw multiline strings using """...""" syntax
    // NOTE: r#"..."# syntax removed in simplified literal architecture
    let test_cases = vec![
        r#""""raw""""#,
        r#""""with backslash \n""""#,
        r#""""with embedded "quotes" inside""""#,
    ];

    for input in test_cases {
        let lexer = Lexer::new(input, FileId::new(0));
        let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

        assert!(!tokens.is_empty(), "Failed for: {}", input);
    }
}

#[test]
fn test_multiline_string() {
    let input = r#""line1
line2
line3""#;
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    assert!(matches!(
        tokens[0].kind,
        TokenKind::Text(_) | TokenKind::Error
    ));
}

// ============================================================================
// Identifier Edge Cases
// ============================================================================

#[test]
fn test_single_character_identifiers() {
    let input = "a b c x y z _ _a";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    // Count only identifier tokens
    let ident_count = tokens
        .iter()
        .filter(|t| matches!(t.kind, TokenKind::Ident(_)))
        .count();
    assert!(
        ident_count >= 8,
        "Expected at least 8 identifiers, found {}",
        ident_count
    );
}

#[test]
fn test_identifier_with_numbers() {
    let input = "var1 x2y3z test123 _42";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    let ident_count = tokens
        .iter()
        .filter(|t| matches!(t.kind, TokenKind::Ident(_)))
        .count();
    assert!(
        ident_count >= 4,
        "Expected at least 4 identifiers, found {}",
        ident_count
    );
}

#[test]
fn test_identifier_starting_with_underscore() {
    let input = "_ _x __private ___";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    let ident_count = tokens
        .iter()
        .filter(|t| matches!(t.kind, TokenKind::Ident(_)))
        .count();
    assert!(
        ident_count >= 4,
        "Expected at least 4 identifiers, found {}",
        ident_count
    );
}

#[test]
fn test_raw_identifiers() {
    // If the language supports raw identifiers (like r#type in Rust)
    let input = "r#type r#match r#fn";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.filter_map(|r| r.ok()).collect();

    // Should handle raw identifiers (if supported)
    assert!(!tokens.is_empty());
}

// ============================================================================
// Operator Edge Cases
// ============================================================================

#[test]
fn test_operator_ambiguity() {
    let test_cases = vec![
        "<<<=", // << <= or <<< =
        ">>>",  // >> > or >>>
        "===",  // == = or ===
        "!==",  // != = or !==
    ];

    for input in test_cases {
        let lexer = Lexer::new(input, FileId::new(0));
        let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

        // Should parse without panic
        assert!(!tokens.is_empty(), "Failed for: {}", input);
    }
}

#[test]
fn test_no_whitespace_between_operators() {
    let input = "x+-y*z/w%m";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    // Should correctly separate operators
    assert!(tokens.len() > 5);
}

#[test]
fn test_dot_ambiguity() {
    let test_cases = vec![
        "3.14", // Float
        "x.y",  // Member access
        "...",  // Range or spread
        "..",   // Range
    ];

    for input in test_cases {
        let lexer = Lexer::new(input, FileId::new(0));
        let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

        assert!(!tokens.is_empty(), "Failed for: {}", input);
    }
}

// ============================================================================
// Comment Edge Cases
// ============================================================================

#[test]
fn test_comment_at_eof() {
    let input = "fn main() {} // comment at end";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    // Should handle comment at EOF
    assert!(tokens.last().unwrap().kind == TokenKind::Eof);
}

#[test]
fn test_block_comment_without_closing() {
    let input = "fn main() { /* unclosed";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.filter_map(|r| r.ok()).collect();

    // Should handle unclosed block comment (may produce error)
    assert!(!tokens.is_empty());
}

#[test]
fn test_doc_comments() {
    let test_cases = vec![
        "/// Documentation",
        "//! Module docs",
        "/** Block doc */",
        "/*! Block module doc */",
    ];

    for input in test_cases {
        let lexer = Lexer::new(input, FileId::new(0));
        let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

        // Should recognize doc comments
        assert!(!tokens.is_empty(), "Failed for: {}", input);
    }
}

#[test]
fn test_comment_with_special_chars() {
    let input = "// Comment with special chars: !@#$%^&*()";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    assert!(!tokens.is_empty());
}

// ============================================================================
// Whitespace Edge Cases
// ============================================================================

#[test]
fn test_all_whitespace_types() {
    let input = " \t\n\r\x0C"; // space, tab, newline, carriage return, form feed
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.filter_map(|r| r.ok()).collect();

    // Should handle all whitespace
    assert_eq!(tokens.last().unwrap().kind, TokenKind::Eof);
}

#[test]
fn test_crlf_line_endings() {
    let input = "fn\r\nmain\r\n{\r\n}\r\n";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    assert!(tokens.len() >= 4); // fn, main, {, }
}

#[test]
fn test_mixed_line_endings() {
    let input = "fn\nmain\r\n{\r}";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    assert!(tokens.len() >= 4);
}

// ============================================================================
// Token Boundary Edge Cases
// ============================================================================

#[test]
fn test_no_space_between_tokens() {
    let input = "fn(){}[]";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    // Should correctly identify all 7 tokens (fn, (, ), {, }, [, ])
    assert!(tokens.len() >= 7);
}

#[test]
fn test_keyword_prefix_identifier() {
    let input = "fnord function letx ifelse";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    // All should be identifiers (not keywords)
    let ident_count = tokens
        .iter()
        .filter(|t| matches!(t.kind, TokenKind::Ident(_)))
        .count();
    assert!(
        ident_count >= 4,
        "Expected at least 4 identifiers, found {}",
        ident_count
    );
}

#[test]
fn test_underscore_keyword_confusion() {
    let input = "_ __ _fn fn_ _42";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    // All should be identifiers
    let ident_count = tokens
        .iter()
        .filter(|t| matches!(t.kind, TokenKind::Ident(_)))
        .count();
    assert!(
        ident_count >= 5,
        "Expected at least 5 identifiers, found {}",
        ident_count
    );
}

// ============================================================================
// Special Character Edge Cases
// ============================================================================

#[test]
fn test_unicode_zero_width_characters() {
    let input = "test\u{200B}identifier"; // Zero-width space
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.filter_map(|r| r.ok()).collect();

    // Behavior depends on spec - should either be one or two identifiers
    assert!(!tokens.is_empty());
}

#[test]
fn test_bom_at_start() {
    let input = "\u{FEFF}fn main() {}";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.filter_map(|r| r.ok()).collect();

    // Should skip BOM and recognize tokens (BOM may be rejected)
    assert!(!tokens.is_empty());
}

#[test]
fn test_right_to_left_marks() {
    let input = "test\u{202E}reversed";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.filter_map(|r| r.ok()).collect();

    // Should handle RTL marks safely (may be rejected)
    assert!(!tokens.is_empty());
}

// ============================================================================
// Delimiter Matching Edge Cases
// ============================================================================

#[test]
fn test_unmatched_delimiters() {
    let test_cases = vec!["(", ")", "[", "]", "{", "}"];

    for input in test_cases {
        let lexer = Lexer::new(input, FileId::new(0));
        let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

        // Lexer should produce token (parser handles matching)
        assert!(tokens.len() >= 2, "Failed for: {}", input); // token + EOF
    }
}

#[test]
fn test_nested_delimiters() {
    let input = "{{{{}}}}[[[[]]]](((())))";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    // Should handle deep nesting
    assert!(tokens.len() > 20);
}

// ============================================================================
// Contract Literal Edge Cases (v6.0-BALANCED)
// ============================================================================

#[test]
fn test_empty_contract_literal() {
    let input = "c{}";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    assert!(matches!(
        tokens[0].kind,
        TokenKind::ContractLiteral(_) | TokenKind::Ident(_)
    ));
}

#[test]
fn test_nested_contract_literals() {
    let input = "c{ c{ x > 0 } }";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    // Should handle nested contracts
    assert!(tokens.len() > 5);
}

// ============================================================================
// EOF Edge Cases
// ============================================================================

#[test]
fn test_eof_is_always_last() {
    let test_cases = vec!["", "fn main() {}", "// comment\n", "   \t\n  "];

    for input in test_cases {
        let lexer = Lexer::new(input, FileId::new(0));
        let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

        assert_eq!(
            tokens.last().unwrap().kind,
            TokenKind::Eof,
            "Failed for: {:?}",
            input
        );
    }
}

#[test]
fn test_multiple_eof_requests() {
    let input = "x";
    let mut lexer = Lexer::new(input, FileId::new(0));

    let _first = lexer.next();
    let eof1 = lexer.next();
    let eof2 = lexer.next();
    let eof3 = lexer.next();

    // Should return EOF once, then None (per Iterator contract)
    assert!(
        matches!(eof1, Some(Ok(ref t)) if t.kind == TokenKind::Eof),
        "Expected EOF token"
    );
    assert!(eof2.is_none(), "After EOF, iterator should return None");
    assert!(eof3.is_none(), "After EOF, iterator should return None");
}
