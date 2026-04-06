#![allow(unexpected_cfgs)]
//! Lexer for the Verum programming language.
//!
//! This crate provides a high-performance lexer built on top of the `logos` crate,
//! which uses code generation to produce fast, zero-allocation tokenization.
//!
//! # Overview
//!
//! The lexer tokenizes Verum source code into a stream of tokens according to the
//! language specification. Verum's lexical grammar defines whitespace and comments
//! (line `//` and block `/* */`), Unicode identifiers (ident_start = letter | '_',
//! ident_continue = letter | digit | '_'), ~41 keywords (3 reserved: `let`, `fn`, `is`;
//! plus primary, control flow, async, modifier, FFI, module, and additional keywords),
//! numeric literals (decimal, hex `0x`, binary `0b`, octal `0o`, floats with exponents,
//! optional unit suffixes), text literals (plain, multiline `"""`, interpolated `f"..."`,
//! tagged `tag#"..."`, contract `contract#"..."`), and operators/delimiters.
//!
//! # Features
//!
//! - **Zero-copy**: The lexer operates directly on the source string slice
//! - **Fast**: Uses logos for optimized DFA-based lexing
//! - **Complete**: Supports all Verum syntax including:
//!   - Keywords (`fn`, `type`, `let`, `match`, `import`, etc.)
//!   - Operators (`|>`, `?.`, `??`, `&`, `%`, etc.)
//!   - CBGR references (`&T`, `&mut T`)
//!   - Ownership references (`%T`, `%mut T`)
//!   - Literals (integers, floats, strings, chars, booleans)
//!   - Comments (line and block)
//! - **Error recovery**: Handles invalid tokens gracefully
//! - **Location tracking**: Preserves source spans for error reporting
//!
//! # Example
//!
//! ```
//! use verum_lexer::{Lexer, TokenKind};
//! use verum_ast::span::FileId;
//! use verum_common::List;
//!
//! let source = "fn add(x: Int, y: Int) -> Int { x + y }";
//! let file_id = FileId::new(0);
//! let lexer = Lexer::new(source, file_id);
//!
//! // Tokenize the entire input
//! let tokens: List<_> = lexer.map(|r| r.unwrap()).collect();
//!
//! // First token should be 'fn'
//! assert!(matches!(tokens[0].kind, TokenKind::Fn));
//! ```
//!
//! # Module Structure
//!
//! - [`token`]: Token type definitions and categorization
//! - [`lexer`]: The main lexer implementation
//! - [`error`]: Lexical error types

#![allow(dead_code)]
// Suppress informational clippy lints
#![allow(clippy::result_large_err)]
#![allow(clippy::large_enum_variant)]
#![allow(clippy::type_complexity)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::collapsible_match)]

pub mod error;
pub mod lexer;
pub mod lossless;
pub mod token;

pub use error::{LexResult, VerumError};
pub use lexer::{Lexer, LookaheadLexer};
pub use lossless::{LosslessLexer, RichToken, Trivia, TriviaItem, TriviaKind};
pub use token::{
    FloatLiteral, IntegerLiteral, InterpolatedStringLiteral, TaggedLiteralData,
    TaggedLiteralDelimiter, Token, TokenKind,
};

// Backwards compatibility alias
pub type LexError = VerumError;

#[cfg(test)]
mod test_ffi_tokenization {
    use super::*;
    use verum_ast::FileId;

    #[test]
    fn test_ffi_keyword_is_tokenized_correctly() {
        let source = "ffi";
        let file_id = FileId::new(0);
        let lexer = Lexer::new(source, file_id);

        let tokens: Vec<_> = lexer.filter_map(|t| t.ok()).collect();
        // Should have Ffi and Eof tokens
        assert!(
            !tokens.is_empty(),
            "Expected at least 1 token, got {}: {:?}",
            tokens.len(),
            tokens
        );

        match &tokens[0].kind {
            TokenKind::Ffi => {} // SUCCESS
            TokenKind::Ident(text) => panic!("FAIL: 'ffi' tokenized as Ident({})", text),
            other => panic!("FAIL: 'ffi' tokenized as {:?}", other),
        }
    }

    #[test]
    fn test_ffi_boundary_tokens() {
        let source = "ffi LibMath { }";
        let file_id = FileId::new(0);
        let lexer = Lexer::new(source, file_id);

        let tokens: Vec<_> = lexer.filter_map(|t| t.ok()).collect();
        assert!(
            tokens.len() >= 4,
            "Expected at least 4 tokens, got {}: {:?}",
            tokens.len(),
            tokens
        );

        assert!(
            matches!(&tokens[0].kind, TokenKind::Ffi),
            "First token should be Ffi, got {:?}",
            tokens[0].kind
        );
        assert!(
            matches!(&tokens[1].kind, TokenKind::Ident(_)),
            "Second token should be Ident, got {:?}",
            tokens[1].kind
        );
        assert!(
            matches!(&tokens[2].kind, TokenKind::LBrace),
            "Third token should be LBrace, got {:?}",
            tokens[2].kind
        );
        assert!(
            matches!(&tokens[3].kind, TokenKind::RBrace),
            "Fourth token should be RBrace, got {:?}",
            tokens[3].kind
        );
    }
}

#[test]
fn test_requires_keyword() {
    use verum_ast::FileId;
    let source = "requires";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);

    let tokens: Vec<_> = lexer.filter_map(|t| t.ok()).collect();
    assert!(!tokens.is_empty());

    match &tokens[0].kind {
        TokenKind::Requires => {} // SUCCESS
        TokenKind::Ident(text) => panic!("FAIL: 'requires' tokenized as Ident({})", text),
        other => panic!("FAIL: 'requires' tokenized as {:?}", other),
    }
}
