use logos::Logos;
use verum_ast::span::FileId;
use verum_common::Text;
use verum_lexer::{Lexer, TokenKind};

// Scanner state transitions and trivia skipping must not consume native stack
// proportional to input length, including in unoptimized builds. Set the stack
// explicitly so the repository's larger RUST_MIN_STACK cannot hide a regression.
fn on_small_stack(check: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(check)
        .expect("spawn lexer worker")
        .join()
        .expect("lexer worker failed");
}

#[test]
fn long_unicode_identifier_retains_value_and_span() {
    on_small_stack(|| {
        let name = "λ数e\u{301}".repeat(32_768);
        let source = format!("{name};");
        let mut lexer = TokenKind::lexer(&source);
        assert_eq!(
            lexer.next(),
            Some(Ok(TokenKind::Ident(Text::from(name.as_str()))))
        );
        assert_eq!(lexer.span(), 0..name.len());
        assert_eq!(lexer.next(), Some(Ok(TokenKind::Semicolon)));
        assert_eq!(lexer.span(), name.len()..source.len());
        assert_eq!(lexer.next(), None);
    });
}

#[test]
fn repeated_trivia_retains_following_token_and_eof_spans() {
    on_small_stack(|| {
        let trivia = "// λ comment\r\n /* outer /* nested */ end */ \t".repeat(32_768);
        let prefix = "\u{feff}#!/usr/bin/env verum\n";
        let source = format!("{prefix}{trivia}fn");
        // Both the raw Logos API and the public lexer must skip iteratively.
        let mut raw = TokenKind::lexer(&source[prefix.len()..]);
        assert_eq!(raw.next(), Some(Ok(TokenKind::Fn)));
        assert_eq!(raw.span(), trivia.len()..trivia.len() + 2);
        assert_eq!(raw.next(), None);
        let mut lexer = Lexer::new(&source, FileId::new(0));
        let token = lexer.next().unwrap().unwrap();
        assert_eq!(token.kind, TokenKind::Fn);
        assert_eq!(token.span.start as usize, prefix.len() + trivia.len());
        assert_eq!(token.span.end as usize, source.len());
        let eof = lexer.next().unwrap().unwrap();
        assert_eq!(eof.kind, TokenKind::Eof);
        assert_eq!(eof.span.start as usize, source.len());
        assert_eq!(eof.span.end as usize, source.len());
        assert!(lexer.next().is_none());
    });
}

#[test]
fn long_unicode_line_comment_retains_following_token() {
    on_small_stack(|| {
        let comment = format!("//{}\n", "λ数".repeat(32_768));
        let source = format!("{comment}fn");
        let mut lexer = TokenKind::lexer(&source);
        assert_eq!(lexer.next(), Some(Ok(TokenKind::Fn)));
        assert_eq!(lexer.span(), comment.len()..source.len());
        assert_eq!(lexer.next(), None);
    });
}
