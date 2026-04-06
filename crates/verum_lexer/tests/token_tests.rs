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
// Basic token recognition tests for the Verum lexer.
// Tests fundamental tokenization: identifiers, operators, delimiters, whitespace, comments.
// Covers: identifiers, keywords, operators, delimiters, whitespace skipping, comments.

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

// ===== Identifier Tests =====

#[test]
fn test_simple_identifier() {
    let tokens = tokenize("hello");
    assert_eq!(tokens.len(), 2); // identifier + EOF
    assert!(matches!(tokens[0], TokenKind::Ident(_)));
}

#[test]
fn test_identifier_with_underscores() {
    let tokens = tokenize("hello_world");
    assert!(matches!(tokens[0], TokenKind::Ident(_)));
}

#[test]
fn test_identifier_with_digits() {
    let tokens = tokenize("var123");
    assert!(matches!(tokens[0], TokenKind::Ident(_)));
}

#[test]
fn test_identifier_leading_underscore() {
    let tokens = tokenize("_private");
    assert!(matches!(tokens[0], TokenKind::Ident(_)));
}

#[test]
fn test_multiple_identifiers() {
    let tokens = tokenize("foo bar baz");
    assert_eq!(tokens.len(), 4); // 3 identifiers + EOF
    assert!(matches!(tokens[0], TokenKind::Ident(_)));
    assert!(matches!(tokens[1], TokenKind::Ident(_)));
    assert!(matches!(tokens[2], TokenKind::Ident(_)));
}

// ===== Operator Tests =====

#[test]
fn test_arithmetic_operators() {
    let tokens = tokenize("+ - * / %");
    assert!(matches!(tokens[0], TokenKind::Plus));
    assert!(matches!(tokens[1], TokenKind::Minus));
    assert!(matches!(tokens[2], TokenKind::Star));
    assert!(matches!(tokens[3], TokenKind::Slash));
    assert!(matches!(tokens[4], TokenKind::Percent));
}

#[test]
fn test_exponentiation_operator() {
    let tokens = tokenize("**");
    assert!(matches!(tokens[0], TokenKind::StarStar));
}

#[test]
fn test_comparison_operators() {
    let tokens = tokenize("== != < > <= >=");
    assert!(matches!(tokens[0], TokenKind::EqEq));
    assert!(matches!(tokens[1], TokenKind::BangEq));
    assert!(matches!(tokens[2], TokenKind::Lt));
    assert!(matches!(tokens[3], TokenKind::Gt));
    assert!(matches!(tokens[4], TokenKind::LtEq));
    assert!(matches!(tokens[5], TokenKind::GtEq));
}

#[test]
fn test_logical_operators() {
    let tokens = tokenize("&& || !");
    assert!(matches!(tokens[0], TokenKind::AmpersandAmpersand));
    assert!(matches!(tokens[1], TokenKind::PipePipe));
    assert!(matches!(tokens[2], TokenKind::Bang));
}

#[test]
fn test_bitwise_operators() {
    let tokens = tokenize("& | ^ << >> ~");
    assert!(matches!(tokens[0], TokenKind::Ampersand));
    assert!(matches!(tokens[1], TokenKind::Pipe));
    assert!(matches!(tokens[2], TokenKind::Caret));
    assert!(matches!(tokens[3], TokenKind::LtLt));
    assert!(matches!(tokens[4], TokenKind::GtGt));
    assert!(matches!(tokens[5], TokenKind::Tilde));
}

#[test]
fn test_assignment_operators() {
    let tokens = tokenize("= += -= *= /= %= &= |= ^= <<= >>=");
    assert!(matches!(tokens[0], TokenKind::Eq));
    assert!(matches!(tokens[1], TokenKind::PlusEq));
    assert!(matches!(tokens[2], TokenKind::MinusEq));
    assert!(matches!(tokens[3], TokenKind::StarEq));
    assert!(matches!(tokens[4], TokenKind::SlashEq));
    assert!(matches!(tokens[5], TokenKind::PercentEq));
    assert!(matches!(tokens[6], TokenKind::AmpersandEq));
    assert!(matches!(tokens[7], TokenKind::PipeEq));
    assert!(matches!(tokens[8], TokenKind::CaretEq));
    assert!(matches!(tokens[9], TokenKind::LtLtEq));
    assert!(matches!(tokens[10], TokenKind::GtGtEq));
}

#[test]
fn test_special_operators() {
    let tokens = tokenize(".. ..= |> -> => ?. ?? ?");
    assert!(matches!(tokens[0], TokenKind::DotDot));
    assert!(matches!(tokens[1], TokenKind::DotDotEq));
    assert!(matches!(tokens[2], TokenKind::PipeGt));
    assert!(matches!(tokens[3], TokenKind::RArrow));
    assert!(matches!(tokens[4], TokenKind::FatArrow));
    assert!(matches!(tokens[5], TokenKind::QuestionDot));
    assert!(matches!(tokens[6], TokenKind::QuestionQuestion));
    assert!(matches!(tokens[7], TokenKind::Question));
}

// ===== Delimiter Tests =====

#[test]
fn test_parentheses() {
    let tokens = tokenize("()");
    assert!(matches!(tokens[0], TokenKind::LParen));
    assert!(matches!(tokens[1], TokenKind::RParen));
}

#[test]
fn test_brackets() {
    let tokens = tokenize("[]");
    assert!(matches!(tokens[0], TokenKind::LBracket));
    assert!(matches!(tokens[1], TokenKind::RBracket));
}

#[test]
fn test_braces() {
    let tokens = tokenize("{}");
    assert!(matches!(tokens[0], TokenKind::LBrace));
    assert!(matches!(tokens[1], TokenKind::RBrace));
}

#[test]
fn test_all_delimiters() {
    let tokens = tokenize("( ) [ ] { }");
    assert_eq!(tokens.len(), 7); // 6 delimiters + EOF
}

// ===== Punctuation Tests =====

#[test]
fn test_punctuation_basic() {
    let tokens = tokenize(", ; : . @ $");
    assert!(matches!(tokens[0], TokenKind::Comma));
    assert!(matches!(tokens[1], TokenKind::Semicolon));
    assert!(matches!(tokens[2], TokenKind::Colon));
    assert!(matches!(tokens[3], TokenKind::Dot));
    assert!(matches!(tokens[4], TokenKind::At));
    assert!(matches!(tokens[5], TokenKind::Dollar));
}

// ===== Whitespace Handling Tests =====

#[test]
fn test_whitespace_between_tokens() {
    let tokens = tokenize("a   b");
    assert_eq!(tokens.len(), 3); // 2 identifiers + EOF
}

#[test]
fn test_newlines_as_whitespace() {
    let tokens = tokenize("a\nb");
    assert_eq!(tokens.len(), 3); // 2 identifiers + EOF
}

#[test]
fn test_mixed_whitespace() {
    let tokens = tokenize("a  \n\t  b");
    assert_eq!(tokens.len(), 3); // 2 identifiers + EOF
}

#[test]
fn test_leading_whitespace() {
    let tokens = tokenize("   hello");
    assert_eq!(tokens.len(), 2);
    assert!(matches!(tokens[0], TokenKind::Ident(_)));
}

#[test]
fn test_trailing_whitespace() {
    let tokens = tokenize("hello   ");
    assert_eq!(tokens.len(), 2);
}

// ===== Comment Handling Tests =====

#[test]
fn test_single_line_comment() {
    let tokens = tokenize("a // this is a comment\nb");
    assert_eq!(tokens.len(), 3); // 2 identifiers + EOF
}

#[test]
fn test_comment_to_end_of_line() {
    let tokens = tokenize("let x = 5; // assignment");
    // let x = 5 ; EOF (comment is skipped)
    assert!(matches!(tokens[0], TokenKind::Let));
}

#[test]
fn test_block_comment() {
    let tokens = tokenize("a /* block comment */ b");
    assert_eq!(tokens.len(), 3); // 2 identifiers + EOF
}

#[test]
fn test_block_comment_multiline() {
    let tokens = tokenize("a /* block\ncomment\nhere */ b");
    assert_eq!(tokens.len(), 3); // 2 identifiers + EOF
}

#[test]
fn test_multiple_block_comments() {
    let tokens = tokenize("/* c1 */ a /* c2 */ b /* c3 */");
    assert_eq!(tokens.len(), 3); // 2 identifiers + EOF
}

#[test]
fn test_comment_at_start() {
    let tokens = tokenize("// comment\nhello");
    assert_eq!(tokens.len(), 2);
    assert!(matches!(tokens[0], TokenKind::Ident(_)));
}

// ===== Complex Token Sequences =====

#[test]
fn test_function_declaration() {
    let tokens = tokenize("fn add(x, y) -> Int { x + y }");
    assert!(matches!(tokens[0], TokenKind::Fn));
    assert!(matches!(tokens[1], TokenKind::Ident(_))); // add
    assert!(matches!(tokens[2], TokenKind::LParen));
}

#[test]
fn test_simple_expression() {
    let tokens = tokenize("x + y * z");
    assert!(matches!(tokens[0], TokenKind::Ident(_))); // x
    assert!(matches!(tokens[1], TokenKind::Plus));
    assert!(matches!(tokens[2], TokenKind::Ident(_))); // y
    assert!(matches!(tokens[3], TokenKind::Star));
    assert!(matches!(tokens[4], TokenKind::Ident(_))); // z
}

// ===== EOF Token Test =====

#[test]
fn test_eof_token() {
    let file_id = FileId::new(0);
    let lexer = Lexer::new("hello", file_id);
    let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
    assert!(matches!(tokens.last().unwrap().kind, TokenKind::Eof));
}

#[test]
fn test_empty_input_produces_eof() {
    let tokens = tokenize("");
    assert_eq!(tokens.len(), 1);
    assert!(matches!(tokens[0], TokenKind::Eof));
}
