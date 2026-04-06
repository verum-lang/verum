//! Tree sinks that consume parser events to build syntax trees.
//!
//! Different sinks produce different representations from the same events:
//! - `GreenTreeSink` builds green trees for the syntax infrastructure
//! - `TextTreeSink` produces a text representation for debugging
//!
//! Tree Sinks consume parser events via the EventSink trait (start_node,
//! finish_node, token, error) and produce different representations:
//! - GreenTreeSink: builds immutable green trees for the syntax infrastructure
//! - TextTreeSink: produces text representation for debugging/testing

use crate::event::EventSink;
use crate::green::{GreenBuilder, GreenNode};
use crate::SyntaxKind;

/// Builds a green tree from parser events.
pub struct GreenTreeSink {
    builder: GreenBuilder,
    errors: Vec<SinkError>,
}

/// Error recorded during tree building.
#[derive(Clone, Debug)]
pub struct SinkError {
    /// Position in token stream where error occurred.
    pub token_pos: usize,
    /// Error message.
    pub message: String,
}

impl GreenTreeSink {
    /// Create a new green tree sink.
    pub fn new() -> Self {
        Self {
            builder: GreenBuilder::new(),
            errors: Vec::new(),
        }
    }

    /// Finish building and return the green node and errors.
    pub fn finish(self) -> (GreenNode, Vec<SinkError>) {
        (self.builder.finish(), self.errors)
    }

    /// Get the current errors.
    pub fn errors(&self) -> &[SinkError] {
        &self.errors
    }

    /// Check if we're currently building.
    pub fn is_building(&self) -> bool {
        self.builder.is_building()
    }
}

impl Default for GreenTreeSink {
    fn default() -> Self {
        Self::new()
    }
}

impl EventSink for GreenTreeSink {
    fn start_node(&mut self, kind: SyntaxKind) {
        self.builder.start_node(kind);
    }

    fn finish_node(&mut self) {
        self.builder.finish_node();
    }

    fn token(&mut self, kind: SyntaxKind, text: &str) {
        self.builder.token(kind, text);
    }

    fn error(&mut self, message: &str) {
        self.errors.push(SinkError {
            token_pos: 0, // Would need to track this
            message: message.to_string(),
        });
    }
}

/// Produces a text representation of the syntax tree for debugging.
pub struct TextTreeSink {
    output: String,
    indent: usize,
    errors: Vec<String>,
}

impl TextTreeSink {
    /// Create a new text tree sink.
    pub fn new() -> Self {
        Self {
            output: String::new(),
            indent: 0,
            errors: Vec::new(),
        }
    }

    /// Finish and return the text output.
    pub fn finish(self) -> String {
        self.output
    }

    /// Get the current output.
    pub fn output(&self) -> &str {
        &self.output
    }

    /// Get the errors.
    pub fn errors(&self) -> &[String] {
        &self.errors
    }

    fn write_indent(&mut self) {
        for _ in 0..self.indent {
            self.output.push_str("  ");
        }
    }
}

impl Default for TextTreeSink {
    fn default() -> Self {
        Self::new()
    }
}

impl EventSink for TextTreeSink {
    fn start_node(&mut self, kind: SyntaxKind) {
        self.write_indent();
        self.output.push_str(&format!("{:?}\n", kind));
        self.indent += 1;
    }

    fn finish_node(&mut self) {
        self.indent = self.indent.saturating_sub(1);
    }

    fn token(&mut self, kind: SyntaxKind, text: &str) {
        self.write_indent();
        self.output.push_str(&format!("{:?} {:?}\n", kind, text));
    }

    fn error(&mut self, message: &str) {
        self.write_indent();
        self.output.push_str(&format!("ERROR: {}\n", message));
        self.errors.push(message.to_string());
    }
}

/// A tree sink that collects statistics about the parse.
pub struct StatsSink {
    node_count: usize,
    token_count: usize,
    max_depth: usize,
    current_depth: usize,
    error_count: usize,
}

impl StatsSink {
    /// Create a new stats sink.
    pub fn new() -> Self {
        Self {
            node_count: 0,
            token_count: 0,
            max_depth: 0,
            current_depth: 0,
            error_count: 0,
        }
    }

    /// Get the number of nodes.
    pub fn node_count(&self) -> usize {
        self.node_count
    }

    /// Get the number of tokens.
    pub fn token_count(&self) -> usize {
        self.token_count
    }

    /// Get the maximum nesting depth.
    pub fn max_depth(&self) -> usize {
        self.max_depth
    }

    /// Get the number of errors.
    pub fn error_count(&self) -> usize {
        self.error_count
    }
}

impl Default for StatsSink {
    fn default() -> Self {
        Self::new()
    }
}

impl EventSink for StatsSink {
    fn start_node(&mut self, _kind: SyntaxKind) {
        self.node_count += 1;
        self.current_depth += 1;
        self.max_depth = self.max_depth.max(self.current_depth);
    }

    fn finish_node(&mut self) {
        self.current_depth = self.current_depth.saturating_sub(1);
    }

    fn token(&mut self, _kind: SyntaxKind, _text: &str) {
        self.token_count += 1;
    }

    fn error(&mut self, _message: &str) {
        self.error_count += 1;
    }
}

/// A multiplexing sink that sends events to multiple sinks.
pub struct MultiplexSink<'a> {
    sinks: Vec<&'a mut dyn EventSink>,
}

impl<'a> MultiplexSink<'a> {
    /// Create a new multiplex sink.
    pub fn new() -> Self {
        Self { sinks: Vec::new() }
    }

    /// Add a sink.
    pub fn add(&mut self, sink: &'a mut dyn EventSink) {
        self.sinks.push(sink);
    }
}

impl Default for MultiplexSink<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl EventSink for MultiplexSink<'_> {
    fn start_node(&mut self, kind: SyntaxKind) {
        for sink in &mut self.sinks {
            sink.start_node(kind);
        }
    }

    fn finish_node(&mut self) {
        for sink in &mut self.sinks {
            sink.finish_node();
        }
    }

    fn token(&mut self, kind: SyntaxKind, text: &str) {
        for sink in &mut self.sinks {
            sink.token(kind, text);
        }
    }

    fn error(&mut self, message: &str) {
        for sink in &mut self.sinks {
            sink.error(message);
        }
    }
}

/// Recovery context for structured error handling.
#[derive(Clone, Debug)]
pub struct RecoveryContext {
    /// Tokens that can start the next valid construct.
    pub recovery_set: &'static [SyntaxKind],
    /// Maximum tokens to skip before giving up.
    pub max_skip: usize,
    /// Whether to create error node for skipped tokens.
    pub create_error_node: bool,
}

impl RecoveryContext {
    /// Create a new recovery context.
    pub const fn new(recovery_set: &'static [SyntaxKind]) -> Self {
        Self {
            recovery_set,
            max_skip: 10,
            create_error_node: true,
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
}

/// Pre-defined recovery sets for Verum grammar.
pub mod recovery_sets {
    use super::*;

    /// Tokens that can start a top-level item.
    pub const ITEM_RECOVERY: &[SyntaxKind] = &[
        SyntaxKind::FN_KW,
        SyntaxKind::TYPE_KW,
        SyntaxKind::PROTOCOL_KW,
        SyntaxKind::IMPLEMENT_KW,
        SyntaxKind::CONTEXT_KW,
        SyntaxKind::PUB_KW,
        SyntaxKind::PUBLIC_KW,
        SyntaxKind::AT,
        SyntaxKind::EOF,
    ];

    /// Tokens that can start a statement.
    pub const STMT_RECOVERY: &[SyntaxKind] = &[
        SyntaxKind::LET_KW,
        SyntaxKind::IF_KW,
        SyntaxKind::WHILE_KW,
        SyntaxKind::FOR_KW,
        SyntaxKind::MATCH_KW,
        SyntaxKind::RETURN_KW,
        SyntaxKind::PROVIDE_KW,
        SyntaxKind::R_BRACE,
        SyntaxKind::SEMICOLON,
    ];

    /// Tokens that can follow an expression in statement position.
    pub const EXPR_STMT_RECOVERY: &[SyntaxKind] = &[
        SyntaxKind::SEMICOLON,
        SyntaxKind::R_BRACE,
        SyntaxKind::R_PAREN,
        SyntaxKind::COMMA,
    ];

    /// Tokens that can appear after a comma-separated item.
    pub const COMMA_RECOVERY: &[SyntaxKind] = &[
        SyntaxKind::COMMA,
        SyntaxKind::R_PAREN,
        SyntaxKind::R_BRACKET,
        SyntaxKind::R_BRACE,
        SyntaxKind::R_ANGLE,
    ];

    /// Tokens that can appear in type position.
    pub const TYPE_RECOVERY: &[SyntaxKind] = &[
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
    ];

    /// Create item recovery context.
    pub const fn item_context() -> RecoveryContext {
        RecoveryContext::new(ITEM_RECOVERY)
    }

    /// Create statement recovery context.
    pub const fn stmt_context() -> RecoveryContext {
        RecoveryContext::new(STMT_RECOVERY)
    }

    /// Create expression statement recovery context.
    pub const fn expr_stmt_context() -> RecoveryContext {
        RecoveryContext::new(EXPR_STMT_RECOVERY)
    }

    /// Create comma-separated list recovery context.
    pub const fn comma_context() -> RecoveryContext {
        RecoveryContext::new(COMMA_RECOVERY)
    }

    /// Create type recovery context.
    pub const fn type_context() -> RecoveryContext {
        RecoveryContext::new(TYPE_RECOVERY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_green_tree_sink() {
        let mut sink = GreenTreeSink::new();

        sink.start_node(SyntaxKind::SOURCE_FILE);
        sink.start_node(SyntaxKind::LET_STMT);
        sink.token(SyntaxKind::LET_KW, "let");
        sink.token(SyntaxKind::WHITESPACE, " ");
        sink.token(SyntaxKind::IDENT, "x");
        sink.finish_node();
        sink.finish_node();

        let (green, errors) = sink.finish();

        assert!(errors.is_empty());
        assert_eq!(green.kind(), SyntaxKind::SOURCE_FILE);
        assert_eq!(green.text(), "let x");
    }

    #[test]
    fn test_text_tree_sink() {
        let mut sink = TextTreeSink::new();

        sink.start_node(SyntaxKind::SOURCE_FILE);
        sink.token(SyntaxKind::LET_KW, "let");
        sink.finish_node();

        let output = sink.finish();

        assert!(output.contains("source file"));
        assert!(output.contains("let"));
    }

    #[test]
    fn test_stats_sink() {
        let mut sink = StatsSink::new();

        sink.start_node(SyntaxKind::SOURCE_FILE);
        sink.start_node(SyntaxKind::LET_STMT);
        sink.token(SyntaxKind::LET_KW, "let");
        sink.token(SyntaxKind::IDENT, "x");
        sink.finish_node();
        sink.finish_node();

        assert_eq!(sink.node_count(), 2);
        assert_eq!(sink.token_count(), 2);
        assert_eq!(sink.max_depth(), 2);
        assert_eq!(sink.error_count(), 0);
    }

    #[test]
    fn test_recovery_context() {
        let ctx = recovery_sets::item_context();
        assert!(ctx.recovery_set.contains(&SyntaxKind::FN_KW));
        assert!(ctx.recovery_set.contains(&SyntaxKind::TYPE_KW));
        assert!(ctx.create_error_node);
    }
}
