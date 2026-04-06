//! Lossless lexer that preserves trivia (whitespace, comments).
//!
//! This module provides a lexer that tracks all source text, including
//! whitespace and comments, enabling lossless source reconstruction.
//!
//! Trivia preservation enables lossless source reconstruction: every whitespace
//! character, newline, line comment (`// ...`), block comment (`/* ... */`),
//! doc comment (`/// ...`), and inner doc comment (`//! ...`) is captured as
//! leading or trailing trivia attached to the nearest token. This supports
//! incremental parsing, IDE refactoring, and exact source round-tripping.

use crate::token::{Token, TokenKind};
use verum_ast::span::{FileId, Span};
use verum_common::List;

/// Trivia attached to a token.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Trivia {
    /// Trivia items
    pub items: List<TriviaItem>,
}

impl Trivia {
    /// Create empty trivia.
    pub fn new() -> Self {
        Self { items: List::new() }
    }

    /// Add a trivia item.
    pub fn push(&mut self, item: TriviaItem) {
        self.items.push(item);
    }

    /// Check if trivia is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Get the total byte length.
    pub fn len(&self) -> usize {
        self.items.iter().map(|i| i.text.len()).sum()
    }

    /// Concatenate all trivia text.
    pub fn text(&self) -> String {
        self.items.iter().map(|i| i.text.as_str()).collect()
    }
}

/// A single trivia item.
#[derive(Debug, Clone, PartialEq)]
pub struct TriviaItem {
    /// Kind of trivia
    pub kind: TriviaKind,
    /// The text content
    pub text: String,
    /// Span in source
    pub span: Span,
}

/// Kinds of trivia.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriviaKind {
    /// Whitespace (spaces, tabs)
    Whitespace,
    /// Newline (LF or CRLF)
    Newline,
    /// Line comment: `// ...`
    LineComment,
    /// Block comment: `/* ... */`
    BlockComment,
    /// Doc comment: `/// ...`
    DocComment,
    /// Inner doc comment: `//! ...`
    InnerDocComment,
}

/// A token with attached trivia for lossless parsing.
#[derive(Debug, Clone, PartialEq)]
pub struct RichToken {
    /// The underlying token
    pub token: Token,
    /// Leading trivia (before the token)
    pub leading_trivia: Trivia,
    /// Trailing trivia (after the token, up to newline)
    pub trailing_trivia: Trivia,
}

impl RichToken {
    /// Create a new rich token with no trivia.
    pub fn new(token: Token) -> Self {
        Self {
            token,
            leading_trivia: Trivia::new(),
            trailing_trivia: Trivia::new(),
        }
    }

    /// Create a rich token with trivia.
    pub fn with_trivia(token: Token, leading: Trivia, trailing: Trivia) -> Self {
        Self {
            token,
            leading_trivia: leading,
            trailing_trivia: trailing,
        }
    }

    /// Get the full span including trivia.
    pub fn full_span(&self) -> Span {
        let start = if self.leading_trivia.is_empty() {
            self.token.span.start
        } else {
            self.leading_trivia.items[0].span.start
        };
        let end = if self.trailing_trivia.is_empty() {
            self.token.span.end
        } else {
            self.trailing_trivia.items.last().map(|i| i.span.end).unwrap_or(self.token.span.end)
        };
        Span::new(start, end, self.token.span.file_id)
    }

    /// Get the full text including trivia.
    pub fn full_text(&self, source: &str) -> String {
        let mut result = String::new();
        result.push_str(&self.leading_trivia.text());
        let span = &self.token.span;
        if (span.start as usize) < source.len() && (span.end as usize) <= source.len() {
            result.push_str(&source[span.start as usize..span.end as usize]);
        }
        result.push_str(&self.trailing_trivia.text());
        result
    }
}

/// Lossless lexer that preserves trivia.
pub struct LosslessLexer<'source> {
    source: &'source str,
    file_id: FileId,
    pos: usize,
    eof_reached: bool,
}

impl<'source> LosslessLexer<'source> {
    /// Create a new lossless lexer.
    pub fn new(source: &'source str, file_id: FileId) -> Self {
        Self {
            source,
            file_id,
            pos: 0,
            eof_reached: false,
        }
    }

    /// Get the remaining source.
    pub fn remaining(&self) -> &'source str {
        &self.source[self.pos..]
    }

    /// Check if at end of input.
    pub fn at_end(&self) -> bool {
        self.pos >= self.source.len()
    }

    /// Peek at the current character.
    fn peek_char(&self) -> Option<char> {
        self.source[self.pos..].chars().next()
    }

    /// Peek at the nth character ahead.
    fn peek_char_nth(&self, n: usize) -> Option<char> {
        self.source[self.pos..].chars().nth(n)
    }

    /// Advance by n bytes.
    fn advance(&mut self, n: usize) {
        self.pos = (self.pos + n).min(self.source.len());
    }

    /// Advance by one character and return it.
    fn advance_char(&mut self) -> Option<char> {
        let c = self.peek_char()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    /// Scan trivia (whitespace and comments).
    fn scan_trivia(&mut self) -> Trivia {
        let mut trivia = Trivia::new();

        loop {
            let start = self.pos;

            // Check for whitespace (spaces, tabs)
            if let Some(c) = self.peek_char() {
                if c == ' ' || c == '\t' {
                    while let Some(c) = self.peek_char() {
                        if c == ' ' || c == '\t' {
                            self.advance_char();
                        } else {
                            break;
                        }
                    }
                    trivia.push(TriviaItem {
                        kind: TriviaKind::Whitespace,
                        text: self.source[start..self.pos].to_string(),
                        span: Span::new(start as u32, self.pos as u32, self.file_id),
                    });
                    continue;
                }
            }

            // Check for newline
            if let Some(c) = self.peek_char() {
                if c == '\n' {
                    self.advance_char();
                    trivia.push(TriviaItem {
                        kind: TriviaKind::Newline,
                        text: "\n".to_string(),
                        span: Span::new(start as u32, self.pos as u32, self.file_id),
                    });
                    continue;
                } else if c == '\r' {
                    self.advance_char();
                    if self.peek_char() == Some('\n') {
                        self.advance_char();
                    }
                    trivia.push(TriviaItem {
                        kind: TriviaKind::Newline,
                        text: self.source[start..self.pos].to_string(),
                        span: Span::new(start as u32, self.pos as u32, self.file_id),
                    });
                    continue;
                }
            }

            // Check for comments
            if self.peek_char() == Some('/') {
                if self.peek_char_nth(1) == Some('/') {
                    // Line comment
                    let is_doc = self.peek_char_nth(2) == Some('/');
                    let is_inner_doc = self.peek_char_nth(2) == Some('!');

                    while let Some(c) = self.peek_char() {
                        if c == '\n' || c == '\r' {
                            break;
                        }
                        self.advance_char();
                    }

                    let kind = if is_inner_doc {
                        TriviaKind::InnerDocComment
                    } else if is_doc {
                        TriviaKind::DocComment
                    } else {
                        TriviaKind::LineComment
                    };

                    trivia.push(TriviaItem {
                        kind,
                        text: self.source[start..self.pos].to_string(),
                        span: Span::new(start as u32, self.pos as u32, self.file_id),
                    });
                    continue;
                } else if self.peek_char_nth(1) == Some('*') {
                    // Block comment (with nesting support)
                    self.advance(2); // consume /*
                    let mut depth = 1;

                    while depth > 0 && !self.at_end() {
                        if self.peek_char() == Some('*') && self.peek_char_nth(1) == Some('/') {
                            self.advance(2);
                            depth -= 1;
                        } else if self.peek_char() == Some('/') && self.peek_char_nth(1) == Some('*') {
                            self.advance(2);
                            depth += 1;
                        } else {
                            self.advance_char();
                        }
                    }

                    trivia.push(TriviaItem {
                        kind: TriviaKind::BlockComment,
                        text: self.source[start..self.pos].to_string(),
                        span: Span::new(start as u32, self.pos as u32, self.file_id),
                    });
                    continue;
                }
            }

            // No more trivia
            break;
        }

        trivia
    }

    /// Scan the next token using logos.
    fn scan_token(&mut self) -> Option<Token> {
        use logos::Logos;

        if self.at_end() {
            return None;
        }

        // Use logos to scan the next token
        let remaining = &self.source[self.pos..];
        let mut lexer = TokenKind::lexer(remaining);

        match lexer.next() {
            Some(Ok(kind)) => {
                let span = lexer.span();
                let token_span = Span::new(
                    (self.pos + span.start) as u32,
                    (self.pos + span.end) as u32,
                    self.file_id,
                );
                self.pos += span.end;
                Some(Token::new(kind, token_span))
            }
            Some(Err(())) => {
                // Invalid token - consume one character
                let start = self.pos;
                self.advance_char();
                Some(Token::new(
                    TokenKind::Error,
                    Span::new(start as u32, self.pos as u32, self.file_id),
                ))
            }
            None => None,
        }
    }

    /// Tokenize the entire input into rich tokens.
    pub fn tokenize(mut self) -> List<RichToken> {
        let mut tokens = List::new();
        let mut pending_trivia = Trivia::new();

        loop {
            // Scan leading trivia
            let leading = self.scan_trivia();

            // Merge with pending trivia from previous token's trailing
            let combined_leading = if pending_trivia.is_empty() {
                leading
            } else {
                let mut combined = pending_trivia;
                for item in leading.items {
                    combined.push(item);
                }
                combined
            };

            // Scan the token
            if let Some(token) = self.scan_token() {
                // Scan trailing trivia (up to newline)
                let trailing_and_next_leading = self.scan_trivia();
                let (trailing, next_leading) = split_trivia_at_newline(trailing_and_next_leading);

                tokens.push(RichToken::with_trivia(token, combined_leading, trailing));
                pending_trivia = next_leading;
            } else {
                // EOF - create EOF token with remaining trivia
                let eof_token = Token::new(
                    TokenKind::Eof,
                    Span::new(self.pos as u32, self.pos as u32, self.file_id),
                );
                tokens.push(RichToken::with_trivia(eof_token, combined_leading, Trivia::new()));
                break;
            }
        }

        tokens
    }
}

/// Split trivia at the first newline.
///
/// Trailing trivia: everything up to (not including) the first newline
/// Leading trivia: everything from the first newline onward
fn split_trivia_at_newline(trivia: Trivia) -> (Trivia, Trivia) {
    let mut trailing = Trivia::new();
    let mut leading = Trivia::new();
    let mut seen_newline = false;

    for item in trivia.items {
        if seen_newline {
            leading.push(item);
        } else if item.kind == TriviaKind::Newline {
            leading.push(item);
            seen_newline = true;
        } else {
            trailing.push(item);
        }
    }

    (trailing, leading)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lossless_simple() {
        let source = "let x = 1;";
        let lexer = LosslessLexer::new(source, FileId::new(0));
        let tokens = lexer.tokenize();

        // Should have: let, x, =, 1, ;, EOF
        assert!(tokens.len() >= 6);
        assert_eq!(tokens[0].token.kind, TokenKind::Let);
    }

    #[test]
    fn test_trivia_preservation() {
        let source = "let  x = 1; // comment\n";
        let lexer = LosslessLexer::new(source, FileId::new(0));
        let tokens = lexer.tokenize();

        // Check that whitespace is preserved
        assert!(!tokens[0].trailing_trivia.is_empty()); // space after 'let'

        // Check that comment is preserved
        let semicolon_idx = tokens.iter().position(|t| t.token.kind == TokenKind::Semicolon);
        assert!(semicolon_idx.is_some());
        let semi = &tokens[semicolon_idx.unwrap()];
        assert!(semi.trailing_trivia.items.iter().any(|t| t.kind == TriviaKind::LineComment));
    }

    #[test]
    fn test_lossless_roundtrip() {
        let sources = [
            "fn foo() { }",
            "fn foo() { /* comment */ }",
            "fn foo() {\n    let x = 1;  // inline\n}",
            "  \n  fn bar()  {\n\n}\n",
        ];

        for source in sources {
            let lexer = LosslessLexer::new(source, FileId::new(0));
            let tokens = lexer.tokenize();

            // Reconstruct source from tokens
            let mut reconstructed = String::new();
            for token in &tokens {
                reconstructed.push_str(&token.leading_trivia.text());
                if token.token.kind != TokenKind::Eof {
                    let span = &token.token.span;
                    reconstructed.push_str(&source[span.start as usize..span.end as usize]);
                }
                reconstructed.push_str(&token.trailing_trivia.text());
            }

            assert_eq!(source, reconstructed, "Lossless roundtrip failed for: {}", source);
        }
    }

    #[test]
    fn test_block_comment() {
        let source = "let /* comment */ x = 1;";
        let lexer = LosslessLexer::new(source, FileId::new(0));
        let tokens = lexer.tokenize();

        // Block comment should be in trailing trivia of 'let' or leading of 'x'
        let has_block_comment = tokens.iter().any(|t| {
            t.leading_trivia.items.iter().any(|i| i.kind == TriviaKind::BlockComment)
                || t.trailing_trivia.items.iter().any(|i| i.kind == TriviaKind::BlockComment)
        });
        assert!(has_block_comment);
    }

    #[test]
    fn test_nested_block_comment() {
        let source = "/* outer /* inner */ end */";
        let lexer = LosslessLexer::new(source, FileId::new(0));
        let tokens = lexer.tokenize();

        // The entire nested comment should be captured
        assert!(!tokens.is_empty());
        let has_full_comment = tokens.iter().any(|t| {
            t.leading_trivia.items.iter().any(|i| i.text == source)
        });
        assert!(has_full_comment);
    }

    #[test]
    fn test_doc_comment() {
        let source = "/// Doc comment\nfn foo() {}";
        let lexer = LosslessLexer::new(source, FileId::new(0));
        let tokens = lexer.tokenize();

        // Doc comment should be in leading trivia of 'fn'
        let fn_token = tokens.iter().find(|t| t.token.kind == TokenKind::Fn);
        assert!(fn_token.is_some());
        let fn_token = fn_token.unwrap();
        assert!(fn_token.leading_trivia.items.iter().any(|i| i.kind == TriviaKind::DocComment));
    }
}
