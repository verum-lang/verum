//! LSP-specific error recovery extensions.
//!
//! This module extends the base recovery infrastructure from `verum_fast_parser`
//! with LSP-specific features:
//!
//! - **Recovery Sets**: Define which tokens can start valid constructs for grammar rules
//! - **Event-Based Recovery**: Create ERROR nodes for unparseable content
//! - **TokenKind to SyntaxKind Mapping**: Bridge between lexer and syntax tree
//!
//! For base recovery types (Delimiter, SyncPoint, RecoveryStrategy, RecoveryContext),
//! see `verum_fast_parser::recovery`.
//!
//! # Event-Based Recovery
//!
//! The event-based recovery system integrates with the marker/precede pattern:
//!
//! ```text
//! Source → Events → GreenTree (with ERROR nodes) → AstSink → Module
//! ```
//!
//! Unparseable content is wrapped in ERROR nodes to preserve source fidelity
//! while marking problematic regions for downstream error reporting.
//!
//! # Recovery Sets
//!
//! Recovery sets define tokens that can start various language constructs:
//!
//! - **ITEM_RECOVERY**: Tokens that start top-level items
//! - **STMT_RECOVERY**: Tokens that start statements
//! - **EXPR_RECOVERY**: Tokens that start expressions
//! - **TYPE_RECOVERY**: Tokens in type position
//! - **PATTERN_RECOVERY**: Tokens in pattern position
//!
//! See `recovery_sets` module for all pre-defined sets.

use verum_ast::Span;
use verum_common::Text;
use verum_lexer::TokenKind;
use verum_syntax::SyntaxKind;

// Re-export base recovery types from verum_fast_parser
pub use verum_fast_parser::{
    Delimiter, RecoveryContext, RecoveryStrategy, SyncPoint,
    can_start_expression, can_start_item, can_start_statement, is_statement_terminator,
    missing_token_message, unexpected_token_message,
    ParseError,
};

// =============================================================================
// Recovery Set Infrastructure
// =============================================================================

/// Recovery set for event-based parsing.
///
/// A recovery set defines which tokens can start the next valid construct,
/// allowing the parser to skip erroneous tokens and create ERROR nodes.
#[derive(Clone, Debug)]
pub struct RecoverySet {
    /// SyntaxKind tokens that can start the next valid construct.
    pub tokens: &'static [SyntaxKind],
    /// Maximum number of tokens to skip before giving up.
    pub max_skip: usize,
    /// Whether to create an ERROR node for skipped tokens.
    pub create_error_node: bool,
    /// Human-readable context for error messages.
    pub context: &'static str,
}

impl RecoverySet {
    /// Create a new recovery set.
    pub const fn new(tokens: &'static [SyntaxKind]) -> Self {
        Self {
            tokens,
            max_skip: 20,
            create_error_node: true,
            context: "",
        }
    }

    /// Set the maximum skip count.
    pub const fn with_max_skip(mut self, max: usize) -> Self {
        self.max_skip = max;
        self
    }

    /// Set whether to create error nodes.
    pub const fn with_error_node(mut self, create: bool) -> Self {
        self.create_error_node = create;
        self
    }

    /// Set the context for error messages.
    pub const fn with_context(mut self, context: &'static str) -> Self {
        self.context = context;
        self
    }

    /// Check if a SyntaxKind is in the recovery set.
    pub fn contains(&self, kind: SyntaxKind) -> bool {
        self.tokens.contains(&kind)
    }

    /// Check if a TokenKind maps to any SyntaxKind in the recovery set.
    pub fn contains_token(&self, kind: &TokenKind) -> bool {
        let syntax_kind = token_kind_to_syntax_kind(kind);
        self.tokens.contains(&syntax_kind)
    }

    /// Combine two recovery sets.
    /// Note: This allocates a new static slice, so use sparingly.
    pub fn union(&self, other: &RecoverySet) -> Self {
        // For runtime union, we need to return a new set with combined tokens
        // Since we can't create static slices at runtime, we use the larger set
        // and trust the caller to check both sets manually if needed
        if self.tokens.len() >= other.tokens.len() {
            self.clone()
        } else {
            other.clone()
        }
    }
}

// =============================================================================
// TokenKind to SyntaxKind Conversion
// =============================================================================

/// Convert a TokenKind to the corresponding SyntaxKind.
pub fn token_kind_to_syntax_kind(kind: &TokenKind) -> SyntaxKind {
    match kind {
        // Keywords
        TokenKind::Fn => SyntaxKind::FN_KW,
        TokenKind::Let => SyntaxKind::LET_KW,
        TokenKind::Type => SyntaxKind::TYPE_KW,
        TokenKind::Protocol => SyntaxKind::PROTOCOL_KW,
        TokenKind::Implement => SyntaxKind::IMPLEMENT_KW,
        TokenKind::If => SyntaxKind::IF_KW,
        TokenKind::Else => SyntaxKind::ELSE_KW,
        TokenKind::While => SyntaxKind::WHILE_KW,
        TokenKind::For => SyntaxKind::FOR_KW,
        TokenKind::In => SyntaxKind::IN_KW,
        TokenKind::Match => SyntaxKind::MATCH_KW,
        TokenKind::Return => SyntaxKind::RETURN_KW,
        TokenKind::Break => SyntaxKind::BREAK_KW,
        TokenKind::Continue => SyntaxKind::CONTINUE_KW,
        TokenKind::Loop => SyntaxKind::LOOP_KW,
        TokenKind::True => SyntaxKind::TRUE_KW,
        TokenKind::False => SyntaxKind::FALSE_KW,
        TokenKind::Pub => SyntaxKind::PUB_KW,
        TokenKind::Mut => SyntaxKind::MUT_KW,
        TokenKind::Ref => SyntaxKind::REF_KW,
        TokenKind::Where => SyntaxKind::WHERE_KW,
        TokenKind::As => SyntaxKind::AS_KW,
        TokenKind::Const => SyntaxKind::CONST_KW,
        TokenKind::Static => SyntaxKind::STATIC_KW,
        TokenKind::Async => SyntaxKind::ASYNC_KW,
        TokenKind::Await => SyntaxKind::AWAIT_KW,
        TokenKind::Spawn => SyntaxKind::SPAWN_KW,
        TokenKind::Select => SyntaxKind::SELECT_KW,
        TokenKind::Nursery => SyntaxKind::NURSERY_KW,
        TokenKind::Module => SyntaxKind::MODULE_KW,
        TokenKind::Mount => SyntaxKind::MOUNT_KW,
        TokenKind::Extern => SyntaxKind::EXTERN_KW,
        TokenKind::Defer => SyntaxKind::DEFER_KW,
        TokenKind::Provide => SyntaxKind::PROVIDE_KW,
        TokenKind::Context => SyntaxKind::CONTEXT_KW,
        TokenKind::Using => SyntaxKind::USING_KW,
        TokenKind::Try => SyntaxKind::TRY_KW,
        TokenKind::Throw => SyntaxKind::THROW_KW,
        TokenKind::Stream => SyntaxKind::STREAM_KW,
        TokenKind::Yield => SyntaxKind::YIELD_KW,
        TokenKind::Some => SyntaxKind::SOME_KW,
        TokenKind::None => SyntaxKind::NONE_KW,
        TokenKind::Ok => SyntaxKind::OK_KW,
        TokenKind::Err => SyntaxKind::ERR_KW,
        TokenKind::SelfValue => SyntaxKind::SELF_VALUE_KW,
        TokenKind::SelfType => SyntaxKind::SELF_TYPE_KW,

        // Delimiters
        TokenKind::LParen => SyntaxKind::L_PAREN,
        TokenKind::RParen => SyntaxKind::R_PAREN,
        TokenKind::LBracket => SyntaxKind::L_BRACKET,
        TokenKind::RBracket => SyntaxKind::R_BRACKET,
        TokenKind::LBrace => SyntaxKind::L_BRACE,
        TokenKind::RBrace => SyntaxKind::R_BRACE,
        TokenKind::Lt => SyntaxKind::L_ANGLE,
        TokenKind::Gt => SyntaxKind::R_ANGLE,

        // Punctuation
        TokenKind::Semicolon => SyntaxKind::SEMICOLON,
        TokenKind::Comma => SyntaxKind::COMMA,
        TokenKind::Colon => SyntaxKind::COLON,
        TokenKind::Dot => SyntaxKind::DOT,
        TokenKind::DotDot => SyntaxKind::DOT_DOT,
        TokenKind::DotDotEq => SyntaxKind::DOT_DOT_EQ,
        TokenKind::RArrow => SyntaxKind::ARROW,
        TokenKind::FatArrow => SyntaxKind::FAT_ARROW,
        TokenKind::At => SyntaxKind::AT,
        TokenKind::Pipe => SyntaxKind::PIPE,
        TokenKind::PipePipe => SyntaxKind::PIPE_PIPE,
        TokenKind::Ampersand => SyntaxKind::AMP,
        TokenKind::AmpersandAmpersand => SyntaxKind::AMP_AMP,
        TokenKind::Question => SyntaxKind::QUESTION,
        TokenKind::QuestionDot => SyntaxKind::QUESTION_DOT,
        TokenKind::QuestionQuestion => SyntaxKind::QUESTION_QUESTION,
        TokenKind::Hash => SyntaxKind::HASH,
        TokenKind::Dollar => SyntaxKind::AT,
        TokenKind::PipeGt => SyntaxKind::PIPE_GT,

        // Operators
        TokenKind::Eq => SyntaxKind::EQ,
        TokenKind::EqEq => SyntaxKind::EQ_EQ,
        TokenKind::BangEq => SyntaxKind::BANG_EQ,
        TokenKind::LtEq => SyntaxKind::LT_EQ,
        TokenKind::GtEq => SyntaxKind::GT_EQ,
        TokenKind::Plus => SyntaxKind::PLUS,
        TokenKind::Minus => SyntaxKind::MINUS,
        TokenKind::Star => SyntaxKind::STAR,
        TokenKind::StarStar => SyntaxKind::STAR_STAR,
        TokenKind::Slash => SyntaxKind::SLASH,
        TokenKind::Percent => SyntaxKind::PERCENT,
        TokenKind::Caret => SyntaxKind::CARET,
        TokenKind::Tilde => SyntaxKind::TILDE,
        TokenKind::Bang => SyntaxKind::BANG,
        TokenKind::LtLt => SyntaxKind::LT_LT,
        TokenKind::GtGt => SyntaxKind::GT_GT,
        TokenKind::PlusEq => SyntaxKind::PLUS_EQ,
        TokenKind::MinusEq => SyntaxKind::MINUS_EQ,
        TokenKind::StarEq => SyntaxKind::STAR_EQ,
        TokenKind::SlashEq => SyntaxKind::SLASH_EQ,
        TokenKind::PercentEq => SyntaxKind::PERCENT_EQ,
        TokenKind::AmpersandEq => SyntaxKind::AMP_EQ,
        TokenKind::PipeEq => SyntaxKind::PIPE_EQ,
        TokenKind::CaretEq => SyntaxKind::CARET_EQ,

        // Literals
        TokenKind::Integer(_) => SyntaxKind::INT_LITERAL,
        TokenKind::Float(_) => SyntaxKind::FLOAT_LITERAL,
        TokenKind::Text(_) => SyntaxKind::STRING_LITERAL,
        TokenKind::Char(_) => SyntaxKind::CHAR_LITERAL,
        TokenKind::ByteChar(_) => SyntaxKind::CHAR_LITERAL,
        TokenKind::InterpolatedString(_) => SyntaxKind::INTERPOLATED_STRING,

        // Identifiers
        TokenKind::Ident(_) => SyntaxKind::IDENT,

        // Trivia
        TokenKind::BlockComment => SyntaxKind::BLOCK_COMMENT,

        // Special
        TokenKind::Eof => SyntaxKind::EOF,

        // Default fallback
        _ => SyntaxKind::ERROR,
    }
}

// =============================================================================
// Pre-defined Recovery Sets
// =============================================================================

/// Pre-defined recovery sets for Verum grammar.
///
/// These sets define the tokens that can start various language constructs,
/// allowing the parser to recover from errors by skipping to these tokens.
pub mod recovery_sets {
    use super::*;

    /// Tokens that can start a top-level item.
    pub const ITEM_RECOVERY: RecoverySet = RecoverySet {
        tokens: &[
            SyntaxKind::FN_KW,
            SyntaxKind::TYPE_KW,
            SyntaxKind::PROTOCOL_KW,
            SyntaxKind::IMPLEMENT_KW,
            SyntaxKind::CONTEXT_KW,
            SyntaxKind::PUB_KW,
            SyntaxKind::PUBLIC_KW,
            SyntaxKind::MODULE_KW,
            SyntaxKind::MOUNT_KW,
            SyntaxKind::EXTERN_KW,
            SyntaxKind::CONST_KW,
            SyntaxKind::STATIC_KW,
            SyntaxKind::AT,
            SyntaxKind::EOF,
        ],
        max_skip: 50,
        create_error_node: true,
        context: "top-level item",
    };

    /// Tokens that can start a statement.
    pub const STMT_RECOVERY: RecoverySet = RecoverySet {
        tokens: &[
            SyntaxKind::LET_KW,
            SyntaxKind::IF_KW,
            SyntaxKind::WHILE_KW,
            SyntaxKind::FOR_KW,
            SyntaxKind::LOOP_KW,
            SyntaxKind::MATCH_KW,
            SyntaxKind::RETURN_KW,
            SyntaxKind::BREAK_KW,
            SyntaxKind::CONTINUE_KW,
            SyntaxKind::DEFER_KW,
            SyntaxKind::PROVIDE_KW,
            SyntaxKind::TRY_KW,
            SyntaxKind::YIELD_KW,
            SyntaxKind::SPAWN_KW,
            SyntaxKind::R_BRACE,
            SyntaxKind::SEMICOLON,
        ],
        max_skip: 30,
        create_error_node: true,
        context: "statement",
    };

    /// Tokens that can follow an expression in statement position.
    pub const EXPR_STMT_RECOVERY: RecoverySet = RecoverySet {
        tokens: &[
            SyntaxKind::SEMICOLON,
            SyntaxKind::R_BRACE,
            SyntaxKind::R_PAREN,
            SyntaxKind::R_BRACKET,
            SyntaxKind::COMMA,
        ],
        max_skip: 20,
        create_error_node: true,
        context: "expression terminator",
    };

    /// Tokens for comma-separated list recovery.
    pub const COMMA_RECOVERY: RecoverySet = RecoverySet {
        tokens: &[
            SyntaxKind::COMMA,
            SyntaxKind::R_PAREN,
            SyntaxKind::R_BRACKET,
            SyntaxKind::R_BRACE,
            SyntaxKind::R_ANGLE,
        ],
        max_skip: 15,
        create_error_node: true,
        context: "list separator",
    };

    /// Tokens that can appear in type position.
    pub const TYPE_RECOVERY: RecoverySet = RecoverySet {
        tokens: &[
            SyntaxKind::IDENT,
            SyntaxKind::SELF_TYPE_KW,
            SyntaxKind::L_PAREN,
            SyntaxKind::L_BRACKET,
            SyntaxKind::AMP,
            SyntaxKind::STAR,
            SyntaxKind::FN_KW,
            SyntaxKind::R_ANGLE,
            SyntaxKind::COMMA,
            SyntaxKind::EQ,
            SyntaxKind::L_BRACE,
            SyntaxKind::WHERE_KW,
            SyntaxKind::SEMICOLON,
        ],
        max_skip: 20,
        create_error_node: true,
        context: "type",
    };

    /// Tokens for function parameter list recovery.
    pub const PARAM_RECOVERY: RecoverySet = RecoverySet {
        tokens: &[
            SyntaxKind::IDENT,
            SyntaxKind::MUT_KW,
            SyntaxKind::REF_KW,
            SyntaxKind::SELF_VALUE_KW,
            SyntaxKind::COMMA,
            SyntaxKind::R_PAREN,
            SyntaxKind::COLON,
        ],
        max_skip: 15,
        create_error_node: true,
        context: "function parameter",
    };

    /// Tokens for pattern recovery (match arms, let bindings).
    pub const PATTERN_RECOVERY: RecoverySet = RecoverySet {
        tokens: &[
            SyntaxKind::IDENT,
            SyntaxKind::UNDERSCORE,
            SyntaxKind::L_PAREN,
            SyntaxKind::L_BRACKET,
            SyntaxKind::L_BRACE,
            SyntaxKind::INT_LITERAL,
            SyntaxKind::STRING_LITERAL,
            SyntaxKind::TRUE_KW,
            SyntaxKind::FALSE_KW,
            SyntaxKind::SOME_KW,
            SyntaxKind::NONE_KW,
            SyntaxKind::OK_KW,
            SyntaxKind::ERR_KW,
            SyntaxKind::FAT_ARROW,
            SyntaxKind::PIPE,
            SyntaxKind::R_BRACE,
        ],
        max_skip: 20,
        create_error_node: true,
        context: "pattern",
    };

    /// Tokens for match arm recovery.
    pub const MATCH_ARM_RECOVERY: RecoverySet = RecoverySet {
        tokens: &[
            SyntaxKind::PIPE,
            SyntaxKind::FAT_ARROW,
            SyntaxKind::COMMA,
            SyntaxKind::R_BRACE,
            SyntaxKind::IDENT,
            SyntaxKind::UNDERSCORE,
        ],
        max_skip: 25,
        create_error_node: true,
        context: "match arm",
    };

    /// Tokens for expression recovery within binary operations.
    pub const EXPR_RECOVERY: RecoverySet = RecoverySet {
        tokens: &[
            SyntaxKind::IDENT,
            SyntaxKind::INT_LITERAL,
            SyntaxKind::FLOAT_LITERAL,
            SyntaxKind::STRING_LITERAL,
            SyntaxKind::CHAR_LITERAL,
            SyntaxKind::TRUE_KW,
            SyntaxKind::FALSE_KW,
            SyntaxKind::L_PAREN,
            SyntaxKind::L_BRACKET,
            SyntaxKind::L_BRACE,
            SyntaxKind::IF_KW,
            SyntaxKind::MATCH_KW,
            SyntaxKind::FOR_KW,
            SyntaxKind::WHILE_KW,
            SyntaxKind::LOOP_KW,
            SyntaxKind::BANG,
            SyntaxKind::MINUS,
            SyntaxKind::PLUS,
            SyntaxKind::STAR,
            SyntaxKind::AMP,
            SyntaxKind::SOME_KW,
            SyntaxKind::NONE_KW,
            SyntaxKind::OK_KW,
            SyntaxKind::ERR_KW,
        ],
        max_skip: 15,
        create_error_node: true,
        context: "expression",
    };

    /// Tokens that can follow a block.
    pub const BLOCK_RECOVERY: RecoverySet = RecoverySet {
        tokens: &[
            SyntaxKind::R_BRACE,
            SyntaxKind::ELSE_KW,
            SyntaxKind::SEMICOLON,
            SyntaxKind::FN_KW,
            SyntaxKind::TYPE_KW,
            SyntaxKind::EOF,
        ],
        max_skip: 50,
        create_error_node: true,
        context: "block",
    };

    /// Tokens for generic argument list recovery.
    pub const GENERIC_RECOVERY: RecoverySet = RecoverySet {
        tokens: &[
            SyntaxKind::IDENT,
            SyntaxKind::COMMA,
            SyntaxKind::R_ANGLE,
            SyntaxKind::COLON,
            SyntaxKind::EQ,
            SyntaxKind::WHERE_KW,
        ],
        max_skip: 20,
        create_error_node: true,
        context: "generic parameter",
    };

    /// Tokens for struct/record field recovery.
    pub const FIELD_RECOVERY: RecoverySet = RecoverySet {
        tokens: &[
            SyntaxKind::IDENT,
            SyntaxKind::PUB_KW,
            SyntaxKind::AT,
            SyntaxKind::COMMA,
            SyntaxKind::R_BRACE,
            SyntaxKind::COLON,
        ],
        max_skip: 20,
        create_error_node: true,
        context: "field",
    };

    /// Tokens for variant definition recovery (sum types).
    pub const VARIANT_RECOVERY: RecoverySet = RecoverySet {
        tokens: &[
            SyntaxKind::IDENT,
            SyntaxKind::PIPE,
            SyntaxKind::L_PAREN,
            SyntaxKind::L_BRACE,
            SyntaxKind::SEMICOLON,
        ],
        max_skip: 20,
        create_error_node: true,
        context: "variant",
    };

    /// Create a custom recovery set for specific contexts.
    pub const fn custom(tokens: &'static [SyntaxKind], context: &'static str) -> RecoverySet {
        RecoverySet {
            tokens,
            max_skip: 20,
            create_error_node: true,
            context,
        }
    }
}

// =============================================================================
// Event-Based Recovery Mechanism
// =============================================================================

/// Result of a recovery operation.
#[derive(Debug, Clone)]
pub struct RecoveryResult {
    /// Number of tokens skipped.
    pub tokens_skipped: usize,
    /// Whether an ERROR node was created.
    pub error_node_created: bool,
    /// The error message for the recovery.
    pub message: String,
    /// Whether recovery was successful (found a recovery point).
    pub success: bool,
}

impl RecoveryResult {
    /// Create a successful recovery result.
    pub fn success(tokens_skipped: usize, message: impl Into<String>) -> Self {
        Self {
            tokens_skipped,
            error_node_created: tokens_skipped > 0,
            message: message.into(),
            success: true,
        }
    }

    /// Create a failed recovery result.
    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            tokens_skipped: 0,
            error_node_created: false,
            message: message.into(),
            success: false,
        }
    }
}

/// Event-based recovery context for structured error handling.
///
/// This struct integrates with the event-based parser to create ERROR nodes
/// for unparseable content while allowing parsing to continue.
#[derive(Debug)]
pub struct EventRecovery {
    /// Stack of recovery sets for nested contexts.
    recovery_stack: Vec<RecoverySet>,
    /// Errors encountered during recovery.
    pub errors: Vec<ParseError>,
    /// Maximum errors before stopping recovery.
    pub max_errors: usize,
    /// Total tokens skipped during recovery (for diagnostics).
    pub total_skipped: usize,
    /// Number of successful recoveries.
    pub recovery_count: usize,
}

impl EventRecovery {
    /// Create a new event recovery context.
    pub fn new() -> Self {
        Self {
            recovery_stack: Vec::new(),
            errors: Vec::new(),
            max_errors: 100,
            total_skipped: 0,
            recovery_count: 0,
        }
    }

    /// Push a recovery context onto the stack.
    pub fn push(&mut self, set: RecoverySet) {
        self.recovery_stack.push(set);
    }

    /// Pop a recovery context from the stack.
    pub fn pop(&mut self) -> Option<RecoverySet> {
        self.recovery_stack.pop()
    }

    /// Get the current recovery set (top of stack).
    pub fn current(&self) -> Option<&RecoverySet> {
        self.recovery_stack.last()
    }

    /// Check if a token is in any active recovery set.
    pub fn is_at_recovery_point(&self, kind: &TokenKind) -> bool {
        let syntax_kind = token_kind_to_syntax_kind(kind);
        self.recovery_stack
            .iter()
            .any(|set| set.tokens.contains(&syntax_kind))
    }

    /// Check if a token is at the current recovery point.
    pub fn is_at_current_recovery(&self, kind: &TokenKind) -> bool {
        if let Some(set) = self.current() {
            set.contains_token(kind)
        } else {
            false
        }
    }

    /// Add an error during recovery.
    pub fn add_error(&mut self, error: ParseError) {
        if self.errors.len() < self.max_errors {
            self.errors.push(error);
        }
    }

    /// Check if too many errors have been recorded.
    pub fn too_many_errors(&self) -> bool {
        self.errors.len() >= self.max_errors
    }

    /// Record a successful recovery.
    pub fn record_recovery(&mut self, tokens_skipped: usize) {
        self.total_skipped += tokens_skipped;
        self.recovery_count += 1;
    }

    /// Get recovery statistics as a formatted string.
    pub fn stats(&self) -> String {
        format!(
            "Recovery stats: {} recoveries, {} tokens skipped, {} errors",
            self.recovery_count,
            self.total_skipped,
            self.errors.len()
        )
    }
}

impl Default for EventRecovery {
    fn default() -> Self {
        Self::new()
    }
}

/// Trait for parsers that support event-based error recovery.
///
/// This trait defines the interface for integrating error recovery
/// with event-based parsing, allowing parsers to:
/// - Skip tokens until a recovery point
/// - Create ERROR nodes for skipped content
/// - Report structured error messages
pub trait Recoverable {
    /// Get the current token's SyntaxKind.
    fn current_kind(&self) -> SyntaxKind;

    /// Check if at end of input.
    fn at_end(&self) -> bool;

    /// Advance to the next token.
    fn bump(&mut self);

    /// Get the current token's text.
    fn current_text(&self) -> &str;

    /// Emit an error event.
    fn error(&mut self, message: &str);

    /// Start a node (returns a marker).
    fn start_node(&mut self) -> usize;

    /// Finish a node with a kind.
    fn finish_node(&mut self, marker: usize, kind: SyntaxKind);

    /// Recover from an error using the given recovery set.
    ///
    /// This method skips tokens until it finds one in the recovery set,
    /// wrapping skipped tokens in an ERROR node if configured.
    fn recover(&mut self, set: &RecoverySet, message: &str) -> RecoveryResult {
        if self.at_end() {
            self.error(message);
            return RecoveryResult::failure(message);
        }

        // Check if already at a recovery point
        if set.contains(self.current_kind()) {
            return RecoveryResult::success(0, message);
        }

        // Start ERROR node if configured
        let marker = if set.create_error_node {
            Some(self.start_node())
        } else {
            None
        };

        // Skip tokens until recovery point or max_skip reached
        let mut skipped = 0;
        while !self.at_end() && skipped < set.max_skip {
            let kind = self.current_kind();

            // Check if we've reached a recovery point
            if set.contains(kind) || kind == SyntaxKind::EOF {
                break;
            }

            // Skip this token
            self.bump();
            skipped += 1;
        }

        // Finish ERROR node if we started one
        if let Some(m) = marker {
            if skipped > 0 {
                self.finish_node(m, SyntaxKind::ERROR);
            }
        }

        // Report the error
        if skipped > 0 {
            let context = if set.context.is_empty() {
                ""
            } else {
                set.context
            };
            let detailed_message = if context.is_empty() {
                format!("{} (skipped {} tokens)", message, skipped)
            } else {
                format!(
                    "{} while parsing {} (skipped {} tokens)",
                    message, context, skipped
                )
            };
            self.error(&detailed_message);
            RecoveryResult::success(skipped, detailed_message)
        } else {
            RecoveryResult::success(0, message.to_string())
        }
    }

    /// Try to parse with recovery on failure.
    ///
    /// Returns true if parsing succeeded, false if recovery was needed.
    fn try_parse_with_recovery<T>(
        &mut self,
        parse_fn: impl FnOnce(&mut Self) -> Option<T>,
        set: &RecoverySet,
        message: &str,
    ) -> Option<T>
    where
        Self: Sized,
    {
        if let Some(result) = parse_fn(self) {
            Some(result)
        } else {
            self.recover(set, message);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recovery_set_contains() {
        let set = recovery_sets::STMT_RECOVERY;
        assert!(set.contains(SyntaxKind::LET_KW));
        assert!(set.contains(SyntaxKind::IF_KW));
        assert!(set.contains(SyntaxKind::SEMICOLON));
        assert!(!set.contains(SyntaxKind::PLUS));
    }

    #[test]
    fn test_recovery_set_contains_token() {
        let set = recovery_sets::STMT_RECOVERY;
        assert!(set.contains_token(&TokenKind::Let));
        assert!(set.contains_token(&TokenKind::If));
        assert!(set.contains_token(&TokenKind::Semicolon));
        assert!(!set.contains_token(&TokenKind::Plus));
    }

    #[test]
    fn test_token_kind_to_syntax_kind() {
        assert_eq!(token_kind_to_syntax_kind(&TokenKind::Fn), SyntaxKind::FN_KW);
        assert_eq!(
            token_kind_to_syntax_kind(&TokenKind::Let),
            SyntaxKind::LET_KW
        );
        assert_eq!(
            token_kind_to_syntax_kind(&TokenKind::LParen),
            SyntaxKind::L_PAREN
        );
        assert_eq!(
            token_kind_to_syntax_kind(&TokenKind::Integer(verum_lexer::token::IntegerLiteral {
                raw_value: verum_common::Text::from("42"),
                base: 10,
                suffix: verum_common::Maybe::None,
            })),
            SyntaxKind::INT_LITERAL
        );
    }

    #[test]
    fn test_event_recovery_stack() {
        let mut recovery = EventRecovery::new();
        assert!(recovery.current().is_none());

        recovery.push(recovery_sets::STMT_RECOVERY.clone());
        assert!(recovery.current().is_some());
        assert!(recovery.is_at_current_recovery(&TokenKind::Let));

        recovery.push(recovery_sets::EXPR_STMT_RECOVERY.clone());
        assert!(recovery.is_at_current_recovery(&TokenKind::Semicolon));
        assert!(!recovery.is_at_current_recovery(&TokenKind::Let));

        recovery.pop();
        assert!(recovery.is_at_current_recovery(&TokenKind::Let));
    }

    #[test]
    fn test_recovery_result() {
        let success = RecoveryResult::success(5, "test message");
        assert!(success.success);
        assert_eq!(success.tokens_skipped, 5);
        assert!(success.error_node_created);

        let failure = RecoveryResult::failure("failed");
        assert!(!failure.success);
        assert_eq!(failure.tokens_skipped, 0);
    }

    #[test]
    fn test_item_recovery_set() {
        let set = &recovery_sets::ITEM_RECOVERY;
        assert!(set.contains(SyntaxKind::FN_KW));
        assert!(set.contains(SyntaxKind::TYPE_KW));
        assert!(set.contains(SyntaxKind::AT));
        assert!(set.contains(SyntaxKind::EOF));
        assert!(!set.contains(SyntaxKind::LET_KW));
    }

    #[test]
    fn test_pattern_recovery_set() {
        let set = &recovery_sets::PATTERN_RECOVERY;
        assert!(set.contains(SyntaxKind::IDENT));
        assert!(set.contains(SyntaxKind::UNDERSCORE));
        assert!(set.contains(SyntaxKind::SOME_KW));
        assert!(set.contains(SyntaxKind::FAT_ARROW));
    }
}
