//! Error types for the lexer.
//!
//! This module re-exports the unified error types from verum_common and provides
//! domain-specific helper functions for lexical analysis errors.

use verum_ast::span::Span;
pub use verum_common::error::{ErrorKind, ErrorLocation, Result as VerumResult, VerumError};
use verum_common::global_span_to_line_col;

/// Result type for lexical operations.
pub type LexResult<T> = Result<T, VerumError>;

// Lexer-specific error helper functions
// Note: We use helper functions instead of impl blocks because VerumError
// is defined in verum_common and Rust doesn't allow inherent impls on foreign types.

/// Helper to create a lexer error with proper file:line:column location
fn make_lex_error(message: &str, span: Span) -> VerumError {
    let line_col = global_span_to_line_col(span);
    VerumError::lex(message).at_location(
        line_col.file.clone(),
        line_col.line as u32,
        line_col.column as u32,
    )
}

/// Create an invalid token error
pub fn invalid_token(span: Span) -> VerumError {
    make_lex_error("invalid token", span)
}

/// Create an unterminated string literal error
pub fn unterminated_string(span: Span) -> VerumError {
    make_lex_error("unterminated string literal", span)
}

/// Create an unterminated character literal error
pub fn unterminated_char(span: Span) -> VerumError {
    make_lex_error("unterminated character literal", span)
}

/// Create an invalid escape sequence error
pub fn invalid_escape(span: Span) -> VerumError {
    make_lex_error("invalid escape sequence", span)
}

/// Create an invalid number literal error
pub fn invalid_number(span: Span) -> VerumError {
    make_lex_error("invalid number literal", span)
}

/// Create an unexpected end of file error
pub fn unexpected_eof() -> VerumError {
    VerumError::lex("unexpected end of file")
}

/// Create an invalid UTF-8 error
pub fn invalid_utf8(span: Span) -> VerumError {
    make_lex_error("invalid UTF-8", span)
}

/// Create an unterminated block comment error
pub fn unterminated_block_comment(span: Span) -> VerumError {
    make_lex_error("unterminated block comment", span)
}
