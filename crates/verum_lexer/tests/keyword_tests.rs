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
// Keyword recognition tests for the Verum lexer.
// Tests that keywords are recognized correctly and that identifiers are not confused with keywords.
// Verum has 3 reserved keywords (let, fn, is) and ~38 contextual keywords.

use verum_ast::span::FileId;
use verum_lexer::{Lexer, TokenKind};

fn tokenize(input: &str) -> Vec<TokenKind> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    lexer
        .filter_map(|r| r.ok())
        .map(|token| token.kind)
        .collect()
}

// ===== Core Keywords (Only 5) =====
// Core keywords: let (bindings), fn (functions), type (type defs), match (pattern matching)
// These are among the most fundamental tokens in the language.

#[test]
fn test_keyword_let() {
    let tokens = tokenize("let");
    assert!(matches!(tokens[0], TokenKind::Let));
}

#[test]
fn test_keyword_fn() {
    let tokens = tokenize("fn");
    assert!(matches!(tokens[0], TokenKind::Fn));
}

#[test]
fn test_keyword_type() {
    let tokens = tokenize("type");
    assert!(matches!(tokens[0], TokenKind::Type));
}

#[test]
fn test_keyword_match() {
    let tokens = tokenize("match");
    assert!(matches!(tokens[0], TokenKind::Match));
}

// ===== New Keywords for Protocol System (Spec v6.0-BALANCED) =====
// Protocol system: `protocol` defines trait-like interfaces (replaces Rust's `trait`),
// `implement` provides implementations (replaces Rust's `impl`)

#[test]
fn test_protocol_keyword() {
    let tokens = tokenize("protocol");
    assert!(matches!(tokens[0], TokenKind::Protocol));
}

#[test]
fn test_implement_keyword() {
    let tokens = tokenize("implement");
    assert!(matches!(tokens[0], TokenKind::Implement));
}

// ===== Contextual Keywords =====

#[test]
fn test_contextual_keyword_is() {
    // 'is' is one of the three core keywords per spec
    let tokens = tokenize("is");
    assert!(matches!(tokens[0], TokenKind::Is));
}

#[test]
fn test_contextual_keyword_where() {
    let tokens = tokenize("where");
    assert!(matches!(tokens[0], TokenKind::Where));
}

#[test]
fn test_contextual_keyword_if() {
    let tokens = tokenize("if");
    assert!(matches!(tokens[0], TokenKind::If));
}

#[test]
fn test_contextual_keyword_else() {
    let tokens = tokenize("else");
    assert!(matches!(tokens[0], TokenKind::Else));
}

#[test]
fn test_contextual_keyword_while() {
    let tokens = tokenize("while");
    assert!(matches!(tokens[0], TokenKind::While));
}

#[test]
fn test_contextual_keyword_for() {
    let tokens = tokenize("for");
    assert!(matches!(tokens[0], TokenKind::For));
}

#[test]
fn test_contextual_keyword_loop() {
    let tokens = tokenize("loop");
    assert!(matches!(tokens[0], TokenKind::Loop));
}

#[test]
fn test_contextual_keyword_break() {
    let tokens = tokenize("break");
    assert!(matches!(tokens[0], TokenKind::Break));
}

#[test]
fn test_contextual_keyword_continue() {
    let tokens = tokenize("continue");
    assert!(matches!(tokens[0], TokenKind::Continue));
}

#[test]
fn test_contextual_keyword_return() {
    let tokens = tokenize("return");
    assert!(matches!(tokens[0], TokenKind::Return));
}

#[test]
fn test_contextual_keyword_mut() {
    let tokens = tokenize("mut");
    assert!(matches!(tokens[0], TokenKind::Mut));
}

#[test]
fn test_contextual_keyword_as() {
    let tokens = tokenize("as");
    assert!(matches!(tokens[0], TokenKind::As));
}

#[test]
fn test_contextual_keyword_in() {
    let tokens = tokenize("in");
    assert!(matches!(tokens[0], TokenKind::In));
}

// ===== Boolean Literals (Special Keywords) =====

#[test]
fn test_keyword_true() {
    let tokens = tokenize("true");
    assert!(matches!(tokens[0], TokenKind::True));
}

#[test]
fn test_keyword_false() {
    let tokens = tokenize("false");
    assert!(matches!(tokens[0], TokenKind::False));
}

// ===== Self Keywords =====

#[test]
fn test_keyword_self_value() {
    let tokens = tokenize("self");
    assert!(matches!(tokens[0], TokenKind::SelfValue));
}

#[test]
fn test_keyword_self_type() {
    let tokens = tokenize("Self");
    assert!(matches!(tokens[0], TokenKind::SelfType));
}

// ===== Visibility Keywords =====

#[test]
fn test_keyword_pub() {
    let tokens = tokenize("pub");
    assert!(matches!(tokens[0], TokenKind::Pub));
}

#[test]
fn test_keyword_public() {
    let tokens = tokenize("public");
    assert!(matches!(tokens[0], TokenKind::Public));
}

#[test]
fn test_keyword_internal() {
    let tokens = tokenize("internal");
    assert!(matches!(tokens[0], TokenKind::Internal));
}

#[test]
fn test_keyword_protected() {
    let tokens = tokenize("protected");
    assert!(matches!(tokens[0], TokenKind::Protected));
}

// ===== Context System Keywords =====
// Capability-based DI: `using` declares required contexts, `context` defines DI interfaces,
// `provide` installs providers. Runtime lookup ~5-30ns via task-local storage.

#[test]
fn test_keyword_using() {
    let tokens = tokenize("using");
    assert!(matches!(tokens[0], TokenKind::Using));
}

#[test]
fn test_keyword_context() {
    let tokens = tokenize("context");
    assert!(matches!(tokens[0], TokenKind::Context));
}

#[test]
fn test_keyword_provide() {
    let tokens = tokenize("provide");
    assert!(matches!(tokens[0], TokenKind::Provide));
}

// ===== Non-Keywords (should be identifiers) =====

#[test]
fn test_non_keyword_identifier() {
    let tokens = tokenize("hello");
    assert!(matches!(tokens[0], TokenKind::Ident(_)));
}

#[test]
fn test_non_keyword_similar_to_let() {
    let tokens = tokenize("letter");
    assert!(matches!(tokens[0], TokenKind::Ident(_)));
}

#[test]
fn test_non_keyword_similar_to_fn() {
    let tokens = tokenize("fun");
    assert!(matches!(tokens[0], TokenKind::Ident(_)));
}

#[test]
fn test_non_keyword_similar_to_type() {
    let tokens = tokenize("types");
    assert!(matches!(tokens[0], TokenKind::Ident(_)));
}

#[test]
fn test_non_keyword_similar_to_is() {
    let tokens = tokenize("island");
    assert!(matches!(tokens[0], TokenKind::Ident(_)));
}

// ===== Keyword Case Sensitivity =====

#[test]
fn test_keyword_case_sensitive_let() {
    let tokens = tokenize("LET");
    assert!(matches!(tokens[0], TokenKind::Ident(_))); // Should be identifier, not Let
}

#[test]
fn test_keyword_case_sensitive_true() {
    let tokens = tokenize("TRUE");
    assert!(matches!(tokens[0], TokenKind::Ident(_))); // Should be identifier, not True
}

// ===== Keywords in Context =====

#[test]
fn test_let_in_assignment() {
    let tokens = tokenize("let x = 5");
    assert!(matches!(tokens[0], TokenKind::Let));
    assert!(matches!(tokens[1], TokenKind::Ident(_))); // x
    assert!(matches!(tokens[2], TokenKind::Eq));
}

#[test]
fn test_fn_in_function_def() {
    let tokens = tokenize("fn add(a, b) -> Int");
    assert!(matches!(tokens[0], TokenKind::Fn));
    assert!(matches!(tokens[1], TokenKind::Ident(_))); // add
}

#[test]
fn test_type_in_type_def() {
    let tokens = tokenize("type Point is { x: Int, y: Int }");
    assert!(matches!(tokens[0], TokenKind::Type));
    assert!(matches!(tokens[1], TokenKind::Ident(_))); // Point
    assert!(matches!(tokens[2], TokenKind::Is));
}

#[test]
fn test_match_in_match_expr() {
    let tokens = tokenize("match x { 1 => 2 }");
    assert!(matches!(tokens[0], TokenKind::Match));
}

#[test]
fn test_if_in_conditional() {
    let tokens = tokenize("if x > 0 { y } else { z }");
    assert!(matches!(tokens[0], TokenKind::If));
}

// ===== Multiple Keywords in Sequence =====

#[test]
fn test_multiple_keywords() {
    let tokens = tokenize("let fn type");
    assert!(matches!(tokens[0], TokenKind::Let));
    assert!(matches!(tokens[1], TokenKind::Fn));
    assert!(matches!(tokens[2], TokenKind::Type));
}

#[test]
fn test_keyword_with_identifiers() {
    let tokens = tokenize("let foo fn bar");
    assert!(matches!(tokens[0], TokenKind::Let));
    assert!(matches!(tokens[1], TokenKind::Ident(_)));
    assert!(matches!(tokens[2], TokenKind::Fn));
    assert!(matches!(tokens[3], TokenKind::Ident(_)));
}
