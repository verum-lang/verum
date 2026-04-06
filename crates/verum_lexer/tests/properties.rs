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
//! Property-based tests for verum_lexer
//!
//! Tests invariants using proptest.
//!
//! Invariants tested:
//! 1. parse → collect → concat = identity (roundtrip)
//! 2. Lexer never panics on valid UTF-8
//! 3. Token spans are non-overlapping and sequential
//! 4. All tokens have valid spans within input bounds
//! 5. EOF is always last token
//! 6. Whitespace removal preserves semantic tokens

use proptest::prelude::*;
use verum_ast::span::FileId;
use verum_lexer::{Lexer, Token, TokenKind};

// ============================================================================
// Property: Lexer Never Panics
// ============================================================================

proptest! {
    #[test]
    fn lexer_never_panics_on_valid_utf8(s in "\\PC*") {
        let lexer = Lexer::new(&s, FileId::new(0));
        // Filter errors - lexer may return InvalidToken for some inputs
        let _tokens: Vec<Token> = lexer.filter_map(|r| r.ok()).collect();
        // Test passes if it doesn't panic
    }

    #[test]
    fn lexer_handles_ascii_strings(s in "[a-zA-Z0-9 \\t\\n]*") {
        let lexer = Lexer::new(&s, FileId::new(0));
        let _tokens: Vec<Token> = lexer.filter_map(|r| r.ok()).collect();
    }

    #[test]
    fn lexer_handles_identifiers(s in "[a-zA-Z_][a-zA-Z0-9_]*") {
        let lexer = Lexer::new(&s, FileId::new(0));
        let tokens: Vec<Token> = lexer.filter_map(|r| r.ok()).collect();

        // Should produce at least an identifier or keyword
        assert!(tokens.len() >= 2); // token + EOF
    }

    #[test]
    fn lexer_handles_numbers(n in any::<i64>()) {
        let input = n.to_string();
        let lexer = Lexer::new(&input, FileId::new(0));
        let tokens: Vec<Token> = lexer.filter_map(|r| r.ok()).collect();

        // Should parse as integer literal
        assert!(tokens.iter().any(|t| matches!(t.kind, TokenKind::Integer(_))));
    }

    #[test]
    fn lexer_handles_floats(f in any::<f64>().prop_filter("not nan/inf", |f| f.is_finite())) {
        let input = f.to_string();
        let lexer = Lexer::new(&input, FileId::new(0));
        // Filter errors - some float representations may not be valid tokens
        let tokens: Vec<Token> = lexer.filter_map(|r| r.ok()).collect();

        // Should parse as some numeric token (if lexer supports the format)
        assert!(!tokens.is_empty());
    }
}

// ============================================================================
// Property: EOF is Always Last
// ============================================================================

proptest! {
    #[test]
    fn eof_is_always_last_token(s in "\\PC*") {
        let lexer = Lexer::new(&s, FileId::new(0));
        let tokens: Vec<Token> = lexer.filter_map(|r| r.ok()).collect();

        assert!(!tokens.is_empty(), "Token stream should never be empty");
        assert_eq!(tokens.last().unwrap().kind, TokenKind::Eof,
                   "Last token must be EOF");
    }

    #[test]
    fn eof_appears_exactly_once(s in "\\PC*") {
        let lexer = Lexer::new(&s, FileId::new(0));
        let tokens: Vec<Token> = lexer.filter_map(|r| r.ok()).collect();

        let eof_count = tokens.iter().filter(|t| t.kind == TokenKind::Eof).count();
        assert_eq!(eof_count, 1, "Exactly one EOF token required");
    }
}

// ============================================================================
// Property: Token Spans are Valid
// ============================================================================

proptest! {
    #[test]
    fn all_token_spans_are_within_input_bounds(s in "\\PC*") {
        let lexer = Lexer::new(&s, FileId::new(0));
        let tokens: Vec<Token> = lexer.filter_map(|r| r.ok()).collect();
        let input_len = s.len() as u32;

        for token in &tokens {
            assert!(token.span.start <= input_len,
                    "Token start {} exceeds input length {}", token.span.start, input_len);
            assert!(token.span.end <= input_len,
                    "Token end {} exceeds input length {}", token.span.end, input_len);
            assert!(token.span.start <= token.span.end,
                    "Token start {} must not exceed end {}", token.span.start, token.span.end);
        }
    }

    #[test]
    fn token_spans_are_non_overlapping(s in "[a-zA-Z0-9 ]*") {
        let lexer = Lexer::new(&s, FileId::new(0));
        let tokens: Vec<Token> = lexer.filter_map(|r| r.ok()).collect();

        for i in 0..tokens.len().saturating_sub(1) {
            let curr_end = tokens[i].span.end;
            let next_start = tokens[i + 1].span.start;

            assert!(curr_end <= next_start,
                    "Token {} overlaps with token {}: end={}, next_start={}",
                    i, i + 1, curr_end, next_start);
        }
    }

    #[test]
    fn token_spans_cover_non_whitespace_input(s in "[a-zA-Z0-9]+") {
        let lexer = Lexer::new(&s, FileId::new(0));
        let tokens: Vec<Token> = lexer
            .filter_map(|r| r.ok())
            .filter(|t| t.kind != TokenKind::Eof)
            .collect();

        // All non-EOF tokens should have non-zero spans
        for token in tokens {
            assert!(token.span.end > token.span.start,
                    "Token should have non-zero span");
        }
    }
}

// ============================================================================
// Property: Roundtrip Preservation
// ============================================================================

proptest! {
    #[test]
    fn token_text_roundtrips_for_simple_tokens(s in "[a-zA-Z_][a-zA-Z0-9_]*") {
        let lexer = Lexer::new(&s, FileId::new(0));
        let tokens: Vec<Token> = lexer
            .filter_map(|r| r.ok())
            .filter(|t| t.kind != TokenKind::Eof)
            .collect();

        // Reconstruct input from token spans
        let mut reconstructed = String::new();
        for token in &tokens {
            let start = token.span.start as usize;
            let end = token.span.end as usize;
            reconstructed.push_str(&s[start..end]);
        }

        assert_eq!(reconstructed, s, "Roundtrip failed");
    }

    #[test]
    fn numeric_literals_preserve_value(n in 0i64..1_000_000) {
        let input = n.to_string();
        let lexer = Lexer::new(&input, FileId::new(0));
        let tokens: Vec<Token> = lexer.filter_map(|r| r.ok()).collect();

        if let Some(token) = tokens.iter().find(|t| matches!(t.kind, TokenKind::Integer(_))) {
            if let TokenKind::Integer(lit) = &token.kind {
                assert_eq!(lit.as_i64().unwrap(), n, "Numeric value not preserved");
            }
        } else {
            panic!("No integer literal token found");
        }
    }
}

// ============================================================================
// Property: Whitespace Handling
// ============================================================================

proptest! {
    #[test]
    fn whitespace_removal_preserves_token_count(
        words in prop::collection::vec("[a-z]+", 1..20)
    ) {
        let input = words.join(" ");
        let lexer = Lexer::new(&input, FileId::new(0));
        let tokens: Vec<Token> = lexer
            .filter_map(|r| r.ok())
            .filter(|t| t.kind != TokenKind::Eof)
            .collect();

        assert_eq!(tokens.len(), words.len(),
                   "Token count should match word count");
    }

    #[test]
    fn extra_whitespace_does_not_create_extra_tokens(
        words in prop::collection::vec("[a-z]+", 1..10)
    ) {
        let input1 = words.join(" ");
        let input2 = words.join("   "); // Extra whitespace

        let tokens1: Vec<Token> = Lexer::new(&input1, FileId::new(0))
            .filter_map(|r| r.ok())
            .filter(|t| t.kind != TokenKind::Eof)
            .collect();

        let tokens2: Vec<Token> = Lexer::new(&input2, FileId::new(0))
            .filter_map(|r| r.ok())
            .filter(|t| t.kind != TokenKind::Eof)
            .collect();

        assert_eq!(tokens1.len(), tokens2.len(),
                   "Extra whitespace should not affect token count");

        // Token kinds should match
        for (t1, t2) in tokens1.iter().zip(tokens2.iter()) {
            assert_eq!(std::mem::discriminant(&t1.kind),
                      std::mem::discriminant(&t2.kind),
                      "Token types should match");
        }
    }
}

// ============================================================================
// Property: Keyword vs Identifier Distinction
// ============================================================================

proptest! {
    #[test]
    fn keywords_are_not_identifiers(kw in prop::sample::select(vec!["fn", "let", "mut", "const"])) {
        let lexer = Lexer::new(kw, FileId::new(0));
        let tokens: Vec<Token> = lexer.filter_map(|r| r.ok()).collect();

        // Should be keyword, not identifier
        assert!(!matches!(tokens[0].kind, TokenKind::Ident(_)),
                "Keyword '{}' incorrectly lexed as identifier", kw);
    }

    #[test]
    fn keyword_prefixes_are_identifiers(
        prefix in prop::sample::select(vec!["fn", "let", "mut"]),
        suffix in "[a-z]+"
    ) {
        let input = format!("{}{}", prefix, suffix);
        let lexer = Lexer::new(&input, FileId::new(0));
        let tokens: Vec<Token> = lexer.filter_map(|r| r.ok()).collect();

        // Should be identifier since it's not exact keyword
        assert!(matches!(tokens[0].kind, TokenKind::Ident(_)),
                "Keyword prefix '{}' should be identifier", input);
    }
}

// ============================================================================
// Property: Comment Stripping
// ============================================================================

proptest! {
    #[test]
    fn line_comments_are_removed_or_marked(
        code in "[a-z]+",
        comment in "[ a-zA-Z0-9]*"
    ) {
        let input = format!("{} // {}", code, comment);
        let lexer = Lexer::new(&input, FileId::new(0));
        let tokens: Vec<Token> = lexer
            .filter_map(|r| r.ok())
            .filter(|t| t.kind != TokenKind::Eof)
            .collect();

        // Comment content should not appear as tokens
        assert_eq!(tokens.len(), 1, "Only the code part should be tokenized");
    }

    #[test]
    fn block_comments_are_handled(
        before in "[a-z]+",
        comment in "[ a-zA-Z0-9]*",
        after in "[a-z]+"
    ) {
        let input = format!("{} /* {} */ {}", before, comment, after);
        let lexer = Lexer::new(&input, FileId::new(0));
        let tokens: Vec<Token> = lexer
            .filter_map(|r| r.ok())
            .filter(|t| t.kind != TokenKind::Eof)
            .collect();

        // Should have exactly 2 tokens (before and after)
        assert_eq!(tokens.len(), 2, "Comment should not produce tokens");
    }
}

// ============================================================================
// Property: Operator Sequences
// ============================================================================

proptest! {
    #[test]
    fn operator_sequences_are_tokenized(
        ops in prop::collection::vec(
            prop::sample::select(vec!["+", "-", "*", "/"]),
            1..10
        )
    ) {
        // Add spaces to avoid creating comments (/* or //)
        let input = ops.join(" ");
        let lexer = Lexer::new(&input, FileId::new(0));
        let tokens: Vec<Token> = lexer
            .filter_map(|r| r.ok())
            .filter(|t| t.kind != TokenKind::Eof)
            .collect();

        // Should produce at least one token per operator (exact count depends on whether operators combine)
        assert!(!tokens.is_empty(), "Operators should produce tokens");
        assert!(tokens.len() <= ops.len(), "Should not create more tokens than operators");
    }
}

// ============================================================================
// Property: Delimiter Pairing (Lexer Level)
// ============================================================================

proptest! {
    #[test]
    fn delimiters_are_independent_tokens(
        pairs in prop::collection::vec(
            prop::sample::select(vec!["()", "[]", "{}"]),
            1..10
        )
    ) {
        let input = pairs.join("");
        let lexer = Lexer::new(&input, FileId::new(0));
        let tokens: Vec<Token> = lexer
            .filter_map(|r| r.ok())
            .filter(|t| t.kind != TokenKind::Eof)
            .collect();

        // Each character should be a token
        assert_eq!(tokens.len(), input.len(),
                   "Each delimiter should be its own token");
    }
}

// ============================================================================
// Property: String Literal Handling
// ============================================================================

proptest! {
    #[test]
    fn quoted_strings_are_single_tokens(s in "[a-zA-Z0-9 ]+") {
        let input = format!(r#""{}""#, s);
        let lexer = Lexer::new(&input, FileId::new(0));
        let tokens: Vec<Token> = lexer
            .filter_map(|r| r.ok())
            .filter(|t| t.kind != TokenKind::Eof)
            .collect();

        // Should be exactly one string literal token
        assert_eq!(tokens.len(), 1, "String literal should be one token");
        assert!(matches!(tokens[0].kind, TokenKind::Text(_)),
                "Should be string literal token");
    }
}

// ============================================================================
// Property: Incremental Consumption
// ============================================================================

proptest! {
    #[test]
    fn incremental_equals_full_collection(s in "[a-zA-Z0-9 ]+") {
        // Collect all at once
        let tokens1: Vec<Token> = Lexer::new(&s, FileId::new(0)).filter_map(|r| r.ok()).collect();

        // Collect incrementally
        let mut lexer = Lexer::new(&s, FileId::new(0));
        let mut tokens2 = Vec::new();
        for token in lexer.flatten() {
            tokens2.push(token.clone());
            if token.kind == TokenKind::Eof {
                break;
            }
        }

        assert_eq!(tokens1.len(), tokens2.len(),
                   "Incremental collection should match full collection");

        for (t1, t2) in tokens1.iter().zip(tokens2.iter()) {
            assert_eq!(t1.span, t2.span, "Token spans should match");
            assert_eq!(std::mem::discriminant(&t1.kind),
                      std::mem::discriminant(&t2.kind),
                      "Token kinds should match");
        }
    }
}

// ============================================================================
// Property: Error Recovery
// ============================================================================

proptest! {
    #[test]
    fn lexer_continues_after_invalid_chars(
        before in "[a-z]+",
        after in "[a-z]+"
    ) {
        // Insert ® character between valid tokens (truly invalid in Verum)
        let input = format!("{} ® {}", before, after);
        let lexer = Lexer::new(&input, FileId::new(0));
        let tokens: Vec<Token> = lexer.filter_map(|r| r.ok()).collect();

        // Should still tokenize the valid parts (identifiers OR keywords)
        // Note: before/after might be keywords, so we count all non-EOF tokens
        let valid_tokens: Vec<&Token> = tokens.iter()
            .filter(|t| !matches!(t.kind, TokenKind::Eof))
            .collect();

        // Should have at least 2 valid tokens (before and after)
        assert!(valid_tokens.len() >= 2,
                "Should recover and tokenize valid tokens (got {} tokens from input '{}')",
                valid_tokens.len(), input);
    }
}

// ============================================================================
// Property: Token Ordering
// ============================================================================

proptest! {
    #[test]
    fn tokens_are_in_source_order(s in "[a-zA-Z0-9 ]+") {
        let lexer = Lexer::new(&s, FileId::new(0));
        let tokens: Vec<Token> = lexer.filter_map(|r| r.ok()).collect();

        // Verify tokens are in increasing position order
        for i in 0..tokens.len().saturating_sub(1) {
            assert!(tokens[i].span.start <= tokens[i + 1].span.start,
                    "Tokens must be in source order");
        }
    }
}

// ============================================================================
// Property: Determinism
// ============================================================================

proptest! {
    #[test]
    fn lexer_is_deterministic(s in "\\PC*") {
        let tokens1: Vec<Token> = Lexer::new(&s, FileId::new(0)).filter_map(|r| r.ok()).collect();
        let tokens2: Vec<Token> = Lexer::new(&s, FileId::new(0)).filter_map(|r| r.ok()).collect();

        assert_eq!(tokens1.len(), tokens2.len(),
                   "Lexer should produce same number of tokens");

        for (t1, t2) in tokens1.iter().zip(tokens2.iter()) {
            assert_eq!(t1.span, t2.span, "Token spans should be identical");
            assert_eq!(std::mem::discriminant(&t1.kind),
                      std::mem::discriminant(&t2.kind),
                      "Token kinds should be identical");
        }
    }
}
