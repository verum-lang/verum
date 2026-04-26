//! Event-based parser with comprehensive error recovery.
//!
//! This module provides the RecoveringEventParser which extends the basic
//! EventBasedParser with industrial-grade error recovery capabilities:
//!
//! - Recovery sets for each grammar rule
//! - ERROR node creation for unparseable content
//! - Structured error reporting with context
//! - Recovery statistics for quality metrics
//!
//! # Example
//!
//! ```rust,ignore
//! use verum_parser::recovery_parser::RecoveringEventParser;
//! use verum_ast::FileId;
//!
//! let source = "fn foo( { }"; // Missing ')'
//! let file_id = FileId::new(0);
//!
//! let mut parser = RecoveringEventParser::new();
//! let result = parser.parse(source, file_id);
//!
//! // Parse succeeded with recovery
//! assert!(!result.errors.is_empty());
//! // ERROR node was created for the malformed section
//! assert!(result.event_count > 0);
//! // Can still query the tree
//! assert_eq!(result.text(), source);
//! ```

use verum_ast::FileId;
use verum_common::List;
use verum_lexer::lossless::{LosslessLexer, RichToken, TriviaKind as LexerTriviaKind};
use verum_lexer::TokenKind;
use verum_syntax::{
    EventBuilder, GreenNode, GreenTreeSink, SyntaxKind, SyntaxNode, TokenSource, TriviaSource,
};
use verum_syntax::event::process;

use crate::recovery::{recovery_sets, EventRecovery};

/// Event-based parser with comprehensive error recovery.
#[derive(Debug)]
pub struct RecoveringEventParser {
    /// Recovery context for tracking parse state.
    recovery: EventRecovery,
}

/// Result of parsing with recovery.
#[derive(Debug)]
pub struct RecoveringParse {
    /// The lossless green tree built from events.
    pub green: GreenNode,
    /// Number of events emitted during parsing.
    pub event_count: usize,
    /// Parse errors collected during event processing.
    pub errors: Vec<String>,
    /// Number of successful recoveries performed.
    pub recovery_count: usize,
    /// Total tokens skipped during recovery.
    pub tokens_skipped: usize,
}

impl RecoveringParse {
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

    /// Check if recovery was needed.
    pub fn had_recovery(&self) -> bool {
        self.recovery_count > 0
    }

    /// Get recovery statistics as a summary string.
    pub fn recovery_summary(&self) -> String {
        if self.recovery_count == 0 {
            "No recovery needed".to_string()
        } else {
            format!(
                "Recovered {} times, skipped {} tokens, {} errors",
                self.recovery_count,
                self.tokens_skipped,
                self.errors.len()
            )
        }
    }
}

impl RecoveringEventParser {
    /// Create a new recovering event-based parser.
    pub fn new() -> Self {
        Self {
            recovery: EventRecovery::new(),
        }
    }

    /// Parse source code with error recovery.
    pub fn parse(&mut self, source: &str, file_id: FileId) -> RecoveringParse {
        // Reset recovery state
        self.recovery = EventRecovery::new();

        // Phase 1: Lossless lexing
        let lossless_lexer = LosslessLexer::new(source, file_id);
        let rich_tokens = lossless_lexer.tokenize();

        // Phase 2: Convert tokens
        let token_sources = convert_tokens_to_sources(source, &rich_tokens);

        // Phase 3: Parse with recovery
        let mut event_builder = EventBuilder::new();
        self.parse_with_recovery(source, &rich_tokens, &mut event_builder);

        // Phase 4: Process events
        let events = event_builder.reorder();
        let event_count = events.len();

        let mut sink = GreenTreeSink::new();
        process(events, &token_sources, &mut sink);

        let (green, sink_errors) = sink.finish();
        let mut errors: Vec<String> = sink_errors.iter().map(|e| e.message.clone()).collect();

        // Add recovery errors
        for err in &self.recovery.errors {
            errors.push(format!("{}", err));
        }

        RecoveringParse {
            green,
            event_count,
            errors,
            recovery_count: self.recovery.recovery_count,
            tokens_skipped: self.recovery.total_skipped,
        }
    }

    /// Parse with recovery, creating ERROR nodes for unparseable content.
    fn parse_with_recovery(
        &mut self,
        source: &str,
        tokens: &List<RichToken>,
        builder: &mut EventBuilder,
    ) {
        let root = builder.start();
        let token_slice: Vec<_> = tokens.iter().collect();
        let mut pos = 0;

        self.recovery.push(recovery_sets::ITEM_RECOVERY.clone());

        while pos < token_slice.len() {
            let token = &token_slice[pos];

            if token.token.kind == TokenKind::Eof {
                builder.token(SyntaxKind::EOF);
                pos += 1;
                continue;
            }

            let (new_pos, parsed) =
                self.parse_item_with_recovery(source, &token_slice, pos, builder);
            if parsed {
                pos = new_pos;
            } else {
                // Recovery: create ERROR node and skip to next item
                let error_marker = builder.start();
                let kind = token_kind_to_syntax_kind(&token.token.kind);
                builder.token(kind);
                pos += 1;

                let mut skipped = 1;
                while pos < token_slice.len() {
                    let tk = &token_slice[pos];
                    if matches!(
                        tk.token.kind,
                        TokenKind::Fn
                            | TokenKind::Type
                            | TokenKind::Protocol
                            | TokenKind::Implement
                            | TokenKind::Module
                            | TokenKind::Mount
                            | TokenKind::Pub
                            | TokenKind::At
                            | TokenKind::Eof
                    ) {
                        break;
                    }
                    let k = token_kind_to_syntax_kind(&tk.token.kind);
                    builder.token(k);
                    pos += 1;
                    skipped += 1;
                }

                error_marker.complete(builder, SyntaxKind::ERROR);
                builder.error(format!(
                    "unexpected token, expected item (skipped {} tokens)",
                    skipped
                ));
                self.recovery.record_recovery(skipped);
            }
        }

        self.recovery.pop();
        root.complete(builder, SyntaxKind::SOURCE_FILE);
    }

    fn parse_item_with_recovery(
        &mut self,
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
            TokenKind::Fn => self.parse_fn_with_recovery(source, tokens, start_pos, builder),
            TokenKind::Type => self.parse_type_def_with_recovery(tokens, start_pos, builder),
            TokenKind::Let => self.parse_let_with_recovery(source, tokens, start_pos, builder),
            TokenKind::Pub | TokenKind::At => {
                let mut pos = start_pos;
                if matches!(token.token.kind, TokenKind::Pub) {
                    builder.token(SyntaxKind::PUB_KW);
                    pos += 1;
                } else if matches!(token.token.kind, TokenKind::At) {
                    let (new_pos, _) = self.parse_attribute(tokens, pos, builder);
                    pos = new_pos;
                }
                if pos < tokens.len() {
                    self.parse_item_with_recovery(source, tokens, pos, builder)
                } else {
                    (pos, false)
                }
            }
            _ => (start_pos, false),
        }
    }

    fn parse_fn_with_recovery(
        &mut self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;
        let fn_marker = builder.start();

        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::Fn) {
            fn_marker.abandon(builder);
            return (start_pos, false);
        }
        builder.token(SyntaxKind::FN_KW);
        pos += 1;

        if pos >= tokens.len() {
            builder.error("expected function name after 'fn'");
            fn_marker.complete(builder, SyntaxKind::FN_DEF);
            return (pos, true);
        }

        if matches!(tokens[pos].token.kind, TokenKind::Ident(_)) {
            builder.token(SyntaxKind::IDENT);
            pos += 1;
        } else {
            let error_marker = builder.start();
            builder.error("expected function name");
            error_marker.complete(builder, SyntaxKind::ERROR);
            while pos < tokens.len() {
                if matches!(
                    tokens[pos].token.kind,
                    TokenKind::LParen | TokenKind::Fn | TokenKind::Type | TokenKind::Eof
                ) {
                    break;
                }
                let k = token_kind_to_syntax_kind(&tokens[pos].token.kind);
                builder.token(k);
                pos += 1;
            }
            self.recovery.record_recovery(1);
        }

        self.recovery.push(recovery_sets::PARAM_RECOVERY.clone());
        let (new_pos, _) = self.parse_param_list_with_recovery(source, tokens, pos, builder);
        pos = new_pos;
        self.recovery.pop();

        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::RArrow) {
            builder.token(SyntaxKind::ARROW);
            pos += 1;
            self.recovery.push(recovery_sets::TYPE_RECOVERY.clone());
            let (new_pos, _) = self.parse_type_expr_with_recovery(tokens, pos, builder);
            pos = new_pos;
            self.recovery.pop();
        }

        self.recovery.push(recovery_sets::BLOCK_RECOVERY.clone());
        let (new_pos, _) = self.parse_block_with_recovery(source, tokens, pos, builder);
        pos = new_pos;
        self.recovery.pop();

        fn_marker.complete(builder, SyntaxKind::FN_DEF);
        (pos, true)
    }

    fn parse_param_list_with_recovery(
        &mut self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;
        let params_marker = builder.start();

        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::LParen) {
            builder.error("expected '(' for parameter list");
            params_marker.complete(builder, SyntaxKind::PARAM_LIST);
            return (pos, true);
        }
        builder.token(SyntaxKind::L_PAREN);
        pos += 1;

        while pos < tokens.len() && !matches!(tokens[pos].token.kind, TokenKind::RParen) {
            if matches!(tokens[pos].token.kind, TokenKind::Eof) {
                builder.error("unexpected end of file in parameter list");
                break;
            }

            let (new_pos, parsed) = self.parse_param_with_recovery(source, tokens, pos, builder);
            if parsed {
                pos = new_pos;
                if pos < tokens.len() {
                    if matches!(tokens[pos].token.kind, TokenKind::Comma) {
                        builder.token(SyntaxKind::COMMA);
                        pos += 1;
                    } else if !matches!(tokens[pos].token.kind, TokenKind::RParen) {
                        builder.error("expected ',' or ')' after parameter");
                    }
                }
            } else {
                let error_marker = builder.start();
                let mut skipped = 0;
                while pos < tokens.len() {
                    if matches!(
                        tokens[pos].token.kind,
                        TokenKind::Comma | TokenKind::RParen | TokenKind::Eof
                    ) {
                        break;
                    }
                    let k = token_kind_to_syntax_kind(&tokens[pos].token.kind);
                    builder.token(k);
                    pos += 1;
                    skipped += 1;
                }
                if skipped > 0 {
                    error_marker.complete(builder, SyntaxKind::ERROR);
                    builder.error(format!("invalid parameter (skipped {} tokens)", skipped));
                    self.recovery.record_recovery(skipped);
                } else {
                    error_marker.abandon(builder);
                }
                if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Comma) {
                    builder.token(SyntaxKind::COMMA);
                    pos += 1;
                }
            }
        }

        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::RParen) {
            builder.token(SyntaxKind::R_PAREN);
            pos += 1;
        } else {
            builder.error("expected ')' to close parameter list");
        }

        params_marker.complete(builder, SyntaxKind::PARAM_LIST);
        (pos, true)
    }

    fn parse_param_with_recovery(
        &mut self,
        _source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;

        if pos >= tokens.len()
            || matches!(tokens[pos].token.kind, TokenKind::RParen | TokenKind::Eof)
        {
            return (pos, false);
        }

        let param_marker = builder.start();

        if !matches!(tokens[pos].token.kind, TokenKind::Ident(_)) {
            param_marker.abandon(builder);
            return (start_pos, false);
        }
        builder.token(SyntaxKind::IDENT);
        pos += 1;

        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Colon) {
            builder.token(SyntaxKind::COLON);
            pos += 1;
            let (new_pos, _) = self.parse_type_expr_with_recovery(tokens, pos, builder);
            pos = new_pos;
        }

        param_marker.complete(builder, SyntaxKind::PARAM);
        (pos, true)
    }

    fn parse_type_expr_with_recovery(
        &mut self,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;

        if pos >= tokens.len() {
            return (pos, false);
        }

        let type_marker = builder.start();

        match &tokens[pos].token.kind {
            TokenKind::Ident(_) => {
                builder.token(SyntaxKind::IDENT);
                pos += 1;

                if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Lt) {
                    builder.token(SyntaxKind::L_ANGLE);
                    pos += 1;

                    let mut depth = 1;
                    while pos < tokens.len() && depth > 0 {
                        match &tokens[pos].token.kind {
                            TokenKind::Lt => {
                                depth += 1;
                                builder.token(SyntaxKind::L_ANGLE);
                            }
                            TokenKind::Gt => {
                                depth -= 1;
                                builder.token(SyntaxKind::R_ANGLE);
                            }
                            _ => {
                                let k = token_kind_to_syntax_kind(&tokens[pos].token.kind);
                                builder.token(k);
                            }
                        }
                        pos += 1;
                    }
                }

                type_marker.complete(builder, SyntaxKind::PATH_TYPE);
                (pos, true)
            }
            _ => {
                builder.error("expected type");
                type_marker.complete(builder, SyntaxKind::ERROR);
                (pos, true)
            }
        }
    }

    fn parse_block_with_recovery(
        &mut self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;
        let block_marker = builder.start();

        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::LBrace) {
            builder.error("expected '{' for block");
            block_marker.complete(builder, SyntaxKind::BLOCK);
            return (pos, true);
        }
        builder.token(SyntaxKind::L_BRACE);
        pos += 1;

        self.recovery.push(recovery_sets::STMT_RECOVERY.clone());

        while pos < tokens.len() && !matches!(tokens[pos].token.kind, TokenKind::RBrace) {
            if matches!(tokens[pos].token.kind, TokenKind::Eof) {
                builder.error("unexpected end of file in block");
                break;
            }

            let (new_pos, parsed) = self.parse_stmt_with_recovery(source, tokens, pos, builder);
            if parsed {
                pos = new_pos;
            } else {
                let error_marker = builder.start();
                let mut skipped = 0;
                while pos < tokens.len() {
                    if matches!(
                        tokens[pos].token.kind,
                        TokenKind::Semicolon
                            | TokenKind::RBrace
                            | TokenKind::Let
                            | TokenKind::Return
                            | TokenKind::If
                            | TokenKind::While
                            | TokenKind::For
                            | TokenKind::Eof
                    ) {
                        break;
                    }
                    let k = token_kind_to_syntax_kind(&tokens[pos].token.kind);
                    builder.token(k);
                    pos += 1;
                    skipped += 1;
                }

                if skipped > 0 {
                    error_marker.complete(builder, SyntaxKind::ERROR);
                    builder.error(format!("invalid statement (skipped {} tokens)", skipped));
                    self.recovery.record_recovery(skipped);
                } else {
                    error_marker.abandon(builder);
                }

                if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Semicolon) {
                    builder.token(SyntaxKind::SEMICOLON);
                    pos += 1;
                }
            }
        }

        self.recovery.pop();

        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::RBrace) {
            builder.token(SyntaxKind::R_BRACE);
            pos += 1;
        } else {
            builder.error("expected '}' to close block");
        }

        block_marker.complete(builder, SyntaxKind::BLOCK);
        (pos, true)
    }

    fn parse_stmt_with_recovery(
        &mut self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        if start_pos >= tokens.len() {
            return (start_pos, false);
        }

        match &tokens[start_pos].token.kind {
            TokenKind::Let => self.parse_let_with_recovery(source, tokens, start_pos, builder),
            TokenKind::Return => {
                let mut pos = start_pos;
                let stmt_marker = builder.start();

                builder.token(SyntaxKind::RETURN_KW);
                pos += 1;

                if pos < tokens.len()
                    && !matches!(
                        tokens[pos].token.kind,
                        TokenKind::Semicolon | TokenKind::RBrace
                    )
                {
                    let (new_pos, _) = self.parse_expr_with_recovery(source, tokens, pos, builder);
                    pos = new_pos;
                }

                if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Semicolon) {
                    builder.token(SyntaxKind::SEMICOLON);
                    pos += 1;
                }

                stmt_marker.complete(builder, SyntaxKind::RETURN_STMT);
                (pos, true)
            }
            _ => {
                let (new_pos, parsed) =
                    self.parse_expr_with_recovery(source, tokens, start_pos, builder);
                if parsed {
                    if new_pos < tokens.len()
                        && matches!(tokens[new_pos].token.kind, TokenKind::Semicolon)
                    {
                        builder.token(SyntaxKind::SEMICOLON);
                        (new_pos + 1, true)
                    } else {
                        (new_pos, true)
                    }
                } else {
                    (start_pos, false)
                }
            }
        }
    }

    fn parse_let_with_recovery(
        &mut self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;
        let let_marker = builder.start();

        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::Let) {
            let_marker.abandon(builder);
            return (start_pos, false);
        }
        builder.token(SyntaxKind::LET_KW);
        pos += 1;

        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::Ident(_)) {
            builder.error("expected variable name after 'let'");
            let_marker.complete(builder, SyntaxKind::LET_STMT);
            return (pos, true);
        }
        builder.token(SyntaxKind::IDENT);
        pos += 1;

        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Colon) {
            builder.token(SyntaxKind::COLON);
            pos += 1;
            let (new_pos, _) = self.parse_type_expr_with_recovery(tokens, pos, builder);
            pos = new_pos;
        }

        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Eq) {
            builder.token(SyntaxKind::EQ);
            pos += 1;
            let (new_pos, _) = self.parse_expr_with_recovery(source, tokens, pos, builder);
            pos = new_pos;
        }

        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Semicolon) {
            builder.token(SyntaxKind::SEMICOLON);
            pos += 1;
        }

        let_marker.complete(builder, SyntaxKind::LET_STMT);
        (pos, true)
    }

    fn parse_type_def_with_recovery(
        &mut self,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;
        let type_marker = builder.start();

        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::Type) {
            type_marker.abandon(builder);
            return (start_pos, false);
        }
        builder.token(SyntaxKind::TYPE_KW);
        pos += 1;

        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::Ident(_)) {
            builder.error("expected type name after 'type'");
            type_marker.complete(builder, SyntaxKind::TYPE_DEF);
            return (pos, true);
        }
        builder.token(SyntaxKind::IDENT);
        pos += 1;

        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::Is) {
            builder.error("expected 'is' after type name");
            type_marker.complete(builder, SyntaxKind::TYPE_DEF);
            return (pos, true);
        }
        builder.token(SyntaxKind::IS_KW);
        pos += 1;

        while pos < tokens.len() {
            let k = token_kind_to_syntax_kind(&tokens[pos].token.kind);
            builder.token(k);
            if matches!(tokens[pos].token.kind, TokenKind::Semicolon) {
                pos += 1;
                break;
            }
            pos += 1;
        }

        type_marker.complete(builder, SyntaxKind::TYPE_DEF);
        (pos, true)
    }

    fn parse_expr_with_recovery(
        &mut self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let (mut pos, parsed) =
            self.parse_primary_with_recovery(source, tokens, start_pos, builder);
        if !parsed {
            return (start_pos, false);
        }

        while pos < tokens.len() {
            match &tokens[pos].token.kind {
                TokenKind::Plus
                | TokenKind::Minus
                | TokenKind::Star
                | TokenKind::Slash
                | TokenKind::EqEq
                | TokenKind::BangEq
                | TokenKind::Lt
                | TokenKind::Gt
                | TokenKind::LtEq
                | TokenKind::GtEq => {
                    let op_kind = token_kind_to_syntax_kind(&tokens[pos].token.kind);
                    builder.token(op_kind);
                    pos += 1;
                    let (new_pos, _) =
                        self.parse_primary_with_recovery(source, tokens, pos, builder);
                    pos = new_pos;
                }
                _ => break,
            }
        }

        (pos, true)
    }

    fn parse_primary_with_recovery(
        &mut self,
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
            TokenKind::Ident(_) => {
                builder.token(SyntaxKind::IDENT);
                pos += 1;

                if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::LParen) {
                    let (new_pos, _) =
                        self.parse_arg_list_with_recovery(source, tokens, pos, builder);
                    pos = new_pos;
                    expr_marker.complete(builder, SyntaxKind::CALL_EXPR);
                } else {
                    expr_marker.complete(builder, SyntaxKind::PATH_EXPR);
                }
                (pos, true)
            }
            TokenKind::LParen => {
                builder.token(SyntaxKind::L_PAREN);
                pos += 1;

                let (new_pos, _) = self.parse_expr_with_recovery(source, tokens, pos, builder);
                pos = new_pos;

                if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::RParen) {
                    builder.token(SyntaxKind::R_PAREN);
                    pos += 1;
                } else {
                    builder.error("expected ')' to close parenthesized expression");
                }

                expr_marker.complete(builder, SyntaxKind::PAREN_EXPR);
                (pos, true)
            }
            TokenKind::LBrace => {
                expr_marker.abandon(builder);
                self.parse_block_with_recovery(source, tokens, pos, builder)
            }
            _ => {
                expr_marker.abandon(builder);
                (start_pos, false)
            }
        }
    }

    fn parse_arg_list_with_recovery(
        &mut self,
        source: &str,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;
        let args_marker = builder.start();

        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::LParen) {
            args_marker.abandon(builder);
            return (start_pos, false);
        }
        builder.token(SyntaxKind::L_PAREN);
        pos += 1;

        while pos < tokens.len() && !matches!(tokens[pos].token.kind, TokenKind::RParen) {
            if matches!(tokens[pos].token.kind, TokenKind::Eof) {
                builder.error("unexpected end of file in argument list");
                break;
            }

            let (new_pos, parsed) = self.parse_expr_with_recovery(source, tokens, pos, builder);
            if parsed {
                pos = new_pos;
                if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Comma) {
                    builder.token(SyntaxKind::COMMA);
                    pos += 1;
                }
            } else {
                let error_marker = builder.start();
                let mut skipped = 0;
                while pos < tokens.len() {
                    if matches!(
                        tokens[pos].token.kind,
                        TokenKind::Comma | TokenKind::RParen | TokenKind::Eof
                    ) {
                        break;
                    }
                    let k = token_kind_to_syntax_kind(&tokens[pos].token.kind);
                    builder.token(k);
                    pos += 1;
                    skipped += 1;
                }
                if skipped > 0 {
                    error_marker.complete(builder, SyntaxKind::ERROR);
                    builder.error(format!("invalid argument (skipped {} tokens)", skipped));
                    self.recovery.record_recovery(skipped);
                } else {
                    error_marker.abandon(builder);
                }
                if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Comma) {
                    builder.token(SyntaxKind::COMMA);
                    pos += 1;
                }
            }
        }

        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::RParen) {
            builder.token(SyntaxKind::R_PAREN);
            pos += 1;
        } else {
            builder.error("expected ')' to close argument list");
        }

        args_marker.complete(builder, SyntaxKind::ARG_LIST);
        (pos, true)
    }

    fn parse_attribute(
        &mut self,
        tokens: &[&RichToken],
        start_pos: usize,
        builder: &mut EventBuilder,
    ) -> (usize, bool) {
        let mut pos = start_pos;
        let attr_marker = builder.start();

        if pos >= tokens.len() || !matches!(tokens[pos].token.kind, TokenKind::At) {
            attr_marker.abandon(builder);
            return (start_pos, false);
        }
        builder.token(SyntaxKind::AT);
        pos += 1;

        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::Ident(_)) {
            builder.token(SyntaxKind::IDENT);
            pos += 1;
        }

        if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::LParen) {
            builder.token(SyntaxKind::L_PAREN);
            pos += 1;

            while pos < tokens.len() && !matches!(tokens[pos].token.kind, TokenKind::RParen) {
                let k = token_kind_to_syntax_kind(&tokens[pos].token.kind);
                builder.token(k);
                pos += 1;
            }

            if pos < tokens.len() && matches!(tokens[pos].token.kind, TokenKind::RParen) {
                builder.token(SyntaxKind::R_PAREN);
                pos += 1;
            }
        }

        attr_marker.complete(builder, SyntaxKind::ATTRIBUTE);
        (pos, true)
    }
}

impl Default for RecoveringEventParser {
    fn default() -> Self {
        Self::new()
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
        LexerTriviaKind::Shebang => SyntaxKind::SHEBANG,
    }
}

/// Convert token kind to syntax kind.
fn token_kind_to_syntax_kind(kind: &TokenKind) -> SyntaxKind {
    crate::recovery::token_kind_to_syntax_kind(kind)
}

/// Helper function to convert tokens to TokenSource format.
fn convert_tokens_to_sources(source: &str, tokens: &List<RichToken>) -> Vec<TokenSource> {
    let mut sources = Vec::new();

    for rich_token in tokens.iter() {
        let leading: Vec<TriviaSource> = rich_token
            .leading_trivia
            .items
            .iter()
            .map(|item| TriviaSource {
                kind: trivia_kind_to_syntax_kind(item.kind),
                text: item.text.clone(),
            })
            .collect();

        let trailing: Vec<TriviaSource> = rich_token
            .trailing_trivia
            .items
            .iter()
            .map(|item| TriviaSource {
                kind: trivia_kind_to_syntax_kind(item.kind),
                text: item.text.clone(),
            })
            .collect();

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

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Basic Valid Code Tests
    // ========================================================================

    #[test]
    fn test_recovering_parser_valid_code() {
        let source = "fn foo() { let x = 1; }";
        let file_id = FileId::new(0);

        let mut parser = RecoveringEventParser::new();
        let result = parser.parse(source, file_id);

        assert!(result.ok(), "Valid code should parse without errors");
        assert!(!result.had_recovery(), "Valid code should not need recovery");
        assert_eq!(result.text(), source, "Should reconstruct source exactly");
    }

    #[test]
    fn test_valid_function_with_params() {
        let source = "fn add(a: Int, b: Int) -> Int { return a + b; }";
        let file_id = FileId::new(0);

        let mut parser = RecoveringEventParser::new();
        let result = parser.parse(source, file_id);

        assert!(result.ok(), "Function with params should parse");
        assert_eq!(result.text(), source);
    }

    #[test]
    fn test_valid_type_definition() {
        let source = "type Point is { x: Float, y: Float };";
        let file_id = FileId::new(0);

        let mut parser = RecoveringEventParser::new();
        let result = parser.parse(source, file_id);

        assert_eq!(result.text(), source, "Type definition should round-trip");
    }

    // ========================================================================
    // Missing Delimiter Recovery Tests
    // ========================================================================

    #[test]
    fn test_recovering_parser_missing_paren() {
        let source = "fn foo( { }";
        let file_id = FileId::new(0);

        let mut parser = RecoveringEventParser::new();
        let result = parser.parse(source, file_id);

        // Should have errors but still produce a tree
        assert!(!result.ok(), "Invalid code should have errors");
        assert!(result.event_count > 0, "Should still produce events");
        assert_eq!(result.text(), source, "Should preserve source text");
    }

    #[test]
    fn test_missing_closing_brace() {
        let source = "fn foo() { let x = 1;";
        let file_id = FileId::new(0);

        let mut parser = RecoveringEventParser::new();
        let result = parser.parse(source, file_id);

        // Should report error but preserve text
        assert!(!result.ok(), "Missing brace should be an error");
        assert_eq!(result.text(), source, "Should preserve source text");
    }

    #[test]
    fn test_missing_opening_brace() {
        let source = "fn foo() let x = 1; }";
        let file_id = FileId::new(0);

        let mut parser = RecoveringEventParser::new();
        let result = parser.parse(source, file_id);

        assert!(!result.ok(), "Missing brace should be an error");
        assert_eq!(result.text(), source, "Should preserve source text");
    }

    // ========================================================================
    // Statement Recovery Tests
    // ========================================================================

    #[test]
    fn test_recovering_parser_missing_semicolon() {
        let source = "fn foo() { let x = 1 }";
        let file_id = FileId::new(0);

        let mut parser = RecoveringEventParser::new();
        let result = parser.parse(source, file_id);

        // Missing semicolon is often tolerated
        assert_eq!(result.text(), source, "Should preserve source text");
    }

    #[test]
    fn test_recovering_parser_garbage_tokens() {
        // Use numeric literals at top level which are invalid items
        let source = "1 2 3 fn foo() { }";
        let file_id = FileId::new(0);

        let mut parser = RecoveringEventParser::new();
        let result = parser.parse(source, file_id);

        // Should recover and still parse the function
        assert!(result.event_count > 0, "Should still produce events");
        assert_eq!(result.text(), source, "Should preserve all tokens");
    }

    #[test]
    fn test_recovering_parser_type_def() {
        let source = "type Point is { x: Float, y: Float };";
        let file_id = FileId::new(0);

        let mut parser = RecoveringEventParser::new();
        let result = parser.parse(source, file_id);

        assert_eq!(result.text(), source, "Type definition should round-trip");
    }

    #[test]
    fn test_recovering_parser_multiple_functions() {
        let source = "fn foo() { } fn bar() { }";
        let file_id = FileId::new(0);

        let mut parser = RecoveringEventParser::new();
        let result = parser.parse(source, file_id);

        assert!(result.ok(), "Multiple functions should parse");
        assert_eq!(result.text(), source, "Should preserve source text");
    }

    #[test]
    fn test_recovery_summary() {
        let source = "fn foo() { let x = 1; }";
        let file_id = FileId::new(0);

        let mut parser = RecoveringEventParser::new();
        let result = parser.parse(source, file_id);

        assert_eq!(result.recovery_summary(), "No recovery needed");
    }

    // ========================================================================
    // Error Recovery with Continuation Tests
    // ========================================================================

    #[test]
    fn test_recovery_continues_after_error() {
        // First function is malformed, second should still parse
        let source = "fn foo( fn bar() { }";
        let file_id = FileId::new(0);

        let mut parser = RecoveringEventParser::new();
        let result = parser.parse(source, file_id);

        assert!(!result.ok(), "Should have errors");
        assert_eq!(result.text(), source, "Should preserve all text");
        // The parser should create ERROR nodes but continue parsing
        assert!(result.event_count > 0, "Should produce events");
    }

    #[test]
    fn test_recovery_multiple_errors() {
        // Multiple errors in one source
        let source = "fn () { } fn bar( { }";
        let file_id = FileId::new(0);

        let mut parser = RecoveringEventParser::new();
        let result = parser.parse(source, file_id);

        assert!(!result.ok(), "Should have errors");
        assert_eq!(result.text(), source, "Should preserve all text");
        // Should report multiple errors
        assert!(!result.errors.is_empty(), "Should have at least one error");
    }

    #[test]
    fn test_recovery_nested_errors() {
        // Error inside a block
        let source = "fn foo() { let = ; let y = 2; }";
        let file_id = FileId::new(0);

        let mut parser = RecoveringEventParser::new();
        let result = parser.parse(source, file_id);

        assert!(!result.ok(), "Should have errors");
        assert_eq!(result.text(), source, "Should preserve all text");
    }

    // ========================================================================
    // Expression Recovery Tests
    // ========================================================================

    #[test]
    fn test_recovery_in_expression() {
        let source = "fn foo() { let x = 1 + + 2; }";
        let file_id = FileId::new(0);

        let mut parser = RecoveringEventParser::new();
        let result = parser.parse(source, file_id);

        assert_eq!(result.text(), source, "Should preserve source text");
    }

    #[test]
    fn test_recovery_unclosed_paren_in_expr() {
        let source = "fn foo() { let x = (1 + 2; }";
        let file_id = FileId::new(0);

        let mut parser = RecoveringEventParser::new();
        let result = parser.parse(source, file_id);

        assert!(!result.ok(), "Unclosed paren should be an error");
        assert_eq!(result.text(), source, "Should preserve source text");
    }

    // ========================================================================
    // Recovery Statistics Tests
    // ========================================================================

    #[test]
    fn test_recovery_statistics_tracked() {
        // Use garbage tokens that are not valid at item position
        let source = "1 2 3 fn foo() { }";
        let file_id = FileId::new(0);

        let mut parser = RecoveringEventParser::new();
        let result = parser.parse(source, file_id);

        // Should have performed recovery for the numeric literals
        assert!(result.had_recovery(), "Should have performed recovery");
        assert!(result.tokens_skipped > 0, "Should have skipped tokens");
        assert!(result.recovery_count > 0, "Should count recoveries");
    }

    #[test]
    fn test_recovery_summary_format() {
        // Use garbage tokens that trigger recovery
        let source = "1 2 3 fn foo() { }";
        let file_id = FileId::new(0);

        let mut parser = RecoveringEventParser::new();
        let result = parser.parse(source, file_id);

        let summary = result.recovery_summary();
        assert!(
            summary.contains("Recovered"),
            "Summary should mention recovery: {}",
            summary
        );
    }

    // ========================================================================
    // Lossless Preservation Tests
    // ========================================================================

    #[test]
    fn test_lossless_with_comments() {
        let source = "// comment\nfn foo() { /* inline */ }";
        let file_id = FileId::new(0);

        let mut parser = RecoveringEventParser::new();
        let result = parser.parse(source, file_id);

        assert_eq!(result.text(), source, "Comments should be preserved");
    }

    #[test]
    fn test_lossless_with_whitespace() {
        let source = "fn   foo  (  )   {   }";
        let file_id = FileId::new(0);

        let mut parser = RecoveringEventParser::new();
        let result = parser.parse(source, file_id);

        assert_eq!(result.text(), source, "Whitespace should be preserved");
    }

    #[test]
    fn test_lossless_error_recovery_preserves_text() {
        // Even with errors, all text should be preserved
        let source = "fn foo( 1 2 { }";
        let file_id = FileId::new(0);

        let mut parser = RecoveringEventParser::new();
        let result = parser.parse(source, file_id);

        assert_eq!(
            result.text(),
            source,
            "Error recovery should preserve all text"
        );
    }

    // ========================================================================
    // Syntax Tree Structure Tests
    // ========================================================================

    #[test]
    fn test_syntax_tree_has_error_nodes() {
        // Use garbage tokens that trigger ERROR node creation
        let source = "1 2 3 fn foo() { }";
        let file_id = FileId::new(0);

        let mut parser = RecoveringEventParser::new();
        let result = parser.parse(source, file_id);

        let syntax = result.syntax();
        assert_eq!(syntax.kind(), SyntaxKind::SOURCE_FILE);

        // Find ERROR nodes in the tree
        let mut has_error = false;
        for child in syntax.children() {
            if child.kind() == SyntaxKind::ERROR {
                has_error = true;
                break;
            }
        }
        assert!(has_error, "Should have ERROR node in syntax tree");
    }

    #[test]
    fn test_syntax_tree_has_fn_def_after_error() {
        // Use garbage tokens followed by valid function
        let source = "1 2 3 fn foo() { }";
        let file_id = FileId::new(0);

        let mut parser = RecoveringEventParser::new();
        let result = parser.parse(source, file_id);

        let syntax = result.syntax();

        // Should still find FN_DEF after the error
        let mut has_fn_def = false;
        for child in syntax.children() {
            if child.kind() == SyntaxKind::FN_DEF {
                has_fn_def = true;
                break;
            }
        }
        assert!(has_fn_def, "Should have FN_DEF node after recovery");
    }
}
