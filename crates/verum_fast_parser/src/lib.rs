#![allow(unexpected_cfgs)]
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
// Intentional: both `self` and `Self` tokens map to PathSegment::SelfValue in expression context
#![allow(clippy::if_same_then_else)]
//! Fast, direct-to-AST parser for Verum compiler.
//!
//! This is an optimized parser specifically designed for compilation use cases.
//! It builds AST directly without intermediate lossless syntax trees, providing
//! faster parsing with lower memory overhead.
//!
//! For IDE features (incremental parsing, lossless syntax trees, formatting),
//! use `verum_parser` instead.
//!
//! # Architecture
//!
//! The parser is organized into several modules:
//!
//! - [`expr`]: Expression parsing (binary ops, pipelines, comprehensions, etc.)
//! - [`ty`]: Type parsing (primitives, refinements, generics, references)
//! - [`pattern`]: Pattern parsing (wildcards, tuples, records, variants, etc.)
//! - [`decl`]: Declaration parsing (functions, types, protocols, implementations)
//! - [`stmt`]: Statement parsing (let bindings, expressions, defer, etc.)
//! - [`error`]: Error recovery and reporting
//! - [`parser`]: Core recursive descent infrastructure
//!
//! # Example
//!
//! ```rust
//! use verum_fast_parser::FastParser;
//! use verum_lexer::Lexer;
//! use verum_ast::span::FileId;
//!
//! let source = r#"
//!  fn factorial(n: Int{>= 0}) -> Int {
//!  match n {
//!  0 => 1,
//!  n => n * factorial(n - 1)
//!  }
//!  }
//! "#;
//!
//! let file_id = FileId::new(0);
//! let lexer = Lexer::new(source, file_id);
//! let parser = FastParser::new();
//! let result = parser.parse_module(lexer, file_id);
//!
//! match result {
//!  Ok(module) => println!("Parsed successfully: {} items", module.items.len()),
//!  Err(errors) => {
//!  for error in errors {
//!  eprintln!("Parse error: {}", error);
//!  }
//!  }
//! }
//! ```
//!
//! # Performance
//!
//! This parser is optimized for compilation speed:
//! - Direct AST construction (no intermediate green tree)
//! - No trivia preservation (comments/whitespace discarded)
//! - No incremental parsing overhead
//! - Minimal allocations

#![allow(unused_variables)]
#![allow(unused_imports)]
#![allow(dead_code)]

pub mod attr_validation;
mod decl;
pub mod error;
mod expr;
pub mod normalize;
mod parser;
mod pattern;
mod proof;
mod recovery;
mod safe_interpolation;
mod stmt;
mod ty;

use verum_ast::{CogKind, FileId, Item, Module, Span};
use verum_common::{List, Text};
use verum_lexer::{Lexer, Token, TokenKind};

pub use attr_validation::{
    AttributeValidationWarning, AttributeValidator, AttributeValidatorTrait, ValidationConfig,
    validate_field_attributes, validate_function_attributes, validate_match_arm_attributes,
    validate_parsed_attributes, validate_type_attributes,
};
pub use error::{ParseError, ParseResult};
pub use recovery::{
    Delimiter, RecoveryContext, RecoveryStrategy, SyncPoint, can_start_expression, can_start_item,
    can_start_statement, is_statement_terminator, missing_token_message, unexpected_token_message,
};

// Export the hand-written parser infrastructure
pub use parser::{RecursiveParser, TokenStream, merge_spans, span_from_tokens};

/// Fast parser for Verum compiler (direct AST construction).
///
/// This parser is optimized for compilation speed, building AST directly
/// without intermediate lossless syntax trees.
pub struct FastParser {
    /// Attribute-validation warnings from the most recent parse.
    ///
    /// T1025: the validator existed, knew the attribute set and had its
    /// message ready, and nothing constructed it — so a one-character typo
    /// in `@verify(thorough)` switched off the verification the author
    /// asked for and the file reported everything proved. Wiring the
    /// validator alone was not enough: `RecursiveParser` collects warnings
    /// into a vector whose only production reader was a doc-comment
    /// example. This is the channel out.
    ///
    /// A `Mutex` rather than a `RefCell` because `parse_module` takes
    /// `&self` and callers hand `FastParser` across threads.
    attr_warnings: std::sync::Mutex<Vec<attr_validation::AttributeValidationWarning>>,
}

impl FastParser {
    /// Create a new fast parser instance.
    pub fn new() -> Self {
        Self {
            attr_warnings: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Take the attribute-validation warnings produced by the most recent
    /// parse, leaving the channel empty.
    ///
    /// A caller that does not drain this is back where T1025 started — the
    /// validator runs and its answer is discarded — so the compile path
    /// drains it and turns each warning into a reported diagnostic.
    pub fn take_attr_warnings(&self) -> Vec<attr_validation::AttributeValidationWarning> {
        match self.attr_warnings.lock() {
            Ok(mut w) => std::mem::take(&mut *w),
            Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
        }
    }

    /// Parse a complete module from a token stream.
    pub fn parse_module(&self, lexer: Lexer, file_id: FileId) -> ParseResult<Module> {
        // Delegate to internal method with None for source
        self.parse_module_internal(lexer, file_id, None)
    }

    /// Parse a complete module from source text.
    ///
    /// This method provides better error diagnostics than `parse_module` because
    /// it can analyze the source text to determine specific lexer error types.
    pub fn parse_module_str(&self, source: &str, file_id: FileId) -> ParseResult<Module> {
        let lexer = Lexer::new(source, file_id);
        self.parse_module_internal(lexer, file_id, Some(source))
    }

    /// Parse a complete module in **script mode** (P1.2).
    ///
    /// In script mode, top-level statements (let-bindings, expression
    /// statements, defer / errdefer / provide) are accepted alongside
    /// regular items. Every collected statement is folded into a
    /// synthesised `__verum_script_main` function appended to the
    /// module — downstream passes treat it like any other private
    /// function. The compiler entry-detection pass uses
    /// `Module::is_script()` (set via the `@![__verum_kind("script")]`
    /// attribute) to recognise the wrapper as the script's entry
    /// point in P1.3.
    pub fn parse_module_script_str(&self, source: &str, file_id: FileId) -> ParseResult<Module> {
        let lexer = Lexer::new(source, file_id);
        self.parse_module_internal_with_script_mode(lexer, file_id, Some(source), true)
    }

    /// Internal parsing implementation with optional source text for error analysis.
    fn parse_module_internal(
        &self,
        lexer: Lexer,
        file_id: FileId,
        source: Option<&str>,
    ) -> ParseResult<Module> {
        self.parse_module_internal_with_script_mode(lexer, file_id, source, false)
    }

    /// Internal parsing implementation with explicit script-mode flag.
    /// All other entry points funnel through here so the script-mode
    /// switch lives in exactly one place.
    fn parse_module_internal_with_script_mode(
        &self,
        lexer: Lexer,
        file_id: FileId,
        source: Option<&str>,
        script_mode: bool,
    ) -> ParseResult<Module> {
        // Collect tokens from lexer, tracking position for error analysis
        let mut tokens = List::new();
        let mut last_end: u32 = 0;

        for result in lexer {
            match result {
                Ok(token) => {
                    last_end = token.span.end;
                    tokens.push(token);
                }
                Err(lex_error) => {
                    // Find actual error position by scanning past whitespace/comments from last_end
                    let error_start = if let Some(src) = source {
                        Self::find_error_position(src, last_end as usize) as u32
                    } else {
                        last_end
                    };

                    // Find end of error token (scan until whitespace or delimiter)
                    let error_end = if let Some(src) = source {
                        Self::find_error_end(src, error_start as usize) as u32
                    } else {
                        error_start + 1
                    };

                    let span = Span::new(error_start, error_end, file_id);

                    // Analyze error type based on source text if available
                    let parse_error = if let Some(src) = source {
                        Self::analyze_lexer_error(src, error_start as usize, span)
                    } else {
                        // Fallback: use error message to determine type
                        let msg = lex_error.message().as_str();
                        Self::error_from_message(msg, span)
                    };
                    return Err(List::from(vec![parse_error]));
                }
            }
        }

        // Add EOF token
        let mut tokens_with_eof = tokens;
        let last_span = tokens_with_eof
            .last()
            .map(|t| t.span)
            .unwrap_or(Span::new(0, 0, file_id));
        tokens_with_eof.push(Token::new(
            TokenKind::Eof,
            Span::new(last_span.end, last_span.end, file_id),
        ));

        // Use RecursiveParser for module parsing.
        //
        // T1025: WITH attribute validation. This is the single site every
        // entry point funnels through, so switching it on here covers the
        // compile path without touching a dozen call sites.
        let mut parser = RecursiveParser::with_attr_validation(&tokens_with_eof, file_id);
        parser.set_script_mode(script_mode);

        let result = parser.parse_module();
        // Drain BEFORE the early returns below: a parse that fails still
        // produced whatever attribute warnings it saw, and a channel only
        // drained on the success path is the same silence one layer in.
        {
            let taken = parser.take_attr_warnings();
            if !taken.is_empty()
                && let Ok(mut sink) = self.attr_warnings.lock()
            {
                sink.extend(taken);
            }
        }

        match result {
            Ok(items) => {
                // Check if there were any errors during parsing
                if !parser.errors.is_empty() {
                    return Err(parser.errors.into());
                }

                // Create module span from first and last items
                let span = items
                    .first()
                    .zip(items.last())
                    .map(|(first, last)| first.span.merge(last.span))
                    .unwrap_or(Span::new(0, 0, file_id));

                // Explicitly convert Vec to List
                let items_list: List<Item> = items.into_iter().collect();
                let mut module = Module::new(items_list, file_id, span);
                // META-SPAN-ALIAS-1: module-level deferred classification
                // of the grammatically ambiguous `type X is BareIdent;`
                // form (alias vs single-variant enum) — see
                // `normalize.rs` module docs. Runs at the single parse
                // funnel so every consumer (typecheck, VBC codegen, AOT,
                // archive metadata, LSP) sees one consistent reading.
                normalize::reclassify_single_variant_aliases(&mut module);
                // Auto-tag script modules so the entry-detection fallback
                // recognises the synthesised `__verum_script_main` wrapper
                // as the program entry. Single source of truth: every caller
                // that opts into script-mode parsing automatically gets the
                // tag. `set_on_module` is idempotent — callers that re-tag
                // explicitly remain correct.
                if script_mode {
                    CogKind::Script.set_on_module(&mut module);
                }
                Ok(module)
            }
            Err(e) => {
                // Collect all errors
                let mut errors = parser.errors;
                errors.push(e);
                Err(errors.into())
            }
        }
    }

    /// Lex an entire string, REPORTING lex errors instead of discarding them.
    ///
    /// The whole-string entry points below used `filter_map(|r| r.ok())`,
    /// which drops every `Err` on the floor: an unlexable character simply
    /// vanished and parsing continued on the surviving tokens, so the
    /// function could return `Ok` for source it had failed to read. Module
    /// parsing never had this hole -- `parse_module_internal_with_script_mode`
    /// stops at the first lex error and reports it -- so this reuses that
    /// same authority (`analyze_lexer_error` over the located span) rather
    /// than inventing a second, weaker diagnosis for the same condition.
    fn lex_all(source: &str, file_id: FileId) -> ParseResult<List<Token>> {
        let mut tokens = List::new();
        let mut last_end: u32 = 0;

        for result in Lexer::new(source, file_id) {
            match result {
                Ok(token) => {
                    last_end = token.span.end;
                    tokens.push(token);
                }
                Err(_) => {
                    // Same position recovery as the module path: the lexer
                    // stops before the offending text, so scan forward from
                    // the last good token past whitespace and comments.
                    let error_start = Self::find_error_position(source, last_end as usize) as u32;
                    let error_end = Self::find_error_end(source, error_start as usize) as u32;
                    let span = Span::new(error_start, error_end, file_id);
                    return Err(List::from(vec![Self::analyze_lexer_error(
                        source,
                        error_start as usize,
                        span,
                    )]));
                }
            }
        }

        Ok(tokens)
    }

    /// Reject input the parser stopped short of consuming.
    ///
    /// The whole-string entry points below promise something the token-slice
    /// entry points do not: that *this text* is an expression, or a type.
    /// The caller hands over a string and gets back a node -- it has no
    /// stream to inspect, so a parse that stopped halfway is indistinguishable
    /// from one that succeeded. Without this check `parse_expr_str` returns
    /// `Ok` for `mat#[[1, 2]]` having read only the composite `mat#[[1, 2]`
    /// -- a literal whose content is the unbalanced `[1, 2` -- and the caller
    /// silently proceeds on a fragment of what it supplied.
    ///
    /// That is a correctness hazard, not a testing nicety. The archived
    /// refinement predicate re-parsed in `verum_vbc::codegen` becomes the
    /// runtime check emitted for a field, so a predicate truncated here used
    /// to emit a *weaker* check with no diagnostic; it now fails to parse and
    /// the field ships unrefined, which is that site's documented fail-open
    /// policy rather than a silently wrong check. `${...}` interpolation in
    /// `sh#"..."` lost part of the interpolated expression the same way.
    ///
    /// The error names the first unconsumed token, which is where the
    /// accepted prefix ended -- the one position that explains the refusal.
    fn require_input_consumed(parser: &RecursiveParser<'_>) -> ParseResult<()> {
        if parser.stream.at_end() {
            return Ok(());
        }

        let trailing = parser.stream.current_span();
        let err = match parser.stream.peek() {
            Some(token) => ParseError::unexpected(&[TokenKind::Eof], token.clone()),
            // `at_end()` is false, so `peek()` is `Some`; this arm is
            // unreachable and kept only to avoid an unwrap.
            None => ParseError::unexpected_eof(&[TokenKind::Eof], trailing),
        }
        .with_help("this entry point parses the whole string; the text after this point is not part of the construct");

        Err(List::from(vec![err]))
    }

    /// Parse a single expression from a string.
    ///
    /// The **entire** string must be an expression: input the parser does not
    /// consume is rejected rather than discarded (see
    /// [`Self::require_input_consumed`]). Lex errors are reported, not
    /// silently dropped; see [`Self::lex_all`].
    pub fn parse_expr_str(&self, source: &str, file_id: FileId) -> ParseResult<verum_ast::Expr> {
        let tokens = Self::lex_all(source, file_id)?;

        let mut parser = RecursiveParser::new(&tokens, file_id);
        let expr = parser.parse_expr().map_err(|e| List::from(vec![e]))?;

        if !parser.errors.is_empty() {
            return Err(parser.errors.into());
        }
        Self::require_input_consumed(&parser)?;

        Ok(expr)
    }

    /// Parse a single type from a string.
    ///
    /// The **entire** string must be a type: input the parser does not consume
    /// is rejected rather than discarded (see
    /// [`Self::require_input_consumed`]). Lex errors are reported, not
    /// silently dropped; see [`Self::lex_all`].
    pub fn parse_type_str(&self, source: &str, file_id: FileId) -> ParseResult<verum_ast::Type> {
        let tokens = Self::lex_all(source, file_id)?;

        let mut parser = RecursiveParser::new(&tokens, file_id);
        let ty = parser.parse_type().map_err(|e| List::from(vec![e]))?;

        if !parser.errors.is_empty() {
            return Err(parser.errors.into());
        }
        Self::require_input_consumed(&parser)?;

        Ok(ty)
    }

    /// [`Self::require_input_consumed`] adapted to the token entry points'
    /// error channel.
    ///
    /// The token entry points report through `Result<_, Text>` while the
    /// string ones report a `List<ParseError>`, so the refusal is joined the
    /// same way each of them already joins `parser.errors`. The EXHAUSTION
    /// RULE ITSELF lives in one place for both families: a second predicate
    /// here would be free to drift from the string entry points, and "did
    /// this parse consume its input" must not have two answers (T0643).
    fn require_tokens_consumed(parser: &RecursiveParser<'_>) -> Result<(), Text> {
        Self::require_input_consumed(parser).map_err(|errors| {
            let joined: Vec<String> = errors.iter().map(|e| format!("{}", e)).collect();
            Text::from(joined.join("; "))
        })
    }

    /// Parse an expression directly from tokens (for meta-programming).
    ///
    /// The **entire** token slice must be the expression: tokens the parser
    /// does not consume are rejected rather than discarded. Before T0643 a
    /// `quote!` stream carrying trailing tokens returned `Ok` on the prefix,
    /// and the caller — which holds no parser and cannot see the cursor —
    /// could not tell a partial parse from a complete one.
    pub fn parse_expr_tokens(&self, tokens: &List<Token>) -> Result<verum_ast::Expr, Text> {
        if tokens.is_empty() {
            return Err(Text::from("Cannot parse empty token list"));
        }

        let file_id = tokens
            .first()
            .map(|t| t.span.file_id)
            .unwrap_or(FileId::new(0));

        let mut parser = RecursiveParser::new(tokens.as_slice(), file_id);

        match parser.parse_expr() {
            Ok(expr) => {
                if parser.errors.is_empty() {
                    Self::require_tokens_consumed(&parser)?;
                    Ok(expr)
                } else {
                    let errors: Vec<String> =
                        parser.errors.iter().map(|e| format!("{}", e)).collect();
                    Err(Text::from(errors.join("; ")))
                }
            }
            Err(e) => Err(Text::from(format!("{}", e))),
        }
    }

    /// Parse a type directly from tokens (for meta-programming).
    ///
    /// The **entire** token slice must be the type; see
    /// [`Self::parse_expr_tokens`] for why a prefix parse is refused.
    pub fn parse_type_tokens(&self, tokens: &List<Token>) -> Result<verum_ast::Type, Text> {
        if tokens.is_empty() {
            return Err(Text::from("Cannot parse empty token list"));
        }

        let file_id = tokens
            .first()
            .map(|t| t.span.file_id)
            .unwrap_or(FileId::new(0));

        let mut parser = RecursiveParser::new(tokens.as_slice(), file_id);

        match parser.parse_type() {
            Ok(ty) => {
                if parser.errors.is_empty() {
                    Self::require_tokens_consumed(&parser)?;
                    Ok(ty)
                } else {
                    let errors: Vec<String> =
                        parser.errors.iter().map(|e| format!("{}", e)).collect();
                    Err(Text::from(errors.join("; ")))
                }
            }
            Err(e) => Err(Text::from(format!("{}", e))),
        }
    }

    /// Parse a SINGLE item directly from tokens (for meta-programming).
    ///
    /// The **entire** token slice must be that one item. A slice carrying two
    /// items used to return the FIRST and drop the rest silently, which is
    /// how a macro expansion could lose half its output with no diagnostic
    /// anywhere; [`Self::parse_items_tokens`] is the multi-item entry point
    /// and is what such a caller wants (T0643).
    pub fn parse_item_tokens(&self, tokens: &List<Token>) -> Result<Item, Text> {
        if tokens.is_empty() {
            return Err(Text::from("Cannot parse empty token list"));
        }

        let file_id = tokens
            .first()
            .map(|t| t.span.file_id)
            .unwrap_or(FileId::new(0));

        let mut parser = RecursiveParser::new(tokens.as_slice(), file_id);

        match parser.parse_item() {
            Ok(item) => {
                if parser.errors.is_empty() {
                    Self::require_tokens_consumed(&parser)?;
                    Ok(item)
                } else {
                    let errors: Vec<String> =
                        parser.errors.iter().map(|e| format!("{}", e)).collect();
                    Err(Text::from(errors.join("; ")))
                }
            }
            Err(e) => Err(Text::from(format!("{}", e))),
        }
    }

    /// Parse multiple items from tokens (for staged metaprogramming).
    ///
    /// This is used when a meta function generates multiple items (e.g., a function
    /// and a type definition). It parses all items in the token stream until EOF.
    ///
    /// # Arguments
    ///
    /// * `tokens` - Token stream containing the generated code
    ///
    /// # Returns
    ///
    /// A list of parsed items, or an error if parsing fails.
    pub fn parse_items_tokens(&self, tokens: &List<Token>) -> Result<List<Item>, Text> {
        if tokens.is_empty() {
            return Ok(List::new()); // Empty token stream produces empty item list
        }

        let file_id = tokens
            .first()
            .map(|t| t.span.file_id)
            .unwrap_or(FileId::new(0));

        let mut parser = RecursiveParser::new(tokens.as_slice(), file_id);

        // Use parse_module which parses items until EOF
        match parser.parse_module() {
            Ok(items) => {
                if parser.errors.is_empty() {
                    // `parse_module` runs to EOF, so this normally holds — it
                    // is asserted anyway so that "the whole slice was
                    // consumed" is guaranteed by the SAME rule for every
                    // token entry point rather than by one of them happening
                    // to loop (T0643).
                    Self::require_tokens_consumed(&parser)?;
                    Ok(items.into_iter().collect())
                } else {
                    let errors: Vec<String> =
                        parser.errors.iter().map(|e| format!("{}", e)).collect();
                    Err(Text::from(errors.join("; ")))
                }
            }
            Err(e) => Err(Text::from(format!("{}", e))),
        }
    }

    /// Find the actual error position by scanning past whitespace and comments.
    fn find_error_position(source: &str, start: usize) -> usize {
        let bytes = source.as_bytes();
        let mut pos = start;
        let len = bytes.len();

        while pos < len {
            let b = bytes[pos];
            // Skip whitespace
            if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
                pos += 1;
                continue;
            }
            // Skip single-line comments
            if pos + 1 < len && bytes[pos] == b'/' && bytes[pos + 1] == b'/' {
                // Skip to end of line
                while pos < len && bytes[pos] != b'\n' {
                    pos += 1;
                }
                continue;
            }
            // Skip block comments
            if pos + 1 < len && bytes[pos] == b'/' && bytes[pos + 1] == b'*' {
                pos += 2;
                let mut depth = 1;
                while pos + 1 < len && depth > 0 {
                    if bytes[pos] == b'/' && bytes[pos + 1] == b'*' {
                        depth += 1;
                        pos += 2;
                    } else if bytes[pos] == b'*' && bytes[pos + 1] == b'/' {
                        depth -= 1;
                        pos += 2;
                    } else {
                        pos += 1;
                    }
                }
                continue;
            }
            // Found non-whitespace, non-comment - this is the error position
            break;
        }
        pos
    }

    /// Find the end of an error token (scan until whitespace, delimiter, or end of source).
    fn find_error_end(source: &str, start: usize) -> usize {
        let bytes = source.as_bytes();
        let mut pos = start;
        let len = bytes.len();

        // Handle string literals - scan to closing quote or end
        if pos < len && bytes[pos] == b'"' {
            pos += 1;
            while pos < len {
                if bytes[pos] == b'"' {
                    pos += 1;
                    break;
                }
                if bytes[pos] == b'\\' && pos + 1 < len {
                    pos += 2; // Skip escape sequence
                } else {
                    pos += 1;
                }
            }
            return pos;
        }

        // Handle char literals
        if pos < len && bytes[pos] == b'\'' {
            pos += 1;
            while pos < len {
                if bytes[pos] == b'\'' {
                    pos += 1;
                    break;
                }
                if bytes[pos] == b'\\' && pos + 1 < len {
                    pos += 2;
                } else {
                    pos += 1;
                }
                // Limit char literal scan to prevent runaway
                if pos > start + 10 {
                    break;
                }
            }
            return pos;
        }

        // Handle number literals (0x, 0b, 0o prefixed)
        if pos + 1 < len && bytes[pos] == b'0' {
            let prefix = bytes[pos + 1];
            if prefix == b'x'
                || prefix == b'X'
                || prefix == b'b'
                || prefix == b'B'
                || prefix == b'o'
                || prefix == b'O'
            {
                pos += 2;
                // Scan alphanumeric (including invalid chars for the base)
                while pos < len && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') {
                    pos += 1;
                }
                return pos;
            }
        }

        // General case: scan until delimiter or whitespace
        while pos < len {
            let b = bytes[pos];
            if b.is_ascii_whitespace()
                || b == b';'
                || b == b','
                || b == b')'
                || b == b']'
                || b == b'}'
            {
                break;
            }
            pos += 1;
        }
        pos
    }

    /// Analyze source text at error position to determine specific lexer error type.
    fn analyze_lexer_error(source: &str, pos: usize, span: Span) -> ParseError {
        // Get the text at the error position
        let remaining = if pos < source.len() {
            &source[pos..]
        } else {
            ""
        };

        // Analyze the pattern to determine error type
        if remaining.starts_with("0x") && remaining.len() <= 2 {
            // Invalid hex: 0x without digits
            ParseError::invalid_number(
                "incomplete hexadecimal literal: missing digits after 0x",
                span,
            )
        } else if remaining.starts_with("0x") {
            // Invalid hex with invalid characters
            ParseError::invalid_number("invalid hexadecimal literal", span)
        } else if remaining.starts_with("0b") && remaining.len() <= 2 {
            // Invalid binary: 0b without digits
            ParseError::invalid_number("incomplete binary literal: missing digits after 0b", span)
        } else if remaining.starts_with("0b") {
            // Invalid binary with invalid characters
            ParseError::invalid_number("invalid binary literal", span)
        } else if remaining.starts_with("0o") && remaining.len() <= 2 {
            // Invalid octal: 0o without digits
            ParseError::invalid_number("incomplete octal literal: missing digits after 0o", span)
        } else if remaining.starts_with("0o") {
            // Invalid octal with invalid characters
            ParseError::invalid_number("invalid octal literal", span)
        } else if remaining.starts_with('"') {
            // Check for invalid escape sequences in string
            if let Some(invalid_escape_err) = check_string_for_invalid_escapes(remaining) {
                return ParseError::invalid_escape(invalid_escape_err, span);
            }
            // Otherwise it's unterminated
            ParseError::unterminated_string(span)
        } else if remaining.starts_with("''") || remaining.starts_with("' '") {
            // Empty character literal
            ParseError::empty_char(span)
        } else if let Some(stripped) = remaining.strip_prefix('\'') {
            // Check if this is a multi-char literal like 'abc' or 'ab'
            // Find the content between quotes
            if let Some(end_quote) = stripped.find('\'') {
                let content = &stripped[..end_quote];
                // If content has more than one character (accounting for escape sequences)
                // it's an invalid multi-char literal
                let char_count = count_chars_in_literal(content);
                if char_count == 0 {
                    return ParseError::empty_char(span);
                }
                if char_count > 1 {
                    return ParseError::empty_char(span); // E004 also used for multi-char
                }
            }
            // No closing quote found - unterminated char
            ParseError::unterminated_char(span)
        } else {
            // Unknown token
            ParseError::unknown_token(span)
        }
    }

    /// Create error from lexer error message (fallback when source not available).
    fn error_from_message(msg: &str, span: Span) -> ParseError {
        if msg.contains("unterminated string") {
            ParseError::unterminated_string(span)
        } else if msg.contains("unterminated character") || msg.contains("unterminated char") {
            ParseError::unterminated_char(span)
        } else if msg.contains("invalid escape") {
            ParseError::invalid_escape(msg, span)
        } else if msg.contains("invalid number")
            || msg.contains("invalid binary")
            || msg.contains("invalid hex")
            || msg.contains("invalid octal")
        {
            ParseError::invalid_number(msg, span)
        } else if msg.contains("empty char") || msg.contains("empty character") {
            ParseError::empty_char(span)
        } else if msg.contains("interpolation") {
            ParseError::invalid_interpolation(msg, span)
        } else {
            ParseError::unknown_token(span)
        }
    }
}

impl Default for FastParser {
    fn default() -> Self {
        Self::new()
    }
}

/// Count the number of characters in a char literal content (handling escape sequences).
fn count_chars_in_literal(content: &str) -> usize {
    let bytes = content.as_bytes();
    let mut count = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            // Escape sequence - counts as one character
            if i + 1 < bytes.len() {
                match bytes[i + 1] {
                    b'x' => {
                        // \xNN - skip 4 bytes total
                        i += 4;
                    }
                    b'u' => {
                        // \u{NNNNNN} - skip until }
                        i += 2; // skip \u
                        while i < bytes.len() && bytes[i] != b'}' {
                            i += 1;
                        }
                        i += 1; // skip }
                    }
                    _ => {
                        // Simple escape like \n, \t - skip 2 bytes
                        i += 2;
                    }
                }
            } else {
                i += 1;
            }
        } else {
            // Regular character (may be multi-byte UTF-8)
            // Just count bytes for simplicity - we just need to know if > 1 char
            i += 1;
        }
        count += 1;
    }
    count
}

/// Check a string literal for invalid escape sequences.
/// Returns Some(error_message) if an invalid escape is found, None otherwise.
fn check_string_for_invalid_escapes(s: &str) -> Option<&'static str> {
    // Skip the opening quote
    let content = s.strip_prefix('"').unwrap_or(s);
    let bytes = content.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'"' {
            // Found closing quote, string is valid (no invalid escapes before here)
            return None;
        }

        if bytes[i] == b'\\' {
            i += 1;
            if i >= bytes.len() {
                return Some("unterminated escape sequence");
            }

            match bytes[i] {
                // Valid simple escapes
                b'n' | b'r' | b't' | b'0' | b'a' | b'b' | b'f' | b'v' | b'\\' | b'"' | b'\'' => {
                    i += 1;
                }
                // Hex escape: \xNN
                b'x' => {
                    i += 1;
                    if i + 2 > bytes.len() {
                        return Some("incomplete hex escape sequence \\x");
                    }
                    if !bytes[i].is_ascii_hexdigit() || !bytes[i + 1].is_ascii_hexdigit() {
                        return Some("invalid hex escape sequence \\x");
                    }
                    i += 2;
                }
                // Unicode escape: \u{NNNNNN}
                b'u' => {
                    i += 1;
                    if i >= bytes.len() || bytes[i] != b'{' {
                        return Some("invalid unicode escape: expected '{' after \\u");
                    }
                    i += 1;
                    let hex_start = i;
                    while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                        i += 1;
                    }
                    let hex_len = i - hex_start;
                    if hex_len == 0 {
                        return Some("invalid unicode escape: empty hex sequence");
                    }
                    if hex_len > 6 {
                        return Some("invalid unicode escape: too many hex digits");
                    }
                    if i >= bytes.len() {
                        return Some("invalid unicode escape: missing closing '}'");
                    }
                    if bytes[i] != b'}' {
                        return Some("invalid unicode escape: non-hex character in sequence");
                    }
                    // Validate code point range
                    if let Ok(hex_str) = std::str::from_utf8(&bytes[hex_start..i]) {
                        if let Ok(code_point) = u32::from_str_radix(hex_str, 16) {
                            if code_point > 0x10FFFF {
                                return Some("invalid unicode escape: code point out of range");
                            }
                        }
                    }
                    i += 1;
                }
                // Invalid escape character
                _ => {
                    return Some("invalid escape sequence");
                }
            }
        } else {
            i += 1;
        }
    }

    // No closing quote found - this is an unterminated string, not an invalid escape
    None
}

// Backwards compatibility alias
pub type VerumParser = FastParser;

/// Simple parser wrapper for testing.
///
/// This provides a simplified interface that takes just a source string,
/// automatically creating the lexer and using a default file ID.
pub struct Parser {
    source: String,
    file_id: FileId,
}

impl Parser {
    /// Create a new parser from source code.
    ///
    /// Uses a default file ID of 0. For production use with proper file tracking,
    /// use `FastParser::parse_module` directly.
    pub fn new(source: &str) -> Self {
        Self {
            source: source.to_string(),
            file_id: FileId::new(0),
        }
    }

    /// Parse a complete module from the source.
    pub fn parse_module(&mut self) -> ParseResult<Module> {
        let lexer = Lexer::new(&self.source, self.file_id);
        let parser = FastParser::new();
        parser.parse_module(lexer, self.file_id)
    }

    /// Parse a single expression from the source.
    pub fn parse_expr(&mut self) -> ParseResult<verum_ast::Expr> {
        let parser = FastParser::new();
        parser.parse_expr_str(&self.source, self.file_id)
    }

    /// Parse a single type from the source.
    pub fn parse_type(&mut self) -> ParseResult<verum_ast::Type> {
        let parser = FastParser::new();
        parser.parse_type_str(&self.source, self.file_id)
    }
}

#[cfg(test)]
mod lex_error_reporting {
    use super::*;

    /// A lex error must FAIL the parse, not disappear.
    ///
    /// `parse_expr_str` / `parse_type_str` used `filter_map(|r| r.ok())`,
    /// which discarded every lexer `Err`. An unlexable character was
    /// dropped and parsing continued on whatever tokens survived, so the
    /// function could report success on source it had failed to read —
    /// the worst shape of failure, since the caller sees a well-formed AST
    /// built from a subset of the input.
    ///
    /// These are inline rather than in `tests/` deliberately: CI runs
    /// `cargo test --workspace --lib --bins`, which does not execute
    /// `tests/`, so a gate placed there would not run.
    #[test]
    fn unlexable_input_is_an_error_not_a_silent_drop() {
        let parser = FastParser::new();
        let file_id = FileId::new(0);

        // An unterminated string cannot be lexed. Before the fix the bad
        // token vanished and `1 +` (or whatever remained) was parsed.
        let result = parser.parse_expr_str("1 + \"unterminated", file_id);
        assert!(
            result.is_err(),
            "unterminated string literal lexed cleanly — the lex error was dropped"
        );
    }

    #[test]
    fn well_formed_input_still_parses() {
        // The guard above must not reject valid source: a fix that fails
        // everything would satisfy the test above and be useless.
        let parser = FastParser::new();
        let file_id = FileId::new(0);

        assert!(parser.parse_expr_str("1 + 2", file_id).is_ok());
        assert!(parser.parse_type_str("Int", file_id).is_ok());
    }
}

#[cfg(test)]
mod input_exhaustion {
    use super::*;

    /// Trailing tokens must FAIL the parse, not be discarded.
    ///
    /// `parse_expr_str` / `parse_type_str` never checked the stream was
    /// exhausted, so a parse that stopped halfway returned `Ok` and the
    /// caller — holding only a string and a node — had no way to tell.
    ///
    /// Inline rather than in `tests/` for the same reason as
    /// [`lex_error_reporting`]: CI runs `cargo test --workspace --lib
    /// --bins`, which does not execute `tests/`.
    ///
    /// The demonstrator is deliberately `mat#[[1, 2]]`. A composite body is
    /// a flat character run — `grammar/verum.ebnf` `composite_char` excludes
    /// `)`, `]` and `}` — so the lexer ends the composite at the first `]`
    /// and the remainder is genuinely unparsed input. (`1 ++ 2` does *not*
    /// work as a demonstrator: `++` is the `Concat` operator, so that input
    /// parses in full.)
    #[test]
    fn trailing_tokens_are_an_error_not_a_silent_truncation() {
        let parser = FastParser::new();
        let file_id = FileId::new(0);

        let result = parser.parse_expr_str("mat#[[1, 2]]", file_id);
        assert!(
            result.is_err(),
            "nested composite body parsed as `Ok` — the unconsumed tail was discarded"
        );
    }

    #[test]
    fn trailing_tokens_after_a_type_are_an_error() {
        let parser = FastParser::new();
        let file_id = FileId::new(0);

        let result = parser.parse_type_str("Int Text", file_id);
        assert!(
            result.is_err(),
            "`Int Text` parsed as `Ok` — the trailing type name was discarded"
        );
    }

    /// The guard must not reject input that legitimately parses whole: a
    /// check that failed everything would satisfy the tests above and be
    /// useless. Each of these exercises a construct whose parse ends on a
    /// closing token rather than running off the end of the stream.
    #[test]
    fn fully_consumed_input_still_parses() {
        let parser = FastParser::new();
        let file_id = FileId::new(0);

        for source in [
            "1 + 2",
            "foo(1, 2)",
            "vec#[1, 2, 3]",
            "[1, 2, 3]",
            "if x { 1 } else { 2 }",
            "|x| x + 1",
            "match x { 0 => 1, n => n }",
        ] {
            assert!(
                parser.parse_expr_str(source, file_id).is_ok(),
                "expression `{source}` was rejected as incompletely consumed"
            );
        }

        for source in [
            "Int",
            "List<Int>",
            "Map<Text, List<Int>>",
            "Int{> 0}",
            "fn(Int) -> Text",
            "(Int, Text)",
        ] {
            assert!(
                parser.parse_type_str(source, file_id).is_ok(),
                "type `{source}` was rejected as incompletely consumed"
            );
        }
    }
}

/// T0643 — the TOKEN-slice entry points enforce the same exhaustion rule.
///
/// `parse_expr_str` / `parse_type_str` gained the rule first (see
/// [`input_exhaustion`]); the `*_tokens` family did not, so every
/// meta-programming caller kept the silent truncation. The caller there is
/// worse off than a string caller: `quote::TokenStream` hands over a slice
/// and gets back a node, holds no parser, and the entry point drops the
/// parser on return — so the consumed count is not observable to it by any
/// route. It cannot detect what it is not told.
#[cfg(test)]
mod token_input_exhaustion {
    use super::*;
    use verum_lexer::Lexer;

    fn lex(source: &str) -> List<Token> {
        Lexer::new(source, FileId::new(0)).filter_map(|r| r.ok()).collect()
    }

    /// Two items in one slice returned the FIRST and dropped the second.
    ///
    /// This is the live macro path: `pipeline/macros.rs` parses each
    /// expansion result with `parse_as_item` -> `parse_item_tokens`, so a
    /// macro that generated a function AND a type used to contribute only
    /// the function, with no diagnostic anywhere. `parse_items_tokens` is
    /// the entry point for that case and it still accepts both.
    #[test]
    fn a_second_item_is_refused_rather_than_dropped() {
        let parser = FastParser::new();
        let tokens = lex("fn a() -> Int { 1 } fn b() -> Int { 2 }");

        let single = parser.parse_item_tokens(&tokens);
        assert!(
            single.is_err(),
            "two items parsed as one `Ok` — the second item was discarded"
        );

        let both = parser
            .parse_items_tokens(&tokens)
            .expect("the multi-item entry point accepts both");
        assert_eq!(both.len(), 2, "parse_items_tokens must return both items");
    }

    /// The expression case, which is what `quote!{...}.parse_as_expr()` runs.
    #[test]
    fn trailing_tokens_after_an_expression_are_refused() {
        let parser = FastParser::new();

        let result = parser.parse_expr_tokens(&lex("1 + 2 fn f() {}"));
        assert!(
            result.is_err(),
            "an expression followed by an item parsed as `Ok` — the tail was discarded"
        );
    }

    /// The type case, which is what `quote!{...}.parse_as_type()` runs.
    #[test]
    fn trailing_tokens_after_a_type_are_refused() {
        let parser = FastParser::new();

        let result = parser.parse_type_tokens(&lex("Int Text"));
        assert!(
            result.is_err(),
            "`Int Text` parsed as `Ok` — the trailing type name was discarded"
        );
    }

    /// A rule that rejected everything would satisfy the three tests above
    /// and be useless, so pin the other direction on each entry point.
    #[test]
    fn fully_consumed_token_slices_still_parse() {
        let parser = FastParser::new();

        for source in ["1 + 2", "foo(1, 2)", "[1, 2, 3]", "if x { 1 } else { 2 }"] {
            assert!(
                parser.parse_expr_tokens(&lex(source)).is_ok(),
                "expression `{source}` was rejected as incompletely consumed"
            );
        }

        for source in ["Int", "List<Int>", "Map<Text, List<Int>>"] {
            assert!(
                parser.parse_type_tokens(&lex(source)).is_ok(),
                "type `{source}` was rejected as incompletely consumed"
            );
        }

        for source in ["fn a() -> Int { 1 }", "type P is { x: Int };"] {
            assert!(
                parser.parse_item_tokens(&lex(source)).is_ok(),
                "item `{source}` was rejected as incompletely consumed"
            );
        }
    }
}
