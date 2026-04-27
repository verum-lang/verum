//! Hand-written recursive descent parser infrastructure.
//!
//! This module provides the core infrastructure for a recursive descent parser,
//! enabling significantly faster compile times (~3 seconds).
//!
//! # Architecture
//!
//! - [`TokenStream`]: Wrapper around token slice with lookahead and position tracking
//! - [`Parser`]: Main parser struct with helper methods for common patterns
//! - [`ParseResult`]: Type alias for parsing results
//!
//! # Design Principles
//!
//! 1. **Zero-cost abstractions**: No heap allocations in hot paths
//! 2. **Error recovery**: Continue parsing after errors for better IDE support
//! 3. **Lookahead**: Support arbitrary lookahead for disambiguation
//! 4. **Span tracking**: Precise source locations for all AST nodes

use crate::attr_validation::{AttributeValidationWarning, AttributeValidator, ValidationConfig};
use crate::error::{ParseError, ParseErrorKind};
use verum_ast::attr::AttributeTarget;
use verum_ast::{FileId, Span};
use verum_common::{List, Text};
use verum_lexer::{Token, TokenKind};

/// Result type for parsing operations (single error).
pub type ParseResult<T> = Result<T, ParseError>;

/// A stream of tokens with position tracking and lookahead.
///
/// This struct wraps a token slice and provides methods for consuming tokens,
/// peeking ahead, and tracking the current position.
#[derive(Debug, Clone)]
pub struct TokenStream<'a> {
    /// The tokens to parse
    tokens: &'a [Token],
    /// Current position in the token stream
    pos: usize,
    /// The file ID for this token stream, used when we need a span but have no tokens
    file_id: FileId,
}

impl<'a> TokenStream<'a> {
    /// Create a new token stream from a slice of tokens.
    ///
    /// The file_id is extracted from the first token if available,
    /// otherwise uses a zero file ID.
    pub fn new(tokens: &'a [Token]) -> Self {
        let file_id = tokens
            .first()
            .map(|t| t.span.file_id)
            .unwrap_or_else(|| FileId::new(0));
        Self {
            tokens,
            pos: 0,
            file_id,
        }
    }

    /// Create a new token stream with an explicit file ID.
    ///
    /// Use this when you need to ensure a specific file ID is used,
    /// especially for empty token streams.
    pub fn with_file_id(tokens: &'a [Token], file_id: FileId) -> Self {
        Self {
            tokens,
            pos: 0,
            file_id,
        }
    }

    /// Get the current token without consuming it.
    #[inline]
    pub fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    /// Get the kind of the current token without consuming it.
    #[inline]
    pub fn peek_kind(&self) -> Option<&TokenKind> {
        self.peek().map(|t| &t.kind)
    }

    /// Look ahead n tokens without consuming them.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let next = stream.peek_nth(1); // Look at next token
    /// let after_next = stream.peek_nth(2); // Look two tokens ahead
    /// ```
    #[inline]
    pub fn peek_nth(&self, n: usize) -> Option<&Token> {
        self.tokens.get(self.pos + n)
    }

    /// Get the kind of the token n positions ahead.
    #[inline]
    pub fn peek_nth_kind(&self, n: usize) -> Option<&TokenKind> {
        self.peek_nth(n).map(|t| &t.kind)
    }

    /// Advance to the next token and return the current one.
    #[inline]
    pub fn advance(&mut self) -> Option<&Token> {
        let token = self.tokens.get(self.pos);
        if token.is_some() {
            self.pos += 1;
        }
        token
    }

    /// Check if the current token matches the given kind without consuming it.
    #[inline]
    pub fn check(&self, kind: &TokenKind) -> bool {
        self.peek_kind() == Some(kind)
    }

    /// Check if the current token matches any of the given kinds.
    #[inline]
    pub fn check_any(&self, kinds: &[TokenKind]) -> bool {
        self.peek_kind().is_some_and(|k| kinds.contains(k))
    }

    /// Consume the current token if it matches the given kind.
    ///
    /// Returns the consumed token, or None if no match.
    #[inline]
    pub fn consume(&mut self, kind: &TokenKind) -> Option<&Token> {
        if self.check(kind) {
            self.advance()
        } else {
            None
        }
    }

    /// Expect the current token to match the given kind and consume it.
    ///
    /// Returns an error if the token doesn't match.
    pub fn expect(&mut self, kind: TokenKind) -> ParseResult<&Token> {
        if self.check(&kind) {
            // SAFETY: check() verifies peek() is Some, so advance() will return Some
            let span = self.last_span();
            self.advance()
                .ok_or_else(|| ParseError::unexpected_eof(std::slice::from_ref(&kind), span))
        } else {
            let found = self.peek();
            let span = found.map(|t| t.span).unwrap_or_else(|| self.last_span());

            if let Some(token) = found {
                Err(ParseError::unexpected(std::slice::from_ref(&kind), token.clone()))
            } else {
                Err(ParseError::unexpected_eof(&[kind], span))
            }
        }
    }

    /// Check if we've reached the end of the token stream.
    #[inline]
    pub fn at_end(&self) -> bool {
        self.pos >= self.tokens.len() || self.check(&TokenKind::Eof)
    }

    /// Get the span of the current token, or a zero-width span at the end.
    pub fn current_span(&self) -> Span {
        self.peek()
            .map(|t| t.span)
            .unwrap_or_else(|| self.last_span())
    }

    /// Get a span representing the last position in the stream.
    ///
    /// Returns a zero-width span at the end of the last token, or at position 0
    /// if there are no tokens. Always uses the stored file_id to ensure valid spans.
    fn last_span(&self) -> Span {
        if let Some(last) = self.tokens.last() {
            Span::new(last.span.end, last.span.end, last.span.file_id)
        } else {
            // Empty token stream - return a zero-width span at position 0
            // with the file_id we were initialized with
            Span::new(0, 0, self.file_id)
        }
    }

    /// Create a span from a start position to the current position.
    ///
    /// This is useful for tracking the span of a parsed construct.
    pub fn make_span(&self, start_pos: usize) -> Span {
        if let (Some(start_token), Some(end_token)) = (
            self.tokens.get(start_pos),
            self.tokens.get(self.pos.saturating_sub(1)),
        ) {
            Span::new(
                start_token.span.start,
                end_token.span.end,
                start_token.span.file_id,
            )
        } else {
            self.current_span()
        }
    }

    /// Get the current position in the token stream.
    #[inline]
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Reset the position to a previous state.
    ///
    /// This is useful for backtracking in the parser.
    #[inline]
    pub fn reset_to(&mut self, pos: usize) {
        self.pos = pos;
    }

    /// Get the remaining tokens as a slice from the current position.
    ///
    /// This is useful for accessing remaining tokens during parsing.
    #[inline]
    pub fn remaining(&self) -> &'a [Token] {
        &self.tokens[self.pos..]
    }

    /// Get the full token slice.
    ///
    /// This is useful for accessing arbitrary token ranges during parsing.
    #[inline]
    pub fn all_tokens(&self) -> &'a [Token] {
        self.tokens
    }
}

/// Maximum number of parse errors to accumulate before stopping error collection
/// This prevents memory exhaustion on pathological inputs with many errors
const MAX_PARSE_ERRORS: usize = 1000;

/// Delimiter context for error recovery.
#[derive(Debug, Clone)]
struct DelimiterContext {
    /// The kind of delimiter
    kind: TokenKind,
    /// Span where the delimiter was opened
    span: Span,
}

/// Safety limit for total parser operations to prevent infinite loops.
/// Even very large files (100K LOC) shouldn't need more than 1M operations.
const MAX_PARSER_OPERATIONS: usize = 1_000_000;

/// Maximum recursion depth for nested expressions, types, and patterns.
/// Prevents stack overflow on deeply nested input like `(((((...)))))`
/// or `List<List<List<...>>>`.
///
/// Sized for rayon worker threads (macOS default stack = 512 KiB; Linux
/// glibc default = 8 MiB but Rust threads often sized to 2 MiB). Each
/// `parse_expr_bp` frame consumes ~2-3 KiB on aarch64 release builds
/// once locals + ParseResult + saved registers are accounted for; 128
/// frames × ~3 KiB ≈ 384 KiB, fitting inside 512 KiB with margin for
/// the calling stack. Real program ASTs rarely exceed depth 30; 128 is
/// more than any *human-written* program needs while staying safe on
/// the smallest worker-thread stack we ship to.
///
/// Tuning history: the bound was 256 before T0.5; the resulting
/// 768 KiB worst-case frame chain reliably hit SIGBUS on macOS
/// rayon workers during stdlib loading. Halving to 128 closes the
/// crash class.
const MAX_RECURSION_DEPTH: usize = 128;

/// Main recursive descent parser with helper methods for common parsing patterns.
///
/// This struct maintains the token stream state and provides high-level
/// parsing utilities like comma-separated lists, delimited expressions,
/// and error recovery.
pub struct RecursiveParser<'a> {
    /// The token stream being parsed
    pub stream: TokenStream<'a>,
    /// File ID for span creation
    pub file_id: FileId,
    /// Accumulated errors for error recovery (capped at MAX_PARSE_ERRORS)
    pub errors: Vec<ParseError>,
    /// Whether error limit has been reached (to log warning once)
    errors_capped: bool,
    /// Pending `>` token from splitting `>>` for nested generics
    /// When we encounter `>>` in generic args, we consume it as one `>` and set this to true
    pub pending_gt: bool,
    /// Pending `*` token from splitting `**` for double pointers
    /// When we encounter `**` in types, we consume it as one `*` and set this to true
    pub pending_star: bool,
    /// Pending `&` token from splitting `&&` for double references
    /// When we encounter `&&` in types, we consume it as one `&` and set this to true
    pub pending_ampersand: bool,
    /// Temporary storage for when clauses parsed in @specialize attributes
    /// These are stored here during attribute parsing and retrieved during impl parsing
    pub when_clauses: Vec<verum_ast::ty::WhereClause>,
    /// Brace depth tracking for disambiguating `>` in refinement types within generic args
    /// When parsing `Option<Int{> 0}>`, the `>` inside `{> 0}` should NOT close the generic
    /// Incremented on `{`, decremented on `}`, checked before treating `>` as generic close
    pub brace_depth: usize,
    /// Stack of open delimiters for better error messages
    delimiter_stack: Vec<DelimiterContext>,
    /// Global operation counter to prevent infinite loops (safety mechanism)
    operation_count: usize,
    /// Set to true when operation limit is reached - causes early exit from all parsing
    aborted: bool,
    /// Optional attribute validator for validating attributes during parsing
    pub attr_validator: Option<AttributeValidator>,
    /// Accumulated attribute validation warnings
    pub attr_warnings: Vec<AttributeValidationWarning>,
    /// Comprehension nesting depth: when > 0, range expressions should not
    /// consume `if`, `for`, or `let` tokens as range-end expressions.
    pub comprehension_depth: usize,
    /// Recursion depth counter for nested parse_expr_bp/parse_type/parse_pattern calls.
    /// Prevents stack overflow on deeply nested input.
    recursion_depth: usize,
    /// When true, `$ident` splice expressions are allowed (inside meta rule bodies).
    pub in_meta_body: bool,
}

impl<'a> RecursiveParser<'a> {
    /// Create a new parser from a token slice.
    pub fn new(tokens: &'a [Token], file_id: FileId) -> Self {
        Self {
            stream: TokenStream::new(tokens),
            file_id,
            errors: Vec::with_capacity(64), // Pre-allocate reasonable capacity
            errors_capped: false,
            pending_gt: false,
            pending_star: false,
            pending_ampersand: false,
            when_clauses: Vec::new(),
            brace_depth: 0,
            delimiter_stack: Vec::new(),
            operation_count: 0,
            aborted: false,
            attr_validator: None,
            attr_warnings: Vec::new(),
            comprehension_depth: 0,
            recursion_depth: 0,
            in_meta_body: false,
        }
    }

    /// Create a new parser with attribute validation enabled.
    pub fn with_attr_validation(tokens: &'a [Token], file_id: FileId) -> Self {
        let mut parser = Self::new(tokens, file_id);
        parser.attr_validator = Some(AttributeValidator::default());
        parser
    }

    /// Create a new parser with custom attribute validation configuration.
    pub fn with_attr_validation_config(
        tokens: &'a [Token],
        file_id: FileId,
        config: ValidationConfig,
    ) -> Self {
        let mut parser = Self::new(tokens, file_id);
        parser.attr_validator = Some(AttributeValidator::new(config));
        parser
    }

    /// Enable attribute validation with default configuration.
    pub fn enable_attr_validation(&mut self) {
        self.attr_validator = Some(AttributeValidator::default());
    }

    /// Enable attribute validation with custom configuration.
    pub fn enable_attr_validation_with_config(&mut self, config: ValidationConfig) {
        self.attr_validator = Some(AttributeValidator::new(config));
    }

    /// Disable attribute validation.
    pub fn disable_attr_validation(&mut self) {
        self.attr_validator = None;
    }

    /// Check if attribute validation is enabled.
    #[must_use]
    pub fn is_attr_validation_enabled(&self) -> bool {
        self.attr_validator.is_some()
    }

    /// Get attribute validation warnings.
    #[must_use]
    pub fn take_attr_warnings(&mut self) -> Vec<AttributeValidationWarning> {
        std::mem::take(&mut self.attr_warnings)
    }

    /// Validate attributes for a specific target and collect warnings.
    pub fn validate_attrs_for_target(
        &mut self,
        attrs: &[verum_ast::attr::Attribute],
        target: AttributeTarget,
    ) {
        if let Some(validator) = &self.attr_validator {
            let warnings = validator.validate_attrs(attrs, target);
            self.attr_warnings.extend(warnings);
        }
    }

    /// Increment operation counter and check for safety limit.
    /// Call this in any loop or recursive call to prevent infinite loops.
    /// Returns true if safe to continue, false if limit reached.
    #[inline]
    pub fn tick(&mut self) -> bool {
        if self.aborted {
            return false;
        }
        self.operation_count += 1;
        if self.operation_count > MAX_PARSER_OPERATIONS {
            self.aborted = true;
            tracing::error!(
                "Parser safety limit reached ({} operations). Aborting to prevent infinite loop.",
                MAX_PARSER_OPERATIONS
            );
            return false;
        }
        true
    }

    /// Check if the parser has been aborted due to safety limits.
    #[inline]
    pub fn is_aborted(&self) -> bool {
        self.aborted
    }

    /// Enter a recursive parse context (expression, type, or pattern).
    /// Returns `Err` if max recursion depth exceeded, aborting the parser.
    #[inline]
    pub fn enter_recursion(&mut self) -> ParseResult<()> {
        self.recursion_depth += 1;
        if self.recursion_depth > MAX_RECURSION_DEPTH {
            self.aborted = true;
            let span = self.stream.current_span();
            return Err(ParseError::invalid_syntax(
                format!("Maximum nesting depth ({MAX_RECURSION_DEPTH}) exceeded"),
                span,
            ));
        }
        Ok(())
    }

    /// Exit a recursive parse context.
    #[inline]
    pub fn exit_recursion(&mut self) {
        self.recursion_depth = self.recursion_depth.saturating_sub(1);
    }

    /// Push a delimiter onto the stack for error recovery.
    fn push_delimiter(&mut self, kind: TokenKind, span: Span) {
        self.delimiter_stack.push(DelimiterContext { kind, span });
    }

    /// Pop a delimiter from the stack, checking for mismatches.
    /// Returns the opening span if successful, None if the stack was empty.
    fn pop_delimiter(&mut self, expected: TokenKind) -> Option<Span> {
        if let Some(ctx) = self.delimiter_stack.pop() {
            let opening_kind = match ctx.kind {
                TokenKind::LParen => TokenKind::RParen,
                TokenKind::LBracket => TokenKind::RBracket,
                TokenKind::LBrace => TokenKind::RBrace,
                TokenKind::Lt => TokenKind::Gt,
                _ => ctx.kind.clone(),
            };

            if opening_kind != expected {
                // Delimiter mismatch - report error
                let current_span = self.stream.current_span();
                let error = ParseError::new(
                    ParseErrorKind::MismatchedDelimiters {
                        open: ctx.kind.clone(),
                        close: expected,
                    },
                    current_span,
                )
                .with_help(format!(
                    "this delimiter was opened at position {}",
                    ctx.span.start
                ));
                self.error(error);
            }
            Some(ctx.span)
        } else {
            None
        }
    }

    /// Get the expected closing delimiter for the most recent opening.
    fn expected_closing_delimiter(&self) -> Option<TokenKind> {
        self.delimiter_stack.last().map(|ctx| match ctx.kind {
            TokenKind::LParen => TokenKind::RParen,
            TokenKind::LBracket => TokenKind::RBracket,
            TokenKind::LBrace => TokenKind::RBrace,
            TokenKind::Lt => TokenKind::Gt,
            _ => ctx.kind.clone(),
        })
    }

    /// Record an error and continue parsing.
    /// MEMORY SAFETY FIX: Errors are capped at MAX_PARSE_ERRORS to prevent memory exhaustion
    pub fn error(&mut self, error: ParseError) {
        if self.errors.len() < MAX_PARSE_ERRORS {
            self.errors.push(error);
        } else if !self.errors_capped {
            self.errors_capped = true;
            tracing::warn!(
                "Parse error limit reached ({}). Additional errors will not be recorded.",
                MAX_PARSE_ERRORS
            );
        }
    }

    /// Create an error at the current position.
    pub fn error_at_current(&mut self, kind: ParseErrorKind) {
        let span = self.stream.current_span();
        self.error(ParseError::new(kind, span));
    }

    /// Parse a comma-separated list of items.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Parse: item1, item2, item3
    /// let items = parser.comma_separated(|p| p.parse_item())?;
    ///
    /// // With trailing comma: item1, item2,
    /// let items = parser.comma_separated(|p| p.parse_item())?;
    /// ```
    pub fn comma_separated<T>(
        &mut self,
        mut parse_fn: impl FnMut(&mut Self) -> ParseResult<T>,
    ) -> ParseResult<Vec<T>> {
        let mut items = Vec::new();

        // Handle empty list
        if self.stream.at_end() {
            return Ok(items);
        }

        // Parse first item
        items.push(parse_fn(self)?);

        // Parse remaining items
        while self.stream.consume(&TokenKind::Comma).is_some() {
            // Allow trailing comma: check for common closing delimiters
            // Note: Only treat > as delimiter if we're not inside braces (for refinement types)
            // LBrace is added to support trailing comma in where clauses: where T: Clone, { ... }
            if self.stream.at_end()
                || self.stream.check(&TokenKind::RBrace)
                || self.stream.check(&TokenKind::LBrace)
                || self.stream.check(&TokenKind::RParen)
                || self.stream.check(&TokenKind::RBracket)
                || (self.brace_depth == 0 && (self.stream.check(&TokenKind::Gt) || self.pending_gt))
            {
                break;
            }

            // Safety: Track position to ensure forward progress
            let pos_before = self.stream.position();
            items.push(parse_fn(self)?);

            // Ensure we advanced at least one token
            if self.stream.position() == pos_before {
                return Err(ParseError::invalid_syntax(
                    "parser made no progress in comma-separated list",
                    self.stream.current_span(),
                ));
            }
        }

        Ok(items)
    }

    /// Parse content delimited by opening and closing tokens.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Parse: { expr }
    /// let expr = parser.delimited(
    ///     TokenKind::LBrace,
    ///     TokenKind::RBrace,
    ///     |p| p.parse_expr()
    /// )?;
    /// ```
    pub fn delimited<T>(
        &mut self,
        open: TokenKind,
        close: TokenKind,
        mut parse: impl FnMut(&mut Self) -> ParseResult<T>,
    ) -> ParseResult<T> {
        self.stream.expect(open)?;
        let result = parse(self)?;
        self.stream.expect(close)?;
        Ok(result)
    }

    /// Try to parse something, returning None if it fails.
    ///
    /// This does not consume tokens on failure and does not record errors.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Try to parse an optional type annotation
    /// let ty = parser.optional(|p| {
    ///     p.stream.expect(TokenKind::Colon)?;
    ///     p.parse_type()
    /// });
    /// ```
    pub fn optional<T>(&mut self, mut parse: impl FnMut(&mut Self) -> ParseResult<T>) -> Option<T> {
        let pos = self.stream.position();
        let error_count = self.errors.len();
        let pending_gt = self.pending_gt;
        let pending_star = self.pending_star;
        let pending_ampersand = self.pending_ampersand;

        match parse(self) {
            Ok(value) => Some(value),
            Err(_) => {
                // Backtrack on failure
                self.stream.reset_to(pos);
                self.errors.truncate(error_count);
                self.pending_gt = pending_gt;
                self.pending_star = pending_star;
                self.pending_ampersand = pending_ampersand;
                None
            }
        }
    }

    /// Synchronize the parser to recover from errors.
    ///
    /// This skips tokens until a statement or item boundary is reached:
    /// - Semicolons
    /// - Closing braces
    /// - Keywords that start declarations (fn, type, etc.)
    /// - Keywords that start statements (let, return, if, while, etc.)
    ///
    /// Returns the number of tokens skipped for diagnostic purposes.
    pub fn synchronize(&mut self) -> usize {
        let start_pos = self.stream.position();

        while !self.stream.at_end() && self.tick() {
            // Skip to the next synchronization point
            match self.stream.peek_kind() {
                Some(TokenKind::Semicolon) => {
                    self.stream.advance();
                    return self.stream.position() - start_pos;
                }
                Some(TokenKind::RBrace) => {
                    // Don't consume the closing brace - let the caller handle it
                    return self.stream.position() - start_pos;
                }
                Some(
                    // Item-level keywords
                    TokenKind::Fn
                    | TokenKind::Type
                    | TokenKind::Protocol
                    | TokenKind::Implement
                    | TokenKind::Module
                    | TokenKind::Mount
                    | TokenKind::Link
                    | TokenKind::Const
                    | TokenKind::Static
                    | TokenKind::Context
                    | TokenKind::Theorem
                    | TokenKind::Axiom
                    | TokenKind::Lemma
                    | TokenKind::Corollary
                    | TokenKind::Proof
                    | TokenKind::Async
                    | TokenKind::Unsafe
                    | TokenKind::Extern
                    | TokenKind::Pub
                    | TokenKind::Public
                    | TokenKind::Pure
                    | TokenKind::Meta
                    | TokenKind::Ffi
                    | TokenKind::View
                    | TokenKind::Layer
                    // Statement-level keywords
                    | TokenKind::Let
                    | TokenKind::Return
                    | TokenKind::If
                    | TokenKind::While
                    | TokenKind::For
                    | TokenKind::Loop
                    | TokenKind::Match
                    | TokenKind::Defer
                    | TokenKind::Provide
                    | TokenKind::Break
                    | TokenKind::Continue,
                ) => {
                    return self.stream.position() - start_pos;
                }
                _ => {
                    self.stream.advance();
                }
            }
        }

        self.stream.position() - start_pos
    }

    /// Synchronize to the next expression boundary.
    ///
    /// This is more fine-grained than `synchronize()` and is used within expressions
    /// to recover from errors like missing operators or malformed sub-expressions.
    pub fn synchronize_expr(&mut self) -> usize {
        let start_pos = self.stream.position();
        let mut depth: i32 = 0;

        while !self.stream.at_end() && self.tick() {
            match self.stream.peek_kind() {
                // Expression terminators at depth 0
                Some(TokenKind::Semicolon) if depth == 0 => {
                    return self.stream.position() - start_pos;
                }
                Some(TokenKind::Comma) if depth == 0 => {
                    return self.stream.position() - start_pos;
                }
                Some(TokenKind::RBrace | TokenKind::RParen | TokenKind::RBracket) if depth == 0 => {
                    return self.stream.position() - start_pos;
                }
                // Track nesting depth
                Some(TokenKind::LBrace | TokenKind::LParen | TokenKind::LBracket) => {
                    depth += 1;
                    self.stream.advance();
                }
                Some(TokenKind::RBrace | TokenKind::RParen | TokenKind::RBracket) => {
                    depth = depth.saturating_sub(1);
                    self.stream.advance();
                }
                _ => {
                    self.stream.advance();
                }
            }
        }

        self.stream.position() - start_pos
    }

    /// Check if the current token is an identifier.
    pub fn is_ident(&self) -> bool {
        matches!(self.stream.peek_kind(), Some(TokenKind::Ident(_)))
    }

    /// Check if a `.` is followed by an identifier-like token.
    ///
    /// This is used to determine if a `.` should be consumed as part of a qualified type
    /// path (like `Self.Item` or `module.Type`) or if it's a separator (like in
    /// `forall i: T . body` where `.` separates the binding from the body).
    ///
    /// Returns true if the current token is `.` and the next token is an identifier,
    /// Self, self, super, crate, or module.
    pub fn is_dot_followed_by_ident(&self) -> bool {
        // Check if current is Dot
        if !self.stream.check(&TokenKind::Dot) {
            return false;
        }
        // Check if next token is an identifier-like token
        // This must match what parse_path_segment() can consume via consume_ident_or_any_keyword()
        match self.stream.peek_nth(1).map(|t| &t.kind) {
            Some(TokenKind::Ident(_)) => true,
            Some(kind) => kind.is_keyword_like(),
            None => false,
        }
    }

    /// Check if semicolon omission is allowed at the current position.
    ///
    /// Semicolons can be omitted when followed by:
    /// - Statement-starting keywords (let, while, for, if, return, etc.)
    /// - Expression-starting keywords (Some, None, Ok, Err, true, false, etc.)
    /// - Identifiers (which can start assignment statements or function calls)
    /// - Block/expression terminators (}, EOF)
    /// - Item-starting keywords (fn, type, protocol, etc.)
    ///
    /// This enables cleaner code without requiring semicolons everywhere.
    pub fn allows_semicolon_omission(&self) -> bool {
        match self.stream.peek_kind() {
            // Statement starters
            Some(TokenKind::Let)
            | Some(TokenKind::While)
            | Some(TokenKind::For)
            | Some(TokenKind::If)
            | Some(TokenKind::Return)
            | Some(TokenKind::Break)
            | Some(TokenKind::Continue)
            | Some(TokenKind::Loop)
            | Some(TokenKind::Match)
            | Some(TokenKind::Defer)
            | Some(TokenKind::Provide)
            | Some(TokenKind::Mount)
            // Item starters
            | Some(TokenKind::Fn)
            | Some(TokenKind::Type)
            | Some(TokenKind::Protocol)
            | Some(TokenKind::Implement)
            | Some(TokenKind::Const)
            | Some(TokenKind::Pub)
            // Expression starters (common keywords that start expressions)
            | Some(TokenKind::Some)
            | Some(TokenKind::None)
            | Some(TokenKind::Ok)
            | Some(TokenKind::Err)
            | Some(TokenKind::True)
            | Some(TokenKind::False)
            | Some(TokenKind::Try)
            | Some(TokenKind::Async)
            | Some(TokenKind::Await)
            | Some(TokenKind::Spawn)
            // Identifiers (can start assignment statements or calls)
            | Some(TokenKind::Ident(_))
            // Block/expression terminators
            | Some(TokenKind::RBrace)
            | Some(TokenKind::Eof) => true,
            _ => false,
        }
    }

    /// Consume an identifier token and return its text.
    pub fn consume_ident(&mut self) -> ParseResult<Text> {
        match self.stream.peek() {
            Some(Token {
                kind: TokenKind::Ident(name),
                ..
            }) => {
                let name = name.clone();
                self.stream.advance();
                Ok(name)
            }
            Some(token) => Err(ParseError::invalid_syntax(
                "expected identifier",
                token.span,
            )),
            None => {
                let span = self.stream.current_span();
                Err(ParseError::unexpected_eof(&[], span))
            }
        }
    }

    /// Consume an identifier or contextual keyword that can be used as an identifier.
    ///
    /// This handles special cases like Some, None, Ok, Err, and proof keywords
    /// which can be used as identifiers in certain contexts (function names, field names).
    /// Does NOT include core reserved keywords like let, fn, if, match, etc.
    pub fn consume_ident_or_keyword(&mut self) -> ParseResult<Text> {
        match self.stream.peek_kind() {
            Some(TokenKind::Ident(name)) => {
                let name = name.clone();
                self.stream.advance();
                Ok(name)
            }
            // Built-in type variants that can be used as identifiers
            Some(TokenKind::Some) => {
                self.stream.advance();
                Ok(Text::from("Some"))
            }
            Some(TokenKind::None) => {
                self.stream.advance();
                Ok(Text::from("None"))
            }
            Some(TokenKind::Ok) => {
                self.stream.advance();
                Ok(Text::from("Ok"))
            }
            Some(TokenKind::Err) => {
                self.stream.advance();
                Ok(Text::from("Err"))
            }
            Some(TokenKind::Result) => {
                self.stream.advance();
                Ok(Text::from("result"))
            }
            Some(TokenKind::Await) => {
                self.stream.advance();
                Ok(Text::from("await"))
            }
            Some(TokenKind::Module) => {
                self.stream.advance();
                Ok(Text::from("module"))
            }
            Some(TokenKind::Cog) => {
                self.stream.advance();
                Ok(Text::from("cog"))
            }
            Some(TokenKind::Super) => {
                self.stream.advance();
                Ok(Text::from("super"))
            }
            Some(TokenKind::Stream) => {
                self.stream.advance();
                Ok(Text::from("stream"))
            }
            // Comprehension keywords that can be used as identifiers (e.g., set.new(), gen.next())
            Some(TokenKind::Set) => {
                self.stream.advance();
                Ok(Text::from("set"))
            }
            Some(TokenKind::Gen) => {
                self.stream.advance();
                Ok(Text::from("gen"))
            }
            // Pure keyword (can be used as function name, e.g., in Monad.pure)
            Some(TokenKind::Pure) => {
                self.stream.advance();
                Ok(Text::from("pure"))
            }
            // Proof keywords (contextual, can be used as identifiers)
            Some(TokenKind::Show) => {
                self.stream.advance();
                Ok(Text::from("show"))
            }
            Some(TokenKind::Have) => {
                self.stream.advance();
                Ok(Text::from("have"))
            }
            Some(TokenKind::Theorem) => {
                self.stream.advance();
                Ok(Text::from("theorem"))
            }
            Some(TokenKind::Axiom) => {
                self.stream.advance();
                Ok(Text::from("axiom"))
            }
            Some(TokenKind::Lemma) => {
                self.stream.advance();
                Ok(Text::from("lemma"))
            }
            Some(TokenKind::Corollary) => {
                self.stream.advance();
                Ok(Text::from("corollary"))
            }
            Some(TokenKind::Proof) => {
                self.stream.advance();
                Ok(Text::from("proof"))
            }
            Some(TokenKind::Calc) => {
                self.stream.advance();
                Ok(Text::from("calc"))
            }
            Some(TokenKind::Suffices) => {
                self.stream.advance();
                Ok(Text::from("suffices"))
            }
            Some(TokenKind::Obtain) => {
                self.stream.advance();
                Ok(Text::from("obtain"))
            }
            Some(TokenKind::By) => {
                self.stream.advance();
                Ok(Text::from("by"))
            }
            Some(TokenKind::Induction) => {
                self.stream.advance();
                Ok(Text::from("induction"))
            }
            Some(TokenKind::Cases) => {
                self.stream.advance();
                Ok(Text::from("cases"))
            }
            Some(TokenKind::Contradiction) => {
                self.stream.advance();
                Ok(Text::from("contradiction"))
            }
            Some(TokenKind::Trivial) => {
                self.stream.advance();
                Ok(Text::from("trivial"))
            }
            Some(TokenKind::Assumption) => {
                self.stream.advance();
                Ok(Text::from("assumption"))
            }
            Some(TokenKind::Simp) => {
                self.stream.advance();
                Ok(Text::from("simp"))
            }
            Some(TokenKind::Ring) => {
                self.stream.advance();
                Ok(Text::from("ring"))
            }
            Some(TokenKind::Field) => {
                self.stream.advance();
                Ok(Text::from("field"))
            }
            Some(TokenKind::Omega) => {
                self.stream.advance();
                Ok(Text::from("omega"))
            }
            Some(TokenKind::Auto) => {
                self.stream.advance();
                Ok(Text::from("auto"))
            }
            Some(TokenKind::Blast) => {
                self.stream.advance();
                Ok(Text::from("blast"))
            }
            Some(TokenKind::Smt) => {
                self.stream.advance();
                Ok(Text::from("smt"))
            }
            Some(TokenKind::Qed) => {
                self.stream.advance();
                Ok(Text::from("qed"))
            }
            // Context system keywords that can be used as identifiers (e.g., function names)
            Some(TokenKind::Provide) => {
                self.stream.advance();
                Ok(Text::from("provide"))
            }
            Some(TokenKind::Using) => {
                self.stream.advance();
                Ok(Text::from("using"))
            }
            Some(TokenKind::Context) => {
                self.stream.advance();
                Ok(Text::from("context"))
            }
            // Contract keywords that can be used as identifiers
            Some(TokenKind::Invariant) => {
                self.stream.advance();
                Ok(Text::from("invariant"))
            }
            Some(TokenKind::Requires) => {
                self.stream.advance();
                Ok(Text::from("requires"))
            }
            Some(TokenKind::Ensures) => {
                self.stream.advance();
                Ok(Text::from("ensures"))
            }
            Some(TokenKind::Forall) => {
                self.stream.advance();
                Ok(Text::from("forall"))
            }
            Some(TokenKind::Exists) => {
                self.stream.advance();
                Ok(Text::from("exists"))
            }
            // Async keywords that can be used as identifiers (common function names)
            Some(TokenKind::Spawn) => {
                self.stream.advance();
                Ok(Text::from("spawn"))
            }
            Some(TokenKind::Select) => {
                self.stream.advance();
                Ok(Text::from("select"))
            }
            Some(TokenKind::Yield) => {
                self.stream.advance();
                Ok(Text::from("yield"))
            }
            // Structured concurrency keyword that can be used as module name
            Some(TokenKind::Nursery) => {
                self.stream.advance();
                Ok(Text::from("nursery"))
            }
            // Error recovery keyword that can be used as method name
            Some(TokenKind::Recover) => {
                self.stream.advance();
                Ok(Text::from("recover"))
            }
            // Pattern matching keyword that can be used as parameter name
            Some(TokenKind::ActivePattern) => {
                self.stream.advance();
                Ok(Text::from("pattern"))
            }
            // Meta keyword that can be used as identifier (e.g., Ok(meta) in filesystem code)
            Some(TokenKind::Meta) => {
                self.stream.advance();
                Ok(Text::from("meta"))
            }
            // Protocol keyword that can be used as identifier (e.g., protocol: Int in socket functions)
            Some(TokenKind::Protocol) => {
                self.stream.advance();
                Ok(Text::from("protocol"))
            }
            // Internal keyword that can be used as identifier (e.g., internal: Int in FFI structs)
            Some(TokenKind::Internal) => {
                self.stream.advance();
                Ok(Text::from("internal"))
            }
            // Protected keyword that can be used as identifier (e.g., protected_count: Int in stats structs)
            Some(TokenKind::Protected) => {
                self.stream.advance();
                Ok(Text::from("protected"))
            }
            // Stage keyword that can be used as identifier (e.g., in where meta stage > 0)
            Some(TokenKind::Stage) => {
                self.stream.advance();
                Ok(Text::from("stage"))
            }
            // Volatile keyword that can be used as identifier (e.g., volatile: bool in FFI params)
            Some(TokenKind::Volatile) => {
                self.stream.advance();
                Ok(Text::from("volatile"))
            }
            // Extends keyword that can be used as identifier (contextual in protocol extension)
            Some(TokenKind::Extends) => {
                self.stream.advance();
                Ok(Text::from("extends"))
            }
            // Implement keyword that can be used as identifier (contextual in impl blocks)
            Some(TokenKind::Implement) => {
                self.stream.advance();
                Ok(Text::from("implement"))
            }
            // Implies keyword that can be used as identifier (contextual in proof contexts)
            Some(TokenKind::Implies) => {
                self.stream.advance();
                Ok(Text::from("implies"))
            }
            // Math/ML keywords that can be used as function/variable names
            Some(TokenKind::Tensor) => {
                self.stream.advance();
                Ok(Text::from("tensor"))
            }
            Some(TokenKind::Linear) => {
                self.stream.advance();
                Ok(Text::from("linear"))
            }
            Some(TokenKind::View) => {
                self.stream.advance();
                Ok(Text::from("view"))
            }
            Some(TokenKind::With) => {
                self.stream.advance();
                Ok(Text::from("with"))
            }
            Some(TokenKind::Link) => {
                self.stream.advance();
                Ok(Text::from("link"))
            }
            Some(TokenKind::Inject) => {
                self.stream.advance();
                Ok(Text::from("inject"))
            }
            Some(TokenKind::Throws) => {
                self.stream.advance();
                Ok(Text::from("throws"))
            }
            Some(TokenKind::Finally) => {
                self.stream.advance();
                Ok(Text::from("finally"))
            }
            Some(TokenKind::Try) => {
                self.stream.advance();
                Ok(Text::from("try"))
            }
            Some(TokenKind::Defer) => {
                self.stream.advance();
                Ok(Text::from("defer"))
            }
            Some(TokenKind::Errdefer) => {
                self.stream.advance();
                Ok(Text::from("errdefer"))
            }
            Some(TokenKind::Layer) => {
                self.stream.advance();
                Ok(Text::from("layer"))
            }
            Some(TokenKind::Inductive) => {
                self.stream.advance();
                Ok(Text::from("inductive"))
            }
            Some(TokenKind::Ffi) => {
                self.stream.advance();
                Ok(Text::from("ffi"))
            }
            Some(TokenKind::Mount) => {
                self.stream.advance();
                Ok(Text::from("mount"))
            }
            Some(TokenKind::Async) => {
                self.stream.advance();
                Ok(Text::from("async"))
            }
            Some(TokenKind::Cofix) => {
                self.stream.advance();
                Ok(Text::from("cofix"))
            }
            // Ref keyword that can be used as parameter name (e.g., ref: &[T] in CBGR functions)
            Some(TokenKind::Ref) => {
                self.stream.advance();
                Ok(Text::from("ref"))
            }
            // Unknown keyword that can be used as identifier (e.g., unknown: Int in config)
            Some(TokenKind::Unknown) => {
                self.stream.advance();
                Ok(Text::from("unknown"))
            }
            Some(_) => {
                let span = self.stream.current_span();
                Err(ParseError::invalid_syntax("expected identifier", span))
            }
            None => {
                let span = self.stream.current_span();
                Err(ParseError::unexpected_eof(&[], span))
            }
        }
    }

    /// Consume any identifier or ANY keyword as an identifier.
    /// Used for FFI parameter names where keywords are allowed.
    pub fn consume_ident_or_any_keyword(&mut self) -> ParseResult<Text> {
        use crate::TokenKind;

        match self.stream.peek_kind() {
            // Identifier
            Some(TokenKind::Ident(name)) => {
                let name = name.clone();
                self.stream.advance();
                Ok(name)
            }
            // All keywords that could be used as identifiers
            Some(kind) if kind.is_keyword_like() => {
                let name = kind.to_ident_string();
                self.stream.advance();
                Ok(name)
            }
            _ => {
                let span = self.stream.current_span();
                Err(ParseError::invalid_syntax(
                    "expected identifier or keyword",
                    span,
                ))
            }
        }
    }

    /// Consume a `>` token, handling nested generics by splitting `>>` tokens.
    ///
    /// For nested generics like `List<Maybe<Int>>`, the lexer produces `GtGt` for `>>`.
    /// This method:
    /// 1. If there's a pending `>` from a previous split, consume it
    /// 2. If current token is `Gt`, consume it normally
    /// 3. If current token is `GtGt`, consume it and set pending_gt = true (one `>` consumed, one pending)
    /// 4. If current token is `GtGtEq`, consume it and set pending_gt = true, then expect `Eq`
    ///
    /// Returns the consumed token's span for error reporting.
    pub fn expect_gt(&mut self) -> ParseResult<Span> {
        // First check if we have a pending > from a previous GtGt split
        if self.pending_gt {
            self.pending_gt = false;
            // Now actually consume the >> token that was left in the stream
            if self.stream.peek_kind() == Some(&TokenKind::GtGt) {
                self.stream.advance();
            }
            return Ok(self.stream.current_span());
        }

        // Check what token we have
        match self.stream.peek_kind() {
            Some(TokenKind::Gt) => {
                // SAFETY: We just matched on peek_kind() being Some
                let span = self
                    .stream
                    .peek()
                    .ok_or_else(|| {
                        ParseError::unexpected_eof(&[TokenKind::Gt], self.stream.current_span())
                    })?
                    .span;
                self.stream.advance();
                Ok(span)
            }
            Some(TokenKind::GtGt) => {
                // Split >> into two > tokens
                // Mark the first > as consumed, but DON'T advance the token stream
                // The next expect_gt() call will see pending_gt=true and "consume" the second >
                let span = self
                    .stream
                    .peek()
                    .ok_or_else(|| {
                        ParseError::unexpected_eof(&[TokenKind::Gt], self.stream.current_span())
                    })?
                    .span;
                // Don't advance! Keep the >> token in the stream
                self.pending_gt = true;
                Ok(span)
            }
            Some(TokenKind::GtGtEq) => {
                // Split >>= into >> and =
                // Consume the first >, leave > and = pending
                let span = self
                    .stream
                    .peek()
                    .ok_or_else(|| {
                        ParseError::unexpected_eof(&[TokenKind::Gt], self.stream.current_span())
                    })?
                    .span;
                self.stream.advance();
                self.pending_gt = true;
                // Note: The caller will need to handle the = if needed
                Ok(span)
            }
            Some(_) => {
                let token = self
                    .stream
                    .peek()
                    .ok_or_else(|| {
                        ParseError::unexpected_eof(&[TokenKind::Gt], self.stream.current_span())
                    })?
                    .clone();
                Err(ParseError::unexpected(&[TokenKind::Gt], token))
            }
            None => {
                let span = self.stream.current_span();
                Err(ParseError::unexpected_eof(&[TokenKind::Gt], span))
            }
        }
    }
}

// ============================================================================
// Span Utilities
// ============================================================================

/// Merge two spans into one that covers both.
///
/// # Panics
///
/// Panics in debug mode if spans are from different files.
#[inline]
pub fn merge_spans(start: Span, end: Span) -> Span {
    start.merge(end)
}

/// Create a span from start and end tokens.
#[inline]
pub fn span_from_tokens(start_token: &Token, end_token: &Token) -> Span {
    Span::new(
        start_token.span.start,
        end_token.span.end,
        start_token.span.file_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_token(kind: TokenKind, start: u32, end: u32) -> Token {
        Token::new(kind, Span::new(start, end, FileId::new(0)))
    }

    fn make_int_literal(value: i64) -> verum_lexer::token::IntegerLiteral {
        verum_lexer::token::IntegerLiteral {
            raw_value: value.to_string().into(),
            base: 10,
            suffix: verum_common::Maybe::None,
        }
    }

    #[test]
    fn test_token_stream_basic() {
        let tokens = vec![
            make_token(TokenKind::Let, 0, 3),
            make_token(TokenKind::Ident(Text::from("x")), 4, 5),
            make_token(TokenKind::Eq, 6, 7),
            make_token(TokenKind::Integer(make_int_literal(42)), 8, 10),
            make_token(TokenKind::Eof, 10, 10),
        ];

        let mut stream = TokenStream::new(&tokens);

        // Test peek
        assert!(matches!(stream.peek_kind(), Some(TokenKind::Let)));
        assert_eq!(stream.position(), 0);

        // Test advance
        stream.advance();
        assert!(matches!(stream.peek_kind(), Some(TokenKind::Ident(_))));
        assert_eq!(stream.position(), 1);

        // Test lookahead
        assert!(matches!(stream.peek_nth_kind(1), Some(TokenKind::Eq)));
        assert!(matches!(
            stream.peek_nth_kind(2),
            Some(TokenKind::Integer(_))
        ));
    }

    #[test]
    fn test_token_stream_check_and_consume() {
        let tokens = vec![
            make_token(TokenKind::Let, 0, 3),
            make_token(TokenKind::Ident(Text::from("x")), 4, 5),
            make_token(TokenKind::Eof, 5, 5),
        ];

        let mut stream = TokenStream::new(&tokens);

        // Test check
        assert!(stream.check(&TokenKind::Let));
        assert!(!stream.check(&TokenKind::Fn));

        // Test consume success
        assert!(stream.consume(&TokenKind::Let).is_some());
        assert_eq!(stream.position(), 1);

        // Test consume failure
        assert!(stream.consume(&TokenKind::Let).is_none());
        assert_eq!(stream.position(), 1); // Position unchanged
    }

    #[test]
    fn test_parser_comma_separated() {
        let tokens = vec![
            make_token(TokenKind::Integer(make_int_literal(1)), 0, 1),
            make_token(TokenKind::Comma, 1, 2),
            make_token(TokenKind::Integer(make_int_literal(2)), 2, 3),
            make_token(TokenKind::Comma, 3, 4),
            make_token(TokenKind::Integer(make_int_literal(3)), 4, 5),
            make_token(TokenKind::Eof, 5, 5),
        ];

        let mut parser = RecursiveParser::new(&tokens, FileId::new(0));

        let items: Vec<i64> = parser
            .comma_separated(|p| match p.stream.peek_kind() {
                Some(TokenKind::Integer(val)) => {
                    let val = val.raw_value.parse::<i64>().unwrap_or(0);
                    p.stream.advance();
                    Ok(val)
                }
                _ => Err(ParseError::invalid_syntax(
                    "expected integer",
                    p.stream.current_span(),
                )),
            })
            .unwrap();

        assert_eq!(items, vec![1, 2, 3]);
    }

    #[test]
    fn test_parser_optional() {
        let tokens = vec![
            make_token(TokenKind::Let, 0, 3),
            make_token(TokenKind::Ident(Text::from("x")), 4, 5),
            make_token(TokenKind::Eof, 5, 5),
        ];

        let mut parser = RecursiveParser::new(&tokens, FileId::new(0));

        // Test successful optional parse
        let result = parser.optional(|p| {
            p.stream.expect(TokenKind::Let)?;
            Ok(())
        });
        assert!(result.is_some());

        // Test failed optional parse (should not advance)
        let pos = parser.stream.position();
        let result = parser.optional(|p| {
            p.stream.expect(TokenKind::Fn)?;
            Ok(())
        });
        assert!(result.is_none());
        assert_eq!(parser.stream.position(), pos);
    }
}
