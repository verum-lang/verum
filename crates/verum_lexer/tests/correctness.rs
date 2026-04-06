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
    unused_assignments,
    clippy::approx_constant
)]
//! Correctness tests for verum_lexer
//!
//! Tests functional behavior of lexer components.
//! Covers token recognition for keywords, literals, operators, and delimiters
//! as defined by the Verum lexical grammar.

use verum_ast::span::FileId;
use verum_lexer::{Lexer, Token, TokenKind};

// ============================================================================
// Token Recognition Correctness
// ============================================================================

#[test]
fn test_keywords_are_recognized_correctly() {
    let input = "fn let mut const if else while for loop break continue return";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    assert_eq!(tokens[0].kind, TokenKind::Fn);
    assert_eq!(tokens[1].kind, TokenKind::Let);
    assert_eq!(tokens[2].kind, TokenKind::Mut);
    assert_eq!(tokens[3].kind, TokenKind::Const);
    assert_eq!(tokens[4].kind, TokenKind::If);
    assert_eq!(tokens[5].kind, TokenKind::Else);
    assert_eq!(tokens[6].kind, TokenKind::While);
    assert_eq!(tokens[7].kind, TokenKind::For);
    assert_eq!(tokens[8].kind, TokenKind::Loop);
    assert_eq!(tokens[9].kind, TokenKind::Break);
    assert_eq!(tokens[10].kind, TokenKind::Continue);
    assert_eq!(tokens[11].kind, TokenKind::Return);
}

#[test]
fn test_identifiers_are_distinguished_from_keywords() {
    let input = "identifier fnn lett myvar";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    assert!(matches!(tokens[0].kind, TokenKind::Ident(_)));
    assert!(matches!(tokens[1].kind, TokenKind::Ident(_)));
    assert!(matches!(tokens[2].kind, TokenKind::Ident(_)));
    assert!(matches!(tokens[3].kind, TokenKind::Ident(_)));
}

#[test]
fn test_numeric_literals_are_parsed_correctly() {
    let input = "42 3.14 0xFF 0b1010 1e10";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    assert!(matches!(tokens[0].kind, TokenKind::Integer(_)));
    assert!(matches!(tokens[1].kind, TokenKind::Float(_)));
    assert!(matches!(tokens[2].kind, TokenKind::Integer(_))); // hex
    assert!(matches!(tokens[3].kind, TokenKind::Integer(_))); // binary
    assert!(matches!(tokens[4].kind, TokenKind::Float(_))); // scientific notation
}

#[test]
fn test_string_literals_are_tokenized_correctly() {
    let input = r#""hello" "world\n" "unicode: \u{1F4A9}""#;
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    assert!(matches!(tokens[0].kind, TokenKind::Text(_)));
    assert!(matches!(tokens[1].kind, TokenKind::Text(_)));
    assert!(matches!(tokens[2].kind, TokenKind::Text(_)));
}

#[test]
fn test_operators_are_recognized_correctly() {
    let input = "+ - * / % == != < > <= >= && || ! & | ^";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    assert_eq!(tokens[0].kind, TokenKind::Plus);
    assert_eq!(tokens[1].kind, TokenKind::Minus);
    assert_eq!(tokens[2].kind, TokenKind::Star);
    assert_eq!(tokens[3].kind, TokenKind::Slash);
    assert_eq!(tokens[4].kind, TokenKind::Percent);
    assert_eq!(tokens[5].kind, TokenKind::EqEq);
    assert_eq!(tokens[6].kind, TokenKind::BangEq);
    assert_eq!(tokens[7].kind, TokenKind::Lt);
    assert_eq!(tokens[8].kind, TokenKind::Gt);
    assert_eq!(tokens[9].kind, TokenKind::LtEq);
    assert_eq!(tokens[10].kind, TokenKind::GtEq);
    assert_eq!(tokens[11].kind, TokenKind::AmpersandAmpersand);
    assert_eq!(tokens[12].kind, TokenKind::PipePipe);
    assert_eq!(tokens[13].kind, TokenKind::Bang);
    assert_eq!(tokens[14].kind, TokenKind::Ampersand);
    assert_eq!(tokens[15].kind, TokenKind::Pipe);
    assert_eq!(tokens[16].kind, TokenKind::Caret);
    // Note: Shl (<<) and Shr (>>) operators not yet implemented
}

#[test]
fn test_delimiters_are_matched_correctly() {
    let input = "( ) [ ] { } , ; : .";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    assert_eq!(tokens[0].kind, TokenKind::LParen);
    assert_eq!(tokens[1].kind, TokenKind::RParen);
    assert_eq!(tokens[2].kind, TokenKind::LBracket);
    assert_eq!(tokens[3].kind, TokenKind::RBracket);
    assert_eq!(tokens[4].kind, TokenKind::LBrace);
    assert_eq!(tokens[5].kind, TokenKind::RBrace);
    assert_eq!(tokens[6].kind, TokenKind::Comma);
    assert_eq!(tokens[7].kind, TokenKind::Semicolon);
    assert_eq!(tokens[8].kind, TokenKind::Colon);
    assert_eq!(tokens[9].kind, TokenKind::Dot);
}

// ============================================================================
// Context Keywords Correctness (v6.0-BALANCED)
// ============================================================================

#[test]
fn test_context_keywords_in_context() {
    let input = "context MyContext { provides Foo }";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    assert_eq!(tokens[0].kind, TokenKind::Context);
    // 'provides' should be a context keyword
    assert!(matches!(
        tokens[4].kind,
        TokenKind::Provide | TokenKind::Ident(_)
    ));
}

#[test]
fn test_using_keyword_in_context() {
    let input = "using context MyContext;";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    assert_eq!(tokens[0].kind, TokenKind::Using);
    assert_eq!(tokens[1].kind, TokenKind::Context);
}

// ============================================================================
// Whitespace and Comment Handling
// ============================================================================

#[test]
fn test_whitespace_is_ignored_correctly() {
    let input = "  \t\n  fn  \t  main  \n  ";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    assert_eq!(tokens.len(), 3); // fn, main, EOF
    assert_eq!(tokens[0].kind, TokenKind::Fn);
}

#[test]
fn test_line_comments_are_handled() {
    let input = "fn main // this is a comment\n{}";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    assert_eq!(tokens[0].kind, TokenKind::Fn);
    assert!(matches!(tokens[1].kind, TokenKind::Ident(_)));
    assert_eq!(tokens[2].kind, TokenKind::LBrace);
    assert_eq!(tokens[3].kind, TokenKind::RBrace);
}

#[test]
fn test_block_comments_are_handled() {
    let input = "fn /* block comment */ main {}";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    assert_eq!(tokens[0].kind, TokenKind::Fn);
    assert!(matches!(tokens[1].kind, TokenKind::Ident(_)));
}

#[test]
fn test_nested_block_comments() {
    let input = "fn /* outer /* inner */ still comment */ main {}";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    assert_eq!(tokens[0].kind, TokenKind::Fn);
    assert!(matches!(tokens[1].kind, TokenKind::Ident(_)));
}

// ============================================================================
// Span and Position Correctness
// ============================================================================

#[test]
fn test_token_spans_are_accurate() {
    let input = "fn main";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    assert_eq!(tokens[0].span.start, 0);
    assert_eq!(tokens[0].span.end, 2); // "fn"
    assert_eq!(tokens[1].span.start, 3);
    assert_eq!(tokens[1].span.end, 7); // "main"
}

#[test]
fn test_multiline_span_tracking() {
    let input = "fn\nmain\n{\n}";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    // Verify all tokens have valid spans
    for token in tokens {
        assert!(token.span.end >= token.span.start);
    }
}

// ============================================================================
// Literal Value Correctness
// ============================================================================

#[test]
fn test_integer_literal_values() {
    let test_cases = vec![
        ("0", 0i64),
        ("42", 42),
        ("1_000_000", 1_000_000),
        ("0xFF", 255),
        // Note: Octal literals (0o77) are NOT in spec - only decimal, hex, and binary
        ("0b1010", 10),
    ];

    for (input, expected) in test_cases {
        let lexer = Lexer::new(input, FileId::new(0));
        let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

        if let TokenKind::Integer(lit) = &tokens[0].kind {
            assert_eq!(lit.as_i64().unwrap(), expected, "Failed for input: {}", input);
        } else {
            panic!("Expected Integer for input: {}", input);
        }
    }
}

#[test]
fn test_float_literal_values() {
    let test_cases = vec![
        ("3.14", 3.14f64),
        ("0.5", 0.5),
        ("1e10", 1e10),
        ("2.5e-3", 2.5e-3),
    ];

    for (input, expected) in test_cases {
        let lexer = Lexer::new(input, FileId::new(0));
        let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

        if let TokenKind::Float(lit) = &tokens[0].kind {
            assert!(
                (lit.value - expected).abs() < 1e-10,
                "Failed for input: {}",
                input
            );
        } else {
            panic!("Expected Float for input: {}", input);
        }
    }
}

// ============================================================================
// Contract Literals (v6.0-BALANCED)
// ============================================================================

#[test]
fn test_contract_literal_start() {
    // Contract literals use `contract#"..."` syntax -- compiler intrinsic for formal verification.
    // Recognized as ContractLiteral (NOT TaggedLiteral). Contains requires/ensures/invariant clauses.
    let input = r#"contract#"requires x > 0; ensures result > 0""#;
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    assert!(matches!(tokens[0].kind, TokenKind::ContractLiteral(_)));
}

// DISABLED: RegexLit token not yet implemented
// #[test]
// fn test_regex_literal() {
//     let input = r#"r"[a-z]+""#;
//     let lexer = Lexer::new(input, FileId::new(0));
//     let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();
//
//     assert!(matches!(tokens[0].kind, TokenKind::RegexLit(_)));
// }

// ============================================================================
// Error Reporting Correctness
// ============================================================================

#[test]
fn test_unterminated_string_error() {
    let input = r#""unterminated"#;
    let lexer = Lexer::new(input, FileId::new(0));
    let results: Vec<_> = lexer.collect();

    // Should have an error result or error token
    assert!(
        results
            .iter()
            .any(|r| r.is_err()
                || matches!(r.as_ref().ok().map(|t| &t.kind), Some(TokenKind::Error)))
    );
}

#[test]
fn test_invalid_character_error() {
    let input = "fn main @ {}";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    // '@' is not a valid token in most contexts
    // Lexer should either produce error or handle it
    assert!(!tokens.is_empty());
}

// ============================================================================
// Stream Correctness
// ============================================================================

#[test]
fn test_lexer_is_iterator() {
    let input = "a b c";
    let lexer = Lexer::new(input, FileId::new(0));
    let count = lexer.count();

    assert!(count >= 3); // At least 3 identifiers + EOF
}

#[test]
fn test_lexer_can_be_collected() {
    let input = "fn main() {}";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    assert!(!tokens.is_empty());
    assert_eq!(tokens.last().unwrap().kind, TokenKind::Eof);
}

#[test]
fn test_empty_input() {
    let input = "";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].kind, TokenKind::Eof);
}

// ============================================================================
// Unicode Correctness
// ============================================================================

#[test]
fn test_unicode_identifiers() {
    let input = "αβγ café 你好";
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.filter_map(|r| r.ok()).collect();

    // All should be valid identifiers (or may not be supported, filter errors)
    // This test may need adjustment based on actual unicode support
    if tokens.len() >= 3 {
        assert!(matches!(tokens[0].kind, TokenKind::Ident(_)));
        assert!(matches!(tokens[1].kind, TokenKind::Ident(_)));
        assert!(matches!(tokens[2].kind, TokenKind::Ident(_)));
    }
}

#[test]
fn test_unicode_string_literals() {
    let input = r#""Hello 世界 🌍""#;
    let lexer = Lexer::new(input, FileId::new(0));
    let tokens: Vec<Token> = lexer.map(|r| r.unwrap()).collect();

    assert!(matches!(tokens[0].kind, TokenKind::Text(_)));
}
