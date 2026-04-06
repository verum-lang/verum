//! Pattern parser for Verum.
//!
//! This module implements parsing for all pattern forms used in:
//! - Let bindings
//! - Function parameters
//! - Match arms
//! - For loops
//!
//! Uses hand-written recursive descent parser for fast compilation and good error recovery.

use verum_ast::ty::PathSegment;
use verum_ast::{Ident, Literal, Path, Pattern, PatternKind, pattern::*};
use verum_common::{List, Maybe, Text};
use verum_lexer::{Token, TokenKind};

use crate::error::ParseError;
use crate::parser::{ParseResult, RecursiveParser};

impl<'a> RecursiveParser<'a> {
    /// Parse a pattern with full support for @ bindings and OR patterns.
    ///
    /// Grammar:
    /// ```text
    /// pattern := or_pattern
    /// or_pattern := at_pattern ('|' at_pattern)*
    /// at_pattern := base_pattern ('@' base_pattern)?
    /// ```
    pub fn parse_pattern(&mut self) -> ParseResult<Pattern> {
        self.parse_pattern_impl(true)
    }

    /// Parse a pattern without OR pattern support.
    ///
    /// This is used in contexts like closure parameters where `|` is a delimiter,
    /// not an OR pattern separator.
    pub fn parse_pattern_no_or(&mut self) -> ParseResult<Pattern> {
        self.parse_pattern_internal(false, true, false)
    }

    /// Parse a pattern without allowing struct/record literals.
    ///
    /// This is used in contexts like `is` expressions in if/while conditions
    /// where `{ }` should not be consumed as part of the pattern.
    /// For example: `if value is None { }` - the `{` is the block start, not a record pattern.
    pub fn parse_pattern_no_struct(&mut self) -> ParseResult<Pattern> {
        self.parse_pattern_internal(true, false, false)
    }

    /// Internal pattern parser with configurable OR pattern and struct support.
    fn parse_pattern_impl(&mut self, allow_or: bool) -> ParseResult<Pattern> {
        self.parse_pattern_internal(allow_or, true, false)
    }

    /// Parse a pattern that allows inline guards (used inside parentheses/brackets).
    ///
    /// This is used in nested contexts where `if` should be interpreted as a
    /// pattern-level guard rather than a match arm guard.
    ///
    /// Spec: Rust RFC 3637 - Guard Patterns
    ///
    /// Example:
    /// ```verum
    /// match user.plan() {
    ///     (Plan.Regular if user.credit() >= 100) |
    ///     (Plan.Premium if user.credit() >= 80) => complete(),
    /// }
    /// ```
    fn parse_pattern_allowing_guard(&mut self) -> ParseResult<Pattern> {
        self.parse_pattern_internal(true, true, true)
    }

    /// Internal pattern parser with full configuration.
    ///
    /// Grammar (Spec: Rust RFC 3637):
    /// ```ebnf
    /// pattern = or_pattern ;
    /// or_pattern = guarded_pattern , { '|' , guarded_pattern } ;
    /// guarded_pattern = and_pattern , [ guard ] ;
    /// and_pattern = primary_pattern , { '&' , primary_pattern } ;
    /// primary_pattern = simple_pattern | active_pattern ;
    /// guard = 'if' , expression ;
    /// ```
    ///
    /// Note: `allow_guard` controls whether inline guards are parsed at the pattern level.
    /// When false (default for top-level patterns), `if` is left for match arm guard parsing.
    /// When true (inside parentheses/brackets), `if` creates a PatternKind::Guard node.
    fn parse_pattern_internal(&mut self, allow_or: bool, allow_struct: bool, allow_guard: bool) -> ParseResult<Pattern> {
        self.enter_recursion()?;
        let result = self.parse_pattern_internal_inner(allow_or, allow_struct, allow_guard);
        self.exit_recursion();
        result
    }

    fn parse_pattern_internal_inner(&mut self, allow_or: bool, allow_struct: bool, allow_guard: bool) -> ParseResult<Pattern> {
        let start_pos = self.stream.position();

        // E088: Check for leading pipe at start of pattern
        if self.stream.check(&TokenKind::Pipe) {
            return Err(ParseError::pattern_invalid_let(
                "unexpected leading '|' in pattern",
                self.stream.current_span(),
            ));
        }

        // E081: Check for double leading pipe (||) at start of pattern
        if self.stream.check(&TokenKind::PipePipe) {
            return Err(ParseError::pattern_invalid_slice(
                "use single '|' for or-patterns, not '||'",
                self.stream.current_span(),
            ));
        }

        // Parse first guarded_pattern (or and_pattern if guards not allowed)
        let first = self.parse_cons_pattern_impl(allow_struct, allow_guard)?;

        // Note: We do NOT check for || here because after parsing a pattern,
        // || might be a logical OR operator in the outer expression context.
        // e.g., `value is None || other is None` - the || is expression-level, not pattern-level.

        // Check for OR pattern (only if allowed)
        if allow_or && self.stream.check(&TokenKind::Pipe) {
            let mut patterns = vec![first];

            while self.stream.consume(&TokenKind::Pipe).is_some() {
                // Safety: prevent infinite loop
                if !self.tick() || self.is_aborted() {
                    break;
                }
                // E089: Check for consecutive pipes (empty or-pattern)
                if self.stream.check(&TokenKind::Pipe) {
                    return Err(ParseError::pattern_empty_or(self.stream.current_span()));
                }
                // E087: Check for trailing pipe (pipe followed by => or if)
                // Note: When guards are allowed, `if` is valid after a pattern
                if self.stream.check(&TokenKind::FatArrow) {
                    return Err(ParseError::pattern_invalid_match_arm(
                        "unexpected trailing '|' in pattern",
                        self.stream.current_span(),
                    ));
                }
                if !allow_guard && self.stream.check(&TokenKind::If) {
                    return Err(ParseError::pattern_invalid_match_arm(
                        "unexpected trailing '|' in pattern",
                        self.stream.current_span(),
                    ));
                }
                patterns.push(self.parse_cons_pattern_impl(allow_struct, allow_guard)?);
            }

            let span = self.stream.make_span(start_pos);
            Ok(Pattern::new(
                PatternKind::Or(patterns.into_iter().collect::<List<_>>()),
                span,
            ))
        } else {
            Ok(first)
        }
    }

    /// Parse a cons pattern: `pattern :: pattern` (right-associative).
    ///
    /// The `::` operator destructures stream/cons-list types.
    /// `a :: b :: rest` is parsed as `Cons(a, Cons(b, rest))`.
    fn parse_cons_pattern_impl(&mut self, allow_struct: bool, allow_guard: bool) -> ParseResult<Pattern> {
        let start_pos = self.stream.position();
        let first = self.parse_guarded_pattern_impl(allow_struct, allow_guard)?;

        // Check for :: (cons pattern)
        if self.stream.check(&TokenKind::ColonColon) {
            self.stream.advance(); // consume ::

            // Right-associative: recursively parse the tail as another cons pattern
            let tail = self.parse_cons_pattern_impl(allow_struct, allow_guard)?;

            let span = self.stream.make_span(start_pos);
            Ok(Pattern::new(
                PatternKind::Cons {
                    head: Box::new(first),
                    tail: Box::new(tail),
                },
                span,
            ))
        } else {
            Ok(first)
        }
    }

    /// Parse a guarded pattern (pattern with optional inline guard).
    ///
    /// Grammar: `and_pattern , [ 'if' , expression ]`
    ///
    /// Spec: Rust RFC 3637 - Guard Patterns
    ///
    /// Inline guards allow guards to nest within or-patterns, enabling per-alternative conditions.
    /// The guard is evaluated after the pattern matches and must return Bool.
    ///
    /// # Example
    /// ```verum
    /// match user.plan() {
    ///     (Plan.Regular if user.credit() >= 100) |
    ///     (Plan.Premium if user.credit() >= 80) => complete(),
    ///     _ => error(),
    /// }
    ///
    /// match (x, y) {
    ///     ((Some(a) if a > 0) | (Some(a) if a < -10), b) => process(a, b),
    ///     _ => default(),
    /// }
    /// ```
    ///
    /// # Semantics
    /// - Pattern is matched first, then guard is evaluated
    /// - If guard returns false, the match continues to next alternative
    /// - Variables bound in pattern are available in guard expression
    /// - Guards in or-patterns are evaluated independently per alternative
    fn parse_guarded_pattern_impl(&mut self, allow_struct: bool, allow_guard: bool) -> ParseResult<Pattern> {
        let start_pos = self.stream.position();

        // Parse the inner and_pattern
        let pattern = self.parse_and_pattern_impl(allow_struct)?;

        // Check for inline guard (only when allowed - i.e., inside parens/brackets)
        if allow_guard && self.stream.check(&TokenKind::If) {
            self.stream.advance(); // consume 'if'

            // E091: Check for missing guard expression (if followed by ) or | or ,)
            if matches!(
                self.stream.peek_kind(),
                Some(TokenKind::RParen)
                    | Some(TokenKind::RBracket)
                    | Some(TokenKind::Pipe)
                    | Some(TokenKind::Comma)
            ) {
                return Err(ParseError::pattern_missing_guard(
                    "expected expression after 'if' in guard pattern",
                    self.stream.current_span(),
                ));
            }

            // Parse guard expression with min binding power 4 to stop before delimiters
            // This prevents the guard from consuming `)`, `|`, `,`, etc.
            let guard = self.parse_expr_bp(4)?;

            let span = self.stream.make_span(start_pos);
            Ok(Pattern::new(
                PatternKind::Guard {
                    pattern: Box::new(pattern),
                    guard: Box::new(guard),
                },
                span,
            ))
        } else {
            Ok(pattern)
        }
    }

    /// Parse an AND pattern (pattern combination with &).
    ///
    /// Grammar: `primary_pattern , { '&' , primary_pattern }`
    ///
    /// AND patterns combine multiple patterns that must all match.
    /// Active patterns (user-defined decompositions) can be combined: `Even() & Positive()`
    ///
    /// Example:
    /// ```verum
    /// match n {
    ///     Even() & Positive() => "positive even",
    ///     _ => "other",
    /// }
    /// ```
    fn parse_and_pattern_impl(&mut self, allow_struct: bool) -> ParseResult<Pattern> {
        let start_pos = self.stream.position();

        // Parse first pattern (with @ binding support)
        let first = self.parse_at_pattern_impl(allow_struct)?;

        // Note: We do NOT check for && here because after parsing a pattern,
        // && might be a logical AND operator in the outer expression context.
        // e.g., `value is Some(x) && x > 0` - the && is expression-level, not pattern-level.

        // Check for & pattern combination
        if self.stream.check(&TokenKind::Ampersand) {
            let mut patterns = vec![first];

            while self.stream.consume(&TokenKind::Ampersand).is_some() {
                let amp_span = self.stream.current_span();

                // Safety: prevent infinite loop
                if !self.tick() || self.is_aborted() {
                    break;
                }

                // E076: Check for double ampersand (&&) - invalid in pattern context
                if self.stream.check(&TokenKind::Ampersand) {
                    return Err(ParseError::pattern_invalid_field(
                        "use single '&' for pattern combination, not '&&'",
                        self.stream.current_span(),
                    ));
                }

                // E084: Check for trailing ampersand (nothing valid after &)
                if matches!(
                    self.stream.peek_kind(),
                    Some(TokenKind::FatArrow)
                        | Some(TokenKind::Pipe)
                        | Some(TokenKind::Comma)
                        | Some(TokenKind::RParen)
                        | Some(TokenKind::RBracket)
                        | Some(TokenKind::RBrace)
                        | Some(TokenKind::If)
                        | None
                ) {
                    return Err(ParseError::pattern_invalid_and(
                        "expected pattern after '&' in and-pattern",
                        amp_span,
                    ));
                }

                patterns.push(self.parse_at_pattern_impl(allow_struct)?);
            }

            let span = self.stream.make_span(start_pos);
            Ok(Pattern::new(
                PatternKind::And(patterns.into_iter().collect::<List<_>>()),
                span,
            ))
        } else {
            Ok(first)
        }
    }

    /// Parse a pattern with @ binding support.
    ///
    /// Grammar: `base_pattern ('@' base_pattern)?`
    fn parse_at_pattern(&mut self) -> ParseResult<Pattern> {
        self.parse_at_pattern_impl(true)
    }

    /// Parse a pattern with @ binding support, with configurable struct support.
    fn parse_at_pattern_impl(&mut self, allow_struct: bool) -> ParseResult<Pattern> {
        let start_pos = self.stream.position();
        let base = self.parse_base_pattern_impl(allow_struct)?;

        // Check for @ binding: x @ Some(_)
        if self.stream.consume(&TokenKind::At).is_some() {
            let subpattern = self.parse_base_pattern_impl(allow_struct)?;

            // The base must be an identifier pattern for @ binding
            if let PatternKind::Ident {
                by_ref,
                mutable,
                name,
                ..
            } = base.kind
            {
                let span = self.stream.make_span(start_pos);
                Ok(Pattern::new(
                    PatternKind::Ident {
                        by_ref,
                        mutable,
                        name,
                        subpattern: Some(Box::new(subpattern)),
                    },
                    span,
                ))
            } else {
                // E071: Invalid identifier pattern - non-identifier before @
                Err(ParseError::pattern_invalid_identifier(
                    "@ binding requires an identifier pattern on the left",
                    base.span,
                ))
            }
        } else {
            Ok(base)
        }
    }

    /// Parse a base pattern (all pattern forms except OR and @ binding).
    fn parse_base_pattern(&mut self) -> ParseResult<Pattern> {
        self.parse_base_pattern_impl(true)
    }

    /// Parse a base pattern with configurable struct support.
    fn parse_base_pattern_impl(&mut self, allow_struct: bool) -> ParseResult<Pattern> {
        let start_pos = self.stream.position();

        match self.stream.peek() {
            // Wildcard: _
            Some(Token {
                kind: TokenKind::Ident(name),
                span,
            }) if name.as_str() == "_" => {
                let span = *span;
                self.stream.advance();
                Ok(Pattern::wildcard(span))
            }

            // Rest pattern: .., ..name OR RangeTo pattern: ..10
            Some(Token {
                kind: TokenKind::DotDot,
                ..
            }) => {
                // Lookahead to check what follows: literal (range), identifier (rest binding), or nothing (plain rest)
                let next_token = self.stream.peek_nth(1);
                let next_is_range_literal = next_token
                    .map(|t| matches!(t.kind, TokenKind::Integer(_) | TokenKind::Char(_) | TokenKind::ByteChar(_)))
                    .unwrap_or(false)
                    || (
                        next_token.map(|t| matches!(t.kind, TokenKind::Minus)).unwrap_or(false)
                        && self.stream.peek_nth(2).map(|t| matches!(t.kind, TokenKind::Integer(_))).unwrap_or(false)
                    );
                let next_is_ident = next_token
                    .map(|t| matches!(t.kind, TokenKind::Ident(_)))
                    .unwrap_or(false);

                if next_is_range_literal {
                    // This is a RangeTo pattern: ..10 or ..'\n'
                    self.parse_literal_or_range_pattern()
                } else if next_is_ident {
                    // This is a Rest pattern with binding: ..name
                    self.parse_rest_pattern_with_binding()
                } else {
                    // This is a plain Rest pattern: ..
                    // SAFETY: match arm confirmed DotDot token exists
                    let token = self.stream.advance()
                        .expect("DotDot match arm guarantees token exists");
                    Ok(Pattern::new(PatternKind::Rest, token.span))
                }
            }

            // Inclusive RangeTo pattern: ..=literal (e.g., ..=-1, ..=10, ..='z')
            Some(Token {
                kind: TokenKind::DotDotEq,
                ..
            }) => {
                self.parse_literal_or_range_pattern()
            }

            // Reference pattern: &x or &mut x
            Some(Token {
                kind: TokenKind::Ampersand,
                ..
            }) => self.parse_reference_pattern(),

            // Tuple or parenthesized pattern: (a, b) or (pattern)
            Some(Token {
                kind: TokenKind::LParen,
                ..
            }) => self.parse_tuple_or_paren_pattern(),

            // Array or slice pattern: [a, b, c] or [a, .., b]
            Some(Token {
                kind: TokenKind::LBracket,
                ..
            }) => self.parse_array_or_slice_pattern(),

            // Stream pattern: stream[first, second, ...rest] or stream[]
            // Consumes elements lazily from an iterator (unlike fixed-size slice patterns)
            Some(Token {
                kind: TokenKind::Stream,
                ..
            }) if self.stream.peek_nth_kind(1) == Some(&TokenKind::LBracket) => {
                self.parse_stream_pattern()
            }

            // Negative integer literal or range pattern: -42, -42..10, -42..=-5, ..=-1
            Some(Token {
                kind: TokenKind::Minus,
                ..
            }) if matches!(
                self.stream.peek_nth(1).map(|t| &t.kind),
                Some(TokenKind::Integer(_))
            ) => {
                self.parse_literal_or_range_pattern()
            }

            // Negative float literal pattern: -0.0, -1.5, -3.14, etc. (no range support)
            Some(Token {
                kind: TokenKind::Minus,
                ..
            }) if matches!(
                self.stream.peek_nth(1).map(|t| &t.kind),
                Some(TokenKind::Float(_))
            ) => {
                let start_pos = self.stream.position();
                self.stream.advance(); // consume '-'
                let float_token = self.stream.peek()
                    .ok_or_else(|| ParseError::unexpected_eof(&[], self.stream.current_span()))?;
                if let TokenKind::Float(val) = &float_token.kind {
                    let neg_val = -val.value;
                    let span = self.stream.make_span(start_pos);
                    self.stream.advance(); // consume float
                    Ok(Pattern::literal(Literal::float(neg_val, span)))
                } else {
                    Err(ParseError::invalid_syntax("expected float literal after '-'", self.stream.current_span()))
                }
            }

            // Literal patterns that can form ranges (integer and char)
            Some(Token {
                kind: TokenKind::Integer(_),
                ..
            })
            | Some(Token {
                kind: TokenKind::Char(_),
                ..
            })
            | Some(Token {
                kind: TokenKind::ByteChar(_),
                ..
            }) => self.parse_literal_or_range_pattern(),
            // Other literal patterns (no range support)
            // E084: Check if these are followed by range operators (invalid range bounds)
            Some(Token {
                kind: TokenKind::Float(_),
                span,
                ..
            })
            | Some(Token {
                kind: TokenKind::Text(_),
                span,
                ..
            })
            | Some(Token {
                kind: TokenKind::True,
                span,
                ..
            })
            | Some(Token {
                kind: TokenKind::False,
                span,
                ..
            }) => {
                let span = *span;
                let lit = self.parse_literal_pattern()?;
                // E084: Check if followed by range operator (invalid range bound type)
                if self.stream.check(&TokenKind::DotDot) || self.stream.check(&TokenKind::DotDotEq) {
                    return Err(ParseError::pattern_invalid_and(
                        "only integer and character literals can be used as range bounds",
                        span,
                    ));
                }
                Ok(lit)
            }

            // Identifier, variant, or record pattern
            Some(Token {
                kind: TokenKind::Ident(_),
                ..
            })
            | Some(Token {
                kind: TokenKind::Ref,
                ..
            })
            | Some(Token {
                kind: TokenKind::Mut,
                ..
            })
            | Some(Token {
                kind: TokenKind::Some,
                ..
            })
            | Some(Token {
                kind: TokenKind::None,
                ..
            })
            | Some(Token {
                kind: TokenKind::Ok,
                ..
            })
            | Some(Token {
                kind: TokenKind::Err,
                ..
            })
            | Some(Token {
                kind: TokenKind::Result,
                ..
            })
            // Contract keywords as contextual identifiers in patterns
            | Some(Token {
                kind: TokenKind::Invariant,
                ..
            })
            | Some(Token {
                kind: TokenKind::Requires,
                ..
            })
            | Some(Token {
                kind: TokenKind::Ensures,
                ..
            })
            | Some(Token {
                kind: TokenKind::Forall,
                ..
            })
            | Some(Token {
                kind: TokenKind::Exists,
                ..
            })
            // Comprehension keywords as contextual identifiers in patterns
            | Some(Token {
                kind: TokenKind::Gen,
                ..
            })
            | Some(Token {
                kind: TokenKind::Set,
                ..
            })
            // Async/stream keywords as contextual identifiers in patterns
            | Some(Token {
                kind: TokenKind::Stream,
                ..
            })
            | Some(Token {
                kind: TokenKind::Spawn,
                ..
            })
            | Some(Token {
                kind: TokenKind::Select,
                ..
            })
            | Some(Token {
                kind: TokenKind::Yield,
                ..
            })
            // Structured concurrency keyword as contextual identifier in patterns
            | Some(Token {
                kind: TokenKind::Nursery,
                ..
            })
            // Context system keywords as contextual identifiers in patterns
            | Some(Token {
                kind: TokenKind::Context,
                ..
            })
            | Some(Token {
                kind: TokenKind::Recover,
                ..
            })
            // Pattern matching keyword as contextual identifier in patterns
            | Some(Token {
                kind: TokenKind::ActivePattern,
                ..
            })
            // Meta keyword as contextual identifier in patterns
            // Common in filesystem code: Ok(meta) => meta.is_file()
            | Some(Token {
                kind: TokenKind::Meta,
                ..
            })
            // Protocol keyword as contextual identifier in patterns
            // Common in networking code: fn socket(domain: Int, socket_type: Int, protocol: Int)
            | Some(Token {
                kind: TokenKind::Protocol,
                ..
            })
            // Internal keyword as contextual identifier in patterns
            // Common in FFI structs: internal: Int, internal_high: Int
            | Some(Token {
                kind: TokenKind::Internal,
                ..
            })
            // Protected keyword as contextual identifier in patterns
            // Common in stats structs: protected_count: Int
            | Some(Token {
                kind: TokenKind::Protected,
                ..
            })
            // Volatile keyword as contextual identifier in patterns
            // Common in FFI: volatile: bool
            | Some(Token {
                kind: TokenKind::Volatile,
                ..
            })
            // Layer keyword as contextual identifier in patterns
            | Some(Token {
                kind: TokenKind::Layer,
                ..
            })
            // Extends keyword as contextual identifier in patterns
            | Some(Token {
                kind: TokenKind::Extends,
                ..
            })
            // Implement keyword as contextual identifier in patterns
            | Some(Token {
                kind: TokenKind::Implement,
                ..
            })
            // Implies keyword as contextual identifier in patterns
            | Some(Token {
                kind: TokenKind::Implies,
                ..
            })
            // Stage keyword as contextual identifier in patterns
            | Some(Token {
                kind: TokenKind::Stage,
                ..
            })
            // Pure keyword as contextual identifier in patterns
            | Some(Token {
                kind: TokenKind::Pure,
                ..
            })
            // Proof keywords as contextual identifiers in patterns
            | Some(Token {
                kind: TokenKind::Show,
                ..
            })
            | Some(Token {
                kind: TokenKind::Have,
                ..
            })
            | Some(Token {
                kind: TokenKind::Theorem,
                ..
            })
            | Some(Token {
                kind: TokenKind::Axiom,
                ..
            })
            | Some(Token {
                kind: TokenKind::Lemma,
                ..
            })
            | Some(Token {
                kind: TokenKind::Corollary,
                ..
            })
            | Some(Token {
                kind: TokenKind::Proof,
                ..
            })
            | Some(Token {
                kind: TokenKind::Calc,
                ..
            })
            | Some(Token {
                kind: TokenKind::Suffices,
                ..
            })
            | Some(Token {
                kind: TokenKind::Obtain,
                ..
            })
            | Some(Token {
                kind: TokenKind::By,
                ..
            })
            | Some(Token {
                kind: TokenKind::Induction,
                ..
            })
            | Some(Token {
                kind: TokenKind::Cases,
                ..
            })
            | Some(Token {
                kind: TokenKind::Contradiction,
                ..
            })
            | Some(Token {
                kind: TokenKind::Trivial,
                ..
            })
            | Some(Token {
                kind: TokenKind::Assumption,
                ..
            })
            | Some(Token {
                kind: TokenKind::Simp,
                ..
            })
            | Some(Token {
                kind: TokenKind::Ring,
                ..
            })
            | Some(Token {
                kind: TokenKind::Field,
                ..
            })
            | Some(Token {
                kind: TokenKind::Omega,
                ..
            })
            | Some(Token {
                kind: TokenKind::Auto,
                ..
            })
            | Some(Token {
                kind: TokenKind::Blast,
                ..
            })
            | Some(Token {
                kind: TokenKind::Smt,
                ..
            })
            | Some(Token {
                kind: TokenKind::Qed,
                ..
            })
            // Provide/Using keywords as contextual identifiers
            | Some(Token {
                kind: TokenKind::Provide,
                ..
            })
            | Some(Token {
                kind: TokenKind::Using,
                ..
            })
            // Module system keywords as contextual identifiers
            | Some(Token {
                kind: TokenKind::Module,
                ..
            })
            | Some(Token {
                kind: TokenKind::Cog,
                ..
            })
            | Some(Token {
                kind: TokenKind::Super,
                ..
            })
            // Await keyword as contextual identifier
            | Some(Token {
                kind: TokenKind::Await,
                ..
            })
            // Link keyword as contextual identifier (common in HTML/networking code)
            | Some(Token {
                kind: TokenKind::Link,
                ..
            }) => self.parse_ident_or_variant_or_record_pattern_impl(allow_struct),

            // E079: Comma at pattern start (invalid in variant, tuple, array patterns)
            Some(Token {
                kind: TokenKind::Comma,
                ..
            }) => {
                let span = self.stream.current_span();
                Err(ParseError::pattern_or_binding(
                    "unexpected comma; expected pattern or ')' or ']'",
                    span,
                ))
            }

            // E070: @ without preceding identifier
            Some(Token {
                kind: TokenKind::At,
                ..
            }) => {
                let span = self.stream.current_span();
                Err(ParseError::pattern_invalid_at(
                    "@ binding requires an identifier before the @ symbol",
                    span,
                ))
            }

            // E071: Reserved keywords used as pattern identifiers
            Some(Token {
                kind: TokenKind::Let,
                span,
            }) => {
                let span = *span;
                Err(ParseError::pattern_invalid_identifier(
                    "'let' is a reserved keyword and cannot be used as a pattern identifier",
                    span,
                ))
            }
            Some(Token {
                kind: TokenKind::Fn,
                span,
            }) => {
                let span = *span;
                Err(ParseError::pattern_invalid_identifier(
                    "'fn' is a reserved keyword and cannot be used as a pattern identifier",
                    span,
                ))
            }
            Some(Token {
                kind: TokenKind::Is,
                span,
            }) => {
                let span = *span;
                Err(ParseError::pattern_invalid_identifier(
                    "'is' is a reserved keyword and cannot be used as a pattern identifier",
                    span,
                ))
            }

            // E086: Triple dots '...' used instead of double dots '..'
            Some(Token {
                kind: TokenKind::DotDotDot,
                span,
            }) => {
                let span = *span;
                Err(ParseError::pattern_invalid_slice_syntax(
                    "use '..' instead of '...' for rest patterns",
                    span,
                ))
            }

            _ => {
                let span = self.stream.current_span();
                Err(ParseError::invalid_syntax("expected pattern", span))
            }
        }
    }

    /// Parse a rest pattern with binding: ..name
    /// This creates an Ident pattern with a Rest subpattern
    fn parse_rest_pattern_with_binding(&mut self) -> ParseResult<Pattern> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::DotDot)?;

        let name = self.consume_ident_or_keyword()?;
        let span = self.stream.make_span(start_pos);
        let name_span = self.stream.current_span();

        // Create an identifier pattern with Rest as subpattern
        // This encoding allows us to distinguish ..name from a regular identifier
        Ok(Pattern::new(
            PatternKind::Ident {
                by_ref: false,
                mutable: false,
                name: Ident::new(name, name_span),
                subpattern: Some(Box::new(Pattern::new(PatternKind::Rest, span))),
            },
            span,
        ))
    }

    /// Parse a reference pattern: &x or &mut x
    fn parse_reference_pattern(&mut self) -> ParseResult<Pattern> {
        let start_pos = self.stream.position();

        self.stream.expect(TokenKind::Ampersand)?;

        // E082: Check for double ampersand &&x
        if self.stream.check(&TokenKind::Ampersand) {
            return Err(ParseError::pattern_invalid_unicode(
                "double '&' in reference pattern; use single '&' for reference patterns",
                self.stream.current_span(),
            ));
        }

        // E082: Check for bare & without pattern (& =>)
        if self.stream.check(&TokenKind::FatArrow) || self.stream.check(&TokenKind::Pipe) {
            return Err(ParseError::pattern_invalid_unicode(
                "reference pattern '&' requires a pattern to follow",
                self.stream.current_span(),
            ));
        }

        let mutable = self.stream.consume(&TokenKind::Mut).is_some();

        // E082: Check for &mut followed by literal (not allowed)
        if mutable {
            if let Some(token) = self.stream.peek() {
                if matches!(
                    token.kind,
                    TokenKind::Integer(_) | TokenKind::Char(_) | TokenKind::ByteChar(_)
                        | TokenKind::Float(_) | TokenKind::True | TokenKind::False
                ) {
                    return Err(ParseError::pattern_invalid_unicode(
                        "&mut cannot be used with literal patterns; literals are immutable",
                        token.span,
                    ));
                }
            }
        }

        let inner = self.parse_base_pattern()?;

        let span = self.stream.make_span(start_pos);
        Ok(Pattern::new(
            PatternKind::Reference {
                mutable,
                inner: Box::new(inner),
            },
            span,
        ))
    }

    /// Parse tuple or parenthesized pattern: (a, b, c) or (pattern)
    ///
    /// Inside parentheses, guard patterns are allowed:
    /// ```verum
    /// match value {
    ///     (x if x > 0) | (y if y < 0) => process(),
    ///     _ => default(),
    /// }
    /// ```
    fn parse_tuple_or_paren_pattern(&mut self) -> ParseResult<Pattern> {
        let start_pos = self.stream.position();

        self.stream.expect(TokenKind::LParen)?;

        // Handle empty tuple: ()
        if self.stream.consume(&TokenKind::RParen).is_some() {
            let span = self.stream.make_span(start_pos);
            return Ok(Pattern::new(PatternKind::Tuple(List::new()), span));
        }

        // E074: Check for leading comma: (,)
        if self.stream.check(&TokenKind::Comma) {
            return Err(ParseError::pattern_empty_tuple(self.stream.current_span()));
        }

        // Use parse_pattern_allowing_guard for nested patterns inside ()
        // This enables guard patterns like: (x if x > 0)
        let mut patterns = vec![self.parse_pattern_allowing_guard()?];

        // Check if this is a tuple (has comma) or just parenthesized
        let mut is_tuple = false;
        while self.stream.consume(&TokenKind::Comma).is_some() {
            // Safety: prevent infinite loop
            if !self.tick() || self.is_aborted() {
                break;
            }
            is_tuple = true;
            // Allow trailing comma
            if self.stream.check(&TokenKind::RParen) {
                break;
            }
            // E074: Check for consecutive commas: (a,, b)
            if self.stream.check(&TokenKind::Comma) {
                return Err(ParseError::pattern_empty_tuple(self.stream.current_span()));
            }
            patterns.push(self.parse_pattern_allowing_guard()?);
        }

        // E072: Unclosed tuple pattern - check for missing closing paren
        if !self.stream.check(&TokenKind::RParen) {
            return Err(ParseError::pattern_invalid_rest(
                "unclosed tuple pattern, expected ')'",
                self.stream.current_span(),
            ));
        }
        self.stream.advance(); // consume )
        let span = self.stream.make_span(start_pos);

        if is_tuple || patterns.len() > 1 {
            // Tuple pattern
            Ok(Pattern::new(
                PatternKind::Tuple(patterns.into_iter().collect::<List<_>>()),
                span,
            ))
        } else {
            // Parenthesized pattern - we know patterns has exactly 1 element
            let pattern = patterns.into_iter().next().ok_or_else(|| {
                ParseError::invalid_syntax(
                    "parenthesized pattern must contain exactly one pattern",
                    span,
                )
            })?;
            Ok(Pattern::new(PatternKind::Paren(Box::new(pattern)), span))
        }
    }

    /// Parse array or slice pattern: [a, b, c] or [a, .., b]
    ///
    /// Inside brackets, guard patterns are allowed:
    /// ```verum
    /// match list {
    ///     [first if first > 0, .., last if last < 100] => process(),
    ///     _ => default(),
    /// }
    /// ```
    fn parse_array_or_slice_pattern(&mut self) -> ParseResult<Pattern> {
        let start_pos = self.stream.position();

        self.stream.expect(TokenKind::LBracket)?;

        // Handle empty array: []
        if self.stream.consume(&TokenKind::RBracket).is_some() {
            let span = self.stream.make_span(start_pos);
            return Ok(Pattern::new(PatternKind::Array(List::new()), span));
        }

        // Use parse_pattern_allowing_guard for nested patterns inside []
        // This enables guard patterns like: [x if x > 0, .., y]
        let patterns = self.comma_separated(|p| p.parse_pattern_allowing_guard())?;
        // E073: Check for unclosed array pattern
        if !self.stream.check(&TokenKind::RBracket) {
            return Err(ParseError::pattern_invalid_mut(
                "unclosed array pattern, expected ']'",
                self.stream.current_span(),
            ));
        }
        self.stream.advance(); // consume ]
        let span = self.stream.make_span(start_pos);

        // Check if any pattern is a rest pattern (..) or rest with binding (..name)
        // Rest with binding is encoded as Ident { subpattern: Some(Rest), .. }
        let patterns_vec: Vec<_> = patterns;

        // Helper to check if a pattern is a rest pattern
        let is_rest_pattern = |p: &Pattern| -> bool {
            match &p.kind {
                PatternKind::Rest => true,
                PatternKind::Ident { subpattern, .. } => {
                    matches!(
                        subpattern,
                        Maybe::Some(boxed) if matches!(boxed.kind, PatternKind::Rest)
                    )
                }
                _ => false,
            }
        };

        // E077: Check for multiple rest patterns (duplicate rest)
        let rest_count = patterns_vec.iter().filter(|p| is_rest_pattern(p)).count();
        if rest_count > 1 {
            return Err(ParseError::pattern_duplicate_field(
                "multiple rest patterns '..' in array pattern",
                span,
            ));
        }

        // Find rest pattern position (either plain Rest or Ident with Rest subpattern)
        let rest_pos = patterns_vec.iter().position(is_rest_pattern);

        if let Some(rest_pos) = rest_pos {
            // This is a slice pattern
            let before: List<_> = patterns_vec[..rest_pos].to_vec().into();
            let after: List<_> = patterns_vec[rest_pos + 1..].to_vec().into();

            // Extract rest binding if present
            let rest_binding = match &patterns_vec[rest_pos].kind {
                PatternKind::Ident {
                    name,
                    subpattern: Maybe::Some(b),
                    ..
                } if matches!(b.kind, PatternKind::Rest) => {
                    // This is a rest with binding (..name)
                    // Create a simple identifier pattern for the binding
                    let binding_span = name.span;
                    Some(Box::new(Pattern::new(
                        PatternKind::Ident {
                            by_ref: false,
                            mutable: false,
                            name: name.clone(),
                            subpattern: Maybe::None,
                        },
                        binding_span,
                    )))
                }
                PatternKind::Rest => {
                    // Plain rest without binding
                    Maybe::None
                }
                _ => Maybe::None,
            };

            Ok(Pattern::new(
                PatternKind::Slice {
                    before,
                    rest: rest_binding,
                    after,
                },
                span,
            ))
        } else {
            // Regular array pattern
            Ok(Pattern::new(
                PatternKind::Array(patterns_vec.into_iter().collect::<List<_>>()),
                span,
            ))
        }
    }

    /// Parse stream pattern: stream[first, second, ...rest] or stream[]
    /// Stream patterns consume elements lazily from an iterator.
    /// Grammar: stream_pattern = 'stream' , '[' , { stream_element } , ']' ;
    ///
    /// Unlike slice patterns which work on fixed collections, stream patterns
    /// consume elements lazily from an iterator.
    ///
    /// Syntax variants:
    /// - `stream[]`                   -> empty stream (matches exhausted iterator)
    /// - `stream[first, second, ...rest]` -> consume head, bind rest as remaining iterator
    /// - `stream[head, ...tail]`      -> consume one, tail is remaining
    /// - `stream[a, b, c]`            -> exact count match (must have exactly 3 elements)
    /// - `stream[first, second, ...]` -> consume and discard rest
    fn parse_stream_pattern(&mut self) -> ParseResult<Pattern> {
        let start_pos = self.stream.position();

        self.stream.expect(TokenKind::Stream)?;
        self.stream.expect(TokenKind::LBracket)?;

        // Empty stream: stream[]
        if self.stream.consume(&TokenKind::RBracket).is_some() {
            let span = self.stream.make_span(start_pos);
            return Ok(Pattern::new(
                PatternKind::Stream {
                    head_patterns: List::new(),
                    rest: Maybe::None,
                },
                span,
            ));
        }

        // E083: Check for leading comma in stream pattern: stream[,]
        if self.stream.check(&TokenKind::Comma) {
            return Err(ParseError::pattern_invalid_variant_args(
                "unexpected leading comma in stream pattern",
                self.stream.current_span(),
            ));
        }

        // E083: Check for '..' instead of '...' (wrong rest syntax): stream[..rest]
        if self.stream.check(&TokenKind::DotDot) {
            return Err(ParseError::pattern_invalid_variant_args(
                "use '...' instead of '..' for stream rest patterns",
                self.stream.current_span(),
            ));
        }

        // Check for leading ...rest pattern: stream[...rest]
        if self.stream.check(&TokenKind::DotDotDot) {
            self.stream.advance(); // consume '...'
            let rest_name_text = self.consume_ident()?;
            let rest_span = self.stream.current_span();
            self.stream.expect(TokenKind::RBracket)?;
            let span = self.stream.make_span(start_pos);
            return Ok(Pattern::new(
                PatternKind::Stream {
                    head_patterns: List::new(),
                    rest: Maybe::Some(Ident::new(rest_name_text, rest_span)),
                },
                span,
            ));
        }

        // Parse head patterns
        let mut head_patterns = Vec::new();
        let mut rest: Maybe<Ident> = Maybe::None;

        loop {
            // E083: Check for '..' instead of '...' (wrong rest syntax in loop)
            if self.stream.check(&TokenKind::DotDot) {
                return Err(ParseError::pattern_invalid_variant_args(
                    "use '...' instead of '..' for stream rest patterns",
                    self.stream.current_span(),
                ));
            }

            // Check for ...rest at current position
            if self.stream.check(&TokenKind::DotDotDot) {
                self.stream.advance(); // consume '...'
                // Optional identifier after ... (if just '...' we discard rest)
                if matches!(self.stream.peek_kind(), Some(TokenKind::Ident(_))) {
                    let rest_name_text = self.consume_ident()?;
                    let rest_span = self.stream.current_span();
                    rest = Maybe::Some(Ident::new(rest_name_text, rest_span));
                }
                // After ...rest, must be ]
                break;
            }

            // Check for end
            if self.stream.check(&TokenKind::RBracket) {
                break;
            }

            // Parse a pattern
            head_patterns.push(self.parse_pattern()?);

            // Check for comma
            if self.stream.consume(&TokenKind::Comma).is_some() {
                // Continue to next pattern or ...rest
            } else {
                // No comma, must be end
                break;
            }
        }

        // E083: Unclosed stream pattern
        if !self.stream.check(&TokenKind::RBracket) {
            return Err(ParseError::pattern_invalid_variant_args(
                "unclosed stream pattern, expected ']'",
                self.stream.current_span(),
            ));
        }
        self.stream.advance(); // consume ]
        let span = self.stream.make_span(start_pos);

        Ok(Pattern::new(
            PatternKind::Stream {
                head_patterns: head_patterns.into(),
                rest,
            },
            span,
        ))
    }

    /// Parse a literal pattern: 42, "hello", true, false
    fn parse_literal_pattern(&mut self) -> ParseResult<Pattern> {
        let token = self
            .stream
            .peek()
            .ok_or_else(|| ParseError::unexpected_eof(&[], self.stream.current_span()))?;

        let lit = match &token.kind {
            TokenKind::Integer(val) => {
                let lit = Literal::int(val.as_i128().unwrap_or(0), token.span);
                self.stream.advance();
                lit
            }
            TokenKind::Float(val) => {
                let lit = Literal::float(val.value, token.span);
                self.stream.advance();
                lit
            }
            TokenKind::Text(val) => {
                let lit = Literal::string(Text::from(val.to_string()), token.span);
                self.stream.advance();
                lit
            }
            TokenKind::Char(val) => {
                let lit = Literal::char(*val, token.span);
                self.stream.advance();
                lit
            }
            TokenKind::ByteChar(val) => {
                let lit = Literal::byte_char(*val, token.span);
                self.stream.advance();
                lit
            }
            TokenKind::True => {
                let lit = Literal::bool(true, token.span);
                self.stream.advance();
                lit
            }
            TokenKind::False => {
                let lit = Literal::bool(false, token.span);
                self.stream.advance();
                lit
            }
            _ => {
                return Err(ParseError::invalid_syntax("expected literal", token.span));
            }
        };

        Ok(Pattern::literal(lit))
    }

    /// Parse literal or range pattern: 42 or 1..10 or 1..=10 or 100.. or ..10
    fn parse_literal_or_range_pattern(&mut self) -> ParseResult<Pattern> {
        let start_pos = self.stream.position();

        // Extract token info before any mutable operations
        let (token_kind, token_span) = {
            let token = self.stream.peek()
                .ok_or_else(|| ParseError::unexpected_eof(&[], self.stream.current_span()))?;
            (token.kind.clone(), token.span)
        };

        // Parse the start literal (integer, negative integer, or char for ranges)
        let start_lit = match &token_kind {
            TokenKind::Integer(val) => {
                let lit = Literal::int(val.as_i128().unwrap_or(0), token_span);
                self.stream.advance();
                Some(lit)
            }
            TokenKind::Char(c) => {
                let lit = Literal::char(*c, token_span);
                self.stream.advance();
                Some(lit)
            }
            TokenKind::ByteChar(b) => {
                let lit = Literal::byte_char(*b, token_span);
                self.stream.advance();
                Some(lit)
            }
            TokenKind::Minus => {
                // Negative integer literal: -42
                let neg_val = if let Some(Token { kind: TokenKind::Integer(val), .. }) = self.stream.peek_nth(1) {
                    Some(-(val.as_i128().unwrap_or(0)))
                } else {
                    None
                };
                if let Some(neg_val) = neg_val {
                    self.stream.advance(); // consume '-'
                    self.stream.advance(); // consume the integer
                    let lit = Literal::int(neg_val, self.stream.make_span(start_pos));
                    Some(lit)
                } else {
                    None
                }
            }
            _ => None,
        };

        // E082: Check for triple dots '...' (invalid range syntax like 1...10)
        if self.stream.check(&TokenKind::DotDotDot) {
            return Err(ParseError::pattern_invalid_unicode(
                "use '..' or '..=' instead of '...' for range patterns",
                self.stream.current_span(),
            ));
        }

        // Check for range operators
        if self.stream.check(&TokenKind::DotDot) || self.stream.check(&TokenKind::DotDotEq) {
            let inclusive = self.stream.consume(&TokenKind::DotDotEq).is_some();
            if !inclusive {
                self.stream.expect(TokenKind::DotDot)?;
            }

            // E082: Check for exclusive range operator followed by < (invalid syntax like 1..<)
            if self.stream.check(&TokenKind::Lt) {
                return Err(ParseError::pattern_invalid_unicode(
                    "invalid exclusive range syntax '..<'; use '..end' for exclusive range",
                    self.stream.current_span(),
                ));
            }

            // Parse end literal (required in pattern context - open-ended ranges not allowed)
            let end_lit = if let Some(end_token) = self.stream.peek() {
                match &end_token.kind {
                    TokenKind::Integer(val) => {
                        let lit = Literal::int(val.as_i128().unwrap_or(0), end_token.span);
                        self.stream.advance();
                        Some(lit)
                    }
                    TokenKind::Char(c) => {
                        let lit = Literal::char(*c, end_token.span);
                        self.stream.advance();
                        Some(lit)
                    }
                    TokenKind::ByteChar(b) => {
                        let lit = Literal::byte_char(*b, end_token.span);
                        self.stream.advance();
                        Some(lit)
                    }
                    TokenKind::Minus => {
                        // Negative integer end literal: ..=-1, 0..=-5
                        let end_pos = self.stream.position();
                        let neg_val = if let Some(Token { kind: TokenKind::Integer(val), .. }) = self.stream.peek_nth(1) {
                            Some(-(val.as_i128().unwrap_or(0)))
                        } else {
                            None
                        };
                        if let Some(neg_val) = neg_val {
                            self.stream.advance(); // consume '-'
                            self.stream.advance(); // consume the integer
                            let lit = Literal::int(neg_val, self.stream.make_span(end_pos));
                            Some(lit)
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            } else {
                None
            };

            // E084: Check for mixed range bound types (e.g., 1..'z')
            if let (Some(s), Some(e)) = (&start_lit, &end_lit) {
                let start_is_int = matches!(s.kind, verum_ast::LiteralKind::Int(_));
                let end_is_int = matches!(e.kind, verum_ast::LiteralKind::Int(_));
                let start_is_char = matches!(s.kind, verum_ast::LiteralKind::Char(_) | verum_ast::LiteralKind::ByteChar(_));
                let end_is_char = matches!(e.kind, verum_ast::LiteralKind::Char(_) | verum_ast::LiteralKind::ByteChar(_));

                if (start_is_int && end_is_char) || (start_is_char && end_is_int) {
                    return Err(ParseError::pattern_invalid_and(
                        "range bounds must be of the same type (both integers or both characters)",
                        self.stream.make_span(start_pos),
                    ));
                }
            }

            let span = self.stream.make_span(start_pos);

            Ok(Pattern::new(
                PatternKind::Range {
                    start: start_lit.map(Box::new),
                    end: end_lit.map(Box::new),
                    inclusive,
                },
                span,
            ))
        } else if let Some(start_lit) = start_lit {
            // Just a literal pattern
            Ok(Pattern::literal(start_lit))
        } else {
            // No start literal and no range operator - this shouldn't happen
            // as this function should only be called when we have an integer or range
            Err(ParseError::invalid_syntax(
                "expected integer or character literal or range pattern",
                token_span,
            ))
        }
    }

    /// Parse identifier, variant, or record pattern.
    ///
    /// This handles:
    /// - Plain identifier: `x`, `mut x`, `ref x`, `ref mut x`
    /// - Tuple variant: `Some(x)`, `Ok(value)`
    /// - Record pattern: `Point { x, y }`, `Point { x: px, .. }`
    /// - Qualified variant: `Operator::Add`, `Color::Red`, `Maybe::Some`, `std::option::Maybe::Some`
    fn parse_ident_or_variant_or_record_pattern(&mut self) -> ParseResult<Pattern> {
        self.parse_ident_or_variant_or_record_pattern_impl(true)
    }

    /// Parse identifier, variant, or record pattern with configurable struct support.
    ///
    /// When `allow_struct` is false, `{ }` is NOT consumed as part of the pattern.
    /// This is used for `is` expressions in if/while conditions where `{ }` starts the block.
    fn parse_ident_or_variant_or_record_pattern_impl(
        &mut self,
        allow_struct: bool,
    ) -> ParseResult<Pattern> {
        let start_pos = self.stream.position();

        // Check for `ref` modifier (must come before `mut`)
        let by_ref = self.stream.consume(&TokenKind::Ref).is_some();

        // E072: Check for invalid ref patterns
        if by_ref {
            // Check for ref alone (no identifier following): ref =>
            if matches!(
                self.stream.peek_kind(),
                Some(TokenKind::FatArrow) | Some(TokenKind::Comma) | Some(TokenKind::RParen)
                    | Some(TokenKind::RBrace) | Some(TokenKind::RBracket) | None
            ) {
                return Err(ParseError::pattern_invalid_rest(
                    "'ref' must be followed by an identifier",
                    self.stream.current_span(),
                ));
            }
            // Check for ref with literal: ref 42
            if matches!(
                self.stream.peek_kind(),
                Some(TokenKind::Integer(_))
                    | Some(TokenKind::Float(_))
                    | Some(TokenKind::Text(_))
                    | Some(TokenKind::Char(_))
                    | Some(TokenKind::True)
                    | Some(TokenKind::False)
            ) {
                return Err(ParseError::pattern_invalid_rest(
                    "'ref' cannot be used with literals",
                    self.stream.current_span(),
                ));
            }
            // Check for ref with wildcard: ref _
            if self.stream.peek_kind() == Some(&TokenKind::Ident(Text::from("_"))) {
                return Err(ParseError::pattern_invalid_rest(
                    "'ref' cannot be used with wildcard '_'",
                    self.stream.current_span(),
                ));
            }
            // Check for double ref: ref ref x
            if self.stream.check(&TokenKind::Ref) {
                return Err(ParseError::pattern_invalid_rest(
                    "duplicate 'ref' modifier in pattern",
                    self.stream.current_span(),
                ));
            }
        }

        // Check for `mut` modifier
        let mutable = self.stream.consume(&TokenKind::Mut).is_some();

        // E072: Check for ref mut mut (after consuming mut)
        if by_ref && mutable {
            if self.stream.check(&TokenKind::Mut) {
                return Err(ParseError::pattern_invalid_rest(
                    "'ref mut' cannot be followed by another 'mut'",
                    self.stream.current_span(),
                ));
            }
        }

        // If we have ref or mut modifiers, we're definitely parsing a binding pattern,
        // not a variant pattern. Parse just the identifier.
        if by_ref || mutable {
            // E073: Check for invalid mut patterns
            if mutable {
                // Check for double mut: mut mut x
                if self.stream.check(&TokenKind::Mut) {
                    return Err(ParseError::pattern_invalid_mut(
                        "duplicate 'mut' modifier in pattern",
                        self.stream.current_span(),
                    ));
                }
                // Check for wrong order: mut ref x (should be ref mut x)
                if self.stream.check(&TokenKind::Ref) {
                    return Err(ParseError::pattern_invalid_mut(
                        "'mut' must come after 'ref' (use 'ref mut' instead of 'mut ref')",
                        self.stream.current_span(),
                    ));
                }
                // Check for mut with wildcard: mut _
                if self.stream.peek_kind() == Some(&TokenKind::Ident(Text::from("_"))) {
                    return Err(ParseError::pattern_invalid_mut(
                        "'mut' cannot be used with wildcard '_'",
                        self.stream.current_span(),
                    ));
                }
                // Check for mut with literal: mut 42
                if matches!(
                    self.stream.peek_kind(),
                    Some(TokenKind::Integer(_))
                        | Some(TokenKind::Float(_))
                        | Some(TokenKind::Text(_))
                        | Some(TokenKind::Char(_))
                        | Some(TokenKind::True)
                        | Some(TokenKind::False)
                ) {
                    return Err(ParseError::pattern_invalid_mut(
                        "'mut' cannot be used with literals",
                        self.stream.current_span(),
                    ));
                }
            }
            // E079: Check for ref/mut followed by rest pattern: ref ..rest or mut ..rest
            if self.stream.check(&TokenKind::DotDot) {
                return Err(ParseError::pattern_or_binding(
                    "'ref' and 'mut' cannot be used with rest patterns '..'; use '..name' instead",
                    self.stream.current_span(),
                ));
            }
            let name = self.consume_ident_or_keyword()?;
            let span = self.stream.make_span(start_pos);
            return Ok(Pattern::new(
                PatternKind::Ident {
                    by_ref,
                    mutable,
                    name: Ident::new(name, span),
                    subpattern: Maybe::None,
                },
                span,
            ));
        }

        // Parse the first identifier (or keyword that can be used as identifier)
        let first_name = self.consume_ident_or_keyword()?;
        let first_span = self.stream.make_span(start_pos);

        // Check for type test pattern: identifier 'is' type
        // Type test pattern: `identifier is Type` — tests if value has given type
        // Example: x is Int, s is Text, u is User
        if self.stream.check(&TokenKind::Is) {
            self.stream.advance(); // consume 'is'

            // E085: Check for missing type after 'is' (x is =>)
            if self.stream.check(&TokenKind::FatArrow)
                || self.stream.check(&TokenKind::Pipe)
                || self.stream.check(&TokenKind::Comma)
                || self.stream.check(&TokenKind::RBrace)
                || self.stream.check(&TokenKind::RParen)
            {
                return Err(ParseError::pattern_trailing_pipe(self.stream.current_span()));
            }

            // E085: Check for double 'is' (x is is Type)
            if self.stream.check(&TokenKind::Is) {
                return Err(ParseError::pattern_missing_guard(
                    "duplicate 'is' keyword in type test pattern",
                    self.stream.current_span(),
                ));
            }

            // E085: Check for literal after 'is' instead of type (x is 42)
            if let Some(token) = self.stream.peek() {
                if matches!(
                    token.kind,
                    TokenKind::Integer(_)
                        | TokenKind::Float(_)
                        | TokenKind::Char(_)
                        | TokenKind::True
                        | TokenKind::False
                        | TokenKind::Text(_)
                ) {
                    return Err(ParseError::pattern_missing_guard(
                        "expected type after 'is', not a literal; use '==' for value comparison",
                        token.span,
                    ));
                }
            }

            let test_type = self.parse_type()?;
            let span = self.stream.make_span(start_pos);
            return Ok(Pattern::new(
                PatternKind::TypeTest {
                    binding: Ident::new(first_name, first_span),
                    test_type,
                },
                span,
            ));
        }

        // Check for qualified path (Type.Variant or Type::Variant)
        let (path, is_qualified) = if self.stream.check(&TokenKind::Dot) {
            // We have a qualified path - rewind and parse the full path
            // Reset to the position before we consumed first_name
            self.stream.reset_to(start_pos);

            // Parse the full path using the existing path parser
            let path = self.parse_path()?;
            let is_qualified = path.segments.len() > 1;
            (path, is_qualified)
        } else if self.stream.check(&TokenKind::ColonColon) {
            // Rust-style qualified path: State::Ready, Error::NotFound
            let mut segments = vec![PathSegment::Name(Ident::new(first_name.clone(), first_span))];
            while self.stream.consume(&TokenKind::ColonColon).is_some() {
                let seg_name = self.consume_ident_or_keyword()?;
                let seg_span = self.stream.current_span();
                segments.push(PathSegment::Name(Ident::new(seg_name, seg_span)));
            }
            let path_span = self.stream.make_span(start_pos);
            let path = Path::new(segments.into_iter().collect(), path_span);
            (path, true)
        } else {
            (
                Path::single(Ident::new(first_name.clone(), first_span)),
                false,
            )
        };

        // Check for view pattern: `view_fn -> inner_pattern`
        // View patterns apply the view function to the scrutinee and match
        // the result against the inner pattern.
        // Example: `parity -> Even(k)` applies parity() then matches Even(k)
        if self.stream.check(&TokenKind::RArrow) {
            self.stream.advance(); // consume '->'

            // The view function is constructed from the parsed path
            let view_fn_expr = Expr::path(path.clone());

            // Parse the inner pattern (recursively, so nested views work: f -> g -> pat)
            let inner = self.parse_base_pattern_impl(allow_struct)?;

            let span = self.stream.make_span(start_pos);
            return Ok(Pattern::new(
                PatternKind::View {
                    view_function: Box::new(view_fn_expr),
                    pattern: Box::new(inner),
                },
                span,
            ));
        }

        // Check what follows
        match self.stream.peek_kind() {
            // Could be:
            // 1. Active pattern: Even() or InRange(0, 100)()
            // 2. Tuple variant: Some(x) or Type.Some(x)
            //
            // Active patterns end with empty parens `()`.
            // Active patterns `Even()` or tuple variants `Some(x)` — disambiguated by empty parens
            Some(TokenKind::LParen) => {
                self.stream.advance();

                // Check for empty parens: `Ident()` or `Ident()(bindings)`
                if self.stream.check(&TokenKind::RParen) {
                    self.stream.advance(); // consume ')'

                    // Get the name from the path (for now, only support simple names)
                    let name = path
                        .segments
                        .last()
                        .and_then(|s| match s {
                            PathSegment::Name(ident) => Some(ident.clone()),
                            _ => None,
                        })
                        .unwrap_or_else(|| Ident::new(Text::from("unknown"), self.stream.current_span()));

                    // Check for trailing bindings: `Even()(bindings)` for partial patterns
                    // This distinguishes:
                    //   Even()     - total pattern, no bindings
                    //   Even()(n)  - partial pattern with binding n (no params)
                    //   ParseInt()(value) - partial pattern with binding
                    //
                    // Trailing bindings for partial active patterns: `Even()(n)`, `ParseInt()(value)`
                    if self.stream.check(&TokenKind::LParen) {
                        self.stream.advance(); // consume '('

                        // Check for empty bindings (redundant but valid): `Even()()`
                        if self.stream.check(&TokenKind::RParen) {
                            self.stream.advance(); // consume ')'
                            let span = self.stream.make_span(start_pos);

                            return Ok(Pattern::new(
                                PatternKind::Active {
                                    name,
                                    params: List::new(),
                                    bindings: List::new(),
                                },
                                span,
                            ));
                        }

                        // Parse non-empty bindings: `ParseInt()(n)` or `HeadTail()(h, t)`
                        let bindings = self.comma_separated_pattern_args()?;

                        self.stream.expect(TokenKind::RParen)?;
                        let span = self.stream.make_span(start_pos);

                        return Ok(Pattern::new(
                            PatternKind::Active {
                                name,
                                params: List::new(),
                                bindings: bindings.into_iter().collect(),
                            },
                            span,
                        ));
                    }

                    // No trailing parens - total pattern with no params: `Even()`
                    // We treat `Ident()` as an Active pattern since Verum sum types
                    // use unit variants without parens (e.g., `None` not `None()`).
                    let span = self.stream.make_span(start_pos);

                    return Ok(Pattern::new(
                        PatternKind::Active {
                            name,
                            params: List::new(),
                            bindings: List::new(),
                        },
                        span,
                    ));
                }

                // E075/E079: Check for leading comma in pattern arguments
                // The error code depends on context:
                // - E075: Active pattern context (ends with `()`) like `InRange(,)()`
                // - E079: Variant pattern context like `Some(,)`
                // We defer the specific error until we know which context we're in
                let has_leading_comma = self.stream.check(&TokenKind::Comma);

                // E074: Check for unclosed active pattern (missing contents)
                // Pattern like `Even( =>` should produce E074 (unclosed active pattern)
                if self.stream.check(&TokenKind::FatArrow) {
                    return Err(ParseError::pattern_unclosed_active(
                        "unclosed pattern, expected pattern or ')' after '('",
                        self.stream.current_span(),
                    ));
                }

                // E078: Check for rest pattern in variant position
                // Pattern like `Some(..)` should produce E078
                if self.stream.check(&TokenKind::DotDot) {
                    let span = self.stream.current_span();
                    return Err(ParseError::pattern_rest_position(
                        "rest pattern '..' is not allowed as variant argument; use wildcard '_' to ignore",
                        span,
                    ));
                }

                // Parse the contents (could be patterns for variant OR expressions for active pattern type args)
                // We need to lookahead after parsing to determine which it is.
                let pos_before_args = self.stream.position();
                let args = self.comma_separated_pattern_args()?;

                // E078: Check if any parsed argument is a rest pattern (in variant context)
                for arg in &args {
                    if matches!(arg.kind, PatternKind::Rest) {
                        return Err(ParseError::pattern_rest_position(
                            "rest pattern '..' is not allowed as variant argument",
                            arg.span,
                        ));
                    }
                }

                // E080: Check for unclosed variant pattern (has content but missing ')')
                // Pattern like `Some(x =>` should produce E080 (unclosed variant pattern)
                if !self.stream.check(&TokenKind::RParen) {
                    return Err(ParseError::pattern_invalid_type(
                        "unclosed variant pattern, expected ')'",
                        self.stream.current_span(),
                    ));
                }
                self.stream.advance(); // consume )

                // Check if followed by another `(...)` - if so, this is an active pattern
                // with the first args being parameters (expressions) and second being bindings (patterns)
                //
                // Grammar:
                //   active_pattern_tail = '(' , ')' (* total, no params *)
                //                       | '(' , pattern_list_nonempty , ')' (* partial with bindings, no params *)
                //                       | '(' , expression_list , ')' , '(' , [ pattern_list ] , ')' ; (* with params *)
                //
                // Examples:
                //   InRange(0, 100)()   - total pattern with params, empty bindings
                //   InRange(0, 100)(n)  - partial pattern with params AND extraction binding
                //   RegexMatch("\\d+")(groups) - partial with params and binding
                //
                if self.stream.check(&TokenKind::LParen) {
                    self.stream.advance(); // consume '('

                    // Get the name from the path
                    let name = path
                        .segments
                        .last()
                        .and_then(|s| match s {
                            PathSegment::Name(ident) => Some(ident.clone()),
                            _ => None,
                        })
                        .unwrap_or_else(|| Ident::new(Text::from("unknown"), self.stream.current_span()));

                    // Convert first set of args to expressions for params
                    // NOTE: This is a simplification - we parse as patterns but need expressions.
                    // For a full implementation, we'd need to either:
                    // 1. Backtrack and reparse as expressions
                    // 2. Convert patterns to expressions
                    // 3. Use a different parsing strategy with lookahead
                    //
                    // For now, we convert literal/identifier patterns to expressions.
                    let params: List<verum_ast::Expr> = args
                        .into_iter()
                        .map(|p| self.pattern_to_expr(p))
                        .collect();

                    // Check if bindings parens are empty: `InRange(0, 100)()`
                    if self.stream.check(&TokenKind::RParen) {
                        self.stream.advance(); // consume ')'

                        // Check for a third paren group: `Split(",")()([first, ...])`
                        // This is the parameterized partial active pattern syntax:
                        //   Name(params)()(extraction_bindings)
                        if self.stream.check(&TokenKind::LParen) {
                            self.stream.advance(); // consume '('

                            if self.stream.check(&TokenKind::RParen) {
                                self.stream.advance(); // consume ')'
                                let span = self.stream.make_span(start_pos);
                                return Ok(Pattern::new(
                                    PatternKind::Active {
                                        name,
                                        params,
                                        bindings: List::new(),
                                    },
                                    span,
                                ));
                            }

                            let bindings = self.comma_separated_pattern_args()?;
                            self.stream.expect(TokenKind::RParen)?;
                            let span = self.stream.make_span(start_pos);

                            return Ok(Pattern::new(
                                PatternKind::Active {
                                    name,
                                    params,
                                    bindings: bindings.into_iter().collect(),
                                },
                                span,
                            ));
                        }

                        let span = self.stream.make_span(start_pos);

                        return Ok(Pattern::new(
                            PatternKind::Active {
                                name,
                                params,
                                bindings: List::new(),
                            },
                            span,
                        ));
                    }

                    // Parse non-empty bindings: `InRange(0, 100)(n)` or `HeadTail()(h, t)`
                    let bindings = self.comma_separated_pattern_args()?;

                    self.stream.expect(TokenKind::RParen)?;
                    let span = self.stream.make_span(start_pos);

                    return Ok(Pattern::new(
                        PatternKind::Active {
                            name,
                            params,
                            bindings: bindings.into_iter().collect(),
                        },
                        span,
                    ));
                }

                // Not followed by `()` - this is a regular tuple variant pattern
                let span = self.stream.make_span(start_pos);

                Ok(Pattern::new(
                    PatternKind::Variant {
                        path,
                        data: Some(VariantPatternData::Tuple(
                            args.into_iter().collect::<List<_>>(),
                        )),
                    },
                    span,
                ))
            }

            // Record pattern: Point { x, y } or Point { .. }
            // OR Variant with record data: Shape.Circle { radius }
            // When allow_struct is false, use lookahead to determine if `{` is struct pattern or block
            Some(TokenKind::LBrace) if allow_struct || self.looks_like_struct_pattern() => {
                self.stream.advance();

                // Check for immediate rest-only pattern: { .. }
                // This must be checked before parse_field_patterns() since .. is not an identifier
                if self.stream.check(&TokenKind::DotDot) {
                    self.stream.advance();
                    self.stream.expect(TokenKind::RBrace)?;
                    let span = self.stream.make_span(start_pos);

                    // If qualified path (Type.Variant), create variant pattern with record data
                    // Otherwise, create record pattern
                    if is_qualified {
                        return Ok(Pattern::new(
                            PatternKind::Variant {
                                path,
                                data: Some(VariantPatternData::Record {
                                    fields: List::new(),
                                    rest: true,
                                }),
                            },
                            span,
                        ));
                    } else {
                        return Ok(Pattern::new(
                            PatternKind::Record {
                                path,
                                fields: List::new(),
                                rest: true,
                            },
                            span,
                        ));
                    }
                }

                let fields = if self.stream.check(&TokenKind::RBrace) {
                    Vec::new()
                } else {
                    self.parse_field_patterns()?
                };

                // Check for rest pattern after fields: { x, y, .. }
                let rest = self.stream.consume(&TokenKind::DotDot).is_some();

                // E075: Check for unclosed record pattern
                if !self.stream.check(&TokenKind::RBrace) {
                    return Err(ParseError::pattern_invalid_active_args(
                        "unclosed record pattern, expected '}'",
                        self.stream.current_span(),
                    ));
                }
                self.stream.advance(); // consume }
                let span = self.stream.make_span(start_pos);

                // If qualified path (Type.Variant), create variant pattern with record data
                // Otherwise, create record pattern
                if is_qualified {
                    Ok(Pattern::new(
                        PatternKind::Variant {
                            path,
                            data: Some(VariantPatternData::Record {
                                fields: fields.into_iter().collect::<List<_>>(),
                                rest,
                            }),
                        },
                        span,
                    ))
                } else {
                    Ok(Pattern::new(
                        PatternKind::Record {
                            path,
                            fields: fields.into_iter().collect::<List<_>>(),
                            rest,
                        },
                        span,
                    ))
                }
            }

            // Qualified unit variant (Type::Variant without data)
            _ if is_qualified => {
                let span = self.stream.make_span(start_pos);
                Ok(Pattern::new(
                    PatternKind::Variant {
                        path,
                        data: Maybe::None,
                    },
                    span,
                ))
            }

            // Unqualified uppercase identifier → unit variant pattern
            // This handles enum variants like Add, Sub, None, Some (without data)
            // Distinguishes between variant constructors (Add) and bindings (add, x)
            _ if first_name.chars().next().is_some_and(|c| c.is_uppercase()) => {
                let span = self.stream.make_span(start_pos);
                Ok(Pattern::new(
                    PatternKind::Variant {
                        path,
                        data: Maybe::None,
                    },
                    span,
                ))
            }

            // Plain identifier pattern (binding)
            _ => {
                let span = self.stream.make_span(start_pos);
                Ok(Pattern::new(
                    PatternKind::Ident {
                        by_ref,
                        mutable,
                        name: Ident::new(first_name, first_span),
                        subpattern: Maybe::None,
                    },
                    span,
                ))
            }
        }
    }

    /// Parse field patterns in a record pattern: `x`, `x: pattern`, `ref x`, `ref mut x`
    ///
    /// Handles the following field pattern forms:
    /// - `x` - shorthand for `x: x` (bind field to variable of same name)
    /// - `x: pattern` - bind field to explicit pattern
    /// - `ref x` - shorthand for `x: ref x` (borrow field by reference)
    /// - `ref mut x` - shorthand for `x: ref mut x` (borrow field mutably)
    /// - `mut x` - shorthand for `x: mut x` (bind field to mutable variable)
    fn parse_field_patterns(&mut self) -> ParseResult<Vec<FieldPattern>> {
        let mut fields = Vec::new();
        let mut seen_fields: std::collections::HashSet<Text> = std::collections::HashSet::new();

        loop {
            // Safety: prevent infinite loop
            if !self.tick() || self.is_aborted() {
                break;
            }

            let field_start = self.stream.position();
            let pos_before = self.stream.position();

            // Check for `ref` modifier before field name
            let by_ref = self.stream.consume(&TokenKind::Ref).is_some();

            // Check for `mut` modifier
            let mutable = self.stream.consume(&TokenKind::Mut).is_some();

            let name = self.consume_ident_or_keyword()?;
            let name_span = self.stream.make_span(field_start);

            // E077: Check for duplicate field names
            if seen_fields.contains(&name) {
                return Err(ParseError::pattern_duplicate_field(
                    format!("field '{}' is bound more than once in this pattern", name),
                    name_span,
                ));
            }
            seen_fields.insert(name.clone());

            // E076: Check for invalid field pattern syntax (using = or => instead of :)
            if self.stream.check(&TokenKind::Eq) {
                return Err(ParseError::pattern_invalid_field(
                    "field pattern uses ':' not '=' (use 'x: pattern' not 'x = pattern')",
                    self.stream.current_span(),
                ));
            }
            if self.stream.check(&TokenKind::FatArrow) {
                return Err(ParseError::pattern_invalid_field(
                    "field pattern uses ':' not '=>' (use 'x: pattern' not 'x => pattern')",
                    self.stream.current_span(),
                ));
            }
            if self.stream.check(&TokenKind::ColonColon) {
                return Err(ParseError::pattern_invalid_field(
                    "field pattern uses single ':' not '::' (use 'x: pattern' not 'x:: pattern')",
                    self.stream.current_span(),
                ));
            }

            // Check for explicit pattern: `x: pattern`
            // Guard patterns allowed in field values: `Point { x: val if val > 0 }`
            let pattern = if self.stream.consume(&TokenKind::Colon).is_some() {
                // E078: Check for missing field value after colon
                // Pattern like `Point { x: }` or `Point { x:, y }` should produce E078
                if self.stream.check(&TokenKind::RBrace)
                    || self.stream.check(&TokenKind::Comma)
                {
                    return Err(ParseError::pattern_rest_position(
                        "expected pattern after ':' in field pattern",
                        self.stream.current_span(),
                    ));
                }
                Some(self.parse_pattern_allowing_guard()?)
            } else if by_ref || mutable {
                // If we have `ref x` or `mut x` shorthand, create implicit pattern
                // `ref x` -> x: ref x
                // `ref mut x` -> x: ref mut x
                // `mut x` -> x: mut x
                let ident_pattern = Pattern::new(
                    PatternKind::Ident {
                        name: Ident::new(name.clone(), name_span),
                        mutable,
                        by_ref,
                        subpattern: Maybe::None,
                    },
                    name_span,
                );
                Some(ident_pattern)
            } else {
                None
            };

            let field_span = self.stream.make_span(field_start);
            fields.push(FieldPattern::new(
                Ident::new(name, name_span),
                pattern,
                field_span,
            ));

            // Safety: Ensure we made forward progress
            if self.stream.position() == pos_before {
                return Err(ParseError::invalid_syntax(
                    "parser made no progress in field patterns",
                    self.stream.current_span(),
                ));
            }

            // Check for more fields
            if self.stream.consume(&TokenKind::Comma).is_some() {
                // Allow trailing comma before } or ..
                if self.stream.check(&TokenKind::RBrace) || self.stream.check(&TokenKind::DotDot) {
                    break;
                }
            } else {
                break;
            }
        }

        Ok(fields)
    }

    /// Parse comma-separated pattern arguments with E075 error for double commas.
    ///
    /// This is used for variant and active pattern arguments where double commas
    /// should produce E075 (invalid active pattern arguments).
    ///
    /// Guard patterns are allowed in arguments: `Some(x if x > 0)`
    fn comma_separated_pattern_args(&mut self) -> ParseResult<Vec<Pattern>> {
        let mut items = Vec::new();

        // Handle empty list
        if self.stream.at_end() {
            return Ok(items);
        }

        // Parse first item - allow guards inside variant/active pattern arguments
        items.push(self.parse_pattern_allowing_guard()?);

        // E081: Check for type annotation without type (x: ) after first pattern
        if self.stream.check(&TokenKind::Colon) {
            let colon_span = self.stream.current_span();
            self.stream.advance(); // consume :
            // Check if followed by closing delimiter (missing type)
            if self.stream.check(&TokenKind::RParen)
                || self.stream.check(&TokenKind::Comma)
                || self.stream.check(&TokenKind::RBracket)
                || self.stream.check(&TokenKind::RBrace)
                || self.stream.check(&TokenKind::Semicolon)
            {
                return Err(ParseError::pattern_invalid_slice(
                    "missing type after ':' in pattern",
                    colon_span,
                ));
            }
            // Otherwise put it back (shouldn't happen in valid Verum patterns)
            // Actually, we can't easily put it back, so let's produce a different error
            return Err(ParseError::pattern_invalid_slice(
                "unexpected ':' in pattern - type annotations are not allowed here",
                colon_span,
            ));
        }

        // Parse remaining items
        while self.stream.consume(&TokenKind::Comma).is_some() {
            // Allow trailing comma: check for common closing delimiters
            if self.stream.at_end()
                || self.stream.check(&TokenKind::RBrace)
                || self.stream.check(&TokenKind::RParen)
                || self.stream.check(&TokenKind::RBracket)
            {
                break;
            }

            // E075: Check for double comma (consecutive commas)
            if self.stream.check(&TokenKind::Comma) {
                return Err(ParseError::pattern_invalid_active_args(
                    "unexpected double comma in pattern arguments",
                    self.stream.current_span(),
                ));
            }

            // Safety: Track position to ensure forward progress
            let pos_before = self.stream.position();
            items.push(self.parse_pattern_allowing_guard()?);

            // E081: Check for type annotation without type (x: ) after pattern
            if self.stream.check(&TokenKind::Colon) {
                let colon_span = self.stream.current_span();
                self.stream.advance(); // consume :
                // Check if followed by closing delimiter (missing type)
                if self.stream.check(&TokenKind::RParen)
                    || self.stream.check(&TokenKind::Comma)
                    || self.stream.check(&TokenKind::RBracket)
                    || self.stream.check(&TokenKind::RBrace)
                    || self.stream.check(&TokenKind::Semicolon)
                {
                    return Err(ParseError::pattern_invalid_slice(
                        "missing type after ':' in pattern",
                        colon_span,
                    ));
                }
                return Err(ParseError::pattern_invalid_slice(
                    "unexpected ':' in pattern - type annotations are not allowed here",
                    colon_span,
                ));
            }

            // Ensure we advanced at least one token
            if self.stream.position() == pos_before {
                return Err(ParseError::invalid_syntax(
                    "parser made no progress in pattern arguments",
                    self.stream.current_span(),
                ));
            }
        }

        Ok(items)
    }

    /// Convert a pattern to an expression.
    ///
    /// This is used for active pattern type arguments which are parsed as patterns
    /// but need to be expressions. Only literal patterns and identifiers are supported.
    ///
    /// Converts literal and identifier patterns to expressions (used for active pattern type args).
    fn pattern_to_expr(&self, pattern: Pattern) -> verum_ast::Expr {
        use verum_ast::{Expr, ExprKind};

        match pattern.kind {
            // Literal patterns convert directly to literal expressions
            PatternKind::Literal(lit) => Expr::new(ExprKind::Literal(lit.clone()), pattern.span),

            // Identifier patterns convert to path expressions
            PatternKind::Ident { name, .. } => {
                let path = Path::single(name);
                Expr::new(ExprKind::Path(path), pattern.span)
            }

            // Variant patterns (like Type.Variant) convert to path expressions
            PatternKind::Variant { path, data: None } => {
                Expr::new(ExprKind::Path(path), pattern.span)
            }

            // Negative literals (handled via Range pattern in some cases)
            PatternKind::Range {
                start: Some(start),
                end: None,
                ..
            } => Expr::new(ExprKind::Literal((*start).clone()), pattern.span),

            // For more complex patterns, we create a placeholder that type checking
            // can catch and report a proper error
            _ => {
                // Create a zero literal as placeholder - type checker will validate
                let lit = Literal::int(0, pattern.span);
                Expr::new(ExprKind::Literal(lit), pattern.span)
            }
        }
    }

    /// Lookahead to determine if `{` starts a struct pattern or a block.
    ///
    /// Returns true if the tokens after `{` look like struct pattern fields:
    /// - `{ identifier, ...` (field shorthand)
    /// - `{ identifier: ...` (field with pattern)
    /// - `{ ref identifier ...` (ref field pattern)
    /// - `{ mut identifier ...` (mut field pattern)
    /// - `{ ref mut identifier ...` (ref mut field pattern)
    /// - `{ .. }` (rest pattern)
    /// - `{ }` followed by `)` (empty struct in parenthesized expression)
    ///
    /// Returns false otherwise (likely a block).
    fn looks_like_struct_pattern(&self) -> bool {
        // We must be at `{` token
        if !self.stream.check(&TokenKind::LBrace) {
            return false;
        }

        // Look at what comes after `{`
        let token_after_brace = self.stream.peek_nth(1);

        match token_after_brace {
            // `{ .. }` - rest pattern
            Some(Token {
                kind: TokenKind::DotDot,
                ..
            }) => true,

            // `{ ref ...` - field pattern with ref modifier
            Some(Token {
                kind: TokenKind::Ref,
                ..
            }) => true,

            // `{ mut ...` - field pattern with mut modifier
            Some(Token {
                kind: TokenKind::Mut,
                ..
            }) => true,

            // `{ identifier ...` - could be field
            Some(Token {
                kind: TokenKind::Ident(_),
                ..
            }) => {
                // Look at what comes after the identifier: `,` or `:` or `}` means struct pattern
                matches!(
                    self.stream.peek_nth(2),
                    Some(Token { kind: TokenKind::Comma, .. })
                        | Some(Token { kind: TokenKind::Colon, .. })
                        | Some(Token { kind: TokenKind::RBrace, .. })
                )
            }

            // `{ }` - empty braces
            // For `is` expressions inside parentheses, this is likely an empty struct pattern
            // e.g., `(value is Point { })` vs `if value is None { }`
            // We'll be conservative: treat `{ }` as empty struct if followed by `)`
            Some(Token {
                kind: TokenKind::RBrace,
                ..
            }) => {
                // Check if closing brace is followed by `)`
                matches!(
                    self.stream.peek_nth(2),
                    Some(Token { kind: TokenKind::RParen, .. })
                )
            }

            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::FileId;
    use verum_lexer::Lexer;

    fn parse_pattern(source: &str) -> ParseResult<Pattern> {
        let file_id = FileId::new(0);
        let lexer = Lexer::new(source, file_id);
        let tokens: Vec<Token> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = RecursiveParser::new(&tokens, file_id);
        parser.parse_pattern()
    }

    #[test]
    fn test_wildcard_pattern() {
        let result = parse_pattern("_");
        assert!(result.is_ok());
        assert!(matches!(result.unwrap().kind, PatternKind::Wildcard));
    }

    #[test]
    fn test_identifier_pattern() {
        let result = parse_pattern("x");
        assert!(result.is_ok());
        let pattern = result.unwrap();
        assert!(matches!(
            pattern.kind,
            PatternKind::Ident { mutable: false, .. }
        ));
    }

    #[test]
    fn test_mutable_identifier_pattern() {
        let result = parse_pattern("mut x");
        assert!(result.is_ok());
        let pattern = result.unwrap();
        assert!(matches!(
            pattern.kind,
            PatternKind::Ident { mutable: true, .. }
        ));
    }

    #[test]
    fn test_tuple_pattern() {
        let result = parse_pattern("(a, b, c)");
        assert!(result.is_ok());
        let pattern = result.unwrap();
        if let PatternKind::Tuple(patterns) = pattern.kind {
            assert_eq!(patterns.len(), 3);
        } else {
            panic!("Expected tuple pattern");
        }
    }

    #[test]
    fn test_literal_pattern() {
        let result = parse_pattern("42");
        assert!(result.is_ok());
        assert!(matches!(result.unwrap().kind, PatternKind::Literal(_)));
    }

    #[test]
    fn test_range_pattern() {
        let result = parse_pattern("1..10");
        assert!(result.is_ok());
        let pattern = result.unwrap();
        assert!(matches!(
            pattern.kind,
            PatternKind::Range {
                inclusive: false,
                ..
            }
        ));
    }

    #[test]
    fn test_inclusive_range_pattern() {
        let result = parse_pattern("1..=10");
        assert!(result.is_ok());
        let pattern = result.unwrap();
        assert!(matches!(
            pattern.kind,
            PatternKind::Range {
                inclusive: true,
                ..
            }
        ));
    }

    #[test]
    fn test_or_pattern() {
        let result = parse_pattern("a | b | c");
        assert!(result.is_ok());
        let pattern = result.unwrap();
        if let PatternKind::Or(patterns) = pattern.kind {
            assert_eq!(patterns.len(), 3);
        } else {
            panic!("Expected OR pattern");
        }
    }

    #[test]
    fn test_reference_pattern() {
        let result = parse_pattern("&x");
        assert!(result.is_ok());
        let pattern = result.unwrap();
        assert!(matches!(
            pattern.kind,
            PatternKind::Reference { mutable: false, .. }
        ));
    }

    #[test]
    fn test_mutable_reference_pattern() {
        let result = parse_pattern("&mut x");
        assert!(result.is_ok());
        let pattern = result.unwrap();
        assert!(matches!(
            pattern.kind,
            PatternKind::Reference { mutable: true, .. }
        ));
    }

    #[test]
    fn test_literal_pattern_in_match() {
        // Test that literal patterns work when parsed individually
        let result = parse_pattern("1");
        assert!(
            result.is_ok(),
            "Failed to parse literal pattern '1': {:?}",
            result
        );
        assert!(matches!(result.unwrap().kind, PatternKind::Literal(_)));
    }

    #[test]
    fn test_literal_pattern_comprehensive() {
        // Test various literal patterns
        for lit in &["1", "42", "0", "123"] {
            let result = parse_pattern(lit);
            assert!(
                result.is_ok(),
                "Failed to parse literal pattern '{}': {:?}",
                lit,
                result
            );
            assert!(matches!(result.unwrap().kind, PatternKind::Literal(_)));
        }

        // Test string literals
        let result = parse_pattern(r#""hello""#);
        assert!(
            result.is_ok(),
            "Failed to parse string pattern: {:?}",
            result
        );

        // Test boolean literals
        let result = parse_pattern("true");
        assert!(result.is_ok(), "Failed to parse true pattern: {:?}", result);

        let result = parse_pattern("false");
        assert!(
            result.is_ok(),
            "Failed to parse false pattern: {:?}",
            result
        );
    }

    #[test]
    fn test_rest_pattern_with_binding() {
        // Test plain rest pattern
        let result = parse_pattern("..");
        assert!(
            result.is_ok(),
            "Failed to parse plain rest pattern: {:?}",
            result
        );
        let pattern = result.unwrap();
        assert!(matches!(pattern.kind, PatternKind::Rest));

        // Test rest pattern with binding: ..name
        let result = parse_pattern("..tail");
        assert!(
            result.is_ok(),
            "Failed to parse rest pattern with binding '..tail': {:?}",
            result
        );
        let pattern = result.unwrap();

        // Rest with binding is encoded as Ident with Rest subpattern
        match &pattern.kind {
            PatternKind::Ident {
                name, subpattern, ..
            } => {
                assert_eq!(name.name.as_str(), "tail");
                assert!(
                    matches!(subpattern, Maybe::Some(boxed) if matches!(boxed.kind, PatternKind::Rest))
                );
            }
            _ => panic!(
                "Expected Ident pattern with Rest subpattern, got {:?}",
                pattern.kind
            ),
        }
    }

    #[test]
    fn test_array_pattern_with_rest_binding() {
        // Test [..tail]
        let result = parse_pattern("[..tail]");
        assert!(
            result.is_ok(),
            "Failed to parse array pattern [..tail]: {:?}",
            result
        );
        let pattern = result.unwrap();

        match &pattern.kind {
            PatternKind::Slice {
                before,
                rest,
                after,
            } => {
                assert_eq!(before.len(), 0);
                assert_eq!(after.len(), 0);
                // rest should contain the binding pattern
                match rest {
                    Maybe::Some(binding) => match &binding.kind {
                        PatternKind::Ident { name, .. } => {
                            assert_eq!(name.name.as_str(), "tail");
                        }
                        _ => panic!(
                            "Expected Ident pattern in rest binding, got {:?}",
                            binding.kind
                        ),
                    },
                    Maybe::None => panic!("Expected rest binding, got None"),
                }
            }
            _ => panic!("Expected Slice pattern, got {:?}", pattern.kind),
        }

        // Test [first, ..rest]
        let result = parse_pattern("[first, ..rest]");
        assert!(
            result.is_ok(),
            "Failed to parse array pattern [first, ..rest]: {:?}",
            result
        );
        let pattern = result.unwrap();

        match &pattern.kind {
            PatternKind::Slice {
                before,
                rest,
                after,
            } => {
                assert_eq!(before.len(), 1);
                assert_eq!(after.len(), 0);
                match rest {
                    Maybe::Some(binding) => match &binding.kind {
                        PatternKind::Ident { name, .. } => {
                            assert_eq!(name.name.as_str(), "rest");
                        }
                        _ => panic!("Expected Ident pattern in rest binding"),
                    },
                    Maybe::None => panic!("Expected rest binding"),
                }
            }
            _ => panic!("Expected Slice pattern"),
        }

        // Test [a, b, ..remaining]
        let result = parse_pattern("[a, b, ..remaining]");
        assert!(
            result.is_ok(),
            "Failed to parse array pattern [a, b, ..remaining]: {:?}",
            result
        );
        let pattern = result.unwrap();

        match &pattern.kind {
            PatternKind::Slice {
                before,
                rest,
                after,
            } => {
                assert_eq!(before.len(), 2);
                assert_eq!(after.len(), 0);
                match rest {
                    Maybe::Some(binding) => match &binding.kind {
                        PatternKind::Ident { name, .. } => {
                            assert_eq!(name.name.as_str(), "remaining");
                        }
                        _ => panic!("Expected Ident pattern in rest binding"),
                    },
                    Maybe::None => panic!("Expected rest binding"),
                }
            }
            _ => panic!("Expected Slice pattern"),
        }
    }

    #[test]
    fn test_qualified_variant_pattern() {
        // Test simple qualified variant: Maybe.Some
        let result = parse_pattern("Maybe.Some(x)");
        assert!(
            result.is_ok(),
            "Failed to parse Maybe.Some(x): {:?}",
            result
        );
        let pattern = result.unwrap();

        match &pattern.kind {
            PatternKind::Variant { path, data } => {
                assert_eq!(path.segments.len(), 2, "Expected 2 path segments");
                // Check that data contains tuple pattern with one element
                match data {
                    Maybe::Some(variant_data) => match variant_data {
                        VariantPatternData::Tuple(patterns) => {
                            assert_eq!(patterns.len(), 1, "Expected 1 pattern in tuple");
                        }
                        _ => panic!("Expected Tuple variant data"),
                    },
                    Maybe::None => panic!("Expected variant data"),
                }
            }
            _ => panic!("Expected Variant pattern, got {:?}", pattern.kind),
        }
    }

    #[test]
    fn test_qualified_variant_pattern_none() {
        // Test simple qualified variant without data: Maybe.None
        let result = parse_pattern("Maybe.None");
        assert!(result.is_ok(), "Failed to parse Maybe.None: {:?}", result);
        let pattern = result.unwrap();

        match &pattern.kind {
            PatternKind::Variant { path, data } => {
                assert_eq!(path.segments.len(), 2, "Expected 2 path segments");
                assert!(matches!(data, Maybe::None), "Expected no variant data");
            }
            _ => panic!("Expected Variant pattern, got {:?}", pattern.kind),
        }
    }

    #[test]
    fn test_result_ok_pattern() {
        // Test Result.Ok pattern
        let result = parse_pattern("Result.Ok(value)");
        assert!(
            result.is_ok(),
            "Failed to parse Result.Ok(value): {:?}",
            result
        );
        let pattern = result.unwrap();

        match &pattern.kind {
            PatternKind::Variant { path, .. } => {
                assert_eq!(
                    path.segments.len(),
                    2,
                    "Expected 2 path segments for Result.Ok"
                );
            }
            _ => panic!("Expected Variant pattern"),
        }
    }

    #[test]
    fn test_result_err_pattern() {
        // Test Result.Err pattern
        let result = parse_pattern("Result.Err(e)");
        assert!(
            result.is_ok(),
            "Failed to parse Result.Err(e): {:?}",
            result
        );
        let pattern = result.unwrap();

        match &pattern.kind {
            PatternKind::Variant { path, .. } => {
                assert_eq!(
                    path.segments.len(),
                    2,
                    "Expected 2 path segments for Result.Err"
                );
            }
            _ => panic!("Expected Variant pattern"),
        }
    }

    #[test]
    fn test_long_qualified_path_pattern() {
        // Test longer qualified path: std.option.Maybe.Some
        let result = parse_pattern("std.option.Maybe.Some(x)");
        assert!(
            result.is_ok(),
            "Failed to parse std.option.Maybe.Some(x): {:?}",
            result
        );
        let pattern = result.unwrap();

        match &pattern.kind {
            PatternKind::Variant { path, data } => {
                assert_eq!(path.segments.len(), 4, "Expected 4 path segments");
                assert!(matches!(data, Maybe::Some(_)), "Expected variant data");
            }
            _ => panic!("Expected Variant pattern, got {:?}", pattern.kind),
        }
    }

    #[test]
    fn test_qualified_record_pattern() {
        // Test qualified variant pattern with record data: Event.UserCreated { id, name }
        // This is correctly parsed as a Variant pattern with Record data (not a plain Record)
        let result = parse_pattern("Event.UserCreated { id, name }");
        assert!(
            result.is_ok(),
            "Failed to parse Event.UserCreated {{ id, name }}: {:?}",
            result
        );
        let pattern = result.unwrap();

        match &pattern.kind {
            PatternKind::Variant { path, data } => {
                assert_eq!(path.segments.len(), 2, "Expected 2 path segments");
                match data {
                    Maybe::Some(VariantPatternData::Record { fields, .. }) => {
                        assert_eq!(fields.len(), 2, "Expected 2 fields");
                    }
                    _ => panic!("Expected Record variant data, got {:?}", data),
                }
            }
            _ => panic!("Expected Variant pattern, got {:?}", pattern.kind),
        }
    }

    #[test]
    fn test_qualified_unit_variant() {
        // Test qualified unit variant without parentheses
        let result = parse_pattern("Color.Red");
        assert!(result.is_ok(), "Failed to parse Color.Red: {:?}", result);
        let pattern = result.unwrap();

        match &pattern.kind {
            PatternKind::Variant { path, data } => {
                assert_eq!(path.segments.len(), 2, "Expected 2 path segments");
                assert!(
                    matches!(data, Maybe::None),
                    "Expected no variant data for unit variant"
                );
            }
            _ => panic!("Expected Variant pattern, got {:?}", pattern.kind),
        }
    }

    #[test]
    fn test_simple_identifier_still_works() {
        // Make sure simple identifiers still work after our changes
        let result = parse_pattern("value");
        assert!(
            result.is_ok(),
            "Failed to parse simple identifier: {:?}",
            result
        );
        let pattern = result.unwrap();

        match &pattern.kind {
            PatternKind::Ident { name, .. } => {
                assert_eq!(name.name.as_str(), "value");
            }
            _ => panic!("Expected Ident pattern, got {:?}", pattern.kind),
        }
    }

    #[test]
    fn test_ref_mut_patterns_still_work() {
        // Test ref pattern
        let result = parse_pattern("ref x");
        assert!(result.is_ok(), "Failed to parse ref x: {:?}", result);
        match &result.unwrap().kind {
            PatternKind::Ident { by_ref, .. } => {
                assert!(*by_ref, "Expected by_ref to be true");
            }
            _ => panic!("Expected Ident pattern"),
        }

        // Test mut pattern
        let result = parse_pattern("mut y");
        assert!(result.is_ok(), "Failed to parse mut y: {:?}", result);
        match &result.unwrap().kind {
            PatternKind::Ident { mutable, .. } => {
                assert!(*mutable, "Expected mutable to be true");
            }
            _ => panic!("Expected Ident pattern"),
        }

        // Test ref mut pattern
        let result = parse_pattern("ref mut z");
        assert!(result.is_ok(), "Failed to parse ref mut z: {:?}", result);
        match &result.unwrap().kind {
            PatternKind::Ident {
                by_ref, mutable, ..
            } => {
                assert!(*by_ref, "Expected by_ref to be true");
                assert!(*mutable, "Expected mutable to be true");
            }
            _ => panic!("Expected Ident pattern"),
        }
    }

    // =============================================================================
    // Active Pattern Tests — user-defined pattern decomposition functions
    // =============================================================================

    #[test]
    fn test_simple_active_pattern() {
        // Test simple active pattern: Even()
        let result = parse_pattern("Even()");
        assert!(
            result.is_ok(),
            "Failed to parse simple active pattern Even(): {:?}",
            result
        );
        let pattern = result.unwrap();

        match &pattern.kind {
            PatternKind::Active { name, params, bindings } => {
                assert_eq!(name.name.as_str(), "Even");
                assert_eq!(params.len(), 0, "Expected no pattern parameters");
                assert_eq!(bindings.len(), 0, "Expected no bindings");
            }
            _ => panic!("Expected Active pattern, got {:?}", pattern.kind),
        }
    }

    #[test]
    fn test_active_pattern_with_params() {
        // Test parameterized active pattern: InRange(0, 100)()
        let result = parse_pattern("InRange(0, 100)()");
        assert!(
            result.is_ok(),
            "Failed to parse parameterized active pattern InRange(0, 100)(): {:?}",
            result
        );
        let pattern = result.unwrap();

        match &pattern.kind {
            PatternKind::Active { name, params, bindings } => {
                assert_eq!(name.name.as_str(), "InRange");
                assert_eq!(params.len(), 2, "Expected 2 pattern parameters");
                assert_eq!(bindings.len(), 0, "Expected no bindings for total pattern");
            }
            _ => panic!("Expected Active pattern, got {:?}", pattern.kind),
        }
    }

    #[test]
    fn test_active_pattern_positive() {
        // Test Positive() active pattern
        let result = parse_pattern("Positive()");
        assert!(
            result.is_ok(),
            "Failed to parse Positive(): {:?}",
            result
        );
        let pattern = result.unwrap();

        match &pattern.kind {
            PatternKind::Active { name, params, bindings } => {
                assert_eq!(name.name.as_str(), "Positive");
                assert!(params.is_empty());
                assert!(bindings.is_empty());
            }
            _ => panic!("Expected Active pattern, got {:?}", pattern.kind),
        }
    }

    // =============================================================================
    // AND Pattern Combination Tests — `&` operator combines patterns that must all match
    // =============================================================================

    #[test]
    fn test_and_pattern_two_patterns() {
        // Test pattern combination with &: a & b
        let result = parse_pattern("a & b");
        assert!(
            result.is_ok(),
            "Failed to parse pattern combination a & b: {:?}",
            result
        );
        let pattern = result.unwrap();

        match &pattern.kind {
            PatternKind::And(patterns) => {
                assert_eq!(patterns.len(), 2, "Expected 2 patterns in And combination");
            }
            _ => panic!("Expected And pattern, got {:?}", pattern.kind),
        }
    }

    #[test]
    fn test_and_pattern_three_patterns() {
        // Test chained pattern combination: a & b & c
        let result = parse_pattern("a & b & c");
        assert!(
            result.is_ok(),
            "Failed to parse chained pattern combination a & b & c: {:?}",
            result
        );
        let pattern = result.unwrap();

        match &pattern.kind {
            PatternKind::And(patterns) => {
                assert_eq!(patterns.len(), 3, "Expected 3 patterns in And combination");
            }
            _ => panic!("Expected And pattern, got {:?}", pattern.kind),
        }
    }

    #[test]
    fn test_and_pattern_with_active_patterns() {
        // Test Even() & Positive() — combining active patterns with & operator
        let result = parse_pattern("Even() & Positive()");
        assert!(
            result.is_ok(),
            "Failed to parse Even() & Positive(): {:?}",
            result
        );
        let pattern = result.unwrap();

        match &pattern.kind {
            PatternKind::And(patterns) => {
                assert_eq!(patterns.len(), 2, "Expected 2 patterns in And combination");
                // Verify both are Active patterns
                for p in patterns {
                    assert!(
                        matches!(p.kind, PatternKind::Active { .. }),
                        "Expected Active pattern in And, got {:?}",
                        p.kind
                    );
                }
            }
            _ => panic!("Expected And pattern, got {:?}", pattern.kind),
        }
    }

    #[test]
    fn test_or_and_combination() {
        // Test that OR has lower precedence than AND: a & b | c & d
        // Should parse as: (a & b) | (c & d)
        let result = parse_pattern("a & b | c & d");
        assert!(
            result.is_ok(),
            "Failed to parse a & b | c & d: {:?}",
            result
        );
        let pattern = result.unwrap();

        match &pattern.kind {
            PatternKind::Or(or_patterns) => {
                assert_eq!(or_patterns.len(), 2, "Expected 2 alternatives in Or pattern");
                for op in or_patterns {
                    assert!(
                        matches!(op.kind, PatternKind::And(_)),
                        "Expected And pattern in Or alternative, got {:?}",
                        op.kind
                    );
                }
            }
            _ => panic!("Expected Or pattern, got {:?}", pattern.kind),
        }
    }

    #[test]
    fn test_variant_pattern_still_works() {
        // Ensure regular variant patterns still work (not confused with active patterns)
        let result = parse_pattern("Some(x)");
        assert!(
            result.is_ok(),
            "Failed to parse Some(x): {:?}",
            result
        );
        let pattern = result.unwrap();

        // Some(x) with non-empty parens should be Variant, not Active
        assert!(
            matches!(pattern.kind, PatternKind::Variant { .. }),
            "Expected Variant pattern for Some(x), got {:?}",
            pattern.kind
        );
    }

    // =============================================================================
    // Active Pattern Extraction Binding Tests
    // Spec: grammar/verum.ebnf Section 2.14 - Active Patterns
    // =============================================================================

    #[test]
    fn test_partial_pattern_single_binding() {
        // Test partial pattern with single binding: ParseInt()(n)
        let result = parse_pattern("ParseInt()(n)");
        assert!(
            result.is_ok(),
            "Failed to parse partial pattern ParseInt()(n): {:?}",
            result
        );
        let pattern = result.unwrap();

        match &pattern.kind {
            PatternKind::Active { name, params, bindings } => {
                assert_eq!(name.name.as_str(), "ParseInt");
                assert!(params.is_empty(), "Expected no params for ParseInt()(n)");
                assert_eq!(bindings.len(), 1, "Expected 1 binding");
                // Verify the binding is an identifier pattern 'n'
                match &bindings[0].kind {
                    PatternKind::Ident { name: binding_name, .. } => {
                        assert_eq!(binding_name.name.as_str(), "n");
                    }
                    _ => panic!("Expected Ident binding, got {:?}", bindings[0].kind),
                }
            }
            _ => panic!("Expected Active pattern, got {:?}", pattern.kind),
        }
    }

    #[test]
    fn test_partial_pattern_multiple_bindings() {
        // Test partial pattern with multiple bindings: HeadTail()(h, t)
        let result = parse_pattern("HeadTail()(h, t)");
        assert!(
            result.is_ok(),
            "Failed to parse partial pattern HeadTail()(h, t): {:?}",
            result
        );
        let pattern = result.unwrap();

        match &pattern.kind {
            PatternKind::Active { name, params, bindings } => {
                assert_eq!(name.name.as_str(), "HeadTail");
                assert!(params.is_empty(), "Expected no params for HeadTail()(h, t)");
                assert_eq!(bindings.len(), 2, "Expected 2 bindings");
            }
            _ => panic!("Expected Active pattern, got {:?}", pattern.kind),
        }
    }

    #[test]
    fn test_partial_pattern_with_wildcard_binding() {
        // Test partial pattern with wildcard binding: ParseInt()(_)
        let result = parse_pattern("ParseInt()(_)");
        assert!(
            result.is_ok(),
            "Failed to parse partial pattern ParseInt()(_): {:?}",
            result
        );
        let pattern = result.unwrap();

        match &pattern.kind {
            PatternKind::Active { name, params, bindings } => {
                assert_eq!(name.name.as_str(), "ParseInt");
                assert!(params.is_empty());
                assert_eq!(bindings.len(), 1);
                assert!(
                    matches!(bindings[0].kind, PatternKind::Wildcard),
                    "Expected Wildcard binding, got {:?}",
                    bindings[0].kind
                );
            }
            _ => panic!("Expected Active pattern, got {:?}", pattern.kind),
        }
    }

    #[test]
    fn test_parameterized_partial_pattern_with_bindings() {
        // Test parameterized partial pattern: RegexMatch("\\d+")(groups)
        let result = parse_pattern("RegexMatch(pattern)(groups)");
        assert!(
            result.is_ok(),
            "Failed to parse RegexMatch(pattern)(groups): {:?}",
            result
        );
        let pattern = result.unwrap();

        match &pattern.kind {
            PatternKind::Active { name, params, bindings } => {
                assert_eq!(name.name.as_str(), "RegexMatch");
                assert_eq!(params.len(), 1, "Expected 1 param");
                assert_eq!(bindings.len(), 1, "Expected 1 binding");
            }
            _ => panic!("Expected Active pattern, got {:?}", pattern.kind),
        }
    }

    #[test]
    fn test_parameterized_partial_pattern_multiple_params_and_bindings() {
        // Test: SplitAt(2, 5)(left, middle, right)
        let result = parse_pattern("SplitAt(2, 5)(left, middle, right)");
        assert!(
            result.is_ok(),
            "Failed to parse SplitAt(2, 5)(left, middle, right): {:?}",
            result
        );
        let pattern = result.unwrap();

        match &pattern.kind {
            PatternKind::Active { name, params, bindings } => {
                assert_eq!(name.name.as_str(), "SplitAt");
                assert_eq!(params.len(), 2, "Expected 2 params");
                assert_eq!(bindings.len(), 3, "Expected 3 bindings");
            }
            _ => panic!("Expected Active pattern, got {:?}", pattern.kind),
        }
    }

    #[test]
    fn test_redundant_empty_bindings() {
        // Test redundant empty bindings: Even()() is same as Even()
        let result = parse_pattern("Even()()");
        assert!(
            result.is_ok(),
            "Failed to parse redundant Even()(): {:?}",
            result
        );
        let pattern = result.unwrap();

        match &pattern.kind {
            PatternKind::Active { name, params, bindings } => {
                assert_eq!(name.name.as_str(), "Even");
                assert!(params.is_empty(), "Expected no params");
                assert!(bindings.is_empty(), "Expected no bindings (empty is valid)");
            }
            _ => panic!("Expected Active pattern, got {:?}", pattern.kind),
        }
    }

    #[test]
    fn test_nested_tuple_binding() {
        // Test nested tuple pattern in binding: Decompose()((a, b))
        let result = parse_pattern("Decompose()((a, b))");
        assert!(
            result.is_ok(),
            "Failed to parse Decompose()((a, b)): {:?}",
            result
        );
        let pattern = result.unwrap();

        match &pattern.kind {
            PatternKind::Active { name, params, bindings } => {
                assert_eq!(name.name.as_str(), "Decompose");
                assert!(params.is_empty());
                assert_eq!(bindings.len(), 1, "Expected 1 binding (tuple pattern)");
                assert!(
                    matches!(bindings[0].kind, PatternKind::Tuple(_)),
                    "Expected Tuple binding, got {:?}",
                    bindings[0].kind
                );
            }
            _ => panic!("Expected Active pattern, got {:?}", pattern.kind),
        }
    }

    #[test]
    fn test_and_pattern_with_partial_active() {
        // Test: Even() & ParseInt()(n) - combining total and partial
        let result = parse_pattern("Even() & SomePartial()(n)");
        assert!(
            result.is_ok(),
            "Failed to parse Even() & SomePartial()(n): {:?}",
            result
        );
        let pattern = result.unwrap();

        match &pattern.kind {
            PatternKind::And(patterns) => {
                assert_eq!(patterns.len(), 2);
                // First should be total active pattern
                match &patterns[0].kind {
                    PatternKind::Active { name, bindings, .. } => {
                        assert_eq!(name.name.as_str(), "Even");
                        assert!(bindings.is_empty());
                    }
                    _ => panic!("Expected Active pattern for Even()"),
                }
                // Second should be partial active pattern
                match &patterns[1].kind {
                    PatternKind::Active { name, bindings, .. } => {
                        assert_eq!(name.name.as_str(), "SomePartial");
                        assert_eq!(bindings.len(), 1);
                    }
                    _ => panic!("Expected Active pattern for SomePartial()(n)"),
                }
            }
            _ => panic!("Expected And pattern, got {:?}", pattern.kind),
        }
    }

    // ==================== VIEW PATTERN TESTS ====================

    #[test]
    fn test_view_pattern_simple() {
        // Simple view pattern: parity -> Even
        let result = parse_pattern("parity -> Even");
        assert!(result.is_ok(), "Failed to parse view pattern: {:?}", result);
        let pattern = result.unwrap();
        match &pattern.kind {
            PatternKind::View {
                view_function,
                pattern: inner,
            } => {
                // View function should be a path expression "parity"
                assert!(matches!(&view_function.kind, verum_ast::ExprKind::Path(p) if p.segments.len() == 1));
                // Inner pattern should be an identifier "Even" (bare name, no parens)
                assert!(
                    matches!(&inner.kind, PatternKind::Ident { .. } | PatternKind::Variant { .. }),
                    "Expected Ident or Variant inner pattern, got {:?}", inner.kind
                );
            }
            _ => panic!("Expected View pattern, got {:?}", pattern.kind),
        }
    }

    #[test]
    fn test_view_pattern_with_variant_binding() {
        // View pattern with variant that has bindings: parity -> Even(k)
        let result = parse_pattern("parity -> Even(k)");
        assert!(
            result.is_ok(),
            "Failed to parse view pattern with variant binding: {:?}",
            result
        );
        let pattern = result.unwrap();
        match &pattern.kind {
            PatternKind::View {
                view_function,
                pattern: inner,
            } => {
                // View function should be "parity"
                if let verum_ast::ExprKind::Path(p) = &view_function.kind {
                    let seg = &p.segments[0];
                    if let verum_ast::ty::PathSegment::Name(ident) = seg {
                        assert_eq!(ident.name.as_str(), "parity");
                    } else {
                        panic!("Expected Name segment");
                    }
                } else {
                    panic!("Expected Path expr for view function");
                }
                // Inner pattern should be Variant with one binding
                match &inner.kind {
                    PatternKind::Variant { path, data } => {
                        let last = path.segments.last().unwrap();
                        if let verum_ast::ty::PathSegment::Name(ident) = last {
                            assert_eq!(ident.name.as_str(), "Even");
                        }
                        assert!(data.is_some());
                    }
                    _ => panic!("Expected Variant inner pattern, got {:?}", inner.kind),
                }
            }
            _ => panic!("Expected View pattern, got {:?}", pattern.kind),
        }
    }

    #[test]
    fn test_view_pattern_nested() {
        // Nested view pattern: f -> g -> Some(x)
        let result = parse_pattern("f -> g -> Some(x)");
        assert!(
            result.is_ok(),
            "Failed to parse nested view pattern: {:?}",
            result
        );
        let pattern = result.unwrap();
        match &pattern.kind {
            PatternKind::View {
                view_function: outer_fn,
                pattern: inner,
            } => {
                // Outer view function is "f"
                if let verum_ast::ExprKind::Path(p) = &outer_fn.kind {
                    let seg = &p.segments[0];
                    if let verum_ast::ty::PathSegment::Name(ident) = seg {
                        assert_eq!(ident.name.as_str(), "f");
                    }
                }
                // Inner should be another View pattern with "g"
                match &inner.kind {
                    PatternKind::View {
                        view_function: inner_fn,
                        pattern: innermost,
                    } => {
                        if let verum_ast::ExprKind::Path(p) = &inner_fn.kind {
                            let seg = &p.segments[0];
                            if let verum_ast::ty::PathSegment::Name(ident) = seg {
                                assert_eq!(ident.name.as_str(), "g");
                            }
                        }
                        // Innermost should be Some(x) variant
                        assert!(matches!(&innermost.kind, PatternKind::Variant { .. }));
                    }
                    _ => panic!("Expected nested View pattern, got {:?}", inner.kind),
                }
            }
            _ => panic!("Expected View pattern, got {:?}", pattern.kind),
        }
    }

    #[test]
    fn test_view_pattern_with_wildcard() {
        // View pattern matching wildcard: transform -> _
        let result = parse_pattern("transform -> _");
        assert!(
            result.is_ok(),
            "Failed to parse view pattern with wildcard: {:?}",
            result
        );
        let pattern = result.unwrap();
        match &pattern.kind {
            PatternKind::View {
                pattern: inner, ..
            } => {
                assert!(matches!(&inner.kind, PatternKind::Wildcard));
            }
            _ => panic!("Expected View pattern, got {:?}", pattern.kind),
        }
    }

    #[test]
    fn test_view_pattern_in_or_pattern() {
        // View patterns combined with or: parity -> Even(k) | parity -> Odd(k)
        let result = parse_pattern("parity -> Even(k) | parity -> Odd(k)");
        assert!(
            result.is_ok(),
            "Failed to parse view pattern in or-pattern: {:?}",
            result
        );
        let pattern = result.unwrap();
        match &pattern.kind {
            PatternKind::Or(alternatives) => {
                assert_eq!(alternatives.len(), 2);
                assert!(matches!(&alternatives[0].kind, PatternKind::View { .. }));
                assert!(matches!(&alternatives[1].kind, PatternKind::View { .. }));
            }
            _ => panic!("Expected Or pattern, got {:?}", pattern.kind),
        }
    }
}
