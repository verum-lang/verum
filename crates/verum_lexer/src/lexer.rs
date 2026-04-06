//! Main lexer implementation.

use crate::error::{LexResult, invalid_token};
use crate::token::{Token, TokenKind};
use logos::Logos;
use verum_ast::span::{FileId, Span};
use verum_common::List;

/// Lexer error with span information preserved.
#[derive(Debug, Clone)]
pub struct LexerError {
    /// The error message
    pub message: verum_common::Text,
    /// The span where the error occurred (byte offsets)
    pub span: Span,
}

impl LexerError {
    /// Create a new lexer error.
    pub fn new(message: impl Into<verum_common::Text>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }
}

/// The main lexer for Verum source code.
///
/// The lexer operates on a source string slice and produces tokens lazily.
/// It wraps the `logos`-generated lexer with error handling and span tracking.
///
/// # Example
///
/// ```
/// use verum_lexer::Lexer;
/// use verum_ast::span::FileId;
///
/// let source = "fn main() { 42 }";
/// let file_id = FileId::new(0);
/// let lexer = Lexer::new(source, file_id);
///
/// for token in lexer {
///     println!("{:?}", token);
/// }
/// ```
pub struct Lexer<'source> {
    /// The underlying logos lexer
    inner: logos::Lexer<'source, TokenKind>,
    /// The file ID for span creation
    file_id: FileId,
    /// Whether we've reached EOF
    eof_reached: bool,
    /// Last error span (preserved for parser use)
    last_error_span: Option<Span>,
}

impl<'source> Lexer<'source> {
    /// Create a new lexer for the given source code.
    pub fn new(source: &'source str, file_id: FileId) -> Self {
        Self {
            inner: TokenKind::lexer(source),
            file_id,
            eof_reached: false,
            last_error_span: None,
        }
    }

    /// Get the span of the last lexer error (if any).
    /// This preserves byte offset information that would otherwise be lost.
    pub fn last_error_span(&self) -> Option<Span> {
        self.last_error_span
    }

    /// Peek at the next token without consuming it.
    pub fn peek(&self) -> Option<Token> {
        let mut clone = self.inner.clone();
        let kind = clone.next()?.ok()?;
        let span = clone.span();
        Some(Token::new(
            kind,
            Span::new(span.start as u32, span.end as u32, self.file_id),
        ))
    }

    /// Get the current position in the source.
    pub fn position(&self) -> u32 {
        self.inner.span().start as u32
    }

    /// Get the remaining source text.
    pub fn remaining(&self) -> &'source str {
        self.inner.remainder()
    }

    /// Tokenize the entire input into a vector of tokens.
    ///
    /// This is a convenience method that consumes the lexer and collects
    /// all tokens, including the EOF token.
    pub fn tokenize(self) -> LexResult<List<Token>> {
        self.collect::<LexResult<List<_>>>()
    }

    /// Skip whitespace and comments (already handled by logos).
    ///
    /// This method exists for API completeness but is a no-op since logos
    /// automatically skips whitespace and comments based on the `#[logos(skip)]`
    /// attributes in the TokenKind definition.
    pub fn skip_trivia(&mut self) {
        // No-op: logos handles this automatically
    }

    /// Get the current span.
    fn current_span(&self) -> Span {
        let span = self.inner.span();
        Span::new(span.start as u32, span.end as u32, self.file_id)
    }

    /// Convert a logos lexer result into our token type.
    fn convert_token(&mut self, kind: Result<TokenKind, ()>) -> LexResult<Token> {
        match kind {
            Ok(kind) => {
                let span = self.current_span();
                Ok(Token::new(kind, span))
            }
            Err(()) => {
                let span = self.current_span();
                // Store the error span for later retrieval
                self.last_error_span = Some(span);
                Err(invalid_token(span))
            }
        }
    }

    /// Get the source text at a given byte range.
    pub fn source_at(&self, start: usize, end: usize) -> &'source str {
        // Get the full source by combining what we've consumed with remainder
        // Note: This is a simplified approach; the lexer tracks position internally
        let source_start = self.inner.span().start;
        if start >= source_start {
            let offset = start - source_start;
            let len = end.saturating_sub(start);
            let remainder = self.inner.remainder();
            if offset < remainder.len() {
                let end_offset = (offset + len).min(remainder.len());
                return &remainder[offset..end_offset];
            }
        }
        ""
    }
}

impl<'source> Iterator for Lexer<'source> {
    type Item = LexResult<Token>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.eof_reached {
            return None;
        }

        match self.inner.next() {
            Some(result) => Some(self.convert_token(result)),
            None => {
                // Reached end of input, return EOF token once
                self.eof_reached = true;
                let pos = self.inner.span().end as u32;
                Some(Ok(Token::new(
                    TokenKind::Eof,
                    Span::new(pos, pos, self.file_id),
                )))
            }
        }
    }
}

/// A stateful lexer that can look ahead multiple tokens.
///
/// This is useful for parser implementations that need lookahead.
pub struct LookaheadLexer<'source> {
    /// The underlying lexer
    lexer: Lexer<'source>,
    /// Buffered tokens for lookahead
    buffer: List<Token>,
}

impl<'source> LookaheadLexer<'source> {
    /// Create a new lookahead lexer.
    pub fn new(source: &'source str, file_id: FileId) -> Self {
        Self {
            lexer: Lexer::new(source, file_id),
            buffer: List::new(),
        }
    }

    /// Peek at the nth token ahead (0 = next token).
    pub fn peek(&mut self, n: usize) -> LexResult<&Token> {
        // Fill buffer up to n+1 tokens
        while self.buffer.len() <= n {
            match self.lexer.next() {
                Some(Ok(token)) => self.buffer.push(token),
                Some(Err(e)) => return Err(e),
                None => {
                    // Reached EOF
                    let pos = self.lexer.position();
                    self.buffer.push(Token::new(
                        TokenKind::Eof,
                        Span::new(pos, pos, self.lexer.file_id),
                    ));
                    break;
                }
            }
        }

        Ok(&self.buffer[n])
    }

    /// Consume and return the next token.
    pub fn next_token(&mut self) -> LexResult<Token> {
        if self.buffer.is_empty() {
            self.lexer.next().unwrap_or_else(|| {
                let pos = self.lexer.position();
                Ok(Token::new(
                    TokenKind::Eof,
                    Span::new(pos, pos, self.lexer.file_id),
                ))
            })
        } else {
            // SAFETY: Buffer is never empty when else branch is reached (checked by if condition)
            Ok(self.buffer.remove(0))
        }
    }

    /// Check if the next token matches a predicate.
    pub fn check<F>(&mut self, predicate: F) -> LexResult<bool>
    where
        F: FnOnce(&TokenKind) -> bool,
    {
        Ok(predicate(&self.peek(0)?.kind))
    }

    /// Consume the next token if it matches the predicate.
    pub fn eat<F>(&mut self, predicate: F) -> LexResult<Option<Token>>
    where
        F: FnOnce(&TokenKind) -> bool,
    {
        if self.check(predicate)? {
            Ok(Some(self.next_token()?))
        } else {
            Ok(None)
        }
    }

    /// Expect the next token to match the predicate, consuming it.
    pub fn expect<F>(&mut self, predicate: F, _expected: &str) -> LexResult<Token>
    where
        F: FnOnce(&TokenKind) -> bool,
    {
        let token = self.next_token()?;
        if predicate(&token.kind) {
            Ok(token)
        } else {
            Err(invalid_token(token.span))
        }
    }
}
