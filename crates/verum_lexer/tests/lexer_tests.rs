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
// Tests for the Lexer implementation.

use verum_ast::span::FileId;
use verum_lexer::{Lexer, LookaheadLexer, TokenKind};
use verum_common::Text;

#[test]
fn test_basic_lexing() {
    let source = "fn add(x: Int, y: Int) -> Int { x + y }";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);

    let tokens: Vec<_> = lexer.map(|r| r.unwrap()).collect();

    assert!(matches!(tokens[0].kind, TokenKind::Fn));
    assert!(matches!(tokens[1].kind, TokenKind::Ident(ref s) if s == &Text::from("add")));
    assert!(matches!(tokens[2].kind, TokenKind::LParen));
    assert!(matches!(tokens[tokens.len() - 1].kind, TokenKind::Eof));
}

#[test]
fn test_lexer_spans() {
    let source = "let x = 42;";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);

    let tokens: Vec<_> = lexer.map(|r| r.unwrap()).collect();

    // Check that spans are correct
    assert_eq!(tokens[0].span.start, 0); // 'let'
    assert_eq!(tokens[0].span.end, 3);
}

#[test]
fn test_keywords_vs_identifiers() {
    let source = "fn function fnord";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);

    let tokens: Vec<_> = lexer.map(|r| r.unwrap()).collect();

    assert!(matches!(tokens[0].kind, TokenKind::Fn));
    assert!(matches!(tokens[1].kind, TokenKind::Ident(ref s) if s == &Text::from("function")));
    assert!(matches!(tokens[2].kind, TokenKind::Ident(ref s) if s == &Text::from("fnord")));
}

#[test]
fn test_operators() {
    let source = "|> ?. ?? & % ** + - * /";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);

    let tokens: Vec<_> = lexer.map(|r| r.unwrap()).collect();

    assert!(matches!(tokens[0].kind, TokenKind::PipeGt));
    assert!(matches!(tokens[1].kind, TokenKind::QuestionDot));
    assert!(matches!(tokens[2].kind, TokenKind::QuestionQuestion));
    assert!(matches!(tokens[3].kind, TokenKind::Ampersand));
    assert!(matches!(tokens[4].kind, TokenKind::Percent));
    assert!(matches!(tokens[5].kind, TokenKind::StarStar));
}

#[test]
fn test_literals() {
    let source = r#"42 3.14 "hello" 'c' true false"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);

    let tokens: Vec<_> = lexer.map(|r| r.unwrap()).collect();

    assert!(
        matches!(tokens[0].kind, TokenKind::Integer(ref lit) if lit.as_i64() == Some(42) && lit.suffix.is_none())
    );
    assert!(
        matches!(tokens[1].kind, TokenKind::Float(ref lit) if (lit.value - 3.14).abs() < 0.001 && lit.suffix.is_none())
    );
    assert!(matches!(tokens[2].kind, TokenKind::Text(ref s) if s == &Text::from("hello")));
    assert!(matches!(tokens[3].kind, TokenKind::Char('c')));
    assert!(matches!(tokens[4].kind, TokenKind::True));
    assert!(matches!(tokens[5].kind, TokenKind::False));
}

#[test]
fn test_comments_are_skipped() {
    let source = "// line comment\nfn /* block comment */ main";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);

    let tokens: Vec<_> = lexer.map(|r| r.unwrap()).collect();

    // Should only get: fn, main, EOF
    assert_eq!(tokens.len(), 3);
    assert!(matches!(tokens[0].kind, TokenKind::Fn));
    assert!(matches!(tokens[1].kind, TokenKind::Ident(ref s) if s == &Text::from("main")));
    assert!(matches!(tokens[2].kind, TokenKind::Eof));
}

#[test]
fn test_lookahead_lexer() {
    let source = "fn add(x: Int)";
    let file_id = FileId::new(0);
    let mut lexer = LookaheadLexer::new(source, file_id);

    // Peek ahead without consuming
    assert!(matches!(lexer.peek(0).unwrap().kind, TokenKind::Fn));
    assert!(matches!(
        lexer.peek(1).unwrap().kind,
        TokenKind::Ident(ref s) if s == &Text::from("add")
    ));
    assert!(matches!(lexer.peek(2).unwrap().kind, TokenKind::LParen));

    // Now consume and check
    assert!(matches!(lexer.next_token().unwrap().kind, TokenKind::Fn));
    assert!(matches!(
        lexer.next_token().unwrap().kind,
        TokenKind::Ident(ref s) if s == &Text::from("add")
    ));
}

#[test]
fn test_number_formats() {
    let source = "42 0x2A 0b101010 1_000_000";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);

    let tokens: Vec<_> = lexer.map(|r| r.unwrap()).collect();

    assert!(matches!(tokens[0].kind, TokenKind::Integer(ref lit) if lit.as_i64() == Some(42)));
    assert!(matches!(tokens[1].kind, TokenKind::Integer(ref lit) if lit.as_i64() == Some(42)));
    assert!(matches!(tokens[2].kind, TokenKind::Integer(ref lit) if lit.as_i64() == Some(42)));
    assert!(matches!(tokens[3].kind, TokenKind::Integer(ref lit) if lit.as_i64() == Some(1_000_000)));
}

#[test]
fn test_stream_syntax() {
    let source = "stream [x for x in data]";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);

    let tokens: Vec<_> = lexer.map(|r| r.unwrap()).collect();

    assert!(matches!(tokens[0].kind, TokenKind::Stream));
    assert!(matches!(tokens[1].kind, TokenKind::LBracket));
    assert!(matches!(tokens[2].kind, TokenKind::Ident(ref s) if s == &Text::from("x")));
    assert!(matches!(tokens[3].kind, TokenKind::For));
    assert!(matches!(tokens[4].kind, TokenKind::Ident(ref s) if s == &Text::from("x")));
    assert!(matches!(tokens[5].kind, TokenKind::In));
    assert!(matches!(tokens[6].kind, TokenKind::Ident(ref s) if s == &Text::from("data")));
    assert!(matches!(tokens[7].kind, TokenKind::RBracket));
}

#[test]
fn test_cbgr_and_ownership_refs() {
    let source = "&T &mut T %T %mut T";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);

    let tokens: Vec<_> = lexer.map(|r| r.unwrap()).collect();

    assert!(matches!(tokens[0].kind, TokenKind::Ampersand));
    assert!(matches!(tokens[1].kind, TokenKind::Ident(ref s) if s == &Text::from("T")));
    assert!(matches!(tokens[2].kind, TokenKind::Ampersand));
    assert!(matches!(tokens[3].kind, TokenKind::Mut));
    assert!(matches!(tokens[4].kind, TokenKind::Ident(ref s) if s == &Text::from("T")));
    assert!(matches!(tokens[5].kind, TokenKind::Percent));
    assert!(matches!(tokens[6].kind, TokenKind::Ident(ref s) if s == &Text::from("T")));
    assert!(matches!(tokens[7].kind, TokenKind::Percent));
    assert!(matches!(tokens[8].kind, TokenKind::Mut));
    assert!(matches!(tokens[9].kind, TokenKind::Ident(ref s) if s == &Text::from("T")));
}
