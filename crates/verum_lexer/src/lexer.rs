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
    /// Byte offset added to all logos spans. Non-zero when a UTF-8 BOM and/or
    /// a shebang line was stripped from the input before being passed to logos.
    /// Keeps diagnostics pointing at original-source byte positions.
    shebang_offset: u32,
    /// The original shebang text, including the leading `#!` and trailing
    /// newline (if any). `None` if the source had no shebang.
    shebang_text: Option<&'source str>,
    /// `true` if the source began with a UTF-8 byte-order mark (`EF BB BF`).
    /// The BOM is stripped before tokenisation; this flag preserves the fact
    /// for downstream lossless reconstruction.
    had_bom: bool,
}

impl<'source> Lexer<'source> {
    /// Create a new lexer for the given source code.
    ///
    /// Two leading-trivia stages run before logos sees the input:
    ///
    /// 1. **UTF-8 BOM** (`EF BB BF`) is stripped if present. Cross-platform
    ///    editors frequently prepend one and logos has no rule for it; left
    ///    in place it would cause a lex error at byte 0 of every BOM-prefixed
    ///    file.
    /// 2. **POSIX shebang** (`#!...\n`) is stripped if present after any BOM.
    ///    The shebang must begin at the first non-BOM byte.
    ///
    /// All emitted token spans add back the combined BOM+shebang prefix
    /// length so diagnostics point at original-source byte positions. Use
    /// [`Lexer::had_shebang`] / [`Lexer::shebang_text`] to inspect the
    /// shebang and [`Lexer::had_bom`] to detect a stripped BOM.
    pub fn new(source: &'source str, file_id: FileId) -> Self {
        let (after_bom, had_bom) = strip_utf8_bom(source);
        let (body, shebang_text) = strip_shebang(after_bom);
        let offset = (source.len() - body.len()) as u32;
        Self {
            inner: TokenKind::lexer(body),
            file_id,
            eof_reached: false,
            last_error_span: None,
            shebang_offset: offset,
            shebang_text,
            had_bom,
        }
    }

    /// `true` when the source started with a `#!` shebang line.
    pub fn had_shebang(&self) -> bool {
        self.shebang_text.is_some()
    }

    /// The shebang text (including `#!` and trailing newline, if present).
    pub fn shebang_text(&self) -> Option<&'source str> {
        self.shebang_text
    }

    /// `true` when the source started with a UTF-8 byte-order mark.
    pub fn had_bom(&self) -> bool {
        self.had_bom
    }

    /// Byte offset of the first non-shebang character in the original source.
    pub fn shebang_offset(&self) -> u32 {
        self.shebang_offset
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
            Span::new(
                span.start as u32 + self.shebang_offset,
                span.end as u32 + self.shebang_offset,
                self.file_id,
            ),
        ))
    }

    /// Get the current position in the source (in original source coordinates).
    pub fn position(&self) -> u32 {
        self.inner.span().start as u32 + self.shebang_offset
    }

    /// Get the remaining source text (after any stripped shebang).
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
        Span::new(
            span.start as u32 + self.shebang_offset,
            span.end as u32 + self.shebang_offset,
            self.file_id,
        )
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
                let pos = self.inner.span().end as u32 + self.shebang_offset;
                Some(Ok(Token::new(
                    TokenKind::Eof,
                    Span::new(pos, pos, self.file_id),
                )))
            }
        }
    }
}

/// Detect a POSIX shebang (`#!...\n`) at the start of `source` and split it
/// from the body. Returns `(body, shebang)`. The shebang slice (when present)
/// includes the trailing newline if any. CRLF line endings are honoured: a
/// shebang ended by `\r\n` is included in full.
///
/// A shebang must begin at byte 0; bytes elsewhere are not affected. If the
/// file consists of only a shebang with no trailing newline, the entire input
/// is treated as the shebang and the body is empty.
///
/// This function does NOT skip a UTF-8 BOM (`EF BB BF`). Sources from
/// cross-platform editors frequently start with one, and a BOM at byte 0
/// would shift the shebang to byte 3 — invisible to this check. The Lexer
/// entry point strips the BOM first (see [`strip_utf8_bom`]) so the shebang
/// detector here can stay byte-precise.
pub fn strip_shebang(source: &str) -> (&str, Option<&str>) {
    let bytes = source.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'#' || bytes[1] != b'!' {
        return (source, None);
    }
    // Find first \n. The shebang slice is everything up to and including it.
    match memchr_newline(bytes) {
        Some(nl_pos) => {
            let split = nl_pos + 1;
            (&source[split..], Some(&source[..split]))
        }
        None => ("", Some(source)),
    }
}

/// UTF-8 byte-order mark (`EF BB BF`). Editors on Windows + cross-platform
/// IDEs frequently prepend this to source files; [`Lexer::new`] strips it
/// before tokenisation so logos sees a clean source slice and downstream
/// spans are computed in original-source coordinates.
pub const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// Strip an optional UTF-8 BOM prefix from `source`. Returns `(body, had_bom)`.
/// Idempotent: a second call on already-stripped input is a no-op.
#[inline]
pub fn strip_utf8_bom(source: &str) -> (&str, bool) {
    if source.as_bytes().starts_with(UTF8_BOM) {
        (&source[UTF8_BOM.len()..], true)
    } else {
        (source, false)
    }
}

#[inline]
fn memchr_newline(bytes: &[u8]) -> Option<usize> {
    bytes.iter().position(|&b| b == b'\n')
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
