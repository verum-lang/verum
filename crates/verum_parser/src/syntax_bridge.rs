//! Bridge between verum_parser and verum_syntax for lossless parsing.
//!
//! This module provides:
//! - Conversion from token stream to green tree
//! - Lossless parsing that preserves all trivia
//! - Event-based parsing using verum_syntax's event infrastructure
//! - Incremental parsing support
//!
//! # Event-Based Architecture
//!
//! The event-based parsing uses the Marker/precede pattern for retroactive tree building:
//!
//! 1. Parser emits events (Start, Token, Finish, Error) via EventBuilder
//! 2. Events support forward_parent for retroactive wrapping (e.g., binary expressions)
//! 3. Events are processed by GreenTreeSink to build the green tree
//!
//! This enables:
//! - Lossless parsing (all trivia preserved)
//! - Structured error nodes for recovery
//! - Different tree representations from the same parse
//!
//! Uses the red-green tree pattern for lossless parsing infrastructure with incremental
//! re-parsing support. The bridge converts between concrete syntax tree (CST) and abstract
//! syntax tree (AST) representations while preserving all source information.

use verum_ast::{FileId, Module, Span};
use verum_common::List;
use verum_lexer::{Lexer, Token, TokenKind};
use verum_lexer::lossless::{LosslessLexer, RichToken, TriviaKind as LexerTriviaKind};
use verum_syntax::{
    GreenBuilder, GreenNode, SyntaxNode, SyntaxKind,
    TextEdit, TextRange, TextSize, IncrementalEngine, ChangeTracker,
    EventBuilder, Event, CompletedMarker, Marker, GreenTreeSink, EventSink,
    TokenSource, TriviaSource,
};
use verum_syntax::event::process;

use crate::ParseError;
use crate::RecursiveParser;
use crate::recovery::{
    recovery_sets, EventRecovery, Recoverable, RecoveryResult, RecoverySet,
    token_kind_to_syntax_kind as recovery_token_to_syntax,
};

/// Result of lossless parsing - includes both AST and syntax tree.
#[derive(Debug)]
pub struct LosslessParse {
    /// The semantic AST (for type checking, codegen).
    pub module: Module,
    /// The lossless green tree (for formatting, refactoring).
    pub green: GreenNode,
    /// Any parse errors encountered.
    pub errors: Vec<ParseError>,
}

impl LosslessParse {
    /// Get a navigable syntax tree (red tree facade).
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }

    /// Reconstruct the original source (lossless).
    pub fn text(&self) -> String {
        self.green.text()
    }

    /// Check if parsing succeeded without errors.
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Lossless parser that produces both AST and green tree.
pub struct LosslessParser;

impl LosslessParser {
    /// Create a new lossless parser.
    pub fn new() -> Self {
        Self
    }

    /// Parse source code losslessly using single-pass architecture.
    ///
    /// Returns both a semantic AST and a lossless green tree.
    /// Uses EventBasedParser to build green tree, then AstSink to convert to AST.
    pub fn parse(&self, source: &str, file_id: FileId) -> LosslessParse {
        // Phase 1: Use EventBasedParser to build structured green tree
        let event_result = EventBasedParser::parse_source(source);

        // Phase 2: Convert green tree to semantic AST using AstSink
        let syntax = SyntaxNode::new_root(event_result.green.clone());
        let ast_result = crate::ast_sink::syntax_to_ast(source, &syntax, file_id);

        // Combine errors from both phases
        let mut errors: Vec<ParseError> = Vec::new();
        for syntax_err in &event_result.errors {
            errors.push(ParseError::invalid_syntax(
                syntax_err.message.clone(),
                Span::new(syntax_err.range.start(), syntax_err.range.end(), file_id),
            ));
        }
        for err in ast_result.errors {
            errors.push(err);
        }

        LosslessParse {
            module: ast_result.module,
            green: event_result.green,
            errors,
        }
    }

    /// Parse source code using legacy dual-parsing approach.
    ///
    /// Kept for comparison and fallback if needed.
    #[deprecated(since = "0.5.0", note = "Use parse() for single-pass architecture")]
    pub fn parse_legacy(&self, source: &str, file_id: FileId) -> LosslessParse {
        // Phase 1: Lossless lexing
        let lossless_lexer = LosslessLexer::new(source, file_id);
        let rich_tokens = lossless_lexer.tokenize();

        // Phase 2: Build green tree from tokens
        let green = self.build_green_tree(source, &rich_tokens, file_id);

        // Phase 3: Parse AST (traditional parser)
        let lexer = Lexer::new(source, file_id);
        let (module, errors) = self.parse_ast(lexer, file_id);

        LosslessParse {
            module,
            green,
            errors,
        }
    }

    /// Build a green tree from rich tokens.
    fn build_green_tree(
        &self,
        source: &str,
        tokens: &List<RichToken>,
        _file_id: FileId,
    ) -> GreenNode {
        let mut builder = GreenBuilder::new();
        builder.start_node(SyntaxKind::SOURCE_FILE);

        for rich_token in tokens.iter() {
            // Add leading trivia
            for item in rich_token.leading_trivia.items.iter() {
                let kind = trivia_kind_to_syntax_kind(item.kind);
                builder.token(kind, &item.text);
            }

            // Add main token (skip EOF)
            if rich_token.token.kind != TokenKind::Eof {
                let kind = token_kind_to_syntax_kind(&rich_token.token.kind);
                let span = &rich_token.token.span;
                let text = if (span.start as usize) < source.len()
                    && (span.end as usize) <= source.len()
                {
                    &source[span.start as usize..span.end as usize]
                } else {
                    ""
                };
                builder.token(kind, text);
            }

            // Add trailing trivia
            for item in rich_token.trailing_trivia.items.iter() {
                let kind = trivia_kind_to_syntax_kind(item.kind);
                builder.token(kind, &item.text);
            }
        }

        builder.finish_node();
        builder.finish()
    }

    /// Parse AST using traditional parser.
    fn parse_ast(&self, lexer: Lexer, file_id: FileId) -> (Module, Vec<ParseError>) {
        let mut tokens = List::new();
        for result in lexer {
            match result {
                Ok(token) => tokens.push(token),
                Err(lex_error) => {
                    let parse_error = ParseError::invalid_syntax(
                        format!("lexer error: {}", lex_error.message()),
                        lex_error
                            .location()
                            .map(|loc| Span::new(loc.line, loc.column, file_id))
                            .unwrap_or(Span::new(0, 0, file_id)),
                    );
                    return (
                        Module::new(List::new(), file_id, Span::new(0, 0, file_id)),
                        vec![parse_error],
                    );
                }
            }
        }

        // Add EOF token
        let last_span = tokens
            .last()
            .map(|t| t.span)
            .unwrap_or(Span::new(0, 0, file_id));
        tokens.push(Token::new(
            TokenKind::Eof,
            Span::new(last_span.end, last_span.end, file_id),
        ));

        let mut parser = RecursiveParser::new(&tokens, file_id);

        match parser.parse_module() {
            Ok(items) => {
                let span = items
                    .first()
                    .zip(items.last())
                    .map(|(first, last)| first.span.merge(last.span))
                    .unwrap_or(Span::new(0, 0, file_id));

                let items_list: List<_> = items.into_iter().collect();
                (Module::new(items_list, file_id, span), parser.errors)
            }
            Err(e) => {
                let mut errors = parser.errors;
                errors.push(e);
                (
                    Module::new(List::new(), file_id, Span::new(0, 0, file_id)),
                    errors,
                )
            }
        }
    }
}

impl Default for LosslessParser {
    fn default() -> Self {
        Self::new()
    }
}

/// Incremental parser for IDE use cases.
///
/// Supports partial re-parsing when source is edited.
/// Uses the IncrementalEngine from verum_syntax for smart subtree replacement.
pub struct IncrementalParser {
    /// Cached green tree from last parse.
    cached_green: Option<GreenNode>,
    /// Cached source for comparison.
    cached_source: String,
    /// Incremental parsing engine.
    engine: IncrementalEngine,
    /// Change tracker for batched updates.
    change_tracker: ChangeTracker,
    /// File ID for parsing.
    file_id: FileId,
}

impl IncrementalParser {
    /// Create a new incremental parser.
    pub fn new() -> Self {
        Self {
            cached_green: None,
            cached_source: String::new(),
            engine: IncrementalEngine::new(),
            change_tracker: ChangeTracker::new(),
            file_id: FileId::new(0),
        }
    }

    /// Create an incremental parser with a specific file ID.
    pub fn with_file_id(file_id: FileId) -> Self {
        Self {
            cached_green: None,
            cached_source: String::new(),
            engine: IncrementalEngine::new(),
            change_tracker: ChangeTracker::new(),
            file_id,
        }
    }

    /// Parse or re-parse source code.
    ///
    /// If the change is small and a cached tree exists, attempts incremental re-parse.
    /// Otherwise falls back to full parse.
    pub fn parse(&mut self, source: &str, file_id: FileId) -> LosslessParse {
        self.file_id = file_id;

        // Check if we can use incremental parsing
        if let Some(cached) = &self.cached_green {
            if let Some(edit) = self.compute_single_edit(source) {
                if self.engine.should_use_incremental(cached, &edit) {
                    // Try incremental parse
                    let parser = LosslessParser::new();
                    let reparse_fn = |s: &str, _context: verum_syntax::ReparseContext| {
                        let result = parser.parse(s, file_id);
                        result.green
                    };

                    let new_green = self.engine.apply_edit(
                        cached,
                        &edit,
                        reparse_fn,
                        &self.cached_source,
                    );

                    // Re-parse AST (we always need fresh AST)
                    let lexer = Lexer::new(source, file_id);
                    let (module, errors) = LosslessParser::new().parse_ast(lexer, file_id);

                    self.cached_green = Some(new_green.clone());
                    self.cached_source = source.to_string();
                    self.change_tracker.clear_pending();

                    return LosslessParse {
                        module,
                        green: new_green,
                        errors,
                    };
                }
            }
        }

        // Full parse
        let parser = LosslessParser::new();
        let result = parser.parse(source, file_id);

        // Cache for future incremental updates
        self.cached_green = Some(result.green.clone());
        self.cached_source = source.to_string();
        self.change_tracker.clear_pending();

        result
    }

    /// Apply an edit and re-parse incrementally.
    ///
    /// The edit is specified as a range in the old source and replacement text.
    pub fn apply_edit(
        &mut self,
        start: usize,
        end: usize,
        replacement: &str,
        file_id: FileId,
    ) -> LosslessParse {
        let edit = TextEdit::replace(
            TextRange::new(
                TextSize::from(start as u32),
                TextSize::from(end as u32),
            ),
            replacement,
        );

        // Record the edit
        self.change_tracker.record_edit(edit.clone());

        // Apply edit to cached source
        let new_source = edit.apply(&self.cached_source);

        // Try incremental parse
        self.parse(&new_source, file_id)
    }

    /// Compute a single edit that transforms cached source to new source.
    fn compute_single_edit(&self, new_source: &str) -> Option<TextEdit> {
        let old = &self.cached_source;
        let new = new_source;

        if old == new {
            return None;
        }

        // Find first differing position
        let start = old.chars().zip(new.chars())
            .position(|(a, b)| a != b)
            .unwrap_or(std::cmp::min(old.len(), new.len()));

        // Find last differing position
        let old_rev: Vec<_> = old.chars().rev().collect();
        let new_rev: Vec<_> = new.chars().rev().collect();
        let end_offset = old_rev.iter().zip(new_rev.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(std::cmp::min(old.len(), new.len()));

        let old_end = old.len().saturating_sub(end_offset);
        let new_end = new.len().saturating_sub(end_offset);

        // Make sure start doesn't exceed end
        let start = std::cmp::min(start, old_end);
        let replacement = &new[start..new_end];

        Some(TextEdit::replace(
            TextRange::new(
                TextSize::from(start as u32),
                TextSize::from(old_end as u32),
            ),
            replacement,
        ))
    }

    /// Get the cached green tree if available.
    pub fn cached_tree(&self) -> Option<&GreenNode> {
        self.cached_green.as_ref()
    }

    /// Clear the cache.
    pub fn clear_cache(&mut self) {
        self.cached_green = None;
        self.cached_source.clear();
        self.change_tracker.clear_pending();
    }

    /// Get the change tracker version.
    pub fn version(&self) -> u64 {
        self.change_tracker.version()
    }
}

impl Default for IncrementalParser {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Event-Based Parsing Infrastructure
// ============================================================================

/// Event-based parser that uses verum_syntax's event infrastructure.
///
/// This parser demonstrates the connection between the event system and the
/// green tree building. It provides:
///
/// 1. Event emission via `EventBuilder` with Marker/precede pattern
/// 2. Event processing through `GreenTreeSink` to build green trees
/// 3. Lossless round-trip via proper trivia attachment
///
/// # Example
///
/// ```rust,ignore
/// use verum_parser::syntax_bridge::EventBasedParser;
/// use verum_ast::FileId;
///
/// let source = "fn foo() { let x = 1; }";
/// let file_id = FileId::new(0);
///
/// let parser = EventBasedParser::new();
/// let result = parser.parse(source, file_id);
///
/// // Events were emitted correctly
/// assert!(result.event_count > 0);
/// // Green tree was built from events
/// assert_eq!(result.green.kind(), SyntaxKind::SOURCE_FILE);
/// // Lossless round-trip
/// assert_eq!(result.green.text(), source);
/// ```
pub struct EventBasedParser;

/// Result of event-based parsing.
#[derive(Debug)]
pub struct EventBasedParse {
    /// The lossless green tree built from events.
    pub green: GreenNode,
    /// Number of events emitted during parsing.
    pub event_count: usize,
    /// Parse errors collected during event processing.
    pub errors: Vec<verum_syntax::ParseError>,
}

impl EventBasedParse {
    /// Get a navigable syntax tree (red tree facade).
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }

    /// Reconstruct the original source (lossless).
    pub fn text(&self) -> String {
        self.green.text()
    }

    /// Check if parsing succeeded without errors.
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

impl EventBasedParser {
    /// Create a new event-based parser.
    pub fn new() -> Self {
        Self
    }

    /// Parse source code using the event-based infrastructure.
    ///
    /// This method:
    /// 1. Tokenizes the source losslessly (preserving trivia)
    /// 2. Parses tokens emitting events via EventBuilder
    /// 3. Processes events through GreenTreeSink to build the green tree
    pub fn parse(&self, source: &str, file_id: FileId) -> EventBasedParse {
        // Phase 1: Lossless lexing to get tokens with trivia
        let lossless_lexer = LosslessLexer::new(source, file_id);
        let rich_tokens = lossless_lexer.tokenize();

        // Phase 2: Convert rich tokens to TokenSource for event processing
        let token_sources = self.convert_tokens_to_sources(source, &rich_tokens);

        // Phase 3: Parse using EventBuilder, emitting events
        let mut event_builder = EventBuilder::new();
        self.parse_with_events(source, &rich_tokens, &mut event_builder);

        // Phase 4: Process events through GreenTreeSink
        let events = event_builder.reorder();
        let event_count = events.len();

        let mut sink = GreenTreeSink::new();
        process(events, &token_sources, &mut sink);

        let (green, sink_errors) = sink.finish();
        let errors: Vec<verum_syntax::ParseError> = sink_errors
            .into_iter()
            .map(|e| verum_syntax::ParseError::at(0, e.message))
            .collect();

        EventBasedParse {
            green,
            event_count,
            errors,
        }
    }

    /// Parse source using events, emitting to the EventBuilder.
    ///
    /// This is a simplified recursive descent parser that demonstrates
    /// the Marker/precede pattern for building syntax trees.
    fn parse_with_events(
        &self,
        source: &str,
        tokens: &List<RichToken>,
        builder: &mut EventBuilder,
    ) {
        // Start the root SOURCE_FILE node
        let root = builder.start();

        // Track token position
        let mut pos = 0;
        let token_slice: Vec<_> = tokens.iter().collect();

        // Parse top-level items
        while pos < token_slice.len() {
            let token = &token_slice[pos];

            // Skip EOF
            if token.token.kind == TokenKind::Eof {
                // Emit the EOF token
                builder.token(SyntaxKind::EOF);
                pos += 1;
                continue;
            }

            // Try to parse a top-level item
            let (new_pos, parsed) = self.parse_item_events(source, &token_slice, pos, builder);
            if parsed {
                pos = new_pos;
            } else {
                // Couldn't parse - emit token as-is and move on
                let kind = token_kind_to_syntax_kind(&token.token.kind);
                builder.token(kind);
                pos += 1;
            }
        }

        // Complete the root node
        root.complete(builder, SyntaxKind::SOURCE_FILE);
    }

    /// Parse a top-level item (fn, type, etc.) using events.
    fn parse_item_events(
        &self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        if start_pos >= tokens.len() {
            return (start_pos, false);
        }

        let token = &tokens[start_pos];

        match &token.token.kind {
            // Visibility modifier - look ahead to determine item type
            TokenKind::Pub => {
                self.parse_item_with_visibility(source, tokens, start_pos, builder)
            }
            // Function modifiers - look ahead to determine if it's a function
            TokenKind::Async | TokenKind::Pure | TokenKind::Meta | TokenKind::Unsafe => {
                self.parse_item_with_modifiers(source, tokens, start_pos, builder)
            }
            // Function definition
            TokenKind::Fn => {
                let (new_pos, parsed) = self.parse_fn_def_events(source, tokens, start_pos, builder);
                (new_pos, parsed)
            }
            // Generator function (fn*)
            TokenKind::Star if start_pos > 0 => {
                // This should be handled by fn parsing
                (start_pos, false)
            }
            // Type definition
            TokenKind::Type => {
                let (new_pos, parsed) = self.parse_type_def_events(source, tokens, start_pos, builder);
                (new_pos, parsed)
            }
            // Implement block
            TokenKind::Implement => {
                self.parse_impl_block_events(source, tokens, start_pos, builder)
            }
            // Using statement (context group alias)
            TokenKind::Using => {
                self.parse_using_stmt_events(source, tokens, start_pos, builder)
            }
            // Const definition
            TokenKind::Const => {
                self.parse_const_def_events(tokens, start_pos, builder)
            }
            // Static definition
            TokenKind::Static => {
                self.parse_static_def_events(tokens, start_pos, builder)
            }
            // Let statement (in script mode)
            TokenKind::Let => {
                let (new_pos, parsed) = self.parse_let_stmt_events(source, tokens, start_pos, builder);
                (new_pos, parsed)
            }
            // Attribute
            TokenKind::At => {
                let (new_pos, parsed) = self.parse_attribute_and_item(source, tokens, start_pos, builder);
                (new_pos, parsed)
            }
            // Skip other tokens for now
            _ => (start_pos, false),
        }
    }

    /// Parse an item that starts with a visibility modifier (pub).
    fn parse_item_with_visibility(
        &self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let pos = start_pos;

        // Look ahead to find the actual item
        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::Pub) {
            return (start_pos, false);
        }

        // Peek at what follows pub
        let next_pos = pos + 1;
        if next_pos >= tokens.len() {
            return (start_pos, false);
        }

        match &tokens[next_pos].token.kind {
            TokenKind::Fn | TokenKind::Async | TokenKind::Pure | TokenKind::Meta | TokenKind::Unsafe => {
                self.parse_fn_def_events(source, tokens, pos, builder)
            }
            TokenKind::Type => {
                self.parse_type_def_events(source, tokens, pos, builder)
            }
            TokenKind::Const => {
                self.parse_const_def_events(tokens, pos, builder)
            }
            TokenKind::Static => {
                self.parse_static_def_events(tokens, pos, builder)
            }
            TokenKind::Implement => {
                self.parse_impl_block_events(source, tokens, pos, builder)
            }
            _ => (start_pos, false),
        }
    }

    /// Parse an item that starts with function modifiers (async, pure, meta, unsafe).
    fn parse_item_with_modifiers(
        &self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        // Look ahead to see if this is a function definition
        let mut look_ahead = start_pos;
        while look_ahead < tokens.len() {
            match &tokens[look_ahead].token.kind {
                TokenKind::Async | TokenKind::Pure | TokenKind::Meta | TokenKind::Unsafe | TokenKind::Pub => {
                    look_ahead += 1;
                }
                TokenKind::Fn => {
                    // This is a function definition with modifiers
                    return self.parse_fn_def_events(source, tokens, start_pos, builder);
                }
                _ => break,
            }
        }
        (start_pos, false)
    }

    /// Parse an attribute followed by an item.
    fn parse_attribute_and_item(
        &self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;

        // Parse attribute
        let (new_pos, parsed) = self.parse_attribute_events(source, tokens, pos, builder);
        if !parsed {
            return (start_pos, false);
        }
        pos = new_pos;

        // Parse the following item
        self.parse_item_events(source, tokens, pos, builder)
    }

    /// Parse an attribute (@name or @name(...)).
    fn parse_attribute_events(
        &self,
        _source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;

        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::At) {
            return (start_pos, false);
        }

        let attr_marker = builder.start();

        // Emit '@'
        builder.token(SyntaxKind::AT);
        pos += 1;

        // Expect identifier (attribute name)
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Ident(_)) {
            builder.token(SyntaxKind::IDENT);
            pos += 1;
        }

        // Optional argument list
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::LParen) {
            builder.token(SyntaxKind::L_PAREN);
            pos += 1;

            // Parse until closing paren
            let mut depth = 1;
            while pos < tokens.len() && depth > 0 {
                match &tokens[pos].token.kind {
                    TokenKind::LParen => depth += 1,
                    TokenKind::RParen => depth -= 1,
                    _ => {}
                }
                let kind = token_kind_to_syntax_kind(&tokens[pos].token.kind);
                builder.token(kind);
                pos += 1;
            }
        }

        attr_marker.complete(builder, SyntaxKind::ATTRIBUTE);
        (pos, true)
    }

    /// Parse a function definition using events.
    /// Handles: [pub] [async|pure|meta|unsafe]* fn[*] name[<generics>](params) [-> Type] [where ...] { body }
    fn parse_fn_def_events(
        &self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;

        // Start FN_DEF node
        let fn_marker = builder.start();

        // Optional visibility: pub
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Pub) {
            builder.token(SyntaxKind::PUB_KW);
            pos += 1;
        }

        // Optional modifiers: async, pure, meta, unsafe (can have multiple)
        while pos < tokens.len() {
            match &tokens[pos].token.kind {
                TokenKind::Async => {
                    builder.token(SyntaxKind::ASYNC_KW);
                    pos += 1;
                }
                TokenKind::Pure => {
                    builder.token(SyntaxKind::PURE_KW);
                    pos += 1;
                }
                TokenKind::Meta => {
                    builder.token(SyntaxKind::META_KW);
                    pos += 1;
                }
                TokenKind::Unsafe => {
                    builder.token(SyntaxKind::UNSAFE_KW);
                    pos += 1;
                }
                TokenKind::Cofix => {
                    builder.token(SyntaxKind::COFIX_KW);
                    pos += 1;
                }
                _ => break,
            }
        }

        // Expect 'fn'
        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::Fn) {
            fn_marker.abandon(builder);
            return (start_pos, false);
        }
        builder.token(SyntaxKind::FN_KW);
        pos += 1;

        // Optional generator marker: *
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Star) {
            builder.token(SyntaxKind::STAR);
            pos += 1;
        }

        // Expect identifier (function name)
        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::Ident(_)) {
            fn_marker.abandon(builder);
            return (start_pos, false);
        }
        builder.token(SyntaxKind::IDENT);
        pos += 1;

        // Optional generic parameters: <T, U>
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Lt) {
            let (new_pos, _) = self.parse_generic_params_events(source, tokens, pos, builder);
            pos = new_pos;
        }

        // Parse parameter list
        let (new_pos, _) = self.parse_param_list_events(source, tokens, pos, builder);
        pos = new_pos;

        // Optional return type: -> Type
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::RArrow) {
            builder.token(SyntaxKind::ARROW);
            pos += 1;

            // Parse return type
            let (new_pos, _) = self.parse_type_events(source, tokens, pos, builder);
            pos = new_pos;
        }

        // Optional where clause
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Where) {
            let (new_pos, _) = self.parse_where_clause_events(source, tokens, pos, builder);
            pos = new_pos;
        }

        // Parse block body (or semicolon for declarations)
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::LBrace) {
            let (new_pos, _) = self.parse_block_events(source, tokens, pos, builder);
            pos = new_pos;
        } else if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Semicolon) {
            builder.token(SyntaxKind::SEMICOLON);
            pos += 1;
        }

        // Complete the FN_DEF node
        fn_marker.complete(builder, SyntaxKind::FN_DEF);
        (pos, true)
    }

    /// Parse generic parameters: <T, U: Bound, V = Default>
    fn parse_generic_params_events(
        &self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;

        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::Lt) {
            return (start_pos, false);
        }

        let generics_marker = builder.start();
        builder.token(SyntaxKind::L_ANGLE);
        pos += 1;

        // Parse generic parameters
        while pos < tokens.len() && !matches!(tokens[pos].token.kind, TokenKind::Gt) {
            // Parse a single generic parameter
            let (new_pos, parsed) = self.parse_generic_param_events(source, tokens, pos, builder);
            if !parsed {
                break;
            }
            pos = new_pos;

            // Consume comma if present
            if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Comma) {
                builder.token(SyntaxKind::COMMA);
                pos += 1;
            }
        }

        // Expect '>'
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Gt) {
            builder.token(SyntaxKind::R_ANGLE);
            pos += 1;
        }

        generics_marker.complete(builder, SyntaxKind::GENERIC_PARAMS);
        (pos, true)
    }

    /// Parse a single generic parameter: T or T: Bound or const N: Int
    fn parse_generic_param_events(
        &self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;

        if pos >= tokens.len() {
            return (start_pos, false);
        }

        let param_marker = builder.start();

        // Check for const generic: const N: Int
        if matches!(tokens[pos].token.kind, TokenKind::Const) {
            builder.token(SyntaxKind::CONST_KW);
            pos += 1;
        }

        // Expect identifier (type parameter name)
        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::Ident(_)) {
            param_marker.abandon(builder);
            return (start_pos, false);
        }
        builder.token(SyntaxKind::IDENT);
        pos += 1;

        // Optional bound: : Bound
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Colon) {
            builder.token(SyntaxKind::COLON);
            pos += 1;

            // Parse bound (simplified - just identifier)
            if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Ident(_)) {
                let (new_pos, _) = self.parse_type_events(source, tokens, pos, builder);
                pos = new_pos;
            }
        }

        // Optional default: = DefaultType
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Eq) {
            builder.token(SyntaxKind::EQ);
            pos += 1;

            let (new_pos, _) = self.parse_type_events(source, tokens, pos, builder);
            pos = new_pos;
        }

        param_marker.complete(builder, SyntaxKind::GENERIC_PARAM);
        (pos, true)
    }

    /// Parse a where clause: where T: Bound, U: OtherBound
    fn parse_where_clause_events(
        &self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;

        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::Where) {
            return (start_pos, false);
        }

        let where_marker = builder.start();
        builder.token(SyntaxKind::WHERE_KW);
        pos += 1;

        // Parse predicates
        while pos < tokens.len() {
            // Stop at block start or semicolon
            if matches!(tokens[pos].token.kind, TokenKind::LBrace | TokenKind::Semicolon) {
                break;
            }

            let (new_pos, parsed) = self.parse_where_predicate_events(source, tokens, pos, builder);
            if !parsed {
                break;
            }
            pos = new_pos;

            // Consume comma if present
            if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Comma) {
                builder.token(SyntaxKind::COMMA);
                pos += 1;
            } else {
                break;
            }
        }

        where_marker.complete(builder, SyntaxKind::WHERE_CLAUSE);
        (pos, true)
    }

    /// Parse a where predicate: T: Bound or meta N > 0
    fn parse_where_predicate_events(
        &self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;

        if pos >= tokens.len() {
            return (start_pos, false);
        }

        let pred_marker = builder.start();

        // Check for meta predicate
        if matches!(tokens[pos].token.kind, TokenKind::Meta) {
            builder.token(SyntaxKind::META_KW);
            pos += 1;
        }

        // Type or name
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Ident(_)) {
            let (new_pos, _) = self.parse_type_events(source, tokens, pos, builder);
            pos = new_pos;
        } else {
            pred_marker.abandon(builder);
            return (start_pos, false);
        }

        // Colon and bounds
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Colon) {
            builder.token(SyntaxKind::COLON);
            pos += 1;

            // Parse bounds (simplified)
            while pos < tokens.len()
                && !matches!(
                    tokens[pos].token.kind,
                    TokenKind::Comma | TokenKind::LBrace | TokenKind::Semicolon
                )
            {
                // Parse bound
                if matches!(tokens[pos].token.kind, TokenKind::Ident(_)) {
                    let (new_pos, _) = self.parse_type_events(source, tokens, pos, builder);
                    pos = new_pos;
                } else {
                    break;
                }

                // Handle + for multiple bounds
                if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Plus) {
                    builder.token(SyntaxKind::PLUS);
                    pos += 1;
                } else {
                    break;
                }
            }
        }

        pred_marker.complete(builder, SyntaxKind::WHERE_PRED);
        (pos, true)
    }

    /// Parse a type definition using events.
    /// Handles: type aliases, records, variants, and protocols
    fn parse_type_def_events(
        &self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;

        // Start TYPE_DEF node
        let type_marker = builder.start();

        // Optional visibility: pub
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Pub) {
            builder.token(SyntaxKind::PUB_KW);
            pos += 1;
        }

        // Expect 'type'
        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::Type) {
            type_marker.abandon(builder);
            return (start_pos, false);
        }
        builder.token(SyntaxKind::TYPE_KW);
        pos += 1;

        // Expect identifier (type name)
        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::Ident(_)) {
            type_marker.abandon(builder);
            return (start_pos, false);
        }
        builder.token(SyntaxKind::IDENT);
        pos += 1;

        // Optional generic parameters: <T, U>
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Lt) {
            let (new_pos, _) = self.parse_generic_params_events(source, tokens, pos, builder);
            pos = new_pos;
        }

        // Expect 'is'
        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::Is) {
            type_marker.abandon(builder);
            return (start_pos, false);
        }
        builder.token(SyntaxKind::IS_KW);
        pos += 1;

        // Parse type body based on what follows
        if pos < tokens.len() {
            match &tokens[pos].token.kind {
                // Record type: type Foo is { x: Int, y: Int };
                TokenKind::LBrace => {
                    let (new_pos, _) = self.parse_field_list_events(source, tokens, pos, builder);
                    pos = new_pos;
                }
                // Protocol type: type Foo is protocol { ... };
                TokenKind::Protocol => {
                    builder.token(SyntaxKind::PROTOCOL_KW);
                    pos += 1;
                    if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::LBrace) {
                        let (new_pos, _) = self.parse_block_events(source, tokens, pos, builder);
                        pos = new_pos;
                    }
                }
                // Check for variant or alias
                // Variant: type Foo is A | B | C;
                // Alias: type Foo is Bar;
                _ => {
                    // Look ahead to detect if this is a variant type (has |)
                    let is_variant = self.is_variant_type(tokens, pos);
                    if is_variant {
                        let (new_pos, _) = self.parse_variant_list_events(source, tokens, pos, builder);
                        pos = new_pos;
                    } else {
                        // Type alias - parse a single type
                        let (new_pos, _) = self.parse_type_events(source, tokens, pos, builder);
                        pos = new_pos;
                    }
                }
            }
        }

        // Optional where clause
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Where) {
            let (new_pos, _) = self.parse_where_clause_events(source, tokens, pos, builder);
            pos = new_pos;
        }

        // Expect semicolon
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Semicolon) {
            builder.token(SyntaxKind::SEMICOLON);
            pos += 1;
        }

        type_marker.complete(builder, SyntaxKind::TYPE_DEF);
        (pos, true)
    }

    /// Check if the type definition starting at pos is a variant type (contains |)
    fn is_variant_type(&self, tokens: &[&RichToken], start_pos: usize) -> bool {
        let mut pos = start_pos;
        let mut brace_depth: usize = 0;
        let mut paren_depth: usize = 0;
        let mut angle_depth: usize = 0;

        while pos < tokens.len() {
            match &tokens[pos].token.kind {
                TokenKind::Semicolon if brace_depth == 0 && paren_depth == 0 && angle_depth == 0 => {
                    return false;
                }
                TokenKind::Pipe if brace_depth == 0 && paren_depth == 0 && angle_depth == 0 => {
                    return true;
                }
                TokenKind::LBrace => brace_depth += 1,
                TokenKind::RBrace => brace_depth = brace_depth.saturating_sub(1),
                TokenKind::LParen => paren_depth += 1,
                TokenKind::RParen => paren_depth = paren_depth.saturating_sub(1),
                TokenKind::Lt => angle_depth += 1,
                TokenKind::Gt => angle_depth = angle_depth.saturating_sub(1),
                _ => {}
            }
            pos += 1;
        }
        false
    }

    /// Parse a field list: { field1: Type1, field2: Type2 }
    fn parse_field_list_events(
        &self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;

        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::LBrace) {
            return (start_pos, false);
        }

        let list_marker = builder.start();
        builder.token(SyntaxKind::L_BRACE);
        pos += 1;

        while pos < tokens.len() && !matches!(tokens[pos].token.kind, TokenKind::RBrace) {
            let (new_pos, parsed) = self.parse_field_events(source, tokens, pos, builder);
            if !parsed {
                break;
            }
            pos = new_pos;

            // Consume comma if present
            if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Comma) {
                builder.token(SyntaxKind::COMMA);
                pos += 1;
            }
        }

        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::RBrace) {
            builder.token(SyntaxKind::R_BRACE);
            pos += 1;
        }

        list_marker.complete(builder, SyntaxKind::FIELD_LIST);
        (pos, true)
    }

    /// Parse a single field: [pub] name: Type
    fn parse_field_events(
        &self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;

        if pos >= tokens.len() {
            return (start_pos, false);
        }

        let field_marker = builder.start();

        // Optional visibility
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Pub) {
            builder.token(SyntaxKind::PUB_KW);
            pos += 1;
        }

        // Expect identifier (field name)
        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::Ident(_)) {
            field_marker.abandon(builder);
            return (start_pos, false);
        }
        builder.token(SyntaxKind::IDENT);
        pos += 1;

        // Expect colon
        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::Colon) {
            field_marker.abandon(builder);
            return (start_pos, false);
        }
        builder.token(SyntaxKind::COLON);
        pos += 1;

        // Parse type
        let (new_pos, _) = self.parse_type_events(source, tokens, pos, builder);
        pos = new_pos;

        field_marker.complete(builder, SyntaxKind::FIELD_DEF);
        (pos, true)
    }

    /// Parse a variant list: A | B(Int) | C { x: Int }
    fn parse_variant_list_events(
        &self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;
        let list_marker = builder.start();

        // Parse first variant
        let (new_pos, parsed) = self.parse_variant_events(source, tokens, pos, builder);
        if !parsed {
            list_marker.abandon(builder);
            return (start_pos, false);
        }
        pos = new_pos;

        // Parse additional variants separated by |
        while pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Pipe) {
            builder.token(SyntaxKind::PIPE);
            pos += 1;

            let (new_pos, parsed) = self.parse_variant_events(source, tokens, pos, builder);
            if !parsed {
                break;
            }
            pos = new_pos;
        }

        list_marker.complete(builder, SyntaxKind::VARIANT_LIST);
        (pos, true)
    }

    /// Parse a single variant: Name | Name(Type) | Name { field: Type }
    /// Variant names can be identifiers or the special keywords None/Some
    fn parse_variant_events(
        &self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;

        // Variant names can be identifiers or the special keywords None/Some
        let is_variant_name = pos < tokens.len() && matches!(
            tokens[pos].token.kind,
            TokenKind::Ident(_) | TokenKind::None | TokenKind::Some
        );
        if !is_variant_name {
            return (start_pos, false);
        }

        let variant_marker = builder.start();
        // Emit the appropriate syntax kind for the variant name
        let kind = match &tokens[pos].token.kind {
            TokenKind::None => SyntaxKind::NONE_KW,
            TokenKind::Some => SyntaxKind::SOME_KW,
            _ => SyntaxKind::IDENT,
        };
        builder.token(kind);
        pos += 1;

        // Check for tuple variant: Name(Type1, Type2)
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::LParen) {
            builder.token(SyntaxKind::L_PAREN);
            pos += 1;

            while pos < tokens.len() && !matches!(tokens[pos].token.kind, TokenKind::RParen) {
                let (new_pos, _) = self.parse_type_events(source, tokens, pos, builder);
                pos = new_pos;

                if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Comma) {
                    builder.token(SyntaxKind::COMMA);
                    pos += 1;
                } else {
                    break;
                }
            }

            if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::RParen) {
                builder.token(SyntaxKind::R_PAREN);
                pos += 1;
            }
        }
        // Check for struct variant: Name { field: Type }
        else if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::LBrace) {
            let (new_pos, _) = self.parse_field_list_events(source, tokens, pos, builder);
            pos = new_pos;
        }

        variant_marker.complete(builder, SyntaxKind::VARIANT_DEF);
        (pos, true)
    }

    /// Parse an implement block using events.
    /// Format: implement [Protocol for] Type { items }
    fn parse_impl_block_events(
        &self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;

        // Start IMPL_BLOCK node
        let impl_marker = builder.start();

        // Optional visibility
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Pub) {
            builder.token(SyntaxKind::PUB_KW);
            pos += 1;
        }

        // Expect 'implement'
        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::Implement) {
            impl_marker.abandon(builder);
            return (start_pos, false);
        }
        builder.token(SyntaxKind::IMPLEMENT_KW);
        pos += 1;

        // Parse protocol or type - simplified: consume until '{' or 'for'
        while pos < tokens.len() {
            match &tokens[pos].token.kind {
                TokenKind::LBrace => break,
                TokenKind::For => {
                    builder.token(SyntaxKind::FOR_KW);
                    pos += 1;
                }
                _ => {
                    let kind = token_kind_to_syntax_kind(&tokens[pos].token.kind);
                    builder.token(kind);
                    pos += 1;
                }
            }
        }

        // Parse block body
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::LBrace) {
            let (new_pos, _) = self.parse_block_events(source, tokens, pos, builder);
            pos = new_pos;
        }

        impl_marker.complete(builder, SyntaxKind::IMPL_BLOCK);
        (pos, true)
    }

    /// Parse a using statement (context group alias) using events.
    /// Format: using Name = [Context1, Context2];
    fn parse_using_stmt_events(
        &self,
        _source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;

        // Start USING_STMT node
        let using_marker = builder.start();

        // Expect 'using'
        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::Using) {
            using_marker.abandon(builder);
            return (start_pos, false);
        }
        builder.token(SyntaxKind::USING_KW);
        pos += 1;

        // Consume until semicolon (simplified)
        while pos < tokens.len() {
            let kind = token_kind_to_syntax_kind(&tokens[pos].token.kind);
            builder.token(kind);
            if matches!(tokens[pos].token.kind, TokenKind::Semicolon) {
                pos += 1;
                break;
            }
            pos += 1;
        }

        using_marker.complete(builder, SyntaxKind::CONTEXT_GROUP_DEF);
        (pos, true)
    }

    /// Parse a const definition using events.
    /// Format: [pub] const NAME: Type = value;
    fn parse_const_def_events(
        &self,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;

        // Start CONST_DEF node
        let const_marker = builder.start();

        // Optional visibility
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Pub) {
            builder.token(SyntaxKind::PUB_KW);
            pos += 1;
        }

        // Expect 'const'
        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::Const) {
            const_marker.abandon(builder);
            return (start_pos, false);
        }
        builder.token(SyntaxKind::CONST_KW);
        pos += 1;

        // Consume until semicolon (simplified)
        while pos < tokens.len() {
            let kind = token_kind_to_syntax_kind(&tokens[pos].token.kind);
            builder.token(kind);
            if matches!(tokens[pos].token.kind, TokenKind::Semicolon) {
                pos += 1;
                break;
            }
            pos += 1;
        }

        const_marker.complete(builder, SyntaxKind::CONST_DEF);
        (pos, true)
    }

    /// Parse a static definition using events.
    /// Format: [pub] static NAME: Type = value;
    fn parse_static_def_events(
        &self,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;

        // Start STATIC_DEF node
        let static_marker = builder.start();

        // Optional visibility
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Pub) {
            builder.token(SyntaxKind::PUB_KW);
            pos += 1;
        }

        // Expect 'static'
        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::Static) {
            static_marker.abandon(builder);
            return (start_pos, false);
        }
        builder.token(SyntaxKind::STATIC_KW);
        pos += 1;

        // Consume until semicolon (simplified)
        while pos < tokens.len() {
            let kind = token_kind_to_syntax_kind(&tokens[pos].token.kind);
            builder.token(kind);
            if matches!(tokens[pos].token.kind, TokenKind::Semicolon) {
                pos += 1;
                break;
            }
            pos += 1;
        }

        static_marker.complete(builder, SyntaxKind::STATIC_DEF);
        (pos, true)
    }

    /// Parse a let statement using events.
    fn parse_let_stmt_events(
        &self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;

        // Start LET_STMT node
        let let_marker = builder.start();

        // Expect 'let'
        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::Let) {
            let_marker.abandon(builder);
            return (start_pos, false);
        }
        builder.token(SyntaxKind::LET_KW);
        pos += 1;

        // Parse pattern (simplified - just identifier wrapped in IDENT_PAT)
        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::Ident(_)) {
            let_marker.abandon(builder);
            return (start_pos, false);
        }
        // Wrap identifier in IDENT_PAT node
        let pat_marker = builder.start();
        builder.token(SyntaxKind::IDENT);
        pat_marker.complete(builder, SyntaxKind::IDENT_PAT);
        pos += 1;

        // Optional type annotation: : Type
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Colon) {
            builder.token(SyntaxKind::COLON);
            pos += 1;

            let (new_pos, _) = self.parse_type_events(source, tokens, pos, builder);
            pos = new_pos;
        }

        // Expect '='
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Eq) {
            builder.token(SyntaxKind::EQ);
            pos += 1;

            // Parse expression
            let (new_pos, _) = self.parse_expr_events(source, tokens, pos, builder);
            pos = new_pos;
        }

        // Optional semicolon
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Semicolon) {
            builder.token(SyntaxKind::SEMICOLON);
            pos += 1;
        }

        let_marker.complete(builder, SyntaxKind::LET_STMT);
        (pos, true)
    }

    /// Parse a parameter list using events.
    fn parse_param_list_events(
        &self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;

        // Start PARAM_LIST node
        let params_marker = builder.start();

        // Expect '('
        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::LParen) {
            params_marker.abandon(builder);
            return (start_pos, false);
        }
        builder.token(SyntaxKind::L_PAREN);
        pos += 1;

        // Parse parameters
        while pos < tokens.len() && !matches!(tokens[pos].token.kind, TokenKind::RParen) {
            // Parse a single parameter
            let (new_pos, parsed) = self.parse_param_events(source, tokens, pos, builder);
            if !parsed {
                break;
            }
            pos = new_pos;

            // Consume comma if present
            if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Comma) {
                builder.token(SyntaxKind::COMMA);
                pos += 1;
            }
        }

        // Expect ')'
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::RParen) {
            builder.token(SyntaxKind::R_PAREN);
            pos += 1;
        }

        params_marker.complete(builder, SyntaxKind::PARAM_LIST);
        (pos, true)
    }

    /// Parse a single parameter using events.
    fn parse_param_events(
        &self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;

        // Skip if we hit ')' or EOF
        if pos >= tokens.len() || matches!(tokens[pos].token.kind, TokenKind::RParen | TokenKind::Eof) {
            return (pos, false);
        }

        // Check if this is a self parameter
        // Patterns: self, &self, &mut self, %self
        let is_self_param = self.is_self_param_start(tokens, pos);

        if is_self_param {
            return self.parse_self_param_events(tokens, pos, builder);
        }

        // Regular parameter: pattern: Type
        let param_marker = builder.start();

        // Parse pattern (for now, just identifier wrapped in IDENT_PAT)
        if !matches!(tokens[pos].token.kind, TokenKind::Ident(_)) {
            param_marker.abandon(builder);
            return (start_pos, false);
        }

        // Wrap identifier in IDENT_PAT node
        let pat_marker = builder.start();
        builder.token(SyntaxKind::IDENT);
        pat_marker.complete(builder, SyntaxKind::IDENT_PAT);
        pos += 1;

        // Optional type annotation: : Type
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Colon) {
            builder.token(SyntaxKind::COLON);
            pos += 1;

            let (new_pos, _) = self.parse_type_events(source, tokens, pos, builder);
            pos = new_pos;
        }

        param_marker.complete(builder, SyntaxKind::PARAM);
        (pos, true)
    }

    /// Check if the current position starts a self parameter.
    fn is_self_param_start(&self, tokens: &[&RichToken], pos: usize) -> bool {
        if pos >= tokens.len() {
            return false;
        }

        match &tokens[pos].token.kind {
            // self
            TokenKind::SelfValue => true,
            // &self or &mut self
            TokenKind::Ampersand => {
                if pos + 1 < tokens.len() {
                    matches!(
                        tokens[pos + 1].token.kind,
                        TokenKind::SelfValue | TokenKind::Mut
                    )
                } else {
                    false
                }
            }
            // %self (owned self)
            TokenKind::Percent => {
                pos + 1 < tokens.len() && matches!(tokens[pos + 1].token.kind, TokenKind::SelfValue)
            }
            _ => false,
        }
    }

    /// Parse a self parameter (self, &self, &mut self, %self).
    fn parse_self_param_events(
        &self,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;
        let param_marker = builder.start();

        match &tokens[pos].token.kind {
            // self
            TokenKind::SelfValue => {
                builder.token(SyntaxKind::SELF_VALUE_KW);
                pos += 1;
            }
            // &self or &mut self
            TokenKind::Ampersand => {
                builder.token(SyntaxKind::AMP);
                pos += 1;

                // Check for mut
                if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Mut) {
                    builder.token(SyntaxKind::MUT_KW);
                    pos += 1;
                }

                // Expect self
                if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::SelfValue) {
                    builder.token(SyntaxKind::SELF_VALUE_KW);
                    pos += 1;
                } else {
                    param_marker.abandon(builder);
                    return (start_pos, false);
                }
            }
            // %self (owned self)
            TokenKind::Percent => {
                builder.token(SyntaxKind::PERCENT);
                pos += 1;

                // Expect self
                if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::SelfValue) {
                    builder.token(SyntaxKind::SELF_VALUE_KW);
                    pos += 1;
                } else {
                    param_marker.abandon(builder);
                    return (start_pos, false);
                }
            }
            _ => {
                param_marker.abandon(builder);
                return (start_pos, false);
            }
        }

        param_marker.complete(builder, SyntaxKind::SELF_PARAM);
        (pos, true)
    }

    /// Parse a type using events.
    ///
    /// Handles: path types, reference types, tuple types, etc.
    fn parse_type_events(
        &self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let pos = start_pos;

        if pos >= tokens.len() {
            return (pos, false);
        }

        match &tokens[pos].token.kind {
            // Reference types: &T, &mut T, &checked T, &checked mut T, &unsafe T, &unsafe mut T
            TokenKind::Ampersand => {
                self.parse_reference_type_events(source, tokens, pos, builder)
            }
            // Tuple types: (T1, T2, ...)
            TokenKind::LParen => {
                self.parse_tuple_type_events(source, tokens, pos, builder)
            }
            // Path types: Ident<T>
            TokenKind::Ident(_) | TokenKind::SelfType => {
                self.parse_path_type_events(source, tokens, pos, builder)
            }
            _ => (start_pos, false),
        }
    }

    /// Parse a reference type: &T, &mut T, &checked T, &checked mut T, &unsafe T, &unsafe mut T
    fn parse_reference_type_events(
        &self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;
        let type_marker = builder.start();

        // Expect &
        if !matches!(tokens[pos].token.kind, TokenKind::Ampersand) {
            type_marker.abandon(builder);
            return (start_pos, false);
        }
        builder.token(SyntaxKind::AMP);
        pos += 1;

        // Check for modifiers: checked, unsafe, mut
        while pos < tokens.len() {
            match &tokens[pos].token.kind {
                TokenKind::Checked => {
                    builder.token(SyntaxKind::CHECKED_KW);
                    pos += 1;
                }
                TokenKind::Unsafe => {
                    builder.token(SyntaxKind::UNSAFE_KW);
                    pos += 1;
                }
                TokenKind::Mut => {
                    builder.token(SyntaxKind::MUT_KW);
                    pos += 1;
                }
                _ => break,
            }
        }

        // Parse the referenced type
        let (new_pos, parsed) = self.parse_type_events(source, tokens, pos, builder);
        if !parsed {
            type_marker.abandon(builder);
            return (start_pos, false);
        }
        pos = new_pos;

        type_marker.complete(builder, SyntaxKind::REFERENCE_TYPE);
        (pos, true)
    }

    /// Parse a tuple type: (T1, T2, ...)
    fn parse_tuple_type_events(
        &self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;
        let type_marker = builder.start();

        // Expect (
        if !matches!(tokens[pos].token.kind, TokenKind::LParen) {
            type_marker.abandon(builder);
            return (start_pos, false);
        }
        builder.token(SyntaxKind::L_PAREN);
        pos += 1;

        // Parse types separated by commas
        let mut first = true;
        while pos < tokens.len() && !matches!(tokens[pos].token.kind, TokenKind::RParen) {
            if !first {
                if matches!(tokens[pos].token.kind, TokenKind::Comma) {
                    builder.token(SyntaxKind::COMMA);
                    pos += 1;
                } else {
                    break;
                }
            }
            first = false;

            let (new_pos, parsed) = self.parse_type_events(source, tokens, pos, builder);
            if !parsed {
                break;
            }
            pos = new_pos;
        }

        // Expect )
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::RParen) {
            builder.token(SyntaxKind::R_PAREN);
            pos += 1;
        }

        type_marker.complete(builder, SyntaxKind::TUPLE_TYPE);
        (pos, true)
    }

    /// Parse a path type: Ident<T, U>
    fn parse_path_type_events(
        &self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;
        let type_marker = builder.start();

        // Expect identifier or Self
        match &tokens[pos].token.kind {
            TokenKind::Ident(_) => {
                builder.token(SyntaxKind::IDENT);
                pos += 1;
            }
            TokenKind::SelfType => {
                builder.token(SyntaxKind::SELF_TYPE_KW);
                pos += 1;
            }
            _ => {
                type_marker.abandon(builder);
                return (start_pos, false);
            }
        }

        // Check for generic args <T, U>
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Lt) {
            builder.token(SyntaxKind::L_ANGLE);
            pos += 1;

            // Parse generic arguments
            let mut first = true;
            while pos < tokens.len() && !matches!(tokens[pos].token.kind, TokenKind::Gt) {
                if !first {
                    if matches!(tokens[pos].token.kind, TokenKind::Comma) {
                        builder.token(SyntaxKind::COMMA);
                        pos += 1;
                    } else {
                        break;
                    }
                }
                first = false;

                let (new_pos, parsed) = self.parse_type_events(source, tokens, pos, builder);
                if !parsed {
                    // Fallback: emit as raw token
                    let kind = token_kind_to_syntax_kind(&tokens[pos].token.kind);
                    builder.token(kind);
                    pos += 1;
                } else {
                    pos = new_pos;
                }
            }

            if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Gt) {
                builder.token(SyntaxKind::R_ANGLE);
                pos += 1;
            }
        }

        type_marker.complete(builder, SyntaxKind::PATH_TYPE);
        (pos, true)
    }

    /// Parse a block using events.
    fn parse_block_events(
        &self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;

        // Start BLOCK node
        let block_marker = builder.start();

        // Expect '{'
        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::LBrace) {
            block_marker.abandon(builder);
            return (start_pos, false);
        }
        builder.token(SyntaxKind::L_BRACE);
        pos += 1;

        // Parse statements inside block
        while pos < tokens.len() && !matches!(tokens[pos].token.kind, TokenKind::RBrace) {
            // Try to parse a statement
            let (new_pos, parsed) = self.parse_stmt_events(source, tokens, pos, builder);
            if parsed {
                pos = new_pos;
            } else {
                // Emit unknown token and continue
                let kind = token_kind_to_syntax_kind(&tokens[pos].token.kind);
                builder.token(kind);
                pos += 1;
            }
        }

        // Expect '}'
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::RBrace) {
            builder.token(SyntaxKind::R_BRACE);
            pos += 1;
        }

        block_marker.complete(builder, SyntaxKind::BLOCK);
        (pos, true)
    }

    /// Parse a statement using events.
    fn parse_stmt_events(
        &self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        if start_pos >= tokens.len() {
            return (start_pos, false);
        }

        match &tokens[start_pos].token.kind {
            TokenKind::Let => self.parse_let_stmt_events(source, tokens, start_pos, builder),
            TokenKind::Return => {
                let mut pos = start_pos;
                let stmt_marker = builder.start();

                builder.token(SyntaxKind::RETURN_KW);
                pos += 1;

                // Optional expression
                if pos < tokens.len()
                    && !matches!(tokens[pos].token.kind, TokenKind::Semicolon | TokenKind::RBrace)
                {
                    let (new_pos, _) = self.parse_expr_events(source, tokens, pos, builder);
                    pos = new_pos;
                }

                // Optional semicolon
                if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Semicolon) {
                    builder.token(SyntaxKind::SEMICOLON);
                    pos += 1;
                }

                stmt_marker.complete(builder, SyntaxKind::RETURN_STMT);
                (pos, true)
            }
            // Other tokens are treated as expression statements
            _ => {
                let (new_pos, parsed) = self.parse_expr_events(source, tokens, start_pos, builder);
                if parsed && new_pos < tokens.len() && matches!(tokens[new_pos].token.kind, TokenKind::Semicolon) {
                    builder.token(SyntaxKind::SEMICOLON);
                    (new_pos + 1, true)
                } else {
                    (new_pos, parsed)
                }
            }
        }
    }

    /// Parse an expression using events (simplified - handles basic cases).
    fn parse_expr_events(
        &self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        // Parse primary expression first
        let (mut pos, parsed) = self.parse_primary_expr_events(source, tokens, start_pos, builder);
        if !parsed {
            return (start_pos, false);
        }

        // Check for binary operators (simplified - uses precede pattern)
        while pos < tokens.len() {
            match &tokens[pos].token.kind {
                TokenKind::Plus | TokenKind::Minus | TokenKind::Star | TokenKind::Slash
                | TokenKind::EqEq | TokenKind::BangEq | TokenKind::Lt | TokenKind::Gt
                | TokenKind::LtEq | TokenKind::GtEq => {
                    // This is where the precede pattern shines!
                    // We already emitted the left operand, now we wrap it in a binary expr
                    let op_kind = token_kind_to_syntax_kind(&tokens[pos].token.kind);
                    builder.token(op_kind);
                    pos += 1;

                    // Parse right operand
                    let (new_pos, _) = self.parse_primary_expr_events(source, tokens, pos, builder);
                    pos = new_pos;
                }
                _ => break,
            }
        }

        (pos, true)
    }

    /// Parse a primary expression using events.
    fn parse_primary_expr_events(
        &self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        if start_pos >= tokens.len() {
            return (start_pos, false);
        }

        let mut pos = start_pos;
        let expr_marker = builder.start();

        match &tokens[pos].token.kind {
            // Literals
            TokenKind::Integer(_) => {
                builder.token(SyntaxKind::INT_LITERAL);
                pos += 1;
                expr_marker.complete(builder, SyntaxKind::LITERAL_EXPR);
                (pos, true)
            }
            TokenKind::Float(_) => {
                builder.token(SyntaxKind::FLOAT_LITERAL);
                pos += 1;
                expr_marker.complete(builder, SyntaxKind::LITERAL_EXPR);
                (pos, true)
            }
            TokenKind::Text(_) => {
                builder.token(SyntaxKind::STRING_LITERAL);
                pos += 1;
                expr_marker.complete(builder, SyntaxKind::LITERAL_EXPR);
                (pos, true)
            }
            TokenKind::True => {
                builder.token(SyntaxKind::TRUE_KW);
                pos += 1;
                expr_marker.complete(builder, SyntaxKind::LITERAL_EXPR);
                (pos, true)
            }
            TokenKind::False => {
                builder.token(SyntaxKind::FALSE_KW);
                pos += 1;
                expr_marker.complete(builder, SyntaxKind::LITERAL_EXPR);
                (pos, true)
            }
            // Identifiers (paths)
            TokenKind::Ident(_) => {
                builder.token(SyntaxKind::IDENT);
                pos += 1;

                // Check for function call
                if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::LParen) {
                    // Convert to CALL_EXPR
                    let (new_pos, _) = self.parse_arg_list_events(source, tokens, pos, builder);
                    pos = new_pos;
                    expr_marker.complete(builder, SyntaxKind::CALL_EXPR);
                } else {
                    expr_marker.complete(builder, SyntaxKind::PATH_EXPR);
                }
                (pos, true)
            }
            // Parenthesized expression
            TokenKind::LParen => {
                builder.token(SyntaxKind::L_PAREN);
                pos += 1;

                // Parse inner expression
                let (new_pos, _) = self.parse_expr_events(source, tokens, pos, builder);
                pos = new_pos;

                if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::RParen) {
                    builder.token(SyntaxKind::R_PAREN);
                    pos += 1;
                }

                expr_marker.complete(builder, SyntaxKind::PAREN_EXPR);
                (pos, true)
            }
            // Block expression
            TokenKind::LBrace => {
                expr_marker.abandon(builder);
                self.parse_block_events(source, tokens, pos, builder)
            }
            _ => {
                expr_marker.abandon(builder);
                (start_pos, false)
            }
        }
    }

    /// Parse argument list using events.
    fn parse_arg_list_events(
        &self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;

        let args_marker = builder.start();

        // Expect '('
        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::LParen) {
            args_marker.abandon(builder);
            return (start_pos, false);
        }
        builder.token(SyntaxKind::L_PAREN);
        pos += 1;

        // Parse arguments
        while pos < tokens.len() && !matches!(tokens[pos].token.kind, TokenKind::RParen) {
            let (new_pos, parsed) = self.parse_expr_events(source, tokens, pos, builder);
            if !parsed {
                break;
            }
            pos = new_pos;

            // Consume comma if present
            if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Comma) {
                builder.token(SyntaxKind::COMMA);
                pos += 1;
            }
        }

        // Expect ')'
        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::RParen) {
            builder.token(SyntaxKind::R_PAREN);
            pos += 1;
        }

        args_marker.complete(builder, SyntaxKind::ARG_LIST);
        (pos, true)
    }

    /// Convert rich tokens to TokenSource format for event processing.
    fn convert_tokens_to_sources(&self, source: &str, tokens: &List<RichToken>) -> Vec<TokenSource> {
        let mut sources = Vec::new();

        for rich_token in tokens.iter() {
            // Convert leading trivia
            let leading: Vec<TriviaSource> = rich_token
                .leading_trivia
                .items
                .iter()
                .map(|item| TriviaSource {
                    kind: trivia_kind_to_syntax_kind(item.kind),
                    text: item.text.clone(),
                })
                .collect();

            // Convert trailing trivia
            let trailing: Vec<TriviaSource> = rich_token
                .trailing_trivia
                .items
                .iter()
                .map(|item| TriviaSource {
                    kind: trivia_kind_to_syntax_kind(item.kind),
                    text: item.text.clone(),
                })
                .collect();

            // Get main token text
            let span = &rich_token.token.span;
            let text = if (span.start as usize) < source.len()
                && (span.end as usize) <= source.len()
            {
                source[span.start as usize..span.end as usize].to_string()
            } else {
                String::new()
            };

            sources.push(TokenSource {
                kind: token_kind_to_syntax_kind(&rich_token.token.kind),
                text,
                leading_trivia: leading,
                trailing_trivia: trailing,
            });
        }

        sources
    }
}

impl Default for EventBasedParser {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBasedParser {
    /// Parse source as a complete module (top-level items).
    ///
    /// This is the default parsing mode using a default file ID.
    pub fn parse_source(source: &str) -> EventBasedParse {
        let parser = Self::new();
        let file_id = FileId::new(0);
        parser.parse_internal(source, file_id)
    }

    /// Parse source as a single top-level item.
    pub fn parse_item(source: &str) -> EventBasedParse {
        // For items, we wrap in a SOURCE_FILE but only parse one item
        let parser = Self::new();
        parser.parse_item_internal(source)
    }

    /// Parse source as a block (statements within braces).
    pub fn parse_block(source: &str) -> EventBasedParse {
        let parser = Self::new();
        parser.parse_block_internal(source)
    }

    /// Parse source as a single statement.
    pub fn parse_statement(source: &str) -> EventBasedParse {
        let parser = Self::new();
        parser.parse_statement_internal(source)
    }

    /// Parse source as an expression.
    pub fn parse_expression(source: &str) -> EventBasedParse {
        let parser = Self::new();
        parser.parse_expression_internal(source)
    }

    /// Parse source as a type.
    pub fn parse_type(source: &str) -> EventBasedParse {
        let parser = Self::new();
        parser.parse_type_internal(source)
    }

    /// Internal: parse as module (default)
    fn parse_internal(&self, source: &str, file_id: FileId) -> EventBasedParse {
        // This delegates to the existing parse method with file_id
        let lossless_lexer = LosslessLexer::new(source, file_id);
        let rich_tokens = lossless_lexer.tokenize();

        let token_sources = self.convert_tokens_to_sources(source, &rich_tokens);

        let mut event_builder = EventBuilder::new();
        self.parse_with_events(source, &rich_tokens, &mut event_builder);

        let events = event_builder.reorder();
        let event_count = events.len();

        let mut sink = GreenTreeSink::new();
        process(events, &token_sources, &mut sink);

        let (green, sink_errors) = sink.finish();
        let errors: Vec<verum_syntax::ParseError> = sink_errors
            .into_iter()
            .map(|e| verum_syntax::ParseError::at(0, e.message))
            .collect();

        EventBasedParse {
            green,
            event_count,
            errors,
        }
    }

    /// Internal: parse as single item
    fn parse_item_internal(&self, source: &str) -> EventBasedParse {
        let file_id = FileId::new(0);
        let lossless_lexer = LosslessLexer::new(source, file_id);
        let rich_tokens = lossless_lexer.tokenize();

        let token_sources = self.convert_tokens_to_sources(source, &rich_tokens);
        let token_slice: Vec<_> = rich_tokens.iter().collect();

        let mut event_builder = EventBuilder::new();

        // Start SOURCE_FILE wrapper
        let root = event_builder.start();

        // Parse single item
        if !token_slice.is_empty() {
            let _ = self.parse_item_events(source, &token_slice, 0, &mut event_builder);
        }

        root.complete(&mut event_builder, SyntaxKind::SOURCE_FILE);

        let events = event_builder.reorder();
        let event_count = events.len();

        let mut sink = GreenTreeSink::new();
        process(events, &token_sources, &mut sink);

        let (green, sink_errors) = sink.finish();
        let errors: Vec<verum_syntax::ParseError> = sink_errors
            .into_iter()
            .map(|e| verum_syntax::ParseError::at(0, e.message))
            .collect();

        EventBasedParse {
            green,
            event_count,
            errors,
        }
    }

    /// Internal: parse as block
    fn parse_block_internal(&self, source: &str) -> EventBasedParse {
        let file_id = FileId::new(0);
        let lossless_lexer = LosslessLexer::new(source, file_id);
        let rich_tokens = lossless_lexer.tokenize();

        let token_sources = self.convert_tokens_to_sources(source, &rich_tokens);
        let token_slice: Vec<_> = rich_tokens.iter().collect();

        let mut event_builder = EventBuilder::new();

        // Start SOURCE_FILE wrapper
        let root = event_builder.start();

        // Parse block
        if !token_slice.is_empty() {
            let _ = self.parse_block_events(source, &token_slice, 0, &mut event_builder);
        }

        root.complete(&mut event_builder, SyntaxKind::SOURCE_FILE);

        let events = event_builder.reorder();
        let event_count = events.len();

        let mut sink = GreenTreeSink::new();
        process(events, &token_sources, &mut sink);

        let (green, sink_errors) = sink.finish();
        let errors: Vec<verum_syntax::ParseError> = sink_errors
            .into_iter()
            .map(|e| verum_syntax::ParseError::at(0, e.message))
            .collect();

        EventBasedParse {
            green,
            event_count,
            errors,
        }
    }

    /// Internal: parse as statement
    fn parse_statement_internal(&self, source: &str) -> EventBasedParse {
        let file_id = FileId::new(0);
        let lossless_lexer = LosslessLexer::new(source, file_id);
        let rich_tokens = lossless_lexer.tokenize();

        let token_sources = self.convert_tokens_to_sources(source, &rich_tokens);
        let token_slice: Vec<_> = rich_tokens.iter().collect();

        let mut event_builder = EventBuilder::new();

        // Start SOURCE_FILE wrapper
        let root = event_builder.start();

        // Parse statement
        if !token_slice.is_empty() {
            let _ = self.parse_stmt_events(source, &token_slice, 0, &mut event_builder);
        }

        root.complete(&mut event_builder, SyntaxKind::SOURCE_FILE);

        let events = event_builder.reorder();
        let event_count = events.len();

        let mut sink = GreenTreeSink::new();
        process(events, &token_sources, &mut sink);

        let (green, sink_errors) = sink.finish();
        let errors: Vec<verum_syntax::ParseError> = sink_errors
            .into_iter()
            .map(|e| verum_syntax::ParseError::at(0, e.message))
            .collect();

        EventBasedParse {
            green,
            event_count,
            errors,
        }
    }

    /// Internal: parse as expression
    fn parse_expression_internal(&self, source: &str) -> EventBasedParse {
        let file_id = FileId::new(0);
        let lossless_lexer = LosslessLexer::new(source, file_id);
        let rich_tokens = lossless_lexer.tokenize();

        let token_sources = self.convert_tokens_to_sources(source, &rich_tokens);
        let token_slice: Vec<_> = rich_tokens.iter().collect();

        let mut event_builder = EventBuilder::new();

        // Start SOURCE_FILE wrapper
        let root = event_builder.start();

        // Parse expression
        if !token_slice.is_empty() {
            let _ = self.parse_expr_events(source, &token_slice, 0, &mut event_builder);
        }

        root.complete(&mut event_builder, SyntaxKind::SOURCE_FILE);

        let events = event_builder.reorder();
        let event_count = events.len();

        let mut sink = GreenTreeSink::new();
        process(events, &token_sources, &mut sink);

        let (green, sink_errors) = sink.finish();
        let errors: Vec<verum_syntax::ParseError> = sink_errors
            .into_iter()
            .map(|e| verum_syntax::ParseError::at(0, e.message))
            .collect();

        EventBasedParse {
            green,
            event_count,
            errors,
        }
    }

    /// Internal: parse as type
    fn parse_type_internal(&self, source: &str) -> EventBasedParse {
        let file_id = FileId::new(0);
        let lossless_lexer = LosslessLexer::new(source, file_id);
        let rich_tokens = lossless_lexer.tokenize();

        let token_sources = self.convert_tokens_to_sources(source, &rich_tokens);
        let token_slice: Vec<_> = rich_tokens.iter().collect();

        let mut event_builder = EventBuilder::new();

        // Start SOURCE_FILE wrapper
        let root = event_builder.start();

        // Parse type
        if !token_slice.is_empty() {
            let _ = self.parse_type_events(source, &token_slice, 0, &mut event_builder);
        }

        root.complete(&mut event_builder, SyntaxKind::SOURCE_FILE);

        let events = event_builder.reorder();
        let event_count = events.len();

        let mut sink = GreenTreeSink::new();
        process(events, &token_sources, &mut sink);

        let (green, sink_errors) = sink.finish();
        let errors: Vec<verum_syntax::ParseError> = sink_errors
            .into_iter()
            .map(|e| verum_syntax::ParseError::at(0, e.message))
            .collect();

        EventBasedParse {
            green,
            event_count,
            errors,
        }
    }
}

/// Convert lexer trivia kind to syntax kind.
fn trivia_kind_to_syntax_kind(kind: LexerTriviaKind) -> SyntaxKind {
    match kind {
        LexerTriviaKind::Whitespace => SyntaxKind::WHITESPACE,
        LexerTriviaKind::Newline => SyntaxKind::NEWLINE,
        LexerTriviaKind::LineComment => SyntaxKind::LINE_COMMENT,
        LexerTriviaKind::BlockComment => SyntaxKind::BLOCK_COMMENT,
        LexerTriviaKind::DocComment => SyntaxKind::DOC_COMMENT,
        LexerTriviaKind::InnerDocComment => SyntaxKind::DOC_COMMENT,
    }
}

/// Convert token kind to syntax kind.
fn token_kind_to_syntax_kind(kind: &TokenKind) -> SyntaxKind {
    match kind {
        // Reserved Keywords
        TokenKind::Let => SyntaxKind::LET_KW,
        TokenKind::Fn => SyntaxKind::FN_KW,
        TokenKind::Is => SyntaxKind::IS_KW,

        // Primary Keywords
        TokenKind::Type => SyntaxKind::TYPE_KW,
        TokenKind::Where => SyntaxKind::WHERE_KW,
        TokenKind::Match => SyntaxKind::MATCH_KW,
        TokenKind::Mount => SyntaxKind::MOUNT_KW,

        // Control Flow Keywords
        TokenKind::If => SyntaxKind::IF_KW,
        TokenKind::Else => SyntaxKind::ELSE_KW,
        TokenKind::While => SyntaxKind::WHILE_KW,
        TokenKind::For => SyntaxKind::FOR_KW,
        TokenKind::Loop => SyntaxKind::LOOP_KW,
        TokenKind::Break => SyntaxKind::BREAK_KW,
        TokenKind::Continue => SyntaxKind::CONTINUE_KW,
        TokenKind::Return => SyntaxKind::RETURN_KW,
        TokenKind::Yield => SyntaxKind::YIELD_KW,

        // Async/Context Keywords
        TokenKind::Async => SyntaxKind::ASYNC_KW,
        TokenKind::Await => SyntaxKind::AWAIT_KW,
        TokenKind::Spawn => SyntaxKind::SPAWN_KW,
        TokenKind::Select => SyntaxKind::SELECT_KW,
        TokenKind::Nursery => SyntaxKind::NURSERY_KW,
        TokenKind::Defer => SyntaxKind::DEFER_KW,
        TokenKind::Errdefer => SyntaxKind::ERRDEFER_KW,
        TokenKind::Try => SyntaxKind::TRY_KW,
        TokenKind::Throw => SyntaxKind::THROW_KW,
        TokenKind::Throws => SyntaxKind::THROWS_KW,
        TokenKind::Recover => SyntaxKind::RECOVER_KW,
        TokenKind::Finally => SyntaxKind::FINALLY_KW,

        // Modifier Keywords
        TokenKind::Pub => SyntaxKind::PUB_KW,
        TokenKind::Public => SyntaxKind::PUBLIC_KW,
        TokenKind::Internal => SyntaxKind::INTERNAL_KW,
        TokenKind::Protected => SyntaxKind::PROTECTED_KW,
        TokenKind::Private => SyntaxKind::PRIVATE_KW,
        TokenKind::Mut => SyntaxKind::MUT_KW,
        TokenKind::Const => SyntaxKind::CONST_KW,
        TokenKind::Volatile => SyntaxKind::VOLATILE_KW,
        TokenKind::Static => SyntaxKind::STATIC_KW,
        TokenKind::Unsafe => SyntaxKind::UNSAFE_KW,
        TokenKind::Meta => SyntaxKind::META_KW,
        TokenKind::QuoteKeyword => SyntaxKind::QUOTE_KW,
        TokenKind::Stage => SyntaxKind::STAGE_KW,
        TokenKind::Lift => SyntaxKind::LIFT_KW,
        TokenKind::Pure => SyntaxKind::PURE_KW,
        TokenKind::Affine => SyntaxKind::AFFINE_KW,
        TokenKind::Linear => SyntaxKind::LINEAR_KW,

        // Module/Type Keywords
        TokenKind::Module => SyntaxKind::MODULE_KW,
        TokenKind::Implement => SyntaxKind::IMPLEMENT_KW,
        TokenKind::Protocol => SyntaxKind::PROTOCOL_KW,
        TokenKind::Extends => SyntaxKind::EXTENDS_KW,
        TokenKind::Context => SyntaxKind::CONTEXT_KW,
        TokenKind::Provide => SyntaxKind::PROVIDE_KW,
        TokenKind::Inject => SyntaxKind::IDENT,
        TokenKind::Layer => SyntaxKind::IDENT,
        TokenKind::Ffi => SyntaxKind::FFI_KW,
        TokenKind::Extern => SyntaxKind::EXTERN_KW,
        TokenKind::Stream => SyntaxKind::STREAM_KW,
        TokenKind::Tensor => SyntaxKind::TENSOR_KW,
        TokenKind::Set => SyntaxKind::SET_KW,
        TokenKind::Gen => SyntaxKind::GEN_KW,
        TokenKind::Using => SyntaxKind::USING_KW,

        // Reference/Value Keywords
        TokenKind::SelfValue => SyntaxKind::SELF_VALUE_KW,
        TokenKind::SelfType => SyntaxKind::SELF_TYPE_KW,
        TokenKind::Super => SyntaxKind::SUPER_KW,
        TokenKind::Cog => SyntaxKind::COG_KW,
        TokenKind::Ref => SyntaxKind::REF_KW,
        TokenKind::Move => SyntaxKind::MOVE_KW,
        TokenKind::As => SyntaxKind::AS_KW,
        TokenKind::In => SyntaxKind::IN_KW,
        TokenKind::Checked => SyntaxKind::CHECKED_KW,

        // Verification Keywords
        TokenKind::Requires => SyntaxKind::REQUIRES_KW,
        TokenKind::Ensures => SyntaxKind::ENSURES_KW,
        TokenKind::Invariant => SyntaxKind::INVARIANT_KW,
        TokenKind::Decreases => SyntaxKind::DECREASES_KW,
        TokenKind::Result => SyntaxKind::RESULT_KW,
        TokenKind::View => SyntaxKind::VIEW_KW,
        TokenKind::ActivePattern => SyntaxKind::PATTERN_KW,
        TokenKind::With => SyntaxKind::WITH_KW,
        TokenKind::Unknown => SyntaxKind::UNKNOWN_KW,
        TokenKind::Typeof => SyntaxKind::TYPEOF_KW,

        // Proof Keywords
        TokenKind::Theorem => SyntaxKind::THEOREM_KW,
        TokenKind::Axiom => SyntaxKind::AXIOM_KW,
        TokenKind::Lemma => SyntaxKind::LEMMA_KW,
        TokenKind::Corollary => SyntaxKind::COROLLARY_KW,
        TokenKind::Proof => SyntaxKind::PROOF_KW,
        TokenKind::Calc => SyntaxKind::CALC_KW,
        TokenKind::Have => SyntaxKind::HAVE_KW,
        TokenKind::Show => SyntaxKind::SHOW_KW,
        TokenKind::Suffices => SyntaxKind::SUFFICES_KW,
        TokenKind::Obtain => SyntaxKind::OBTAIN_KW,
        TokenKind::By => SyntaxKind::BY_KW,
        TokenKind::Induction => SyntaxKind::INDUCTION_KW,
        TokenKind::Cases => SyntaxKind::CASES_KW,
        TokenKind::Contradiction => SyntaxKind::CONTRADICTION_KW,
        TokenKind::Trivial => SyntaxKind::TRIVIAL_KW,
        TokenKind::Assumption => SyntaxKind::ASSUMPTION_KW,
        TokenKind::Simp => SyntaxKind::SIMP_KW,
        TokenKind::Ring => SyntaxKind::RING_KW,
        TokenKind::Field => SyntaxKind::FIELD_KW,
        TokenKind::Omega => SyntaxKind::OMEGA_KW,
        TokenKind::Auto => SyntaxKind::AUTO_KW,
        TokenKind::Blast => SyntaxKind::BLAST_KW,
        TokenKind::Smt => SyntaxKind::SMT_KW,
        TokenKind::Qed => SyntaxKind::QED_KW,
        TokenKind::Forall => SyntaxKind::FORALL_KW,
        TokenKind::Exists => SyntaxKind::EXISTS_KW,
        TokenKind::Cofix => SyntaxKind::COFIX_KW,
        TokenKind::Implies => SyntaxKind::IMPLIES_KW,

        // Boolean Literals
        TokenKind::True => SyntaxKind::TRUE_KW,
        TokenKind::False => SyntaxKind::FALSE_KW,
        TokenKind::None => SyntaxKind::NONE_KW,
        TokenKind::Some => SyntaxKind::SOME_KW,
        TokenKind::Ok => SyntaxKind::OK_KW,
        TokenKind::Err => SyntaxKind::ERR_KW,

        // Delimiters
        TokenKind::LParen => SyntaxKind::L_PAREN,
        TokenKind::RParen => SyntaxKind::R_PAREN,
        TokenKind::LBracket => SyntaxKind::L_BRACKET,
        TokenKind::RBracket => SyntaxKind::R_BRACKET,
        TokenKind::LBrace => SyntaxKind::L_BRACE,
        TokenKind::RBrace => SyntaxKind::R_BRACE,

        // Punctuation
        TokenKind::Semicolon => SyntaxKind::SEMICOLON,
        TokenKind::Comma => SyntaxKind::COMMA,
        TokenKind::Colon => SyntaxKind::COLON,
        TokenKind::ColonColon => SyntaxKind::COLON_COLON,
        TokenKind::Dot => SyntaxKind::DOT,
        TokenKind::DotDot => SyntaxKind::DOT_DOT,
        TokenKind::DotDotDot => SyntaxKind::DOT_DOT, // Use DOT_DOT as placeholder since there's no DOT_DOT_DOT
        TokenKind::DotDotEq => SyntaxKind::DOT_DOT_EQ,
        TokenKind::At => SyntaxKind::AT,
        TokenKind::Hash => SyntaxKind::HASH,
        TokenKind::Question => SyntaxKind::QUESTION,
        TokenKind::QuestionDot => SyntaxKind::QUESTION_DOT,
        TokenKind::QuestionQuestion => SyntaxKind::QUESTION_QUESTION,
        TokenKind::Dollar => SyntaxKind::IDENT, // Treat as identifier context

        // Operators
        TokenKind::PlusPlus => SyntaxKind::PLUS, // ++ maps to PLUS in legacy parser
        TokenKind::Plus => SyntaxKind::PLUS,
        TokenKind::Minus => SyntaxKind::MINUS,
        TokenKind::Star => SyntaxKind::STAR,
        TokenKind::Slash => SyntaxKind::SLASH,
        TokenKind::Percent => SyntaxKind::PERCENT,
        TokenKind::StarStar => SyntaxKind::STAR_STAR,
        TokenKind::Eq => SyntaxKind::EQ,
        TokenKind::EqEq => SyntaxKind::EQ_EQ,
        TokenKind::BangEq => SyntaxKind::BANG_EQ,
        TokenKind::Lt => SyntaxKind::L_ANGLE,
        TokenKind::Gt => SyntaxKind::R_ANGLE,
        TokenKind::LtEq => SyntaxKind::LT_EQ,
        TokenKind::GtEq => SyntaxKind::GT_EQ,
        TokenKind::AmpersandAmpersand => SyntaxKind::AMP_AMP,
        TokenKind::PipePipe => SyntaxKind::PIPE_PIPE,
        TokenKind::Bang => SyntaxKind::BANG,
        TokenKind::Ampersand => SyntaxKind::AMP,
        TokenKind::Pipe => SyntaxKind::PIPE,
        TokenKind::Caret => SyntaxKind::CARET,
        TokenKind::Tilde => SyntaxKind::TILDE,
        TokenKind::LtLt => SyntaxKind::LT_LT,
        TokenKind::GtGt => SyntaxKind::GT_GT,
        TokenKind::RArrow => SyntaxKind::ARROW,
        TokenKind::FatArrow => SyntaxKind::FAT_ARROW,
        TokenKind::PipeGt => SyntaxKind::PIPE_GT,

        // Compound Assignment
        TokenKind::PlusEq => SyntaxKind::PLUS_EQ,
        TokenKind::MinusEq => SyntaxKind::MINUS_EQ,
        TokenKind::StarEq => SyntaxKind::STAR_EQ,
        TokenKind::SlashEq => SyntaxKind::SLASH_EQ,
        TokenKind::PercentEq => SyntaxKind::PERCENT_EQ,
        TokenKind::AmpersandEq => SyntaxKind::AMP_EQ,
        TokenKind::PipeEq => SyntaxKind::PIPE_EQ,
        TokenKind::CaretEq => SyntaxKind::CARET_EQ,
        TokenKind::LtLtEq => SyntaxKind::LT_LT_EQ,
        TokenKind::GtGtEq => SyntaxKind::GT_GT_EQ,

        // Literals
        TokenKind::Integer(_) => SyntaxKind::INT_LITERAL,
        TokenKind::Float(_) => SyntaxKind::FLOAT_LITERAL,
        TokenKind::Text(_) => SyntaxKind::STRING_LITERAL,
        TokenKind::Char(_) => SyntaxKind::CHAR_LITERAL,
        TokenKind::ByteChar(_) => SyntaxKind::CHAR_LITERAL,
        TokenKind::ByteString(_) => SyntaxKind::BYTE_STRING_LITERAL,
        TokenKind::InterpolatedString(_) => SyntaxKind::INTERPOLATED_STRING,
        TokenKind::TaggedLiteral(_) => SyntaxKind::TAGGED_LITERAL,
        TokenKind::ContractLiteral(_) => SyntaxKind::CONTRACT_LITERAL,
        TokenKind::HexColor(_) => SyntaxKind::HEX_COLOR,

        // Identifiers
        TokenKind::Ident(_) => SyntaxKind::IDENT,
        TokenKind::Lifetime(_) => SyntaxKind::IDENT, // Treat as identifier

        // Comments (when not trivia)
        TokenKind::BlockComment => SyntaxKind::BLOCK_COMMENT,

        // Dependent type keywords (future v2.0+)
        TokenKind::Inductive => SyntaxKind::IDENT,
        TokenKind::Coinductive => SyntaxKind::IDENT,

        // NOTE: PlusPlus is already mapped above (line 2800)

        // Biconditional
        TokenKind::Iff => SyntaxKind::LT_EQ, // <-> maps to LT_EQ in legacy parser

        // Special
        TokenKind::Eof => SyntaxKind::EOF,
        TokenKind::Error => SyntaxKind::ERROR,
        TokenKind::Link => SyntaxKind::ERROR, // Link token treated as error in legacy parser
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lossless_parse_simple() {
        let source = "fn foo() { }";
        let file_id = FileId::new(0);

        let parser = LosslessParser::new();
        let result = parser.parse(source, file_id);

        assert!(result.ok(), "Parse should succeed");
        assert_eq!(result.text(), source, "Should reconstruct source exactly");
    }

    #[test]
    fn test_lossless_parse_with_comments() {
        let source = "// Comment\nfn foo() { /* inline */ }";
        let file_id = FileId::new(0);

        let parser = LosslessParser::new();
        let result = parser.parse(source, file_id);

        assert!(result.ok(), "Parse should succeed");
        assert_eq!(result.text(), source, "Should preserve comments");
    }

    #[test]
    fn test_lossless_parse_with_whitespace() {
        let source = "fn   foo  (   )   {   }";
        let file_id = FileId::new(0);

        let parser = LosslessParser::new();
        let result = parser.parse(source, file_id);

        assert!(result.ok(), "Parse should succeed");
        assert_eq!(result.text(), source, "Should preserve whitespace");
    }

    #[test]
    fn test_incremental_parser() {
        let source = "fn foo() { }";
        let file_id = FileId::new(0);

        let mut parser = IncrementalParser::new();
        let result = parser.parse(source, file_id);

        assert!(result.ok());
        assert!(parser.cached_tree().is_some());

        // Apply edit: change "foo" to "bar"
        let result2 = parser.apply_edit(3, 6, "bar", file_id);
        assert!(result2.ok());
        assert_eq!(result2.text(), "fn bar() { }");
    }

    #[test]
    fn test_syntax_navigation() {
        let source = "fn foo() { let x = 1; }";
        let file_id = FileId::new(0);

        let parser = LosslessParser::new();
        let result = parser.parse(source, file_id);

        let syntax = result.syntax();
        assert_eq!(syntax.kind(), SyntaxKind::SOURCE_FILE);

        // Find first token
        let first_token = syntax.first_token();
        assert!(first_token.is_some());
        assert_eq!(first_token.unwrap().text(), "fn");
    }

    // ========================================================================
    // Event-Based Parser Integration Tests
    // ========================================================================

    #[test]
    fn test_event_based_parser_simple_function() {
        let source = "fn foo() { }";
        let file_id = FileId::new(0);

        let parser = EventBasedParser::new();
        let result = parser.parse(source, file_id);

        // Verify events were emitted
        assert!(result.event_count > 0, "Events should be emitted");

        // Verify green tree was built
        assert_eq!(
            result.green.kind(),
            SyntaxKind::SOURCE_FILE,
            "Root should be SOURCE_FILE"
        );

        // Verify lossless round-trip
        assert_eq!(
            result.text(),
            source,
            "Should reconstruct source exactly via event-based parsing"
        );
    }

    #[test]
    fn test_event_based_parser_with_let() {
        let source = "fn foo() { let x = 1; }";
        let file_id = FileId::new(0);

        let parser = EventBasedParser::new();
        let result = parser.parse(source, file_id);

        assert!(result.ok(), "Parse should succeed without errors");
        assert!(result.event_count > 5, "Should have multiple events");
        assert_eq!(result.text(), source, "Should preserve all text");

        // Navigate the tree to verify structure
        let syntax = result.syntax();
        assert_eq!(syntax.kind(), SyntaxKind::SOURCE_FILE);
    }

    #[test]
    fn test_event_based_parser_with_comments() {
        let source = "// Leading comment\nfn foo() { /* inline */ }";
        let file_id = FileId::new(0);

        let parser = EventBasedParser::new();
        let result = parser.parse(source, file_id);

        // Comments should be preserved in the green tree
        assert_eq!(
            result.text(),
            source,
            "Comments should be preserved in event-based parsing"
        );
    }

    #[test]
    fn test_event_based_parser_with_whitespace() {
        let source = "fn   foo  (   )   {   }";
        let file_id = FileId::new(0);

        let parser = EventBasedParser::new();
        let result = parser.parse(source, file_id);

        // Whitespace should be preserved
        assert_eq!(
            result.text(),
            source,
            "Whitespace should be preserved in event-based parsing"
        );
    }

    #[test]
    fn test_event_based_parser_type_definition() {
        let source = "type Point is { x: Float, y: Float };";
        let file_id = FileId::new(0);

        let parser = EventBasedParser::new();
        let result = parser.parse(source, file_id);

        assert_eq!(result.text(), source, "Type definition should round-trip");
    }

    #[test]
    fn test_event_based_parser_expressions() {
        let source = "fn main() { let x = 1 + 2; }";
        let file_id = FileId::new(0);

        let parser = EventBasedParser::new();
        let result = parser.parse(source, file_id);

        assert_eq!(
            result.text(),
            source,
            "Binary expression should round-trip"
        );
    }

    #[test]
    fn test_event_based_parser_function_with_params() {
        let source = "fn add(a: Int, b: Int) { return a; }";
        let file_id = FileId::new(0);

        let parser = EventBasedParser::new();
        let result = parser.parse(source, file_id);

        assert_eq!(
            result.text(),
            source,
            "Function with parameters should round-trip"
        );
    }

    #[test]
    fn test_event_based_parser_function_with_return_type() {
        let source = "fn foo() -> Int { return 42; }";
        let file_id = FileId::new(0);

        let parser = EventBasedParser::new();
        let result = parser.parse(source, file_id);

        assert_eq!(
            result.text(),
            source,
            "Function with return type should round-trip"
        );
    }

    #[test]
    fn test_event_based_parser_call_expression() {
        let source = "fn main() { print(42); }";
        let file_id = FileId::new(0);

        let parser = EventBasedParser::new();
        let result = parser.parse(source, file_id);

        assert_eq!(result.text(), source, "Call expression should round-trip");
    }

    #[test]
    fn test_event_based_parser_multiple_statements() {
        let source = "fn main() { let x = 1; let y = 2; }";
        let file_id = FileId::new(0);

        let parser = EventBasedParser::new();
        let result = parser.parse(source, file_id);

        assert_eq!(
            result.text(),
            source,
            "Multiple statements should round-trip"
        );
    }

    #[test]
    fn test_event_based_parser_tree_structure() {
        let source = "fn foo() { let x = 1; }";
        let file_id = FileId::new(0);

        let parser = EventBasedParser::new();
        let result = parser.parse(source, file_id);

        let syntax = result.syntax();

        // Verify root is SOURCE_FILE
        assert_eq!(syntax.kind(), SyntaxKind::SOURCE_FILE);

        // Traverse to find FN_DEF
        let mut found_fn_def = false;
        for child in syntax.children() {
            if child.kind() == SyntaxKind::FN_DEF {
                found_fn_def = true;
                break;
            }
        }
        assert!(found_fn_def, "Should find FN_DEF node in syntax tree");
    }

    #[test]
    fn test_event_based_parser_vs_lossless_parser_equivalence() {
        let source = "fn foo() { let x = 1; }";
        let file_id = FileId::new(0);

        // Parse with both parsers
        let lossless_parser = LosslessParser::new();
        let lossless_result = lossless_parser.parse(source, file_id);

        let event_parser = EventBasedParser::new();
        let event_result = event_parser.parse(source, file_id);

        // Both should produce the same text reconstruction
        assert_eq!(
            lossless_result.text(),
            event_result.text(),
            "Both parsers should produce the same text reconstruction"
        );

        // Both should have SOURCE_FILE as root
        assert_eq!(lossless_result.syntax().kind(), event_result.syntax().kind());
    }

    #[test]
    fn test_event_based_parser_empty_function() {
        let source = "fn empty() { }";
        let file_id = FileId::new(0);

        let parser = EventBasedParser::new();
        let result = parser.parse(source, file_id);

        assert_eq!(result.text(), source);
        assert!(result.ok());
    }

    #[test]
    fn test_event_based_parser_nested_blocks() {
        let source = "fn outer() { { let x = 1; } }";
        let file_id = FileId::new(0);

        let parser = EventBasedParser::new();
        let result = parser.parse(source, file_id);

        assert_eq!(result.text(), source, "Nested blocks should round-trip");
    }

    #[test]
    fn test_event_based_parser_complex_source() {
        let source = r#"// Module header
fn add(a: Int, b: Int) -> Int {
    return a + b;
}

fn main() {
    let result = add(1, 2);
}"#;
        let file_id = FileId::new(0);

        let parser = EventBasedParser::new();
        let result = parser.parse(source, file_id);

        assert_eq!(
            result.text(),
            source,
            "Complex source with multiple functions should round-trip"
        );
    }

    #[test]
    fn test_event_count_matches_structure() {
        let source = "fn f() { }";
        let file_id = FileId::new(0);

        let parser = EventBasedParser::new();
        let result = parser.parse(source, file_id);

        // Should have events for:
        // - SOURCE_FILE (start + finish)
        // - FN_DEF (start + finish)
        // - PARAM_LIST (start + finish)
        // - BLOCK (start + finish)
        // - Plus token events
        // This is a basic sanity check
        assert!(result.event_count >= 8, "Should have at least 8 events for minimal function");
    }

}
