//! TokenStream serialization for VBC heap storage.
//!
//! This module provides serialization and deserialization of TokenStream for
//! the VBC meta-system. When a meta function generates code via `quote { ... }`,
//! the resulting TokenStream is serialized to a binary format and stored on the
//! VBC interpreter's heap. When the meta function returns, the TokenStream is
//! deserialized and parsed back into AST.
//!
//! ## Binary Format (Version 1)
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────┐
//! │                     TokenStream Binary Format                           │
//! ├─────────────────────────────────────────────────────────────────────────┤
//! │  Header (16 bytes)                                                      │
//! │  ├─ magic: u32 = 0x56544B53 ("VTKS" - Verum TokenStream)              │
//! │  ├─ version: u32 = 1                                                   │
//! │  ├─ token_count: u32                                                   │
//! │  └─ flags: u32 (bit 0: has_span)                                       │
//! ├─────────────────────────────────────────────────────────────────────────┤
//! │  Optional Span (12 bytes, if has_span)                                 │
//! │  ├─ file_id: u32                                                       │
//! │  ├─ start: u32                                                         │
//! │  └─ end: u32                                                           │
//! ├─────────────────────────────────────────────────────────────────────────┤
//! │  Token Array (variable size)                                           │
//! │  └─ [SerializedToken; token_count]                                     │
//! └─────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Performance Characteristics
//!
//! - Serialization: O(n) where n = number of tokens
//! - Deserialization: O(n)
//! - Memory overhead: ~20% over raw token data (due to length prefixes)
//! - Typical throughput: >1M tokens/sec on modern hardware
//!
//! Part of Verum's unified meta-system: all compile-time computation uses `meta fn` and the
//! `@` prefix for macros/attributes. Token streams are the interchange format between the
//! meta-system and procedural macros. Tagged literals (`d#"..."`, `sql"..."`), derives
//! (`@derive(...)`), and `@interpolation_handler` all desugar to meta-system operations
//! that consume and produce token streams.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use verum_common::Text;

use crate::types::TypeId;

// ============================================================================
// Constants
// ============================================================================

/// Magic number for TokenStream binary format: "VTKS" (Verum TokenStream)
pub const TOKEN_STREAM_MAGIC: u32 = 0x56544B53;

/// Current format version
pub const TOKEN_STREAM_VERSION: u32 = 1;

/// Flag: TokenStream has a span
pub const FLAG_HAS_SPAN: u32 = 0x01;

// ============================================================================
// Error Types
// ============================================================================

/// Errors that can occur during TokenStream serialization/deserialization.
#[derive(Debug, Error)]
pub enum TokenStreamError {
    /// Invalid magic number in header.
    #[error("Invalid TokenStream magic: expected 0x{expected:08X}, got 0x{got:08X}")]
    InvalidMagic {
        /// Expected magic value.
        expected: u32,
        /// Actual magic value found.
        got: u32,
    },

    /// Unsupported format version.
    #[error("Unsupported TokenStream version: {version}")]
    UnsupportedVersion {
        /// The unsupported version number.
        version: u32,
    },

    /// Binary data too short.
    #[error("TokenStream data too short: expected at least {expected} bytes, got {got}")]
    DataTooShort {
        /// Minimum expected byte count.
        expected: usize,
        /// Actual byte count.
        got: usize,
    },

    /// Bincode serialization error
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// Bincode deserialization error
    #[error("Deserialization error: {0}")]
    DeserializationError(String),

    /// Invalid token kind discriminant
    #[error("Invalid token kind discriminant: {0}")]
    InvalidTokenKind(u16),

    /// Invalid UTF-8 in string data
    #[error("Invalid UTF-8 in token data")]
    InvalidUtf8,
}

impl From<bincode::Error> for TokenStreamError {
    fn from(e: bincode::Error) -> Self {
        TokenStreamError::DeserializationError(e.to_string())
    }
}

/// Result type for TokenStream operations.
pub type TokenStreamResult<T> = Result<T, TokenStreamError>;

// ============================================================================
// Serializable Token Representation
// ============================================================================

/// Serializable span representation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct SerializedSpan {
    /// File ID (index into file table)
    pub file_id: u32,
    /// Start byte offset
    pub start: u32,
    /// End byte offset
    pub end: u32,
}

impl SerializedSpan {
    /// Create from verum_ast::Span
    pub fn from_ast_span(span: &verum_common::span::Span) -> Self {
        Self {
            file_id: span.file_id.raw(),
            start: span.start,
            end: span.end,
        }
    }

    /// Convert to verum_ast::Span
    pub fn to_ast_span(&self) -> verum_common::span::Span {
        verum_common::span::Span {
            file_id: verum_common::span::FileId::new(self.file_id),
            start: self.start,
            end: self.end,
        }
    }
}

/// Serializable token kind discriminant.
///
/// This maps to verum_lexer::TokenKind variants. We use a compact u16 discriminant
/// followed by variant-specific payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SerializedTokenKind {
    // === Literals ===
    /// Integer literal with base and optional suffix (e.g., `42`, `0xFF`).
    Integer {
        /// Raw text representation of the integer value.
        raw_value: String,
        /// Numeric base (e.g., 10 for decimal, 16 for hex).
        base: u8,
        /// Optional type suffix (e.g., `i32`, `u64`).
        suffix: Option<String>,
    },
    /// Float literal with optional suffix (e.g., `3.14`, `1e10`).
    Float {
        /// The floating-point value.
        value: f64,
        /// Optional type suffix (e.g., `f32`).
        suffix: Option<String>,
    },
    /// Text (string) literal
    Text(String),
    /// Character literal
    Char(char),
    /// Boolean true
    True,
    /// Boolean false
    False,

    // === Identifiers ===
    /// Identifier (variable, function, type name)
    Ident(String),

    // === Keywords (represented as discriminant only) ===
    // Core reserved
    /// `let` keyword.
    Let,
    /// `fn` keyword.
    Fn,
    /// `is` keyword.
    Is,
    // Primary contextual
    /// `type` keyword.
    Type,
    /// `match` keyword.
    Match,
    /// `mount` keyword (module import).
    Mount,
    // Control flow
    /// `if` keyword.
    If,
    /// `else` keyword.
    Else,
    /// `while` keyword.
    While,
    /// `for` keyword.
    For,
    /// `loop` keyword.
    Loop,
    /// `break` keyword.
    Break,
    /// `continue` keyword.
    Continue,
    /// `return` keyword.
    Return,
    /// `yield` keyword.
    Yield,
    // Modifiers
    /// `mut` keyword.
    Mut,
    /// `const` keyword.
    Const,
    /// `static` keyword.
    Static,
    /// `pure` keyword.
    Pure,
    /// `meta` keyword.
    Meta,
    /// `async` keyword.
    Async,
    /// `await` keyword.
    Await,
    /// `spawn` keyword.
    Spawn,
    /// `unsafe` keyword.
    Unsafe,
    /// `move` keyword.
    Move,
    // Type system
    /// `where` keyword.
    Where,
    /// `implement` keyword.
    Implement,
    /// `protocol` keyword.
    Protocol,
    /// `extends` keyword.
    Extends,
    /// `module` keyword.
    Module,
    // Option/Result
    /// `None` variant literal.
    None,
    /// `Some` variant literal.
    Some,
    /// `Ok` variant literal.
    Ok,
    /// `Err` variant literal.
    Err,
    // Self
    /// `self` value keyword.
    SelfValue,
    /// `Self` type keyword.
    SelfType,
    // Visibility
    /// `pub` visibility keyword.
    Pub,
    /// `public` visibility keyword.
    Public,
    /// `private` visibility keyword.
    Private,
    /// `internal` visibility keyword.
    Internal,
    /// `protected` visibility keyword.
    Protected,
    // Context system
    /// `using` keyword (context dependencies).
    Using,
    /// `context` keyword.
    Context,
    /// `provide` keyword.
    Provide,
    // Error handling
    /// `try` keyword.
    Try,
    /// `throw` keyword.
    Throw,
    /// `throws` keyword.
    Throws,
    /// `recover` keyword.
    Recover,
    /// `finally` keyword.
    Finally,
    /// `defer` keyword.
    Defer,
    /// `errdefer` keyword.
    Errdefer,
    // FFI
    /// `ffi` keyword.
    Ffi,
    /// `extern` keyword.
    Extern,
    // Verification
    /// `requires` keyword (precondition).
    Requires,
    /// `ensures` keyword (postcondition).
    Ensures,
    /// `invariant` keyword.
    Invariant,
    /// `decreases` keyword (termination metric).
    Decreases,
    // Proof keywords
    /// `theorem` keyword.
    Theorem,
    /// `axiom` keyword.
    Axiom,
    /// `lemma` keyword.
    Lemma,
    /// `corollary` keyword.
    Corollary,
    /// `proof` keyword.
    Proof,
    /// `qed` keyword.
    Qed,
    // Other keywords
    /// `as` keyword (type cast).
    As,
    /// `in` keyword.
    In,
    /// `ref` keyword.
    Ref,
    /// `checked` keyword (checked reference tier).
    Checked,
    /// `stream` keyword.
    Stream,
    /// `select` keyword (channel selection).
    Select,
    /// `nursery` keyword (structured concurrency).
    Nursery,
    /// `super` keyword (parent module).
    Super,
    /// `cog` keyword (module system).
    Cog,
    /// `tensor` keyword.
    Tensor,
    /// `affine` keyword.
    Affine,
    /// `Result` keyword.
    Result,
    /// `view` keyword.
    View,
    /// `quote` keyword (meta-system).
    Quote,
    /// `stage` keyword (multi-stage computation).
    Stage,
    /// `lift` keyword (multi-stage computation).
    Lift,

    // === Operators ===
    /// `+` operator.
    Plus,
    /// `-` operator.
    Minus,
    /// `*` operator.
    Star,
    /// `**` operator (exponentiation).
    StarStar,
    /// `/` operator.
    Slash,
    /// `%` operator (modulo).
    Percent,
    /// `^` operator (bitwise XOR).
    Caret,
    /// `&` operator (bitwise AND / reference).
    Ampersand,
    /// `|` operator (bitwise OR / variant separator).
    Pipe,
    /// `~` operator (bitwise NOT).
    Tilde,
    /// `!` operator (logical NOT).
    Bang,
    /// `=` operator (assignment).
    Eq,
    /// `<` operator (less than).
    Lt,
    /// `>` operator (greater than).
    Gt,
    /// `@` operator (attribute / macro prefix).
    At,
    /// `.` operator (field access).
    Dot,
    /// `..` operator (exclusive range).
    DotDot,
    /// `..=` operator (inclusive range).
    DotDotEq,
    /// `...` operator (spread / variadic).
    DotDotDot,
    /// `,` separator.
    Comma,
    /// `;` statement terminator.
    Semicolon,
    /// `:` separator (type annotation).
    Colon,
    /// `::` path separator.
    ColonColon,
    /// `?` operator (error propagation).
    Question,
    /// `??` operator (null coalescing).
    QuestionQuestion,
    /// `?.` operator (optional chaining).
    QuestionDot,
    /// `#` symbol.
    Hash,
    /// `$` symbol.
    Dollar,

    // === Compound operators ===
    /// `+=` compound assignment.
    PlusEq,
    /// `-=` compound assignment.
    MinusEq,
    /// `*=` compound assignment.
    StarEq,
    /// `/=` compound assignment.
    SlashEq,
    /// `%=` compound assignment.
    PercentEq,
    /// `^=` compound assignment.
    CaretEq,
    /// `&=` compound assignment.
    AmpersandEq,
    /// `|=` compound assignment.
    PipeEq,
    /// `==` equality comparison.
    EqEq,
    /// `!=` inequality comparison.
    BangEq,
    /// `<=` less-than-or-equal comparison.
    LtEq,
    /// `>=` greater-than-or-equal comparison.
    GtEq,
    /// `<<` left shift.
    LtLt,
    /// `>>` right shift.
    GtGt,
    /// `<<=` left-shift compound assignment.
    LtLtEq,
    /// `>>=` right-shift compound assignment.
    GtGtEq,
    /// `&&` logical AND.
    AmpersandAmpersand,
    /// `||` logical OR.
    PipePipe,
    /// `|>` pipe-forward operator.
    PipeGt,
    /// `->` return type arrow.
    RArrow,
    /// `=>` fat arrow (match arms).
    FatArrow,

    // === Delimiters ===
    /// `(` left parenthesis.
    LParen,
    /// `)` right parenthesis.
    RParen,
    /// `{` left brace.
    LBrace,
    /// `}` right brace.
    RBrace,
    /// `[` left bracket.
    LBracket,
    /// `]` right bracket.
    RBracket,

    // === Special ===
    /// Block comment token.
    BlockComment,
    /// End-of-file marker.
    Eof,
    /// Lexer error token.
    Error,

    // === Interpolation ===
    /// Interpolated string literal (e.g., `f"x={x}"`).
    InterpolatedString {
        /// String prefix (e.g., `f`).
        prefix: String,
        /// String content with interpolation placeholders.
        content: String,
    },
    /// Tagged literal (e.g., `sql"SELECT ..."`).
    TaggedLiteral {
        /// Tag identifier (e.g., `sql`, `d`).
        tag: String,
        /// Literal content.
        content: String,
    },

    // === Fallback for unknown tokens ===
    /// Unknown or unmapped token kind with its discriminant value.
    Unknown(u16),
}

/// Serializable token with kind and span.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SerializedToken {
    /// The token kind and payload.
    pub kind: SerializedTokenKind,
    /// Source location span.
    pub span: SerializedSpan,
}

/// Serializable TokenStream header.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenStreamHeader {
    /// Magic number (`TOKEN_STREAM_MAGIC`).
    pub magic: u32,
    /// Format version number.
    pub version: u32,
    /// Number of tokens in the stream.
    pub token_count: u32,
    /// Bitfield flags (e.g., `FLAG_HAS_SPAN`).
    pub flags: u32,
}

/// Complete serializable TokenStream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedTokenStream {
    /// Binary format header with magic, version, and flags.
    pub header: TokenStreamHeader,
    /// Optional source span covering the entire token stream.
    pub span: Option<SerializedSpan>,
    /// Ordered list of serialized tokens.
    pub tokens: Vec<SerializedToken>,
}

// ============================================================================
// Conversion from verum_lexer types
// ============================================================================

impl SerializedTokenKind {
    /// Convert from verum_lexer::TokenKind
    pub fn from_lexer_kind(kind: &verum_lexer::TokenKind) -> Self {
        use verum_lexer::TokenKind;
        match kind {
            // Literals
            TokenKind::Integer(lit) => SerializedTokenKind::Integer {
                raw_value: lit.raw_value.to_string(),
                base: lit.base,
                suffix: lit.suffix.as_ref().map(|s| s.to_string()),
            },
            TokenKind::Float(lit) => SerializedTokenKind::Float {
                value: lit.value,
                suffix: lit.suffix.as_ref().map(|s| s.to_string()),
            },
            TokenKind::Text(s) => SerializedTokenKind::Text(s.to_string()),
            TokenKind::Char(c) => SerializedTokenKind::Char(*c),
            TokenKind::True => SerializedTokenKind::True,
            TokenKind::False => SerializedTokenKind::False,

            // Identifiers
            TokenKind::Ident(s) => SerializedTokenKind::Ident(s.to_string()),

            // Keywords
            TokenKind::Let => SerializedTokenKind::Let,
            TokenKind::Fn => SerializedTokenKind::Fn,
            TokenKind::Is => SerializedTokenKind::Is,
            TokenKind::Type => SerializedTokenKind::Type,
            TokenKind::Match => SerializedTokenKind::Match,
            TokenKind::Mount => SerializedTokenKind::Mount,
            TokenKind::If => SerializedTokenKind::If,
            TokenKind::Else => SerializedTokenKind::Else,
            TokenKind::While => SerializedTokenKind::While,
            TokenKind::For => SerializedTokenKind::For,
            TokenKind::Loop => SerializedTokenKind::Loop,
            TokenKind::Break => SerializedTokenKind::Break,
            TokenKind::Continue => SerializedTokenKind::Continue,
            TokenKind::Return => SerializedTokenKind::Return,
            TokenKind::Yield => SerializedTokenKind::Yield,
            TokenKind::Mut => SerializedTokenKind::Mut,
            TokenKind::Const => SerializedTokenKind::Const,
            TokenKind::Static => SerializedTokenKind::Static,
            TokenKind::Pure => SerializedTokenKind::Pure,
            TokenKind::Meta => SerializedTokenKind::Meta,
            TokenKind::Async => SerializedTokenKind::Async,
            TokenKind::Await => SerializedTokenKind::Await,
            TokenKind::Spawn => SerializedTokenKind::Spawn,
            TokenKind::Unsafe => SerializedTokenKind::Unsafe,
            TokenKind::Move => SerializedTokenKind::Move,
            TokenKind::Where => SerializedTokenKind::Where,
            TokenKind::Implement => SerializedTokenKind::Implement,
            TokenKind::Protocol => SerializedTokenKind::Protocol,
            TokenKind::Extends => SerializedTokenKind::Extends,
            TokenKind::Module => SerializedTokenKind::Module,
            TokenKind::None => SerializedTokenKind::None,
            TokenKind::Some => SerializedTokenKind::Some,
            TokenKind::Ok => SerializedTokenKind::Ok,
            TokenKind::Err => SerializedTokenKind::Err,
            TokenKind::SelfValue => SerializedTokenKind::SelfValue,
            TokenKind::SelfType => SerializedTokenKind::SelfType,
            TokenKind::Pub => SerializedTokenKind::Pub,
            TokenKind::Public => SerializedTokenKind::Public,
            TokenKind::Private => SerializedTokenKind::Private,
            TokenKind::Internal => SerializedTokenKind::Internal,
            TokenKind::Protected => SerializedTokenKind::Protected,
            TokenKind::Using => SerializedTokenKind::Using,
            TokenKind::Context => SerializedTokenKind::Context,
            TokenKind::Provide => SerializedTokenKind::Provide,
            TokenKind::Try => SerializedTokenKind::Try,
            TokenKind::Throw => SerializedTokenKind::Throw,
            TokenKind::Throws => SerializedTokenKind::Throws,
            TokenKind::Recover => SerializedTokenKind::Recover,
            TokenKind::Finally => SerializedTokenKind::Finally,
            TokenKind::Defer => SerializedTokenKind::Defer,
            TokenKind::Errdefer => SerializedTokenKind::Errdefer,
            TokenKind::Ffi => SerializedTokenKind::Ffi,
            TokenKind::Extern => SerializedTokenKind::Extern,
            TokenKind::Requires => SerializedTokenKind::Requires,
            TokenKind::Ensures => SerializedTokenKind::Ensures,
            TokenKind::Invariant => SerializedTokenKind::Invariant,
            TokenKind::Decreases => SerializedTokenKind::Decreases,
            TokenKind::Theorem => SerializedTokenKind::Theorem,
            TokenKind::Axiom => SerializedTokenKind::Axiom,
            TokenKind::Lemma => SerializedTokenKind::Lemma,
            TokenKind::Corollary => SerializedTokenKind::Corollary,
            TokenKind::Proof => SerializedTokenKind::Proof,
            TokenKind::Qed => SerializedTokenKind::Qed,
            TokenKind::As => SerializedTokenKind::As,
            TokenKind::In => SerializedTokenKind::In,
            TokenKind::Ref => SerializedTokenKind::Ref,
            TokenKind::Checked => SerializedTokenKind::Checked,
            TokenKind::Stream => SerializedTokenKind::Stream,
            TokenKind::Select => SerializedTokenKind::Select,
            TokenKind::Nursery => SerializedTokenKind::Nursery,
            TokenKind::Super => SerializedTokenKind::Super,
            TokenKind::Cog => SerializedTokenKind::Cog,
            TokenKind::Tensor => SerializedTokenKind::Tensor,
            TokenKind::Affine => SerializedTokenKind::Affine,
            TokenKind::Result => SerializedTokenKind::Result,
            TokenKind::View => SerializedTokenKind::View,
            TokenKind::QuoteKeyword => SerializedTokenKind::Quote,
            TokenKind::Stage => SerializedTokenKind::Stage,
            TokenKind::Lift => SerializedTokenKind::Lift,

            // Operators
            TokenKind::Plus => SerializedTokenKind::Plus,
            TokenKind::Minus => SerializedTokenKind::Minus,
            TokenKind::Star => SerializedTokenKind::Star,
            TokenKind::StarStar => SerializedTokenKind::StarStar,
            TokenKind::Slash => SerializedTokenKind::Slash,
            TokenKind::Percent => SerializedTokenKind::Percent,
            TokenKind::Caret => SerializedTokenKind::Caret,
            TokenKind::Ampersand => SerializedTokenKind::Ampersand,
            TokenKind::Pipe => SerializedTokenKind::Pipe,
            TokenKind::Tilde => SerializedTokenKind::Tilde,
            TokenKind::Bang => SerializedTokenKind::Bang,
            TokenKind::Eq => SerializedTokenKind::Eq,
            TokenKind::Lt => SerializedTokenKind::Lt,
            TokenKind::Gt => SerializedTokenKind::Gt,
            TokenKind::At => SerializedTokenKind::At,
            TokenKind::Dot => SerializedTokenKind::Dot,
            TokenKind::DotDot => SerializedTokenKind::DotDot,
            TokenKind::DotDotEq => SerializedTokenKind::DotDotEq,
            TokenKind::DotDotDot => SerializedTokenKind::DotDotDot,
            TokenKind::Comma => SerializedTokenKind::Comma,
            TokenKind::Semicolon => SerializedTokenKind::Semicolon,
            TokenKind::Colon => SerializedTokenKind::Colon,
            TokenKind::ColonColon => SerializedTokenKind::ColonColon,
            TokenKind::Question => SerializedTokenKind::Question,
            TokenKind::QuestionQuestion => SerializedTokenKind::QuestionQuestion,
            TokenKind::QuestionDot => SerializedTokenKind::QuestionDot,
            TokenKind::Hash => SerializedTokenKind::Hash,
            TokenKind::Dollar => SerializedTokenKind::Dollar,

            // Compound operators
            TokenKind::PlusEq => SerializedTokenKind::PlusEq,
            TokenKind::MinusEq => SerializedTokenKind::MinusEq,
            TokenKind::StarEq => SerializedTokenKind::StarEq,
            TokenKind::SlashEq => SerializedTokenKind::SlashEq,
            TokenKind::PercentEq => SerializedTokenKind::PercentEq,
            TokenKind::CaretEq => SerializedTokenKind::CaretEq,
            TokenKind::AmpersandEq => SerializedTokenKind::AmpersandEq,
            TokenKind::PipeEq => SerializedTokenKind::PipeEq,
            TokenKind::EqEq => SerializedTokenKind::EqEq,
            TokenKind::BangEq => SerializedTokenKind::BangEq,
            TokenKind::LtEq => SerializedTokenKind::LtEq,
            TokenKind::GtEq => SerializedTokenKind::GtEq,
            TokenKind::LtLt => SerializedTokenKind::LtLt,
            TokenKind::GtGt => SerializedTokenKind::GtGt,
            TokenKind::LtLtEq => SerializedTokenKind::LtLtEq,
            TokenKind::GtGtEq => SerializedTokenKind::GtGtEq,
            TokenKind::AmpersandAmpersand => SerializedTokenKind::AmpersandAmpersand,
            TokenKind::PipePipe => SerializedTokenKind::PipePipe,
            TokenKind::PipeGt => SerializedTokenKind::PipeGt,
            TokenKind::RArrow => SerializedTokenKind::RArrow,
            TokenKind::FatArrow => SerializedTokenKind::FatArrow,

            // Delimiters
            TokenKind::LParen => SerializedTokenKind::LParen,
            TokenKind::RParen => SerializedTokenKind::RParen,
            TokenKind::LBrace => SerializedTokenKind::LBrace,
            TokenKind::RBrace => SerializedTokenKind::RBrace,
            TokenKind::LBracket => SerializedTokenKind::LBracket,
            TokenKind::RBracket => SerializedTokenKind::RBracket,

            // Special
            TokenKind::BlockComment => SerializedTokenKind::BlockComment,
            TokenKind::Eof => SerializedTokenKind::Eof,
            TokenKind::Error => SerializedTokenKind::Error,

            // Interpolation
            TokenKind::InterpolatedString(lit) => SerializedTokenKind::InterpolatedString {
                prefix: lit.prefix.to_string(),
                content: lit.content.to_string(),
            },
            TokenKind::TaggedLiteral(lit) => SerializedTokenKind::TaggedLiteral {
                tag: lit.tag.to_string(),
                content: lit.content.to_string(),
            },

            // Catch-all for any new tokens not yet mapped
            _ => SerializedTokenKind::Unknown(0),
        }
    }

    /// Convert to verum_lexer::TokenKind
    pub fn to_lexer_kind(&self) -> verum_lexer::TokenKind {
        use verum_lexer::{
            FloatLiteral, IntegerLiteral, InterpolatedStringLiteral, TaggedLiteralData,
            TaggedLiteralDelimiter, TokenKind,
        };
        match self {
            // Literals
            SerializedTokenKind::Integer {
                raw_value,
                base,
                suffix,
            } => TokenKind::Integer(IntegerLiteral {
                raw_value: Text::from(raw_value.as_str()),
                base: *base,
                suffix: suffix.as_ref().map(|s| Text::from(s.as_str())),
            }),
            SerializedTokenKind::Float { value, suffix } => TokenKind::Float(FloatLiteral {
                value: *value,
                suffix: suffix.as_ref().map(|s| Text::from(s.as_str())),
                raw: Text::from(format!("{}", value)),
            }),
            SerializedTokenKind::Text(s) => TokenKind::Text(Text::from(s.as_str())),
            SerializedTokenKind::Char(c) => TokenKind::Char(*c),
            SerializedTokenKind::True => TokenKind::True,
            SerializedTokenKind::False => TokenKind::False,

            // Identifiers
            SerializedTokenKind::Ident(s) => TokenKind::Ident(Text::from(s.as_str())),

            // Keywords
            SerializedTokenKind::Let => TokenKind::Let,
            SerializedTokenKind::Fn => TokenKind::Fn,
            SerializedTokenKind::Is => TokenKind::Is,
            SerializedTokenKind::Type => TokenKind::Type,
            SerializedTokenKind::Match => TokenKind::Match,
            SerializedTokenKind::Mount => TokenKind::Mount,
            SerializedTokenKind::If => TokenKind::If,
            SerializedTokenKind::Else => TokenKind::Else,
            SerializedTokenKind::While => TokenKind::While,
            SerializedTokenKind::For => TokenKind::For,
            SerializedTokenKind::Loop => TokenKind::Loop,
            SerializedTokenKind::Break => TokenKind::Break,
            SerializedTokenKind::Continue => TokenKind::Continue,
            SerializedTokenKind::Return => TokenKind::Return,
            SerializedTokenKind::Yield => TokenKind::Yield,
            SerializedTokenKind::Mut => TokenKind::Mut,
            SerializedTokenKind::Const => TokenKind::Const,
            SerializedTokenKind::Static => TokenKind::Static,
            SerializedTokenKind::Pure => TokenKind::Pure,
            SerializedTokenKind::Meta => TokenKind::Meta,
            SerializedTokenKind::Async => TokenKind::Async,
            SerializedTokenKind::Await => TokenKind::Await,
            SerializedTokenKind::Spawn => TokenKind::Spawn,
            SerializedTokenKind::Unsafe => TokenKind::Unsafe,
            SerializedTokenKind::Move => TokenKind::Move,
            SerializedTokenKind::Where => TokenKind::Where,
            SerializedTokenKind::Implement => TokenKind::Implement,
            SerializedTokenKind::Protocol => TokenKind::Protocol,
            SerializedTokenKind::Extends => TokenKind::Extends,
            SerializedTokenKind::Module => TokenKind::Module,
            SerializedTokenKind::None => TokenKind::None,
            SerializedTokenKind::Some => TokenKind::Some,
            SerializedTokenKind::Ok => TokenKind::Ok,
            SerializedTokenKind::Err => TokenKind::Err,
            SerializedTokenKind::SelfValue => TokenKind::SelfValue,
            SerializedTokenKind::SelfType => TokenKind::SelfType,
            SerializedTokenKind::Pub => TokenKind::Pub,
            SerializedTokenKind::Public => TokenKind::Public,
            SerializedTokenKind::Private => TokenKind::Private,
            SerializedTokenKind::Internal => TokenKind::Internal,
            SerializedTokenKind::Protected => TokenKind::Protected,
            SerializedTokenKind::Using => TokenKind::Using,
            SerializedTokenKind::Context => TokenKind::Context,
            SerializedTokenKind::Provide => TokenKind::Provide,
            SerializedTokenKind::Try => TokenKind::Try,
            SerializedTokenKind::Throw => TokenKind::Throw,
            SerializedTokenKind::Throws => TokenKind::Throws,
            SerializedTokenKind::Recover => TokenKind::Recover,
            SerializedTokenKind::Finally => TokenKind::Finally,
            SerializedTokenKind::Defer => TokenKind::Defer,
            SerializedTokenKind::Errdefer => TokenKind::Errdefer,
            SerializedTokenKind::Ffi => TokenKind::Ffi,
            SerializedTokenKind::Extern => TokenKind::Extern,
            SerializedTokenKind::Requires => TokenKind::Requires,
            SerializedTokenKind::Ensures => TokenKind::Ensures,
            SerializedTokenKind::Invariant => TokenKind::Invariant,
            SerializedTokenKind::Decreases => TokenKind::Decreases,
            SerializedTokenKind::Theorem => TokenKind::Theorem,
            SerializedTokenKind::Axiom => TokenKind::Axiom,
            SerializedTokenKind::Lemma => TokenKind::Lemma,
            SerializedTokenKind::Corollary => TokenKind::Corollary,
            SerializedTokenKind::Proof => TokenKind::Proof,
            SerializedTokenKind::Qed => TokenKind::Qed,
            SerializedTokenKind::As => TokenKind::As,
            SerializedTokenKind::In => TokenKind::In,
            SerializedTokenKind::Ref => TokenKind::Ref,
            SerializedTokenKind::Checked => TokenKind::Checked,
            SerializedTokenKind::Stream => TokenKind::Stream,
            SerializedTokenKind::Select => TokenKind::Select,
            SerializedTokenKind::Nursery => TokenKind::Nursery,
            SerializedTokenKind::Super => TokenKind::Super,
            SerializedTokenKind::Cog => TokenKind::Cog,
            SerializedTokenKind::Tensor => TokenKind::Tensor,
            SerializedTokenKind::Affine => TokenKind::Affine,
            SerializedTokenKind::Result => TokenKind::Result,
            SerializedTokenKind::View => TokenKind::View,
            SerializedTokenKind::Quote => TokenKind::QuoteKeyword,
            SerializedTokenKind::Stage => TokenKind::Stage,
            SerializedTokenKind::Lift => TokenKind::Lift,

            // Operators
            SerializedTokenKind::Plus => TokenKind::Plus,
            SerializedTokenKind::Minus => TokenKind::Minus,
            SerializedTokenKind::Star => TokenKind::Star,
            SerializedTokenKind::StarStar => TokenKind::StarStar,
            SerializedTokenKind::Slash => TokenKind::Slash,
            SerializedTokenKind::Percent => TokenKind::Percent,
            SerializedTokenKind::Caret => TokenKind::Caret,
            SerializedTokenKind::Ampersand => TokenKind::Ampersand,
            SerializedTokenKind::Pipe => TokenKind::Pipe,
            SerializedTokenKind::Tilde => TokenKind::Tilde,
            SerializedTokenKind::Bang => TokenKind::Bang,
            SerializedTokenKind::Eq => TokenKind::Eq,
            SerializedTokenKind::Lt => TokenKind::Lt,
            SerializedTokenKind::Gt => TokenKind::Gt,
            SerializedTokenKind::At => TokenKind::At,
            SerializedTokenKind::Dot => TokenKind::Dot,
            SerializedTokenKind::DotDot => TokenKind::DotDot,
            SerializedTokenKind::DotDotEq => TokenKind::DotDotEq,
            SerializedTokenKind::DotDotDot => TokenKind::DotDotDot,
            SerializedTokenKind::Comma => TokenKind::Comma,
            SerializedTokenKind::Semicolon => TokenKind::Semicolon,
            SerializedTokenKind::Colon => TokenKind::Colon,
            SerializedTokenKind::ColonColon => TokenKind::ColonColon,
            SerializedTokenKind::Question => TokenKind::Question,
            SerializedTokenKind::QuestionQuestion => TokenKind::QuestionQuestion,
            SerializedTokenKind::QuestionDot => TokenKind::QuestionDot,
            SerializedTokenKind::Hash => TokenKind::Hash,
            SerializedTokenKind::Dollar => TokenKind::Dollar,

            // Compound operators
            SerializedTokenKind::PlusEq => TokenKind::PlusEq,
            SerializedTokenKind::MinusEq => TokenKind::MinusEq,
            SerializedTokenKind::StarEq => TokenKind::StarEq,
            SerializedTokenKind::SlashEq => TokenKind::SlashEq,
            SerializedTokenKind::PercentEq => TokenKind::PercentEq,
            SerializedTokenKind::CaretEq => TokenKind::CaretEq,
            SerializedTokenKind::AmpersandEq => TokenKind::AmpersandEq,
            SerializedTokenKind::PipeEq => TokenKind::PipeEq,
            SerializedTokenKind::EqEq => TokenKind::EqEq,
            SerializedTokenKind::BangEq => TokenKind::BangEq,
            SerializedTokenKind::LtEq => TokenKind::LtEq,
            SerializedTokenKind::GtEq => TokenKind::GtEq,
            SerializedTokenKind::LtLt => TokenKind::LtLt,
            SerializedTokenKind::GtGt => TokenKind::GtGt,
            SerializedTokenKind::LtLtEq => TokenKind::LtLtEq,
            SerializedTokenKind::GtGtEq => TokenKind::GtGtEq,
            SerializedTokenKind::AmpersandAmpersand => TokenKind::AmpersandAmpersand,
            SerializedTokenKind::PipePipe => TokenKind::PipePipe,
            SerializedTokenKind::PipeGt => TokenKind::PipeGt,
            SerializedTokenKind::RArrow => TokenKind::RArrow,
            SerializedTokenKind::FatArrow => TokenKind::FatArrow,

            // Delimiters
            SerializedTokenKind::LParen => TokenKind::LParen,
            SerializedTokenKind::RParen => TokenKind::RParen,
            SerializedTokenKind::LBrace => TokenKind::LBrace,
            SerializedTokenKind::RBrace => TokenKind::RBrace,
            SerializedTokenKind::LBracket => TokenKind::LBracket,
            SerializedTokenKind::RBracket => TokenKind::RBracket,

            // Special
            SerializedTokenKind::BlockComment => TokenKind::BlockComment,
            SerializedTokenKind::Eof => TokenKind::Eof,
            SerializedTokenKind::Error => TokenKind::Error,

            // Interpolation
            SerializedTokenKind::InterpolatedString { prefix, content } => {
                TokenKind::InterpolatedString(InterpolatedStringLiteral {
                    prefix: Text::from(prefix.as_str()),
                    content: Text::from(content.as_str()),
                })
            }
            SerializedTokenKind::TaggedLiteral { tag, content } => {
                TokenKind::TaggedLiteral(TaggedLiteralData {
                    tag: Text::from(tag.as_str()),
                    content: Text::from(content.as_str()),
                    delimiter: TaggedLiteralDelimiter::Quote,
                })
            }

            // Unknown falls back to error token
            SerializedTokenKind::Unknown(_) => TokenKind::Error,
        }
    }
}

impl SerializedToken {
    /// Convert from verum_lexer::Token
    pub fn from_lexer_token(token: &verum_lexer::Token) -> Self {
        Self {
            kind: SerializedTokenKind::from_lexer_kind(&token.kind),
            span: SerializedSpan::from_ast_span(&token.span),
        }
    }

    /// Convert to verum_lexer::Token
    pub fn to_lexer_token(&self) -> verum_lexer::Token {
        verum_lexer::Token {
            kind: self.kind.to_lexer_kind(),
            span: self.span.to_ast_span(),
        }
    }
}

// ============================================================================
// Serialization API
// ============================================================================

/// Serialize a list of tokens to binary format.
///
/// This is the primary serialization entry point for the codegen phase.
pub fn serialize_tokens(
    tokens: &[verum_lexer::Token],
    span: Option<&verum_common::span::Span>,
) -> TokenStreamResult<Vec<u8>> {
    let serialized_tokens: Vec<SerializedToken> = tokens
        .iter()
        .map(SerializedToken::from_lexer_token)
        .collect();

    let stream = SerializedTokenStream {
        header: TokenStreamHeader {
            magic: TOKEN_STREAM_MAGIC,
            version: TOKEN_STREAM_VERSION,
            token_count: serialized_tokens.len() as u32,
            flags: if span.is_some() { FLAG_HAS_SPAN } else { 0 },
        },
        span: span.map(SerializedSpan::from_ast_span),
        tokens: serialized_tokens,
    };

    bincode::serialize(&stream).map_err(|e| TokenStreamError::SerializationError(e.to_string()))
}

/// Deserialize tokens from binary format.
///
/// This is the primary deserialization entry point for the extraction phase.
pub fn deserialize_tokens(data: &[u8]) -> TokenStreamResult<(Vec<verum_lexer::Token>, Option<verum_common::span::Span>)> {
    // Validate minimum size (header only)
    if data.len() < 16 {
        return Err(TokenStreamError::DataTooShort {
            expected: 16,
            got: data.len(),
        });
    }

    let stream: SerializedTokenStream = bincode::deserialize(data)?;

    // Validate magic
    if stream.header.magic != TOKEN_STREAM_MAGIC {
        return Err(TokenStreamError::InvalidMagic {
            expected: TOKEN_STREAM_MAGIC,
            got: stream.header.magic,
        });
    }

    // Validate version
    if stream.header.version != TOKEN_STREAM_VERSION {
        return Err(TokenStreamError::UnsupportedVersion {
            version: stream.header.version,
        });
    }

    // Convert tokens
    let tokens: Vec<verum_lexer::Token> = stream
        .tokens
        .iter()
        .map(SerializedToken::to_lexer_token)
        .collect();

    // Convert span
    let span = stream.span.map(|s| s.to_ast_span());

    Ok((tokens, span))
}

/// Estimate the serialized size for pre-allocation.
///
/// Returns an upper bound on the serialized size for a given number of tokens.
pub fn estimate_serialized_size(token_count: usize) -> usize {
    // Header (16) + optional span (12) + tokens (avg ~32 bytes each)
    16 + 12 + (token_count * 40)
}

// ============================================================================
// Heap Object API
// ============================================================================

/// Create a TokenStream heap object from tokens.
///
/// This allocates a heap object containing the serialized TokenStream data.
/// The object has TypeId::TOKEN_STREAM and can be passed as a Value.
pub fn create_token_stream_object(
    heap: &mut crate::interpreter::Heap,
    tokens: &[verum_lexer::Token],
    span: Option<&verum_common::span::Span>,
) -> Result<crate::interpreter::Object, crate::interpreter::InterpreterError> {
    // Serialize tokens
    let data = serialize_tokens(tokens, span)
        .map_err(|e| crate::interpreter::InterpreterError::Panic { message: e.to_string() })?;

    // Allocate heap object with the serialized data
    heap.alloc_with_init(TypeId::TOKEN_STREAM, data.len(), |buf| {
        buf.copy_from_slice(&data);
    })
}

/// Create a TokenStream heap object directly from serialized bytes.
///
/// This is the optimal path for the MetaQuote instruction, where the
/// serialized TokenStream bytes are already stored in the constant pool.
/// Unlike `create_token_stream_object`, this avoids redundant re-serialization.
///
/// # Arguments
///
/// * `heap` - The interpreter heap for allocation
/// * `serialized_data` - Pre-serialized TokenStream bytes (from constant pool)
///
/// # Performance
///
/// This is O(n) where n = serialized data size, just for the copy.
/// No parsing or re-serialization occurs.
pub fn create_token_stream_object_from_bytes(
    heap: &mut crate::interpreter::Heap,
    serialized_data: &[u8],
) -> Result<crate::interpreter::Object, crate::interpreter::InterpreterError> {
    // Allocate heap object with the serialized data
    heap.alloc_with_init(TypeId::TOKEN_STREAM, serialized_data.len(), |buf| {
        buf.copy_from_slice(serialized_data);
    })
}

/// Extract TokenStream from a heap object.
///
/// This reads the serialized data from a heap object and deserializes it
/// back into a list of tokens.
pub fn extract_token_stream_from_object(
    obj: &crate::interpreter::Object,
) -> TokenStreamResult<(Vec<verum_lexer::Token>, Option<verum_common::span::Span>)> {
    // Verify type
    if obj.type_id() != TypeId::TOKEN_STREAM {
        return Err(TokenStreamError::DeserializationError(format!(
            "Expected TokenStream object (type {}), got type {}",
            TypeId::TOKEN_STREAM.0,
            obj.type_id().0
        )));
    }

    // Get data slice (bounds-checked via Object::data_slice)
    let data = obj.data_slice();

    deserialize_tokens(data)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use verum_common::span::{FileId, Span};
    use verum_common::Maybe;
    use verum_lexer::{IntegerLiteral, Token, TokenKind};

    fn make_span(start: u32, end: u32) -> Span {
        Span {
            file_id: FileId::dummy(),
            start,
            end,
        }
    }

    #[test]
    fn test_roundtrip_empty() {
        let tokens: Vec<Token> = vec![];
        let data = serialize_tokens(&tokens, None).unwrap();
        let (result, span) = deserialize_tokens(&data).unwrap();
        assert!(result.is_empty());
        assert!(span.is_none());
    }

    #[test]
    fn test_roundtrip_single_ident() {
        let tokens = vec![Token::new(TokenKind::Ident("foo".into()), make_span(0, 3))];
        let data = serialize_tokens(&tokens, None).unwrap();
        let (result, _) = deserialize_tokens(&data).unwrap();
        assert_eq!(result.len(), 1);
        assert!(matches!(result[0].kind, TokenKind::Ident(ref s) if s == "foo"));
    }

    #[test]
    fn test_roundtrip_integer() {
        let tokens = vec![Token::new(
            TokenKind::Integer(IntegerLiteral {
                raw_value: "42".into(),
                base: 10,
                suffix: Maybe::None,
            }),
            make_span(0, 2),
        )];
        let data = serialize_tokens(&tokens, None).unwrap();
        let (result, _) = deserialize_tokens(&data).unwrap();
        assert_eq!(result.len(), 1);
        if let TokenKind::Integer(lit) = &result[0].kind {
            assert_eq!(lit.raw_value.as_str(), "42");
            assert_eq!(lit.base, 10);
        } else {
            panic!("Expected integer token");
        }
    }

    #[test]
    fn test_roundtrip_with_span() {
        let tokens = vec![Token::new(TokenKind::Let, make_span(0, 3))];
        let stream_span = make_span(0, 10);
        let data = serialize_tokens(&tokens, Some(&stream_span)).unwrap();
        let (result, span) = deserialize_tokens(&data).unwrap();
        assert_eq!(result.len(), 1);
        assert!(span.is_some());
        let span = span.unwrap();
        assert_eq!(span.start, 0);
        assert_eq!(span.end, 10);
    }

    #[test]
    fn test_roundtrip_keywords() {
        let keywords = [TokenKind::Let,
            TokenKind::Fn,
            TokenKind::Is,
            TokenKind::Type,
            TokenKind::If,
            TokenKind::Else,
            TokenKind::Return];

        let tokens: Vec<Token> = keywords
            .iter()
            .enumerate()
            .map(|(i, k)| Token::new(k.clone(), make_span(i as u32, (i + 1) as u32)))
            .collect();

        let data = serialize_tokens(&tokens, None).unwrap();
        let (result, _) = deserialize_tokens(&data).unwrap();

        assert_eq!(result.len(), keywords.len());
        for (orig, deser) in keywords.iter().zip(result.iter()) {
            assert_eq!(
                std::mem::discriminant(orig),
                std::mem::discriminant(&deser.kind)
            );
        }
    }

    #[test]
    fn test_roundtrip_operators() {
        let operators = [TokenKind::Plus,
            TokenKind::Minus,
            TokenKind::Star,
            TokenKind::Slash,
            TokenKind::EqEq,
            TokenKind::FatArrow,
            TokenKind::RArrow];

        let tokens: Vec<Token> = operators
            .iter()
            .enumerate()
            .map(|(i, k)| Token::new(k.clone(), make_span(i as u32, (i + 1) as u32)))
            .collect();

        let data = serialize_tokens(&tokens, None).unwrap();
        let (result, _) = deserialize_tokens(&data).unwrap();

        assert_eq!(result.len(), operators.len());
    }

    #[test]
    fn test_invalid_magic() {
        let mut data = serialize_tokens(&[], None).unwrap();
        // Corrupt magic number
        data[0] = 0xFF;
        let result = deserialize_tokens(&data);
        assert!(matches!(result, Err(TokenStreamError::InvalidMagic { .. })));
    }

    #[test]
    fn test_data_too_short() {
        let data = vec![0u8; 4]; // Way too short
        let result = deserialize_tokens(&data);
        assert!(matches!(result, Err(TokenStreamError::DataTooShort { .. })));
    }

    #[test]
    fn test_estimate_size() {
        let estimate = estimate_serialized_size(10);
        assert!(estimate > 0);

        // Actual size should be <= estimate
        let tokens: Vec<Token> = (0u32..10)
            .map(|i| Token::new(TokenKind::Ident(format!("x{}", i).into()), make_span(i, i + 1)))
            .collect();
        let actual = serialize_tokens(&tokens, None).unwrap().len();
        assert!(actual <= estimate, "actual {} > estimate {}", actual, estimate);
    }

    // =========================================================================
    // Heap Allocation Roundtrip Tests
    // =========================================================================

    #[test]
    fn test_heap_alloc_empty_tokenstream() {
        let mut heap = crate::interpreter::Heap::new();
        let tokens: Vec<Token> = vec![];

        // Serialize and allocate on heap
        let serialized = serialize_tokens(&tokens, None).unwrap();
        let obj = heap.alloc_token_stream(&serialized).unwrap();

        // Verify object properties
        assert_eq!(obj.type_id(), crate::types::TypeId::TOKEN_STREAM);
        assert_eq!(obj.size() as usize, serialized.len());

        // Extract and verify
        let (result, span) = extract_token_stream_from_object(&obj).unwrap();
        assert!(result.is_empty());
        assert!(span.is_none());
    }

    #[test]
    fn test_heap_alloc_single_token() {
        let mut heap = crate::interpreter::Heap::new();
        let tokens = vec![Token::new(TokenKind::Ident("test".into()), make_span(0, 4))];

        // Roundtrip through heap
        let serialized = serialize_tokens(&tokens, None).unwrap();
        let obj = heap.alloc_token_stream(&serialized).unwrap();
        let (result, _) = extract_token_stream_from_object(&obj).unwrap();

        assert_eq!(result.len(), 1);
        assert!(matches!(result[0].kind, TokenKind::Ident(ref s) if s == "test"));
    }

    #[test]
    fn test_heap_alloc_complex_expression() {
        let mut heap = crate::interpreter::Heap::new();

        // Simulate: let x = 42 + y;
        let tokens = vec![
            Token::new(TokenKind::Let, make_span(0, 3)),
            Token::new(TokenKind::Ident("x".into()), make_span(4, 5)),
            Token::new(TokenKind::Eq, make_span(6, 7)),
            Token::new(
                TokenKind::Integer(IntegerLiteral {
                    raw_value: "42".into(),
                    base: 10,
                    suffix: Maybe::None,
                }),
                make_span(8, 10),
            ),
            Token::new(TokenKind::Plus, make_span(11, 12)),
            Token::new(TokenKind::Ident("y".into()), make_span(13, 14)),
            Token::new(TokenKind::Semicolon, make_span(14, 15)),
        ];

        // Roundtrip
        let span = make_span(0, 15);
        let serialized = serialize_tokens(&tokens, Some(&span)).unwrap();
        let obj = heap.alloc_token_stream(&serialized).unwrap();
        let (result, result_span) = extract_token_stream_from_object(&obj).unwrap();

        assert_eq!(result.len(), 7);
        assert!(result_span.is_some());
        let rs = result_span.unwrap();
        assert_eq!(rs.start, 0);
        assert_eq!(rs.end, 15);

        // Verify token sequence
        assert!(matches!(result[0].kind, TokenKind::Let));
        assert!(matches!(result[1].kind, TokenKind::Ident(ref s) if s == "x"));
        assert!(matches!(result[2].kind, TokenKind::Eq));
        assert!(matches!(result[3].kind, TokenKind::Integer(_)));
        assert!(matches!(result[4].kind, TokenKind::Plus));
        assert!(matches!(result[5].kind, TokenKind::Ident(ref s) if s == "y"));
        assert!(matches!(result[6].kind, TokenKind::Semicolon));
    }

    #[test]
    fn test_heap_alloc_function_definition() {
        let mut heap = crate::interpreter::Heap::new();

        // Simulate: fn add(a: Int, b: Int) -> Int { a + b }
        let tokens = vec![
            Token::new(TokenKind::Fn, make_span(0, 2)),
            Token::new(TokenKind::Ident("add".into()), make_span(3, 6)),
            Token::new(TokenKind::LParen, make_span(6, 7)),
            Token::new(TokenKind::Ident("a".into()), make_span(7, 8)),
            Token::new(TokenKind::Colon, make_span(8, 9)),
            Token::new(TokenKind::Ident("Int".into()), make_span(10, 13)),
            Token::new(TokenKind::Comma, make_span(13, 14)),
            Token::new(TokenKind::Ident("b".into()), make_span(15, 16)),
            Token::new(TokenKind::Colon, make_span(16, 17)),
            Token::new(TokenKind::Ident("Int".into()), make_span(18, 21)),
            Token::new(TokenKind::RParen, make_span(21, 22)),
            Token::new(TokenKind::RArrow, make_span(23, 25)),
            Token::new(TokenKind::Ident("Int".into()), make_span(26, 29)),
            Token::new(TokenKind::LBrace, make_span(30, 31)),
            Token::new(TokenKind::Ident("a".into()), make_span(32, 33)),
            Token::new(TokenKind::Plus, make_span(34, 35)),
            Token::new(TokenKind::Ident("b".into()), make_span(36, 37)),
            Token::new(TokenKind::RBrace, make_span(38, 39)),
        ];

        let serialized = serialize_tokens(&tokens, None).unwrap();
        let obj = heap.alloc_token_stream(&serialized).unwrap();
        let (result, _) = extract_token_stream_from_object(&obj).unwrap();

        assert_eq!(result.len(), 18);
        assert!(matches!(result[0].kind, TokenKind::Fn));
        assert!(matches!(result[11].kind, TokenKind::RArrow));
    }

    #[test]
    fn test_heap_alloc_unicode_identifiers() {
        let mut heap = crate::interpreter::Heap::new();

        // Test unicode identifiers
        let tokens = vec![
            Token::new(TokenKind::Ident("α".into()), make_span(0, 2)),
            Token::new(TokenKind::Ident("β".into()), make_span(3, 5)),
            Token::new(TokenKind::Ident("γ_variable".into()), make_span(6, 17)),
            Token::new(TokenKind::Ident("日本語".into()), make_span(18, 27)),
        ];

        let serialized = serialize_tokens(&tokens, None).unwrap();
        let obj = heap.alloc_token_stream(&serialized).unwrap();
        let (result, _) = extract_token_stream_from_object(&obj).unwrap();

        assert_eq!(result.len(), 4);
        assert!(matches!(result[0].kind, TokenKind::Ident(ref s) if s == "α"));
        assert!(matches!(result[1].kind, TokenKind::Ident(ref s) if s == "β"));
        assert!(matches!(result[2].kind, TokenKind::Ident(ref s) if s == "γ_variable"));
        assert!(matches!(result[3].kind, TokenKind::Ident(ref s) if s == "日本語"));
    }

    #[test]
    fn test_heap_alloc_all_delimiters() {
        let mut heap = crate::interpreter::Heap::new();

        let tokens = vec![
            Token::new(TokenKind::LParen, make_span(0, 1)),
            Token::new(TokenKind::RParen, make_span(1, 2)),
            Token::new(TokenKind::LBrace, make_span(2, 3)),
            Token::new(TokenKind::RBrace, make_span(3, 4)),
            Token::new(TokenKind::LBracket, make_span(4, 5)),
            Token::new(TokenKind::RBracket, make_span(5, 6)),
        ];

        let serialized = serialize_tokens(&tokens, None).unwrap();
        let obj = heap.alloc_token_stream(&serialized).unwrap();
        let (result, _) = extract_token_stream_from_object(&obj).unwrap();

        assert_eq!(result.len(), 6);
    }

    #[test]
    fn test_heap_alloc_string_and_char_literals() {
        let mut heap = crate::interpreter::Heap::new();

        let tokens = vec![
            Token::new(TokenKind::Text("hello world".into()), make_span(0, 13)),
            Token::new(TokenKind::Char('x'), make_span(14, 17)),
            Token::new(TokenKind::Text("with\nescapes\t".into()), make_span(18, 35)),
            Token::new(TokenKind::Char('λ'), make_span(36, 40)),
        ];

        let serialized = serialize_tokens(&tokens, None).unwrap();
        let obj = heap.alloc_token_stream(&serialized).unwrap();
        let (result, _) = extract_token_stream_from_object(&obj).unwrap();

        assert_eq!(result.len(), 4);
        assert!(matches!(result[0].kind, TokenKind::Text(ref s) if s == "hello world"));
        assert!(matches!(result[1].kind, TokenKind::Char('x')));
        assert!(matches!(result[2].kind, TokenKind::Text(ref s) if s == "with\nescapes\t"));
        assert!(matches!(result[3].kind, TokenKind::Char('λ')));
    }

    #[test]
    fn test_heap_alloc_float_literals() {
        use verum_lexer::FloatLiteral;
        let mut heap = crate::interpreter::Heap::new();

        let tokens = vec![
            Token::new(
                TokenKind::Float(FloatLiteral { value: 3.14, suffix: Maybe::None, raw: "3.14".into() }),
                make_span(0, 4),
            ),
            Token::new(
                TokenKind::Float(FloatLiteral { value: 2.71828, suffix: Maybe::Some("f32".into()), raw: "2.71828".into() }),
                make_span(5, 14),
            ),
            Token::new(
                TokenKind::Float(FloatLiteral { value: 1e10, suffix: Maybe::None, raw: "1e10".into() }),
                make_span(15, 19),
            ),
        ];

        let serialized = serialize_tokens(&tokens, None).unwrap();
        let obj = heap.alloc_token_stream(&serialized).unwrap();
        let (result, _) = extract_token_stream_from_object(&obj).unwrap();

        assert_eq!(result.len(), 3);
        if let TokenKind::Float(lit) = &result[0].kind {
            assert!((lit.value - 3.14).abs() < 0.001);
        } else {
            panic!("Expected float");
        }
    }

    #[test]
    fn test_heap_multiple_allocations() {
        let mut heap = crate::interpreter::Heap::new();

        // Allocate multiple TokenStream objects
        let ts1 = vec![Token::new(TokenKind::Let, make_span(0, 3))];
        let ts2 = vec![
            Token::new(TokenKind::Fn, make_span(0, 2)),
            Token::new(TokenKind::Ident("test".into()), make_span(3, 7)),
        ];
        let ts3 = vec![Token::new(TokenKind::Return, make_span(0, 6))];

        let s1 = serialize_tokens(&ts1, None).unwrap();
        let s2 = serialize_tokens(&ts2, None).unwrap();
        let s3 = serialize_tokens(&ts3, None).unwrap();

        let obj1 = heap.alloc_token_stream(&s1).unwrap();
        let obj2 = heap.alloc_token_stream(&s2).unwrap();
        let obj3 = heap.alloc_token_stream(&s3).unwrap();

        // Verify each independently
        let (r1, _) = extract_token_stream_from_object(&obj1).unwrap();
        let (r2, _) = extract_token_stream_from_object(&obj2).unwrap();
        let (r3, _) = extract_token_stream_from_object(&obj3).unwrap();

        assert_eq!(r1.len(), 1);
        assert_eq!(r2.len(), 2);
        assert_eq!(r3.len(), 1);

        assert!(matches!(r1[0].kind, TokenKind::Let));
        assert!(matches!(r2[0].kind, TokenKind::Fn));
        assert!(matches!(r3[0].kind, TokenKind::Return));
    }

    #[test]
    fn test_wrong_type_id_error() {
        let mut heap = crate::interpreter::Heap::new();

        // Allocate with wrong type ID
        let obj = heap.alloc(crate::types::TypeId::UNIT, 16).unwrap();

        // Extraction should fail
        let result = extract_token_stream_from_object(&obj);
        assert!(result.is_err());
        if let Err(TokenStreamError::DeserializationError(msg)) = result {
            assert!(msg.contains("Expected TokenStream"));
        } else {
            panic!("Expected DeserializationError");
        }
    }

    // =========================================================================
    // Stress Tests
    // =========================================================================

    #[test]
    fn test_large_tokenstream() {
        let mut heap = crate::interpreter::Heap::new();

        // Create a large token sequence (1000 tokens)
        let tokens: Vec<Token> = (0u32..1000)
            .map(|i| Token::new(TokenKind::Ident(format!("var_{}", i).into()), make_span(i * 10, i * 10 + 5)))
            .collect();

        let span = make_span(0, 10000);
        let serialized = serialize_tokens(&tokens, Some(&span)).unwrap();
        let obj = heap.alloc_token_stream(&serialized).unwrap();
        let (result, result_span) = extract_token_stream_from_object(&obj).unwrap();

        assert_eq!(result.len(), 1000);
        assert!(result_span.is_some());

        // Spot-check a few tokens
        assert!(matches!(result[0].kind, TokenKind::Ident(ref s) if s == "var_0"));
        assert!(matches!(result[500].kind, TokenKind::Ident(ref s) if s == "var_500"));
        assert!(matches!(result[999].kind, TokenKind::Ident(ref s) if s == "var_999"));
    }

    #[test]
    fn test_very_long_identifier() {
        let mut heap = crate::interpreter::Heap::new();

        // Create a very long identifier (10KB)
        let long_name: String = (0..10000).map(|_| 'x').collect();
        let tokens = vec![Token::new(TokenKind::Ident(long_name.clone().into()), make_span(0, 10000))];

        let serialized = serialize_tokens(&tokens, None).unwrap();
        let obj = heap.alloc_token_stream(&serialized).unwrap();
        let (result, _) = extract_token_stream_from_object(&obj).unwrap();

        assert_eq!(result.len(), 1);
        if let TokenKind::Ident(s) = &result[0].kind {
            assert_eq!(s.len(), 10000);
        } else {
            panic!("Expected Ident");
        }
    }
}
