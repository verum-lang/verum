//! Lossless syntax tree infrastructure for the Verum language.
//!
//! This crate provides the foundation for lossless parsing, enabling:
//! - Perfect source reconstruction (including comments and whitespace)
//! - IDE-quality code actions and refactoring
//! - Structured error recovery with ERROR nodes
//! - Incremental parsing (partial reparse on edits)
//!
//! # Architecture
//!
//! The crate implements the **red-green tree pattern** originated in Roslyn
//! and adopted by rust-analyzer:
//!
//! ```text
//!                     ┌──────────────────────────────────┐
//!                     │         RED TREE (Facade)        │
//!                     │ ┌──────────────────────────────┐ │
//!                     │ │ SyntaxNode {                 │ │
//!                     │ │   green: GreenNode,          │ │
//!                     │ │   parent: Option<SyntaxNode>,│ │◄── Built on-demand
//!                     │ │   offset: TextSize,          │ │    (discarded per edit)
//!                     │ │ }                            │ │
//!                     │ └──────────────────────────────┘ │
//!                     └───────────────┬──────────────────┘
//!                                     │ references
//!                     ┌───────────────▼──────────────────┐
//!                     │       GREEN TREE (Core)          │
//!                     │ ┌──────────────────────────────┐ │
//!                     │ │ GreenNode {                  │ │
//!                     │ │   kind: SyntaxKind,          │ │◄── Immutable
//!                     │ │   width: TextSize,           │ │    (persists across edits)
//!                     │ │   children: Vec<GreenChild>, │ │
//!                     │ │ }                            │ │
//!                     │ └──────────────────────────────┘ │
//!                     └──────────────────────────────────┘
//! ```
//!
//! Key insight: Green tree stores **widths** (relative), not offsets (absolute).
//! This enables O(log n) updates on edit.
//!
//! # Modules
//!
//! - [`syntax_kind`] - Enum of all node and token kinds
//! - [`trivia`] - Trivia types (whitespace, comments)
//! - [`green`] - Immutable green tree (core)
//! - [`red`] - Red tree facade with parent pointers
//! - [`event`] - Event-based parsing infrastructure
//! - [`sink`] - Tree builders that consume events
//!
//! # Usage
//!
//! ```rust,ignore
//! use verum_syntax::{GreenBuilder, SyntaxNode, SyntaxKind};
//!
//! // Build a green tree
//! let mut builder = GreenBuilder::new();
//! builder.start_node(SyntaxKind::SOURCE_FILE);
//! builder.token(SyntaxKind::LET_KW, "let");
//! builder.token(SyntaxKind::WHITESPACE, " ");
//! builder.token(SyntaxKind::IDENT, "x");
//! builder.finish_node();
//! let green = builder.finish();
//!
//! // Create a red tree for navigation
//! let root = SyntaxNode::new_root(green);
//! assert_eq!(root.text(), "let x");
//!
//! // Find token at offset
//! let token = root.token_at_offset(0).unwrap();
//! assert_eq!(token.text(), "let");
//! ```
//!
//! # Event-Based Parsing
//!
//! The parser emits events instead of building trees directly:
//!
//! ```rust,ignore
//! use verum_syntax::event::{EventBuilder, Marker};
//! use verum_syntax::SyntaxKind;
//!
//! let mut builder = EventBuilder::new();
//!
//! // Start a node
//! let m = builder.start();
//! builder.token(SyntaxKind::LET_KW);
//! builder.token(SyntaxKind::IDENT);
//! m.complete(&mut builder, SyntaxKind::LET_STMT);
//!
//! // Get the events
//! let events = builder.finish();
//! ```
//!
//! Verum uses a red-green tree pattern (originated in Roslyn, adopted by rust-analyzer)
//! for lossless syntax representation. The green tree is immutable and stores relative
//! widths; the red tree is a facade providing parent pointers and absolute positions.
//! Three parser types: LosslessParser (full green/red tree), EventBasedParser
//! (structured events), RecoveringEventParser (error recovery with ERROR nodes).

#![warn(missing_docs)]
#![warn(rust_2018_idioms)]

pub mod event;
pub mod green;
pub mod incremental;
pub mod red;
pub mod sink;
pub mod syntax_kind;
pub mod trivia;

// Re-exports for convenience
pub use event::{CompletedMarker, Event, EventBuilder, EventSink, Marker, TokenSource, TriviaSource};
pub use green::{GreenBuilder, GreenChild, GreenNode, GreenToken, TextRange, TextSize};
pub use incremental::{
    AffectedSubtree, ChangeTracker, IncrementalEngine, IncrementalStats, LspChange, LspRange,
    NodeStability, NodeStabilityAnalyzer, ReparseContext, TextEdit, lsp_change_to_edit,
    lsp_range_to_text_range, offset_to_line_col,
};
pub use red::{SyntaxElement, SyntaxNode, SyntaxToken};
pub use sink::{GreenTreeSink, TextTreeSink};
pub use syntax_kind::SyntaxKind;
pub use trivia::{classify_trivia, DocCommentKind, Trivia, TriviaList, TriviaListExt, TriviaText};

/// Parse result containing the syntax tree and any errors.
#[derive(Debug)]
pub struct Parse {
    /// The root of the green tree.
    pub green: GreenNode,
    /// Parse errors collected during parsing.
    pub errors: Vec<ParseError>,
}

impl Parse {
    /// Create a new parse result.
    pub fn new(green: GreenNode, errors: Vec<ParseError>) -> Self {
        Self { green, errors }
    }

    /// Get the root syntax node.
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }

    /// Check if parsing succeeded without errors.
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }

    /// Get the source text (lossless reconstruction).
    pub fn text(&self) -> String {
        self.green.text()
    }
}

/// A parse error with location and message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// The range in the source where the error occurred.
    pub range: TextRange,
    /// The error message.
    pub message: String,
}

impl ParseError {
    /// Create a new parse error.
    pub fn new(range: TextRange, message: impl Into<String>) -> Self {
        Self {
            range,
            message: message.into(),
        }
    }

    /// Create a parse error at a specific offset.
    pub fn at(offset: TextSize, message: impl Into<String>) -> Self {
        Self {
            range: TextRange::empty(offset),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.range, self.message)
    }
}

impl std::error::Error for ParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_result() {
        let mut builder = GreenBuilder::new();
        builder.start_node(SyntaxKind::SOURCE_FILE);
        builder.token(SyntaxKind::LET_KW, "let");
        builder.token(SyntaxKind::WHITESPACE, " ");
        builder.token(SyntaxKind::IDENT, "x");
        builder.finish_node();

        let green = builder.finish();
        let parse = Parse::new(green, vec![]);

        assert!(parse.ok());
        assert_eq!(parse.text(), "let x");

        let root = parse.syntax();
        assert_eq!(root.kind(), SyntaxKind::SOURCE_FILE);
    }

    #[test]
    fn test_parse_error() {
        let error = ParseError::new(TextRange::new(0, 5), "unexpected token");
        assert_eq!(error.message, "unexpected token");
        assert_eq!(error.range, TextRange::new(0, 5));
    }

    #[test]
    fn test_lossless_roundtrip() {
        let source = "fn  foo ( ) { }";

        let mut builder = GreenBuilder::new();
        builder.start_node(SyntaxKind::SOURCE_FILE);
        builder.start_node(SyntaxKind::FN_DEF);
        builder.token(SyntaxKind::FN_KW, "fn");
        builder.token(SyntaxKind::WHITESPACE, "  ");
        builder.token(SyntaxKind::IDENT, "foo");
        builder.token(SyntaxKind::WHITESPACE, " ");
        builder.token(SyntaxKind::L_PAREN, "(");
        builder.token(SyntaxKind::WHITESPACE, " ");
        builder.token(SyntaxKind::R_PAREN, ")");
        builder.token(SyntaxKind::WHITESPACE, " ");
        builder.start_node(SyntaxKind::BLOCK);
        builder.token(SyntaxKind::L_BRACE, "{");
        builder.token(SyntaxKind::WHITESPACE, " ");
        builder.token(SyntaxKind::R_BRACE, "}");
        builder.finish_node();
        builder.finish_node();
        builder.finish_node();

        let green = builder.finish();
        let reconstructed = green.text();

        assert_eq!(source, reconstructed, "Lossless roundtrip failed");
    }
}
