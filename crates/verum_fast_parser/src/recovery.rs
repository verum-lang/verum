//! Error recovery strategies for fast parser resilience.
//!
//! Error recovery uses synchronization-point strategy: after a parse error,
//! skip tokens until reaching a semicolon (statement boundary), closing brace
//! (block end), declaration keyword like `fn`/`type`/`let` (new item), or EOF.
//! Missing delimiters are auto-inserted when the paired opener exists.
//! Grammar: error_recovery ::= synchronize_on_semicolon | synchronize_on_brace
//!          | synchronize_on_keyword | insert_missing_delimiter
//!
//! This module implements error recovery techniques for the compiler:
//!
//! - **Synchronization**: Skip to known safe points (semicolons, braces, keywords)
//! - **Delimiter matching**: Auto-insert missing delimiters
//! - **Error productions**: Parse common mistakes explicitly
//! - **Multiple errors**: Continue parsing after errors
//!
//! # Error Recovery Philosophy
//!
//! The goal is to report ALL errors in a single pass while maintaining
//! a valid AST structure for downstream compilation.

use verum_ast::{FileId, Span};
use verum_common::{List, Text};
use verum_lexer::{Token, TokenKind};

use crate::error::{ParseError, ParseErrorKind};

/// Delimiter type for matched pairs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delimiter {
    /// Parentheses: ( )
    Paren,
    /// Square brackets: [ ]
    Bracket,
    /// Curly braces: { }
    Brace,
    /// Angle brackets: < >
    Angle,
}

impl Delimiter {
    /// Get the opening token for this delimiter.
    pub fn opening(&self) -> TokenKind {
        match self {
            Delimiter::Paren => TokenKind::LParen,
            Delimiter::Bracket => TokenKind::LBracket,
            Delimiter::Brace => TokenKind::LBrace,
            Delimiter::Angle => TokenKind::Lt,
        }
    }

    /// Get the closing token for this delimiter.
    pub fn closing(&self) -> TokenKind {
        match self {
            Delimiter::Paren => TokenKind::RParen,
            Delimiter::Bracket => TokenKind::RBracket,
            Delimiter::Brace => TokenKind::RBrace,
            Delimiter::Angle => TokenKind::Gt,
        }
    }

    /// Get the character representation.
    pub fn open_char(&self) -> char {
        match self {
            Delimiter::Paren => '(',
            Delimiter::Bracket => '[',
            Delimiter::Brace => '{',
            Delimiter::Angle => '<',
        }
    }

    /// Get the closing character representation.
    pub fn close_char(&self) -> char {
        match self {
            Delimiter::Paren => ')',
            Delimiter::Bracket => ']',
            Delimiter::Brace => '}',
            Delimiter::Angle => '>',
        }
    }

    /// Check if a token is the closing delimiter.
    pub fn is_closing(&self, token: &Token) -> bool {
        token.kind == self.closing()
    }

    /// Check if a token is the opening delimiter.
    pub fn is_opening(&self, token: &Token) -> bool {
        token.kind == self.opening()
    }

    /// Try to get a delimiter from an opening token kind.
    pub fn from_opening(kind: TokenKind) -> Option<Self> {
        match kind {
            TokenKind::LParen => Some(Delimiter::Paren),
            TokenKind::LBracket => Some(Delimiter::Bracket),
            TokenKind::LBrace => Some(Delimiter::Brace),
            TokenKind::Lt => Some(Delimiter::Angle),
            _ => None,
        }
    }

    /// Try to get a delimiter from a closing token kind.
    pub fn from_closing(kind: TokenKind) -> Option<Self> {
        match kind {
            TokenKind::RParen => Some(Delimiter::Paren),
            TokenKind::RBracket => Some(Delimiter::Bracket),
            TokenKind::RBrace => Some(Delimiter::Brace),
            TokenKind::Gt => Some(Delimiter::Angle),
            _ => None,
        }
    }
}

/// Synchronization point for error recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncPoint {
    /// Sync at semicolon
    Semicolon,
    /// Sync at comma
    Comma,
    /// Sync at closing brace
    CloseBrace,
    /// Sync at closing paren
    CloseParen,
    /// Sync at closing bracket
    CloseBracket,
    /// Sync at statement boundary (fn, let, type, etc.)
    Statement,
    /// Sync at item boundary (fn, type, protocol, impl, etc.)
    Item,
}

impl SyncPoint {
    /// Check if a token is this synchronization point.
    pub fn matches(&self, kind: &TokenKind) -> bool {
        use TokenKind::*;
        match self {
            SyncPoint::Semicolon => matches!(kind, Semicolon),
            SyncPoint::Comma => matches!(kind, Comma),
            SyncPoint::CloseBrace => matches!(kind, RBrace),
            SyncPoint::CloseParen => matches!(kind, RParen),
            SyncPoint::CloseBracket => matches!(kind, RBracket),
            SyncPoint::Statement => matches!(
                kind,
                Semicolon | RBrace | Fn | Type | Protocol | Implement | Let | Return
            ),
            SyncPoint::Item => matches!(kind, Fn | Type | Protocol | Implement | Module | Eof),
        }
    }

    /// Get expected tokens for better error messages.
    pub fn expected_tokens(&self) -> &'static [&'static str] {
        match self {
            SyncPoint::Semicolon => &[";"],
            SyncPoint::Comma => &[","],
            SyncPoint::CloseBrace => &["}"],
            SyncPoint::CloseParen => &[")"],
            SyncPoint::CloseBracket => &["]"],
            SyncPoint::Statement => &[";", "}", "fn", "type", "let", "return"],
            SyncPoint::Item => &["fn", "type", "protocol", "impl", "module"],
        }
    }
}

/// Error recovery strategy.
#[derive(Debug, Clone)]
pub enum RecoveryStrategy {
    /// Skip tokens until synchronization point
    SkipUntil(SyncPoint),
    /// Insert missing token
    InsertMissing(TokenKind),
    /// Delete/skip current token
    DeleteCurrent,
    /// Balance delimiters by inserting missing ones
    BalanceDelimiters,
}

impl RecoveryStrategy {
    /// Get a human-readable description of this strategy.
    pub fn description(&self) -> Text {
        match self {
            RecoveryStrategy::SkipUntil(sync_point) => Text::from(format!(
                "skipping until {}",
                sync_point.expected_tokens().join(", ")
            )),
            RecoveryStrategy::InsertMissing(kind) => {
                Text::from(format!("inserting missing '{}'", kind.description()))
            }
            RecoveryStrategy::DeleteCurrent => "skipping this token".into(),
            RecoveryStrategy::BalanceDelimiters => "balancing delimiters".into(),
        }
    }
}

/// Error recovery context for tracking parse state.
#[derive(Debug, Clone)]
pub struct RecoveryContext {
    /// Stack of open delimiters with their opening spans
    delimiter_stack: List<(Delimiter, Span)>,
    /// Current nesting depth
    pub nesting_depth: usize,
    /// Accumulated errors
    pub errors: List<ParseError>,
    /// Maximum errors before giving up
    pub max_errors: usize,
}

impl RecoveryContext {
    /// Create a new recovery context.
    pub fn new() -> Self {
        Self {
            delimiter_stack: List::new(),
            nesting_depth: 0,
            errors: List::new(),
            max_errors: 100, // Prevent infinite error cascades
        }
    }

    /// Push an opening delimiter.
    pub fn push_delimiter(&mut self, delim: Delimiter, span: Span) {
        self.delimiter_stack.push((delim, span));
        self.nesting_depth += 1;
    }

    /// Pop a closing delimiter, checking for mismatches.
    /// Returns Ok(()) if matched correctly, Err if mismatch or underflow.
    pub fn pop_delimiter(&mut self, expected: Delimiter, span: Span) -> Result<(), ParseError> {
        if let Some((actual, open_span)) = self.delimiter_stack.pop() {
            if actual == expected {
                self.nesting_depth -= 1;
                Ok(())
            } else {
                // Delimiter mismatch
                let error = ParseError::new(
                    ParseErrorKind::MissingClosingDelimiter(actual.close_char()),
                    span,
                )
                .with_help(Text::from(format!(
                    "expected '{}' to match '{}' at {}",
                    actual.close_char(),
                    actual.open_char(),
                    open_span
                )));
                Err(error)
            }
        } else {
            // No matching opening delimiter - create an InvalidSyntax error
            let error = ParseError::new(
                ParseErrorKind::InvalidSyntax {
                    message: Text::from(format!(
                        "unexpected closing delimiter '{}' without matching opening",
                        expected.close_char()
                    )),
                },
                span,
            );
            Err(error)
        }
    }

    /// Check if a token is at any synchronization point.
    pub fn is_at_sync_point(&self, kind: &TokenKind) -> bool {
        SyncPoint::Semicolon.matches(kind)
            || SyncPoint::Comma.matches(kind)
            || SyncPoint::CloseBrace.matches(kind)
            || SyncPoint::CloseParen.matches(kind)
            || SyncPoint::CloseBracket.matches(kind)
            || SyncPoint::Statement.matches(kind)
            || SyncPoint::Item.matches(kind)
    }

    /// Find the next synchronization point in the token stream.
    /// Returns the index of the sync point, or None if not found.
    pub fn find_sync_point(
        &self,
        tokens: &[Token],
        start: usize,
        sync: SyncPoint,
    ) -> Option<usize> {
        let mut pos = start;
        while pos < tokens.len() {
            if sync.matches(&tokens[pos].kind) {
                return Some(pos);
            }
            pos += 1;
        }
        None
    }

    /// Add an error to the context.
    pub fn add_error(&mut self, error: ParseError) {
        if self.errors.len() < self.max_errors {
            self.errors.push(error);
        }
    }

    /// Check if too many errors have occurred.
    pub fn too_many_errors(&self) -> bool {
        self.errors.len() >= self.max_errors
    }

    /// Get the expected closing delimiter for the most recent opening.
    pub fn expected_closing_delimiter(&self) -> Option<Delimiter> {
        self.delimiter_stack.last().map(|(delim, _)| *delim)
    }

    /// Choose the best recovery strategy for an error.
    pub fn choose_strategy(&self, error: &ParseError) -> RecoveryStrategy {
        match error.kind {
            ParseErrorKind::UnclosedDelimiter(ch) => {
                let delimiter = match ch {
                    '(' => Delimiter::Paren,
                    '[' => Delimiter::Bracket,
                    '{' => Delimiter::Brace,
                    '<' => Delimiter::Angle,
                    _ => return RecoveryStrategy::DeleteCurrent,
                };
                RecoveryStrategy::InsertMissing(delimiter.closing())
            }
            ParseErrorKind::MissingClosingDelimiter(_) => {
                if let Some((delimiter, _)) = self.delimiter_stack.last() {
                    RecoveryStrategy::InsertMissing(delimiter.closing())
                } else {
                    RecoveryStrategy::DeleteCurrent
                }
            }
            ParseErrorKind::MissingSemicolon => {
                RecoveryStrategy::InsertMissing(TokenKind::Semicolon)
            }
            ParseErrorKind::UnexpectedToken { .. } => {
                // Choose based on nesting depth
                if self.nesting_depth > 0 {
                    RecoveryStrategy::SkipUntil(SyncPoint::CloseBrace)
                } else {
                    RecoveryStrategy::SkipUntil(SyncPoint::Statement)
                }
            }
            _ => RecoveryStrategy::SkipUntil(SyncPoint::Statement),
        }
    }

    /// Get all accumulated errors.
    pub fn into_errors(self) -> List<ParseError> {
        self.errors
    }
}

impl Default for RecoveryContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Skip tokens until reaching a synchronization point.
/// Returns the index of the first token at or after start that matches the sync point.
pub fn skip_until(tokens: &[Token], start: usize, sync: SyncPoint) -> usize {
    let mut pos = start;
    while pos < tokens.len() {
        if sync.matches(&tokens[pos].kind) {
            return pos;
        }
        // Also stop at EOF
        if tokens[pos].kind == TokenKind::Eof {
            return pos;
        }
        pos += 1;
    }
    // If we reach the end, return the last position
    tokens.len()
}

/// Apply a recovery strategy to advance the parser state.
/// Returns the new position and an optional synthetic token to insert.
///
/// # Arguments
///
/// * `strategy` - The recovery strategy to apply
/// * `tokens` - The token slice being parsed
/// * `error_pos` - The position where the error occurred
/// * `file_id` - The file ID to use when creating synthetic spans
pub fn apply_recovery(
    strategy: &RecoveryStrategy,
    tokens: &[Token],
    error_pos: usize,
    file_id: FileId,
) -> (usize, Option<Token>) {
    match strategy {
        RecoveryStrategy::SkipUntil(sync_point) => {
            let new_pos = skip_until(tokens, error_pos, *sync_point);
            (new_pos, None)
        }
        RecoveryStrategy::InsertMissing(token_kind) => {
            // Create a synthetic token at the error position
            // Try to derive span from adjacent tokens for better error reporting
            let span = if error_pos > 0 && error_pos <= tokens.len() {
                // Use end of previous token
                let prev_span = tokens[error_pos - 1].span;
                Span::new(prev_span.end, prev_span.end, prev_span.file_id)
            } else if error_pos < tokens.len() {
                // Use current token's span
                tokens[error_pos].span
            } else if let Some(last) = tokens.last() {
                // Use end of last token
                Span::new(last.span.end, last.span.end, last.span.file_id)
            } else {
                // Empty token stream - use provided file_id at position 0
                Span::new(0, 0, file_id)
            };
            let synthetic_token = Token::new(token_kind.clone(), span);
            (error_pos, Some(synthetic_token))
        }
        RecoveryStrategy::DeleteCurrent => {
            // Skip this token and continue
            (error_pos + 1, None)
        }
        RecoveryStrategy::BalanceDelimiters => {
            // For now, just skip to the next statement boundary
            let new_pos = skip_until(tokens, error_pos, SyncPoint::Statement);
            (new_pos, None)
        }
    }
}

// ============================================================================
// Helper functions for recovery
// ============================================================================

/// Check if a token kind should be treated as a statement terminator.
pub fn is_statement_terminator(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Semicolon | TokenKind::RBrace | TokenKind::Eof
    )
}

/// Check if a token kind can start an expression.
pub fn can_start_expression(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Ident(_)
            | TokenKind::Integer(_)
            | TokenKind::Float(_)
            | TokenKind::Text(_)
            | TokenKind::Char(_)
            | TokenKind::ByteChar(_)
            | TokenKind::True
            | TokenKind::False
            | TokenKind::LParen
            | TokenKind::LBracket
            | TokenKind::LBrace
            | TokenKind::If
            | TokenKind::Match
            | TokenKind::For
            | TokenKind::While
            | TokenKind::Loop
            | TokenKind::Bang
            | TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Ampersand
            | TokenKind::Some
            | TokenKind::None
            | TokenKind::Ok
            | TokenKind::Err
    )
}

/// Check if a token kind can start a statement.
pub fn can_start_statement(kind: &TokenKind) -> bool {
    can_start_expression(kind)
        || matches!(
            kind,
            TokenKind::Let
                | TokenKind::Return
                | TokenKind::Break
                | TokenKind::Continue
                | TokenKind::Defer
                | TokenKind::Provide
                | TokenKind::Try
                | TokenKind::Yield
                | TokenKind::Spawn
        )
}

/// Check if a token kind can start an item.
pub fn can_start_item(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Fn
            | TokenKind::Type
            | TokenKind::Protocol
            | TokenKind::Implement
            | TokenKind::Const
            | TokenKind::Static
            | TokenKind::Module
            | TokenKind::Mount
            | TokenKind::Link
            | TokenKind::Extern
            | TokenKind::Context
            | TokenKind::Pub
            | TokenKind::Public
            | TokenKind::At
            | TokenKind::Async
            | TokenKind::Unsafe
            | TokenKind::Pure
            | TokenKind::Meta
            | TokenKind::Theorem
            | TokenKind::Axiom
            | TokenKind::Lemma
            | TokenKind::Corollary
            | TokenKind::Proof
            | TokenKind::Ffi
            | TokenKind::View
            | TokenKind::Layer
    )
}

/// Get a helpful error message for unexpected token.
pub fn unexpected_token_message(found: &TokenKind, context: &str) -> String {
    let found_desc = found.description();
    if context.is_empty() {
        format!("unexpected {}", found_desc)
    } else {
        format!("unexpected {} while parsing {}", found_desc, context)
    }
}

/// Get a helpful error message for missing token.
pub fn missing_token_message(expected: &str, context: &str) -> String {
    if context.is_empty() {
        format!("expected {}", expected)
    } else {
        format!("expected {} in {}", expected, context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delimiter_from_opening() {
        assert_eq!(Delimiter::from_opening(TokenKind::LParen), Some(Delimiter::Paren));
        assert_eq!(Delimiter::from_opening(TokenKind::LBracket), Some(Delimiter::Bracket));
        assert_eq!(Delimiter::from_opening(TokenKind::LBrace), Some(Delimiter::Brace));
        assert_eq!(Delimiter::from_opening(TokenKind::Lt), Some(Delimiter::Angle));
        assert_eq!(Delimiter::from_opening(TokenKind::Plus), None);
    }

    #[test]
    fn test_sync_point_matches() {
        assert!(SyncPoint::Semicolon.matches(&TokenKind::Semicolon));
        assert!(!SyncPoint::Semicolon.matches(&TokenKind::Comma));

        assert!(SyncPoint::Statement.matches(&TokenKind::Fn));
        assert!(SyncPoint::Statement.matches(&TokenKind::Let));
        assert!(SyncPoint::Statement.matches(&TokenKind::Semicolon));
    }

    #[test]
    fn test_can_start_helpers() {
        assert!(can_start_expression(&TokenKind::Ident(Text::from("x"))));
        assert!(can_start_expression(&TokenKind::If));
        assert!(!can_start_expression(&TokenKind::Fn));

        assert!(can_start_statement(&TokenKind::Let));
        assert!(can_start_statement(&TokenKind::Return));
        assert!(can_start_statement(&TokenKind::If));

        assert!(can_start_item(&TokenKind::Fn));
        assert!(can_start_item(&TokenKind::Type));
        assert!(!can_start_item(&TokenKind::Let));
    }
}
