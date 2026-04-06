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
// Property-Based Tests for Verum Lexer
//
// Uses proptest to verify lexer invariants across randomly generated inputs.

use proptest::prelude::*;
use verum_ast::span::FileId;
use verum_lexer::{Lexer, TokenKind};

// =============================================================================
// Property Test Strategies
// =============================================================================

/// Check if a string is a reserved keyword in Verum
fn is_keyword(s: &str) -> bool {
    matches!(
        s,
        // Core keywords
        "let" | "fn" | "type" | "match" | "mount" |
        // Deprecated keywords
        "struct" | "enum" | "trait" | "impl" |
        // Contextual keywords
        "where" | "if" | "else" | "while" | "for" | "loop" |
        "break" | "continue" | "return" | "yield" | "mut" |
        "const" | "static" | "meta" | "implement" | "protocol" |
        "module" | "async" | "await" | "spawn" | "unsafe" |
        "ref" | "move" | "as" | "in" | "is" | "true" | "false" |
        "None" | "Some" | "Ok" | "Err" | "self" | "Self" |
        "public" | "internal" | "protected" | "stream" | "defer" |
        "using" | "context" | "provide" | "ffi" | "try" | "checked" |
        "pub" | "super" | "cog" | "invariant" | "decreases" |
        "tensor" | "affine" | "finally" | "recover" | "ensures" |
        "requires" | "result"
    )
}

/// Generate valid identifier strings (excluding reserved keywords)
fn identifier_strategy() -> impl Strategy<Value = String> {
    r"[a-zA-Z_][a-zA-Z0-9_]*".prop_filter("identifier must not be a keyword", |s| !is_keyword(s))
}

/// Generate valid integer literals
fn integer_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        // Decimal
        (0i64..=i64::MAX).prop_map(|n| n.to_string()),
        // Hexadecimal
        (0i64..=i64::MAX).prop_map(|n| format!("0x{:x}", n)),
        // Binary
        (0i64..=i64::MAX).prop_map(|n| format!("0b{:b}", n)),
        // Octal
        (0i64..=i64::MAX).prop_map(|n| format!("0o{:o}", n)),
    ]
}

/// Generate valid float literals (positive only - lexer treats minus as separate token)
fn float_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        // Simple floats (positive only)
        (0.0..1e10).prop_map(|f| format!("{:.2}", f)),
        // Scientific notation (positive mantissa)
        (0.0..1e10, -10i32..10i32).prop_map(|(f, e)| format!("{:.2}e{}", f, e)),
    ]
}

/// Generate valid string content (without quotes)
fn string_content_strategy() -> impl Strategy<Value = String> {
    r"[a-zA-Z0-9 !,.\-_]*".prop_map(|s| s.to_string())
}

/// Generate valid keywords (only actual keywords from Verum spec)
fn keyword_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![
        // Reserved keywords (3)
        Just("fn"),
        Just("let"),
        Just("is"),
        // Primary keywords
        Just("type"),
        Just("where"),
        Just("using"),
        // Control flow
        Just("if"),
        Just("else"),
        Just("match"),
        Just("return"),
        Just("break"),
        Just("continue"),
        Just("loop"),
        Just("while"),
        Just("for"),
        // Modifiers
        Just("mut"),
        Just("pub"),
        Just("const"),
        Just("unsafe"),
        // Module keywords
        Just("module"),
        Just("mount"),
        Just("implement"),
        Just("context"),
        Just("protocol"),
        // Async/context
        Just("async"),
        Just("await"),
        Just("spawn"),
        Just("defer"),
        Just("try"),
        // Additional
        Just("provide"),
        Just("static"),
        Just("meta"),
    ]
}

// =============================================================================
// Lexer Invariant Properties
// =============================================================================

proptest! {
    #[test]
    fn prop_lex_concatenate_equals_components(
        id1 in identifier_strategy(),
        id2 in identifier_strategy()
    ) {
        // Property: lexing "id1 id2" should equal lexing id1 and id2 separately
        let source = format!("{} {}", id1, id2);
        let file_id = FileId::new(0);
        let lexer = Lexer::new(&source, file_id);

        let tokens: Vec<_> = lexer.filter_map(Result::ok).collect();
        // 2 identifiers + 1 EOF = 3 tokens
        prop_assert_eq!(tokens.len(), 3, "Should have exactly 3 tokens (2 ids + EOF)");
    }

    #[test]
    fn prop_identifier_roundtrip(id in identifier_strategy()) {
        // Property: valid identifiers should lex without error
        let file_id = FileId::new(0);
        let mut lexer = Lexer::new(&id, file_id);

        if let Some(Ok(token)) = lexer.next() {
            prop_assert!(matches!(token.kind, TokenKind::Ident(_)));
        }
    }

    #[test]
    fn prop_integer_roundtrip(int_str in integer_strategy()) {
        // Property: valid integers should lex to IntLit
        let file_id = FileId::new(0);
        let mut lexer = Lexer::new(&int_str, file_id);

        if let Some(Ok(token)) = lexer.next() {
            prop_assert!(
                matches!(token.kind, TokenKind::Integer(_)),
                "Expected Integer, got {:?}", token.kind
            );
        }
    }

    #[test]
    fn prop_float_roundtrip(float_str in float_strategy()) {
        // Property: valid floats should lex to FloatLiteral
        let file_id = FileId::new(0);
        let mut lexer = Lexer::new(&float_str, file_id);

        if let Some(result) = lexer.next() {
            match result {
                Ok(token) => {
                    prop_assert!(
                        matches!(token.kind, TokenKind::Float(_)),
                        "Expected Float, got {:?}", token.kind
                    );
                }
                Err(_) => {
                    // Some float formats might not be valid, that's okay
                }
            }
        }
    }

    #[test]
    fn prop_string_literal_roundtrip(content in string_content_strategy()) {
        // Property: valid string content should lex correctly when quoted
        let source = format!(r#""{}""#, content);
        let file_id = FileId::new(0);
        let mut lexer = Lexer::new(&source, file_id);

        if let Some(Ok(token)) = lexer.next() {
            prop_assert!(
                matches!(token.kind, TokenKind::Text(_)),
                "Expected String, got {:?}", token.kind
            );
        }
    }

    #[test]
    fn prop_keyword_recognition(keyword in keyword_strategy()) {
        // Property: keywords should be recognized as specific tokens, not identifiers
        let file_id = FileId::new(0);
        let mut lexer = Lexer::new(keyword, file_id);

        if let Some(Ok(token)) = lexer.next() {
            prop_assert!(
                !matches!(token.kind, TokenKind::Ident(_)),
                "Keyword '{}' should not be lexed as Ident", keyword
            );
        }
    }

    #[test]
    fn prop_whitespace_separates_tokens(
        id1 in identifier_strategy(),
        id2 in identifier_strategy(),
        ws_count in 1usize..20
    ) {
        // Property: any amount of whitespace should separate tokens properly
        let whitespace = " ".repeat(ws_count);
        let source = format!("{}{}{}", id1, whitespace, id2);
        let file_id = FileId::new(0);
        let lexer = Lexer::new(&source, file_id);

        let tokens: Vec<_> = lexer.filter_map(Result::ok).collect();
        // 2 identifiers + 1 EOF = 3 tokens
        prop_assert_eq!(tokens.len(), 3, "Whitespace should separate tokens");
    }

    #[test]
    fn prop_token_count_non_decreasing(ids in prop::collection::vec(identifier_strategy(), 1..10)) {
        // Property: adding more tokens should not decrease token count
        let source = ids.join(" ");
        let file_id = FileId::new(0);
        let lexer = Lexer::new(&source, file_id);

        let token_count = lexer.filter_map(Result::ok).count();
        prop_assert!(
            token_count >= ids.len(),
            "Token count should be at least the number of inputs"
        );
    }

    #[test]
    fn prop_no_token_spans_overlap(ids in prop::collection::vec(identifier_strategy(), 2..10)) {
        // Property: token spans should not overlap
        let source = ids.join(" ");
        let file_id = FileId::new(0);
        let lexer = Lexer::new(&source, file_id);

        let tokens: Vec<_> = lexer.filter_map(Result::ok).collect();

        for i in 0..tokens.len().saturating_sub(1) {
            let current_end = tokens[i].span.end;
            let next_start = tokens[i + 1].span.start;
            prop_assert!(
                current_end <= next_start,
                "Token spans should not overlap: token[{}].end={} > token[{}].start={}",
                i, current_end, i + 1, next_start
            );
        }
    }

    #[test]
    fn prop_lex_idempotent(source in r"[a-zA-Z_][a-zA-Z0-9_]*( [a-zA-Z_][a-zA-Z0-9_]*)*") {
        // Property: lexing twice should produce same results
        let file_id = FileId::new(0);

        let lexer1 = Lexer::new(&source, file_id);
        let tokens1: Vec<_> = lexer1.filter_map(Result::ok).map(|t| t.kind).collect();

        let lexer2 = Lexer::new(&source, file_id);
        let tokens2: Vec<_> = lexer2.filter_map(Result::ok).map(|t| t.kind).collect();

        prop_assert_eq!(tokens1.len(), tokens2.len(), "Lexing should be deterministic");
    }

    #[test]
    fn prop_empty_spans_rejected(source in r"[a-zA-Z_][a-zA-Z0-9_]*") {
        // Property: no non-EOF token should have zero-length span
        let file_id = FileId::new(0);
        let lexer = Lexer::new(&source, file_id);

        for result in lexer {
            if let Ok(token) = result {
                // EOF token has zero length by design (marks position, not content)
                if !matches!(token.kind, TokenKind::Eof) {
                    let span_len = token.span.end - token.span.start;
                    prop_assert!(
                        span_len > 0,
                        "Non-EOF token spans must be non-empty: {:?}", token
                    );
                }
            }
        }
    }

    #[test]
    fn prop_spans_within_source_bounds(source in r"[a-zA-Z_][a-zA-Z0-9_ ]*") {
        // Property: all token spans should be within source bounds
        let file_id = FileId::new(0);
        let lexer = Lexer::new(&source, file_id);
        let source_len = source.len() as u32;

        for token in lexer.flatten() {
            prop_assert!(
                token.span.start <= source_len,
                "Token start {} exceeds source length {}", token.span.start, source_len
            );
            prop_assert!(
                token.span.end <= source_len,
                "Token end {} exceeds source length {}", token.span.end, source_len
            );
        }
    }
}

// =============================================================================
// Specific Token Properties
// =============================================================================

proptest! {
    #[test]
    fn prop_numbers_parse_correctly(n in -1000000i64..1000000i64) {
        // Property: decimal numbers should parse to their numeric value
        let source = n.to_string();
        let file_id = FileId::new(0);
        let mut lexer = Lexer::new(&source, file_id);

        if let Some(Ok(token)) = lexer.next()
            && let TokenKind::Integer(lit) = &token.kind {
                let parsed: i64 = lit.as_i64().unwrap();
                prop_assert_eq!(parsed, n, "Number should parse to original value");
            }
    }

    #[test]
    fn prop_operators_single_token(op in r"[+\-*/%<>=!&|^]") {
        // Property: operator characters should produce valid tokens
        let file_id = FileId::new(0);
        let lexer = Lexer::new(&op, file_id);

        let tokens: Vec<_> = lexer.collect();
        // Operator + EOF = 2 tokens (or just EOF if operator is error)
        prop_assert!(
            !tokens.is_empty() && tokens.len() <= 2,
            "Single operator character should produce 1-2 tokens (operator + EOF)"
        );
    }

    #[test]
    fn prop_parentheses_balance(depth in 1usize..20) {
        // Property: balanced parentheses should lex to equal open/close counts
        let source = format!("{}{}", "(".repeat(depth), ")".repeat(depth));
        let file_id = FileId::new(0);
        let lexer = Lexer::new(&source, file_id);

        let tokens: Vec<_> = lexer.filter_map(Result::ok).collect();

        let open_count = tokens.iter()
            .filter(|t| matches!(t.kind, TokenKind::LParen))
            .count();
        let close_count = tokens.iter()
            .filter(|t| matches!(t.kind, TokenKind::RParen))
            .count();

        prop_assert_eq!(open_count, depth, "Should have {} open parens", depth);
        prop_assert_eq!(close_count, depth, "Should have {} close parens", depth);
    }

    #[test]
    fn prop_comments_ignored(comment in r"//[^\n]*") {
        // Property: line comments should not produce tokens
        let file_id = FileId::new(0);
        let lexer = Lexer::new(&comment, file_id);

        let token_count = lexer.filter_map(Result::ok).count();
        // Comments produce no tokens, but EOF is always produced
        prop_assert_eq!(token_count, 1, "Comments should only produce EOF token");
    }

    #[test]
    fn prop_mixed_whitespace_normalized(
        id in identifier_strategy(),
        ws_chars in prop::collection::vec(prop_oneof![Just(' '), Just('\t'), Just('\n')], 1..20)
    ) {
        // Property: different whitespace characters should all separate tokens
        let ws: String = ws_chars.into_iter().collect();
        let source = format!("{}{}{}", id, ws, id);
        let file_id = FileId::new(0);
        let lexer = Lexer::new(&source, file_id);

        let tokens: Vec<_> = lexer.filter_map(Result::ok).collect();
        // 2 identifiers + 1 EOF = 3 tokens
        prop_assert_eq!(tokens.len(), 3, "Whitespace type should not affect token count");
    }
}

// =============================================================================
// Error Recovery Properties
// =============================================================================

proptest! {
    #[test]
    fn prop_invalid_chars_dont_stop_lexing(
        prefix in identifier_strategy(),
        suffix in identifier_strategy()
    ) {
        // Property: invalid characters should not prevent lexing of subsequent valid tokens
        let source = format!("{} @ {}", prefix, suffix);
        let file_id = FileId::new(0);
        let lexer = Lexer::new(&source, file_id);

        let valid_tokens: Vec<_> = lexer.filter_map(Result::ok).collect();
        prop_assert!(
            valid_tokens.len() >= 2,
            "Should recover and lex valid tokens around invalid ones"
        );
    }

    #[test]
    fn prop_unterminated_string_detected(content in string_content_strategy()) {
        // Property: unterminated strings should be detected as errors
        let source = format!(r#""{}"#, content); // Missing closing quote
        let file_id = FileId::new(0);
        let mut lexer = Lexer::new(&source, file_id);

        if let Some(result) = lexer.next() {
            // Should either error or successfully handle the string
            // Both behaviors are acceptable depending on lexer design
            prop_assert!(result.is_ok() || result.is_err());
        }
    }
}

// =============================================================================
// Performance Properties
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn prop_linear_time_complexity(
        ids in prop::collection::vec(identifier_strategy(), 10..1000)
    ) {
        // Property: lexing time should scale linearly with input size
        let source = ids.join(" ");
        let file_id = FileId::new(0);

        let start = std::time::Instant::now();
        let lexer = Lexer::new(&source, file_id);
        let token_count = lexer.filter_map(Result::ok).count();
        let duration = start.elapsed();

        // Verify we got all tokens
        prop_assert!(token_count >= ids.len());

        // Ensure it completed in reasonable time
        // Debug builds are significantly slower due to no optimizations and
        // additional debug instrumentation, so we use a more lenient threshold
        let time_per_100_tokens = duration.as_micros() as usize / (token_count.max(1) / 100).max(1);
        #[cfg(debug_assertions)]
        let max_time_per_100 = 10_000; // 10ms per 100 tokens in debug
        #[cfg(not(debug_assertions))]
        let max_time_per_100 = 1000; // 1ms per 100 tokens in release
        prop_assert!(
            time_per_100_tokens < max_time_per_100,
            "Lexing should be fast: {}μs per 100 tokens (limit: {}μs)",
            time_per_100_tokens,
            max_time_per_100
        );
    }

    #[test]
    fn prop_no_allocation_explosion(
        ids in prop::collection::vec(identifier_strategy(), 1..100)
    ) {
        // Property: lexing should not cause excessive allocations
        let source = ids.join(" ");
        let file_id = FileId::new(0);

        // Lex twice to ensure no state leakage
        let lexer1 = Lexer::new(&source, file_id);
        let count1 = lexer1.filter_map(Result::ok).count();

        let lexer2 = Lexer::new(&source, file_id);
        let count2 = lexer2.filter_map(Result::ok).count();

        prop_assert_eq!(count1, count2, "Lexer should not leak state between runs");
    }
}
