//! Expression parser for Verum using hand-written recursive descent with Pratt parsing.
//!
//! This module implements parsing for all Verum expressions, including:
//! - Literals and paths
//! - Binary and unary operators with proper precedence
//! - Function calls and method calls
//! - Pipeline operator (`|>`) and null coalescing (`??`)
//! - Optional chaining (`?.`)
//! - Comprehensions (list and stream)
//! - Control flow (if, match, loops)
//! - Closures and async blocks
//!
//! # Operator Precedence (Pratt Parsing)
//!
//! From lowest to highest binding power:
//! 1. Pipeline `|>` (left-associative) - BP 1
//! 2. Null coalescing `??` (right-associative) - BP 2
//! 3. Assignment `=`, `+=`, etc. (right-associative) - BP 3
//! 4. Logical OR `||` - BP 4
//! 5. Logical AND `&&` - BP 5
//! 6. Comparison `==`, `!=`, `<`, `>`, `<=`, `>=` - BP 6
//! 7. Bitwise OR `|` - BP 7
//! 8. Bitwise XOR `^` - BP 8
//! 9. Bitwise AND `&` - BP 9
//! 10. Shift `<<`, `>>` - BP 10
//! 11. Addition `+`, `-` - BP 11
//! 12. Multiplication `*`, `/`, `%` - BP 12
//! 13. Exponentiation `**` (right-associative) - BP 13
//! 14. Range `..`, `..=` - BP 14
//! 15. Unary prefix `!`, `-`, `~`, `&`, `*` - BP 15
//! 16. Postfix `.`, `?.`, `()`, `[]`, `?`, `as`, `::` - BP 16

use verum_ast::{
    BinOp, Block, Expr, ExprKind, Literal, Path, PathSegment, Span, Type, TypeKind, UnOp, expr::*,
    pattern::{FieldPattern, MatchArm, Pattern, PatternKind},
    ty::Ident,
};
use verum_common::{Heap, List, Maybe, Text};
use verum_lexer::{Token, TokenKind};

use crate::attr_validation::AttributeValidationWarning;
use crate::error::ParseError;
use crate::parser::{ParseResult, RecursiveParser};

// Note: parse_type(), parse_pattern(), and parse_stmt() are defined as methods on RecursiveParser
// in their respective modules (ty.rs, pattern.rs, stmt.rs). They are called directly as self.method().

// =============================================================================
// KNOWN @ CONSTRUCTS (per grammar/verum.ebnf)
// =============================================================================

/// Known meta-function names as defined in grammar/verum.ebnf Section 2.20.6
/// These are valid after @ prefix in expression context.
///
/// Meta-functions use `@` prefix in expression context. Grammar:
/// meta_function_call = '@' , meta_function_name , [ '(' , arg_list , ')' ] ;
/// Only known meta-function names are valid after `@` prefix.
const KNOWN_META_FUNCTIONS: &[&str] = &[
    // Compile-time evaluation
    "const",
    // Diagnostics
    "error",
    "warning",
    // Token manipulation
    "stringify",
    "concat",
    // Configuration
    "cfg",
    // Source location introspection
    "file",
    "line",
    "column",
    "module",
    "function",
    // Type introspection meta-functions for compile-time type reflection
    "type_name",
    "type_fields",
    "field_access",
    "type_of",
    "fields_of",
    "variants_of",
    "is_struct",
    "is_enum",
    "is_tuple",
    "implements",
    // VBC intrinsic calls (core/math library)
    "vbc",
    "vbc_raw",
    // MLIR intrinsic calls (core/math/internal)
    "mlir",
    "mlir_typed",
    // Context manager for parameter binding
    "with_params",
    // Intrinsic function call by name
    "intrinsic",
    // Runtime intrinsics callable via @name(args) syntax
    "get_tag",
    "abs",
    "sin",
    "cos",
    "sqrt",
    "log",
    "exp",
    "floor",
    "ceil",
    "round",
    "min",
    "max",
    "clamp",
    "pow",
    "has_gpu",
    // Error handling and async meta-functions
    "catch",
    "catch_cbgr_violation",
    "block_on",
    "timeout",
    // Memory management meta-functions
    "forget",
    "ref_eq",
    "get_generation",
    "get_stored_generation",
    // Collection/utility meta-functions
    "unwrap",
    "list_with_capacity",
];

/// Attributes that are ONLY valid on declarations, NOT as expressions.
/// Using these as @name(...) in expression context is an ERROR.
///
/// These are declaration-level attributes per grammar/verum.ebnf Section 2.16
///
/// NOTE: This does NOT include function MODIFIERS like `pure`, `async`, `unsafe`, `meta`
/// which are keywords (e.g., `pure fn foo()`), not attributes (`@pure fn foo()`).
const DECLARATION_ONLY_ATTRIBUTES: &[&str] = &[
    // Function attributes (per grammar/verum.ebnf attribute production)
    // NOTE: "intrinsic" is NOT here — @intrinsic("name", arg1, arg2) is a valid
    // expression-level meta-function call (used in stdlib intrinsic definitions).
    // @intrinsic("name") as a declaration attribute is handled by the attribute parser.
    "extern",    // @extern("C") is attribute on fn declarations
    "inline",
    "cold",
    "hot",
    "test",
    "bench",
    "export",
    "no_mangle",
    // Type attributes
    "derive", // @derive(...) - grammar: derive_attribute
    "repr",
    "sealed",
    // Impl attributes
    "specialize", // @specialize - grammar: specialize_attribute
    // Verification attributes
    "verify", // @verify(runtime|static|formal) - grammar: verify_attribute
    // Lint/diagnostic attributes
    "deprecated",
    "doc",
    "allow",
    "warn",
    "deny",
];

/// Check if a name is a declaration-only attribute (not valid in expression context).
fn is_declaration_only_attribute(name: &str) -> bool {
    DECLARATION_ONLY_ATTRIBUTES.contains(&name)
}

/// Calculate edit distance between two strings for similarity matching.
#[allow(clippy::needless_range_loop)] // DP initialization is clearer with direct indexing
fn edit_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();

    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }

    let mut dp = vec![vec![0; n + 1]; m + 1];

    for i in 0..=m {
        dp[i][0] = i;
    }
    for j in 0..=n {
        dp[0][j] = j;
    }

    for i in 1..=m {
        for j in 1..=n {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            dp[i][j] = (dp[i - 1][j] + 1) // deletion
                .min(dp[i][j - 1] + 1) // insertion
                .min(dp[i - 1][j - 1] + cost); // substitution
        }
    }

    dp[m][n]
}

/// Find similar meta-function names for "did you mean?" suggestions.
fn find_similar_meta_function(unknown: &str) -> Option<&'static str> {
    let mut best_match: Option<&str> = None;
    let mut best_distance = usize::MAX;
    let max_distance = 3; // Maximum edit distance for suggestions

    for &known in KNOWN_META_FUNCTIONS {
        let distance = edit_distance(unknown, known);
        if distance < best_distance && distance <= max_distance {
            best_distance = distance;
            best_match = Some(known);
        }
    }

    // Also check if unknown is a prefix of any known name
    if best_match.is_none() {
        for &known in KNOWN_META_FUNCTIONS {
            if known.starts_with(unknown) || unknown.starts_with(known) {
                return Some(known);
            }
        }
    }

    best_match
}

/// Check if a name is a known meta-function.
fn is_known_meta_function(name: &str) -> bool {
    KNOWN_META_FUNCTIONS.contains(&name)
}

/// Map Rust macro names to their Verum equivalents.
/// Returns None if the name is not a known Rust macro.
fn rust_macro_to_verum(name: &str) -> Option<&'static str> {
    match name {
        "println" => Some("print(...)"),
        "print" => Some("print(...) (without !)"),
        "eprintln" => Some("eprint(...)"),
        "eprint" => Some("eprint(...) (without !)"),
        "format" => Some("f\"...\" (format string literal)"),
        "panic" => Some("panic(...) (without !)"),
        "assert" => Some("assert(...) (without !)"),
        "assert_eq" => Some("assert_eq(...) (without !)"),
        "assert_ne" => Some("assert_ne(...) (without !)"),
        "unreachable" => Some("unreachable() (without !)"),
        "unimplemented" => Some("unimplemented() (without !)"),
        "todo" => Some("todo() (without !)"),
        "vec" => Some("List(...) or [a, b, c]"),
        "dbg" => Some("debug(...) (without !)"),
        "write" => Some("write(...) (without !)"),
        "writeln" => Some("writeln(...) (without !)"),
        "matches" => Some("x is Pattern (is operator)"),
        "include_str" => Some("@include_str(...)"),
        "include_bytes" => Some("@include_bytes(...)"),
        "cfg" => Some("@cfg(...)"),
        "env" => Some("@env(...)"),
        "concat" => Some("@concat(...)"),
        "stringify" => Some("@stringify(...)"),
        _ => None,
    }
}

/// Map Rust type names to their Verum semantic type equivalents.
/// Returns None if the name is not a known Rust type.
pub(crate) fn rust_type_to_verum(name: &str) -> Option<&'static str> {
    match name {
        "String" => Some("Text"),
        "Vec" => Some("List"),
        "HashMap" => Some("Map"),
        "HashSet" => Some("Set"),
        "BTreeMap" => Some("Map"),
        "BTreeSet" => Some("Set"),
        "Box" => Some("Heap"),
        "Rc" => Some("Shared"),
        "Arc" => Some("Shared"),
        "Option" => Some("Maybe"),
        "Cell" => Some("Mut"),
        "RefCell" => Some("Mut"),
        "Mutex" => Some("Mutex"),
        "RwLock" => Some("RwLock"),
        _ => None,
    }
}

impl<'a> RecursiveParser<'a> {
    /// Parse an expression with full operator precedence using Pratt parsing.
    pub fn parse_expr(&mut self) -> ParseResult<Expr> {
        self.parse_expr_bp(0)
    }

    /// Parse an expression without allowing struct literals.
    /// This is used in contexts like `for x in expr { }` where we need to
    /// prevent `expr` from consuming `{ }` as a struct literal.
    /// Also used for function contract clauses (requires/ensures) to prevent
    /// the function body `{` from being consumed.
    pub(crate) fn parse_expr_no_struct(&mut self) -> ParseResult<Expr> {
        self.parse_expr_bp_no_struct(0)
    }

    /// Parse an expression for meta predicates in type definitions.
    /// This variant doesn't parse struct literals AND doesn't treat `is` as an infix operator.
    /// Used for `where meta SIZE > 0 is { ... }` to prevent `is` from being consumed.
    pub(crate) fn parse_expr_for_meta_predicate(&mut self) -> ParseResult<Expr> {
        self.parse_expr_bp_for_meta_predicate(0)
    }

    /// Parse expression with binding power for meta predicates.
    /// Excludes struct literals and `is` operator to support `where meta N > 0 is { ... }`.
    fn parse_expr_bp_for_meta_predicate(&mut self, min_bp: u8) -> ParseResult<Expr> {
        self.enter_recursion()?;
        let result = self.parse_expr_bp_for_meta_predicate_inner(min_bp);
        self.exit_recursion();
        result
    }

    fn parse_expr_bp_for_meta_predicate_inner(&mut self, min_bp: u8) -> ParseResult<Expr> {
        // Parse prefix/primary expression (without struct literals)
        let mut lhs = self.parse_prefix_expr_no_struct()?;

        // Parse infix and postfix operators (excluding `is` operator)
        loop {
            // Safety: prevent infinite loop and check abort flag
            if !self.tick() || self.is_aborted() {
                break;
            }

            // CRITICAL: Check for `is` token BEFORE infix handling.
            // In meta predicate context, `is` is used for type definition syntax
            // (e.g., `where meta N > 0 is { ... }`), not as a pattern test operator.
            if self.stream.check(&TokenKind::Is) {
                break;
            }

            // Handle postfix operators first (highest precedence)
            let (new_lhs, found_postfix) = self.try_parse_postfix_impl(lhs, false)?;
            lhs = new_lhs;
            if found_postfix {
                continue;
            }

            // Check for infix operators
            let op_kind = match self.stream.peek_kind() {
                Some(kind) => kind.clone(),
                None => break,
            };

            // Get binding power for this operator
            let (left_bp, right_bp) = match self.infix_binding_power(&op_kind) {
                Some(bp) => bp,
                None => break,
            };

            if left_bp < min_bp {
                break;
            }

            self.stream.advance();

            // Handle special operators
            if op_kind == TokenKind::PipeGt {
                let rhs = self.parse_expr_bp_for_meta_predicate(right_bp)?;
                let span = lhs.span.merge(rhs.span);
                lhs = Expr::new(
                    ExprKind::Pipeline {
                        left: Box::new(lhs),
                        right: Box::new(rhs),
                    },
                    span,
                );
                continue;
            }

            if op_kind == TokenKind::QuestionQuestion {
                let rhs = self.parse_expr_bp_for_meta_predicate(right_bp)?;
                let span = lhs.span.merge(rhs.span);
                lhs = Expr::new(
                    ExprKind::NullCoalesce {
                        left: Box::new(lhs),
                        right: Box::new(rhs),
                    },
                    span,
                );
                continue;
            }

            if op_kind == TokenKind::DotDot || op_kind == TokenKind::DotDotEq {
                let inclusive = op_kind == TokenKind::DotDotEq;
                let end = if self.stream.at_end()
                    || !self.stream.peek().is_some_and(|t| t.starts_expr())
                {
                    Maybe::None
                } else {
                    Maybe::Some(Box::new(self.parse_expr_bp_for_meta_predicate(right_bp)?))
                };
                let span = if let Maybe::Some(ref end_expr) = end {
                    lhs.span.merge(end_expr.span)
                } else {
                    lhs.span
                };
                lhs = Expr::new(
                    ExprKind::Range {
                        start: Maybe::Some(Box::new(lhs)),
                        end,
                        inclusive,
                    },
                    span,
                );
                continue;
            }

            if op_kind == TokenKind::As {
                // Use parse_type_with_lookahead to support refined casts like `x as Int{> 0}`
                // The lookahead logic distinguishes refinements ({> 0}) from blocks ({ break; })
                let ty = self.parse_type_with_lookahead()?;
                let span = lhs.span.merge(ty.span);
                lhs = Expr::new(
                    ExprKind::Cast {
                        expr: Box::new(lhs),
                        ty,
                    },
                    span,
                );
                continue;
            }

            // Standard binary operator
            let bin_op = self.token_to_binop(&op_kind)?;

            // E046: Validate assignment LHS
            if bin_op.is_assignment() {
                if self.stream.check(&TokenKind::Eq) {
                    return Err(ParseError::compound_assign_invalid(
                        "invalid compound assignment operator",
                        self.stream.current_span(),
                    ));
                }
                if self.stream.check(&TokenKind::Semicolon) || self.stream.check(&TokenKind::RBrace)
                {
                    return Err(ParseError::let_missing_value(self.stream.current_span())
                        .with_help("assignment requires a value on the right-hand side"));
                }

                // Check if this is a destructuring assignment (tuple, array, record on LHS)
                if Self::is_destructuring_target(&lhs) {
                    let pattern = Self::expr_to_assignment_pattern(&lhs)?;
                    let rhs = self.parse_expr_bp_for_meta_predicate(right_bp)?;
                    let span = lhs.span.merge(rhs.span);
                    lhs = Expr::new(
                        ExprKind::DestructuringAssign {
                            pattern,
                            op: bin_op,
                            value: Heap::new(rhs),
                        },
                        span,
                    );
                    continue;
                }

                if !Self::is_valid_assignment_target(&lhs) {
                    return Err(ParseError::assignment_invalid(
                        "invalid left-hand side of assignment",
                        lhs.span,
                    ));
                }
            }

            let rhs = self.parse_expr_bp_for_meta_predicate(right_bp)?;

            let span = lhs.span.merge(rhs.span);
            lhs = Expr::new(
                ExprKind::Binary {
                    op: bin_op,
                    left: Box::new(lhs),
                    right: Box::new(rhs),
                },
                span,
            );
        }

        Ok(lhs)
    }

    /// Parse expression with binding power, but don't parse struct literals.
    pub(crate) fn parse_expr_bp_no_struct(&mut self, min_bp: u8) -> ParseResult<Expr> {
        self.enter_recursion()?;
        let result = self.parse_expr_bp_no_struct_inner(min_bp);
        self.exit_recursion();
        result
    }

    fn parse_expr_bp_no_struct_inner(&mut self, min_bp: u8) -> ParseResult<Expr> {
        let start_pos = self.stream.position();

        // Parse prefix/primary expression (without struct literals)
        let mut lhs = self.parse_prefix_expr_no_struct()?;

        // Parse infix and postfix operators (same as regular parse_expr_bp)
        loop {
            // Safety: prevent infinite loop and check abort flag
            if !self.tick() || self.is_aborted() {
                break;
            }

            // Handle postfix operators first (highest precedence)
            // PERF: try_parse_postfix takes ownership and returns (expr, found)
            // CRITICAL: In no_struct mode, don't parse .field { } as variant constructor
            let (new_lhs, found_postfix) = self.try_parse_postfix_impl(lhs, false)?;
            lhs = new_lhs;
            if found_postfix {
                continue;
            }

            // Check for infix operators
            let op_kind = match self.stream.peek_kind() {
                Some(kind) => kind.clone(),
                None => break,
            };

            // Get binding power for this operator
            let (left_bp, right_bp) = match self.infix_binding_power(&op_kind) {
                Some(bp) => bp,
                None => break,
            };

            if left_bp < min_bp {
                break;
            }

            self.stream.advance();

            // Handle special operators
            if op_kind == TokenKind::PipeGt {
                // Pipeline: lhs |> rhs  OR  lhs |> .method(args)
                if self.stream.check(&TokenKind::Dot) {
                    lhs = self.parse_pipe_method_call(lhs, start_pos)?;
                } else {
                    let rhs = self.parse_expr_bp_no_struct(right_bp)?;
                    let span = lhs.span.merge(rhs.span);
                    lhs = Expr::new(
                        ExprKind::Pipeline {
                            left: Box::new(lhs),
                            right: Box::new(rhs),
                        },
                        span,
                    );
                }
                continue;
            }

            if op_kind == TokenKind::QuestionQuestion {
                let rhs = self.parse_expr_bp_no_struct(right_bp)?;
                let span = lhs.span.merge(rhs.span);
                lhs = Expr::new(
                    ExprKind::NullCoalesce {
                        left: Box::new(lhs),
                        right: Box::new(rhs),
                    },
                    span,
                );
                continue;
            }

            if op_kind == TokenKind::DotDot || op_kind == TokenKind::DotDotEq {
                let inclusive = op_kind == TokenKind::DotDotEq;
                // Inside comprehensions, `if`/`for`/`let` are clause keywords,
                // not range-end expressions.
                // In no-struct mode, `{` is NOT a valid range end — it's the loop body.
                // e.g., `for i in 0.. { body }` — the `{` starts the loop body, not a range end.
                let end = if self.stream.at_end()
                    || !self.stream.peek().is_some_and(|t| t.starts_expr())
                    || self.stream.check(&TokenKind::LBrace)
                    || (self.comprehension_depth > 0
                        && self.stream.peek().is_some_and(|t| {
                            matches!(
                                t.kind,
                                TokenKind::If | TokenKind::For | TokenKind::Let
                            )
                        }))
                {
                    Maybe::None
                } else {
                    Maybe::Some(Box::new(self.parse_expr_bp_no_struct(right_bp)?))
                };
                let span = if let Maybe::Some(ref end_expr) = end {
                    lhs.span.merge(end_expr.span)
                } else {
                    lhs.span
                };
                lhs = Expr::new(
                    ExprKind::Range {
                        start: Maybe::Some(Box::new(lhs)),
                        end,
                        inclusive,
                    },
                    span,
                );
                continue;
            }

            if op_kind == TokenKind::As {
                // Use parse_type_with_lookahead to support refined casts like `x as Int{> 0}`
                // The lookahead logic distinguishes refinements ({> 0}) from blocks ({ break; })
                let ty = self.parse_type_with_lookahead()?;
                let span = lhs.span.merge(ty.span);
                lhs = Expr::new(
                    ExprKind::Cast {
                        expr: Box::new(lhs),
                        ty,
                    },
                    span,
                );
                continue;
            }

            if op_kind == TokenKind::Is {
                // Pattern test: lhs is Pattern or lhs is not Pattern
                // Check for optional 'not' keyword (parsed as identifier)
                let negated = if let Some(TokenKind::Ident(name)) = self.stream.peek_kind() {
                    if name.as_str() == "not" {
                        self.stream.advance();
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };
                // Use parse_pattern_no_struct to prevent `{ }` from being consumed as a record pattern.
                // This is important in if/while conditions where `{ }` starts the block, not the pattern.
                // For example: `if value is None { }` - the `{` is the block start, not `None { }` record.
                let pattern = self.parse_pattern_no_struct()?;
                let span = lhs.span.merge(pattern.span);
                lhs = Expr::new(
                    ExprKind::Is {
                        expr: Box::new(lhs),
                        pattern,
                        negated,
                    },
                    span,
                );
                continue;
            }

            // Standard binary operator
            let bin_op = self.token_to_binop(&op_kind)?;

            // E046: Validate assignment LHS
            if bin_op.is_assignment() {
                // E047: Check for invalid compound assignment (e.g., x +== 5)
                // This happens when we have += followed by another = or similar
                if self.stream.check(&TokenKind::Eq) {
                    return Err(ParseError::compound_assign_invalid(
                        "invalid compound assignment operator",
                        self.stream.current_span(),
                    ));
                }

                // E041: Check for missing RHS (x =;)
                if self.stream.check(&TokenKind::Semicolon) || self.stream.check(&TokenKind::RBrace) {
                    return Err(ParseError::let_missing_value(self.stream.current_span())
                        .with_help("assignment requires a value on the right-hand side"));
                }

                // Check if this is a destructuring assignment (tuple, array, record on LHS)
                if Self::is_destructuring_target(&lhs) {
                    // Convert expression to assignment pattern
                    let pattern = Self::expr_to_assignment_pattern(&lhs)?;

                    // Parse RHS value
                    let rhs = self.parse_expr_bp_no_struct(right_bp)?;

                    let span = lhs.span.merge(rhs.span);
                    lhs = Expr::new(
                        ExprKind::DestructuringAssign {
                            pattern,
                            op: bin_op,
                            value: Heap::new(rhs),
                        },
                        span,
                    );
                    continue;
                }

                // Not a destructuring target - check if it's a valid simple assignment target
                if !Self::is_valid_assignment_target(&lhs) {
                    return Err(ParseError::assignment_invalid(
                        "invalid left-hand side of assignment",
                        lhs.span,
                    ));
                }
            }

            let rhs = self.parse_expr_bp_no_struct(right_bp)?;

            // Note: Verum DOES allow chained assignment (a = b = c = 42)
            // Assignment is right-associative, so a = b = c = 42 parses as a = (b = (c = 42))

            let span = lhs.span.merge(rhs.span);
            lhs = Expr::new(
                ExprKind::Binary {
                    op: bin_op,
                    left: Heap::new(lhs),
                    right: Heap::new(rhs),
                },
                span,
            );
        }

        Ok(lhs)
    }

    /// Check if an expression is a valid assignment target (lvalue).
    fn is_valid_assignment_target(expr: &Expr) -> bool {
        match &expr.kind {
            // Paths (identifiers and qualified names) are valid targets
            ExprKind::Path(_) => true,
            // Field access is valid
            ExprKind::Field { .. } => true,
            // Index access is valid
            ExprKind::Index { .. } => true,
            // Dereference is valid
            ExprKind::Unary { op, .. } if *op == UnOp::Deref => true,
            // Tuple index access is valid
            ExprKind::TupleIndex { .. } => true,
            // Everything else is invalid (including literals, calls, binary ops, etc.)
            _ => false,
        }
    }

    /// Check if an expression is a potential destructuring target.
    ///
    /// This returns true for expressions that can be converted to assignment patterns:
    /// - Tuples: `(a, b, c)`
    /// - Arrays: `[a, b, c]`
    /// - Records: `Point { x, y }`
    /// - Parenthesized expressions (which might contain destructuring)
    ///
    /// These are distinguished from simple assignment targets (place expressions).
    fn is_destructuring_target(expr: &Expr) -> bool {
        matches!(
            &expr.kind,
            ExprKind::Tuple(_) | ExprKind::Array(_) | ExprKind::Record { .. } | ExprKind::Paren(_)
        )
    }

    /// Convert an expression to an assignment pattern for destructuring assignment.
    ///
    /// This function converts tuple, array, and record expressions to their
    /// corresponding pattern forms. Each element in the pattern must be
    /// either a valid place expression (assignable) or a wildcard.
    ///
    /// Returns an error if the expression cannot be converted to a valid
    /// assignment pattern (e.g., contains literals or function calls).
    ///
    /// Unified destructuring system: converts LHS expressions to assignment patterns.
    /// Converts an expression into an assignment pattern for destructuring.
    ///
    /// Destructuring assignment allows extracting values from compound structures
    /// into individual variables. This function converts the LHS expression into
    /// a pattern that can be used to perform the destructuring.
    ///
    /// # Supported Patterns
    ///
    /// | Expression | Pattern | Example |
    /// |------------|---------|---------|
    /// | Identifier | Binding | `x = value` |
    /// | Wildcard | Discard | `_ = value` |
    /// | Tuple | Tuple | `(a, b) = (1, 2)` |
    /// | Array | Array | `[a, b] = [1, 2]` |
    /// | Array with rest | Slice | `[first, ..rest] = list` |
    /// | Record | Struct | `Point { x, y } = point` |
    /// | Record with rest | Struct | `Config { timeout, .. } = cfg` |
    ///
    /// # Errors
    ///
    /// Returns `ParseError::assignment_invalid` for expressions that cannot
    /// be valid assignment targets (literals, function calls, operators, etc.).
    fn expr_to_assignment_pattern(expr: &Expr) -> Result<Pattern, ParseError> {
        match &expr.kind {
            // ─────────────────────────────────────────────────────────────────
            // BINDING PATTERNS: Convert identifiers to variable bindings
            // ─────────────────────────────────────────────────────────────────
            ExprKind::Path(path) if path.segments.len() == 1 => {
                Self::path_segment_to_pattern(&path.segments[0], expr.span)
            }

            // ─────────────────────────────────────────────────────────────────
            // COMPOUND PATTERNS: Recursively convert nested structures
            // ─────────────────────────────────────────────────────────────────
            ExprKind::Tuple(elements) => {
                Self::elements_to_tuple_pattern(elements, expr.span)
            }

            ExprKind::Paren(inner) => {
                let inner_pattern = Self::expr_to_assignment_pattern(inner)?;
                Ok(Pattern::new(PatternKind::Paren(Heap::new(inner_pattern)), expr.span))
            }

            ExprKind::Array(ArrayExpr::List(elements)) => {
                Self::elements_to_array_pattern(elements, expr.span)
            }

            ExprKind::Record { path, fields, base } => {
                Self::record_expr_to_pattern(path, fields, base, expr.span)
            }

            // ─────────────────────────────────────────────────────────────────
            // REST PATTERNS: Handle `..` and `..rest` in arrays
            // ─────────────────────────────────────────────────────────────────
            ExprKind::Range { start: Maybe::None, end, inclusive: false } => {
                Self::range_to_rest_pattern(end, expr.span)
            }

            // ─────────────────────────────────────────────────────────────────
            // PLACE EXPRESSIONS: Allow field/index/deref as assignment targets
            // in destructuring patterns like `(items[i], items[j]) = ...`
            // These become wildcard patterns at the parser level;
            // the actual place assignment is handled by codegen.
            // ─────────────────────────────────────────────────────────────────
            ExprKind::Field { .. }
            | ExprKind::Index { .. }
            | ExprKind::TupleIndex { .. }
            | ExprKind::Unary { op: UnOp::Deref, .. } => {
                Ok(Pattern::wildcard(expr.span))
            }

            // ─────────────────────────────────────────────────────────────────
            // INVALID PATTERNS: Specific error messages for common mistakes
            // ─────────────────────────────────────────────────────────────────
            ExprKind::Literal(_) => {
                Err(ParseError::assignment_invalid(
                    "literals cannot be assignment targets; use an identifier to bind the value",
                    expr.span,
                ))
            }

            ExprKind::Call { .. } | ExprKind::MethodCall { .. } => {
                Err(ParseError::assignment_invalid(
                    "function and method calls cannot be assignment targets; they compute values, not receive them",
                    expr.span,
                ))
            }

            ExprKind::Binary { op, .. } if !op.is_assignment() => {
                Err(ParseError::assignment_invalid(
                    "binary expressions cannot be assignment targets; only identifiers and destructuring patterns are valid",
                    expr.span,
                ))
            }

            ExprKind::Unary { .. } => {
                Err(ParseError::assignment_invalid(
                    "unary expressions cannot be assignment targets",
                    expr.span,
                ))
            }

            ExprKind::If { .. } | ExprKind::Match { .. } | ExprKind::Loop { .. } => {
                Err(ParseError::assignment_invalid(
                    "control flow expressions cannot be assignment targets",
                    expr.span,
                ))
            }

            ExprKind::Closure { .. } => {
                Err(ParseError::assignment_invalid(
                    "closures cannot be assignment targets",
                    expr.span,
                ))
            }

            // ─────────────────────────────────────────────────────────────────
            // CATCH-ALL: All other expressions are invalid targets
            // ─────────────────────────────────────────────────────────────────
            _ => Err(ParseError::assignment_invalid(
                "expression cannot be used as destructuring assignment target",
                expr.span,
            )),
        }
    }

    /// Converts a single path segment to a pattern.
    /// Handles identifiers and special paths (self, super, crate).
    fn path_segment_to_pattern(segment: &PathSegment, span: Span) -> Result<Pattern, ParseError> {
        match segment {
            PathSegment::Name(ident) if ident.name.as_str() == "_" => {
                // Underscore `_` is a wildcard pattern that discards the value
                Ok(Pattern::new(PatternKind::Wildcard, span))
            }
            PathSegment::Name(ident) => {
                // Regular identifier becomes a binding pattern
                Ok(Pattern::new(
                    PatternKind::Ident {
                        by_ref: false,
                        mutable: false, // Mutability is checked semantically
                        name: Ident {
                            name: ident.name.clone(),
                            span: ident.span,
                        },
                        subpattern: Maybe::None,
                    },
                    span,
                ))
            }
            // Special paths (self, super, crate, relative) are not valid assignment targets
            _ => Err(ParseError::assignment_invalid(
                "special paths (self, super, crate) cannot be used in destructuring assignment",
                span,
            )),
        }
    }

    /// Converts tuple elements to a tuple pattern.
    fn elements_to_tuple_pattern(elements: &List<Expr>, span: Span) -> Result<Pattern, ParseError> {
        let patterns: Result<List<Pattern>, ParseError> = elements
            .iter()
            .map(Self::expr_to_assignment_pattern)
            .collect();
        Ok(Pattern::new(PatternKind::Tuple(patterns?), span))
    }

    /// Converts array elements to an array pattern.
    fn elements_to_array_pattern(elements: &List<Expr>, span: Span) -> Result<Pattern, ParseError> {
        let patterns: Result<List<Pattern>, ParseError> = elements
            .iter()
            .map(Self::expr_to_assignment_pattern)
            .collect();
        Ok(Pattern::new(PatternKind::Array(patterns?), span))
    }

    /// Converts a record expression to a record pattern.
    ///
    /// Handles two cases:
    /// - `Point { x, y }` - destructures all fields
    /// - `Config { timeout, .. }` - destructures some fields, ignores rest
    ///
    /// Note: Struct update syntax `Point { x, ..base }` is NOT allowed in
    /// destructuring assignment; use `..` alone to ignore remaining fields.
    fn record_expr_to_pattern(
        path: &Path,
        fields: &List<FieldInit>,
        base: &Maybe<Box<Expr>>,
        span: Span,
    ) -> Result<Pattern, ParseError> {
        // Determine if this has a rest pattern (..)
        // We use an empty tuple as a sentinel for rest-only (no base expression)
        let has_rest = match base {
            Maybe::Some(base_expr) => {
                if matches!(&base_expr.kind, ExprKind::Tuple(elements) if elements.is_empty()) {
                    true // Rest-only pattern: `{ x, y, .. }`
                } else {
                    // Struct update syntax `{ ..base }` is not valid in destructuring
                    return Err(ParseError::assignment_invalid(
                        "struct update syntax `..base` is not allowed in destructuring assignment; \
                         use `..` alone to ignore remaining fields",
                        span,
                    ));
                }
            }
            Maybe::None => false,
        };

        // Convert each field to a field pattern
        let field_patterns: Result<List<FieldPattern>, ParseError> = fields
            .iter()
            .map(|field| {
                let pattern = match &field.value {
                    Maybe::Some(value) => Maybe::Some(Self::expr_to_assignment_pattern(value)?),
                    Maybe::None => Maybe::None, // Shorthand: `{ x }` means `{ x: x }`
                };
                Ok(FieldPattern::new(field.name.clone(), pattern, field.span))
            })
            .collect();

        Ok(Pattern::new(
            PatternKind::Record {
                path: path.clone(),
                fields: field_patterns?,
                rest: has_rest,
            },
            span,
        ))
    }

    /// Converts a range expression to a rest pattern for arrays.
    ///
    /// - `..` → anonymous rest (discard remaining elements)
    /// - `..rest` → named rest (capture remaining elements into `rest`)
    fn range_to_rest_pattern(end: &Maybe<Box<Expr>>, span: Span) -> Result<Pattern, ParseError> {
        match end {
            Maybe::None => {
                // Anonymous rest: `[first, ..]`
                Ok(Pattern::new(PatternKind::Rest, span))
            }
            Maybe::Some(end_expr) => {
                // Named rest: `[first, ..rest]` - bind remaining to `rest`
                if let ExprKind::Path(path) = &end_expr.kind {
                    if path.segments.len() == 1 {
                        if let PathSegment::Name(ident) = &path.segments[0] {
                            return Ok(Pattern::new(
                                PatternKind::Ident {
                                    by_ref: false,
                                    mutable: false,
                                    name: Ident {
                                        name: ident.name.clone(),
                                        span: ident.span,
                                    },
                                    subpattern: Maybe::None,
                                },
                                span,
                            ));
                        }
                    }
                }
                Err(ParseError::assignment_invalid(
                    "invalid rest pattern: expected `..` or `..name`",
                    span,
                ))
            }
        }
    }

    /// Parse a prefix expression without allowing struct literals.
    fn parse_prefix_expr_no_struct(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();

        // Check for unary operators (same as regular prefix parsing)
        let op = match self.stream.peek_kind() {
            Some(TokenKind::Bang) => {
                self.stream.advance();
                Some(UnOp::Not)
            }
            Some(TokenKind::Minus) => {
                self.stream.advance();
                Some(UnOp::Neg)
            }
            Some(TokenKind::Tilde) => {
                self.stream.advance();
                Some(UnOp::BitNot)
            }
            Some(TokenKind::Star) => {
                self.stream.advance();
                Some(UnOp::Deref)
            }
            // Handle ** (StarStar) token by splitting it into two * tokens for double dereference: **x
            Some(TokenKind::StarStar) => {
                if self.pending_star {
                    // Consume the pending * from a previous ** split
                    self.pending_star = false;
                    // Advance past the ** token
                    if self.stream.peek_kind() == Some(&TokenKind::StarStar) {
                        self.stream.advance();
                    }
                    Some(UnOp::Deref)
                } else {
                    // Split ** into two * tokens
                    self.pending_star = true;
                    Some(UnOp::Deref)
                }
            }
            Some(TokenKind::Ampersand) => {
                self.stream.advance();
                // Check for three-tier references: &checked, &unsafe, &mut
                if self.stream.consume(&TokenKind::Checked).is_some() {
                    if self.stream.consume(&TokenKind::Mut).is_some() {
                        Some(UnOp::RefCheckedMut)
                    } else {
                        Some(UnOp::RefChecked)
                    }
                } else if self.stream.consume(&TokenKind::Unsafe).is_some() {
                    if self.stream.consume(&TokenKind::Mut).is_some() {
                        Some(UnOp::RefUnsafeMut)
                    } else {
                        Some(UnOp::RefUnsafe)
                    }
                } else if self.stream.consume(&TokenKind::Mut).is_some() {
                    Some(UnOp::RefMut)
                } else {
                    Some(UnOp::Ref)
                }
            }
            // Handle && (AmpersandAmpersand) token by splitting it into two & tokens for double reference: &&x
            Some(TokenKind::AmpersandAmpersand) => {
                if self.pending_ampersand {
                    // Consume the pending & from a previous && split
                    self.pending_ampersand = false;
                    // Advance past the && token
                    if self.stream.peek_kind() == Some(&TokenKind::AmpersandAmpersand) {
                        self.stream.advance();
                    }
                    if self.stream.consume(&TokenKind::Mut).is_some() {
                        Some(UnOp::RefMut)
                    } else {
                        Some(UnOp::Ref)
                    }
                } else {
                    // Split && into two & tokens
                    self.pending_ampersand = true;
                    Some(UnOp::Ref)
                }
            }
            Some(TokenKind::Percent) => {
                self.stream.advance();
                if self.stream.consume(&TokenKind::Mut).is_some() {
                    Some(UnOp::OwnMut)
                } else {
                    Some(UnOp::Own)
                }
            }
            _ => None,
        };

        if let Some(unary_op) = op {
            let expr = self.parse_expr_bp_no_struct(15)?;
            let span = self.stream.make_span(start_pos);
            return Ok(Expr::new(
                ExprKind::Unary {
                    op: unary_op,
                    expr: Box::new(expr),
                },
                span,
            ));
        }

        // Check for range with no start
        if self.stream.check(&TokenKind::DotDot) || self.stream.check(&TokenKind::DotDotEq) {
            let inclusive = self.stream.consume(&TokenKind::DotDotEq).is_some();
            if !inclusive {
                self.stream.consume(&TokenKind::DotDot);
            }

            let end =
                if self.stream.at_end()
                    || !self.stream.peek().is_some_and(|t| t.starts_expr())
                    || (self.comprehension_depth > 0
                        && self.stream.peek().is_some_and(|t| {
                            matches!(
                                t.kind,
                                TokenKind::If | TokenKind::For | TokenKind::Let
                            )
                        }))
                {
                    Maybe::None
                } else {
                    Maybe::Some(Box::new(self.parse_expr_bp_no_struct(15)?))
                };

            let span = self.stream.make_span(start_pos);
            return Ok(Expr::new(
                ExprKind::Range {
                    start: Maybe::None,
                    end,
                    inclusive,
                },
                span,
            ));
        }

        // Parse a primary expression without struct literals
        self.parse_primary_expr_no_struct()
    }

    /// Parse a primary expression without allowing struct literals.
    /// This is identical to parse_primary_expr but calls parse_path_only() instead of parse_path_or_record().
    fn parse_primary_expr_no_struct(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();

        match self.stream.peek_kind() {
            // Literals
            Some(TokenKind::Integer(_)) => self.parse_integer_literal(),
            Some(TokenKind::Float(_)) => self.parse_float_literal(),
            Some(TokenKind::Text(_)) => self.parse_string_literal(),
            Some(TokenKind::Char(_)) => self.parse_char_literal(),
            Some(TokenKind::ByteChar(_)) => self.parse_byte_char_literal(),
            Some(TokenKind::ByteString(_)) => self.parse_byte_string_literal(),
            Some(TokenKind::True) | Some(TokenKind::False) => self.parse_bool_literal(),
            Some(TokenKind::InterpolatedString(_)) => self.parse_interpolated_string(),
            Some(TokenKind::ContractLiteral(_)) => self.parse_contract_literal(),
            Some(TokenKind::TaggedLiteral(_)) => self.parse_tagged_literal(),
            Some(TokenKind::HexColor(_)) => self.parse_hex_color(),

            // Path only (no record literal) or macro call
            Some(TokenKind::Ident(_))
            | Some(TokenKind::Some)
            | Some(TokenKind::None)
            | Some(TokenKind::Ok)
            | Some(TokenKind::Err)
            | Some(TokenKind::Result)
            | Some(TokenKind::SelfValue)
            | Some(TokenKind::SelfType)
            | Some(TokenKind::Super)
            | Some(TokenKind::Cog) => {
                // Check for macro call: identifier!
                if self.is_macro_call() {
                    self.parse_macro_call()
                } else {
                    // CRITICAL FIX: In expression context, `.` is for postfix operators
                    // (field access, method call). Only super/crate paths need multi-segment parsing.
                    // For identifiers, self, Some, None, etc., use single-segment parsing
                    // so that `.` is left for postfix processing.
                    let path = match self.stream.peek_kind() {
                        Some(TokenKind::Super) | Some(TokenKind::Cog) => {
                            // Module paths like super.module.item need multi-segment parsing
                            self.parse_path()?
                        }
                        _ => {
                            // Regular identifiers, self, Some, None, etc.
                            // Use single-segment parsing, leave . for postfix field/method access
                            self.parse_simple_expr_path()?
                        }
                    };
                    Ok(Expr::path(path))
                }
            }

            // All other cases same as parse_primary_expr
            Some(TokenKind::LParen) => self.parse_paren_or_tuple(),
            Some(TokenKind::LBracket) => self.parse_array_or_comprehension(),
            Some(TokenKind::LBrace) => self.parse_brace_expr(),
            Some(TokenKind::If) => self.parse_if_expr(),
            Some(TokenKind::Match) => self.parse_match_expr(),
            Some(TokenKind::Loop) => self.parse_loop_expr(Maybe::None),
            Some(TokenKind::While) => self.parse_while_expr(Maybe::None),
            Some(TokenKind::For) => self.parse_for_expr(Maybe::None),
            // Labeled loops: 'label: loop/while/for
            // Must check for colon after the lifetime - if missing, it's an unterminated char literal
            Some(TokenKind::Lifetime(_)) => {
                // Check if next token is colon (labeled loop pattern: 'label: loop)
                if self.stream.peek_nth_kind(1) == Some(&TokenKind::Colon) {
                    self.parse_labeled_loop()
                } else {
                    // No colon after 'x - this is likely an unterminated character literal
                    // Consume the token and report E001
                    let span = self.stream.current_span();
                    let token = self.stream.advance()
                        .ok_or_else(|| ParseError::unterminated_char(span))?;
                    Err(ParseError::unterminated_char(token.span))
                }
            }
            Some(TokenKind::Try) => self.parse_try_expr(),
            // async { ... } or async move { ... } blocks
            // NOTE: These must come BEFORE the async closure cases to properly disambiguate
            Some(TokenKind::Async) if self.stream.peek_nth_kind(1) == Some(&TokenKind::LBrace) => {
                self.parse_async_expr()
            }
            Some(TokenKind::Async)
                if self.stream.peek_nth_kind(1) == Some(&TokenKind::Move)
                    && self.stream.peek_nth_kind(2) == Some(&TokenKind::LBrace) =>
            {
                self.parse_async_expr()
            }
            // Closure (including empty closures: || expr)
            Some(TokenKind::Pipe) | Some(TokenKind::PipePipe) => self.parse_closure_expr(),
            // async |...| closure
            Some(TokenKind::Async)
                if matches!(
                    self.stream.peek_nth_kind(1),
                    Some(&TokenKind::Pipe) | Some(&TokenKind::PipePipe)
                ) =>
            {
                self.parse_closure_expr()
            }
            // async move |...| closure (not async move { block })
            Some(TokenKind::Async)
                if self.stream.peek_nth_kind(1) == Some(&TokenKind::Move)
                    && matches!(
                        self.stream.peek_nth_kind(2),
                        Some(&TokenKind::Pipe) | Some(&TokenKind::PipePipe)
                    ) =>
            {
                self.parse_closure_expr()
            }
            Some(TokenKind::Move)
                if matches!(
                    self.stream.peek_nth_kind(1),
                    Some(&TokenKind::Pipe) | Some(&TokenKind::PipePipe)
                ) =>
            {
                self.parse_closure_expr()
            }
            Some(TokenKind::Unsafe) => self.parse_unsafe_expr(),
            // Meta block (meta { ... }) vs meta as identifier (e.g., let meta = ...)
            Some(TokenKind::Meta) if self.stream.peek_nth_kind(1) == Some(&TokenKind::LBrace) => {
                self.parse_meta_expr()
            }
            // Meta keyword used as identifier when not followed by { (e.g., Ok(meta) => meta.is_file())
            // Use single-segment parsing to leave `.` for postfix processing
            Some(TokenKind::Meta) => {
                let path = self.parse_simple_expr_path()?;
                Ok(Expr::path(path))
            }
            // Quantifier expressions: `forall(x: T) expr` / `exists(x: T) expr`
            // Disambiguate: only parse as quantifier if followed by binding syntax
            // (identifier or parenthesized bindings). Otherwise treat as identifier.
            Some(TokenKind::Forall) => {
                let next = self.stream.peek_nth_kind(1);
                if matches!(next, Some(TokenKind::LParen) | Some(TokenKind::Ident(_)) | Some(TokenKind::Mut)
                    | Some(TokenKind::Some) | Some(TokenKind::None) | Some(TokenKind::Ok) | Some(TokenKind::Err)) {
                    self.parse_forall_expr()
                } else {
                    let path = self.parse_simple_expr_path()?;
                    Ok(Expr::path(path))
                }
            }
            Some(TokenKind::Exists) => {
                let next = self.stream.peek_nth_kind(1);
                if matches!(next, Some(TokenKind::LParen) | Some(TokenKind::Ident(_)) | Some(TokenKind::Mut)
                    | Some(TokenKind::Some) | Some(TokenKind::None) | Some(TokenKind::Ok) | Some(TokenKind::Err)) {
                    self.parse_exists_expr()
                } else {
                    let path = self.parse_simple_expr_path()?;
                    Ok(Expr::path(path))
                }
            }
            // Calculational proof block: calc { expr == { by ... } expr ... }
            // Disambiguate: `calc {` is calc block, otherwise treat as identifier
            Some(TokenKind::Calc) => {
                if self.stream.peek_nth_kind(1) == Some(&TokenKind::LBrace) {
                    let chain = self.parse_calc_chain()?;
                    let span = chain.span;
                    Ok(Expr::new(ExprKind::CalcBlock(chain), span))
                } else {
                    // Treat `calc` as an identifier
                    let path = self.parse_simple_expr_path()?;
                    Ok(Expr::path(path))
                }
            }
            // Stream comprehension or stream:: qualified path
            // Disambiguate: `stream [...]` is comprehension, `stream::...` is a path
            Some(TokenKind::Stream) => {
                if self.stream.peek_nth_kind(1) == Some(&TokenKind::LBracket) {
                    self.parse_stream_expr()
                } else {
                    // Treat `stream` as an identifier - use single-segment parsing
                    // to leave `.` for postfix processing (e.g., stream.map(...))
                    let path = self.parse_simple_expr_path()?;
                    Ok(Expr::path(path))
                }
            }
            Some(TokenKind::Tensor) => self.parse_tensor_literal(),
            // In no_struct mode, `set {` and `gen {` should NOT be parsed as comprehensions
            // because the `{` is likely a block (e.g., for loop body: `for x in set { ... }`)
            Some(TokenKind::Set) | Some(TokenKind::Gen) => {
                let path = self.parse_simple_expr_path()?;
                Ok(Expr::path(path))
            }
            // Select expression for async multiplexing
            // Disambiguate: `select { }` or `select biased { }` is expression, `select(...)` is function call
            Some(TokenKind::Select) if self.is_select_expr_lookahead() => {
                self.parse_select_expr()
            }
            // select followed by ( or . or other - treat as path/function call
            // Use single-segment parsing to leave `.` for postfix processing
            Some(TokenKind::Select) => {
                let path = self.parse_simple_expr_path()?;
                Ok(Expr::path(path))
            }
            // Nursery expression for structured concurrency
            // Grammar: nursery_expr = 'nursery' , [ nursery_options ] , block_expr , [ nursery_handlers ] ;
            // Disambiguate: `nursery { ... }` or `nursery( options ) { ... }` is nursery expression,
            // `nursery = ...` or `nursery.field` uses `nursery` as an identifier
            Some(TokenKind::Nursery) if self.is_nursery_expr_lookahead() => {
                self.parse_nursery_expr()
            }
            // nursery followed by =, ., or other - treat as identifier
            // Use single-segment parsing to leave `.` for postfix processing
            Some(TokenKind::Nursery) => {
                let path = self.parse_simple_expr_path()?;
                Ok(Expr::path(path))
            }
            // Anonymous function expression: fn(params) [using [...]] [-> Type] { body }
            Some(TokenKind::Fn)
                if matches!(
                    self.stream.peek_nth_kind(1),
                    Some(&TokenKind::LParen) | Some(&TokenKind::Lt)
                ) =>
            {
                self.parse_anonymous_fn_expr()
            }
            // Context system keywords used as identifiers in expressions
            // Use single-segment parsing to leave `.` for postfix processing
            Some(TokenKind::Context) | Some(TokenKind::Recover) => {
                let path = self.parse_simple_expr_path()?;
                Ok(Expr::path(path))
            }
            // Pattern matching keyword used as identifier in expressions
            // Use single-segment parsing to leave `.` for postfix processing
            Some(TokenKind::ActivePattern) => {
                let path = self.parse_simple_expr_path()?;
                Ok(Expr::path(path))
            }
            // Protocol, Internal, and Protected keywords used as identifiers in expressions
            // (e.g., socket(domain, sock_type, protocol), internal/protected field access)
            // CRITICAL: Use parse_simple_expr_path() to only parse single segment, leaving
            // `.` for postfix processing (field access, method calls).
            // Using parse_path() would consume `protected.binary_search` as a 2-segment path
            // instead of correctly parsing `.binary_search(...)` as a method call.
            Some(TokenKind::Protocol) | Some(TokenKind::Internal) | Some(TokenKind::Protected) => {
                let path = self.parse_simple_expr_path()?;
                Ok(Expr::path(path))
            }
            // Contextual keywords used as identifiers in expressions
            // Note: Only add keywords NOT already handled elsewhere in this match
            Some(TokenKind::Stage) | Some(TokenKind::By) | Some(TokenKind::Field)
            | Some(TokenKind::Layer) | Some(TokenKind::Extends) | Some(TokenKind::Throws)
            | Some(TokenKind::Requires)
            | Some(TokenKind::Linear) | Some(TokenKind::Ffi) | Some(TokenKind::Mount)
            | Some(TokenKind::Inductive) | Some(TokenKind::Cofix) | Some(TokenKind::Implies)
            | Some(TokenKind::Volatile) | Some(TokenKind::View) => {
                let path = self.parse_simple_expr_path()?;
                Ok(Expr::path(path))
            }
            Some(TokenKind::Break) => self.parse_break_expr(),
            Some(TokenKind::Continue) => self.parse_continue_expr(),
            Some(TokenKind::Return) => self.parse_return_expr(),
            Some(TokenKind::Throw) => self.parse_throw_expr(),
            Some(TokenKind::Yield) => self.parse_yield_expr(),

            // Meta-function expressions: @file, @line, @cfg(cond), @const(expr), etc.
            Some(TokenKind::At) => self.parse_meta_function_expr(),

            // typeof expression: `typeof(expr)` returns runtime type information
            Some(TokenKind::Typeof) => self.parse_typeof_expr(),

            // Splice operator '$' — allowed in meta rule bodies and quote blocks
            Some(TokenKind::Dollar) => {
                if self.in_meta_body {
                    // Parse $ident as a meta parameter reference.
                    // Represented as a Path with "$name" to distinguish from regular vars.
                    let start = self.stream.position();
                    self.stream.advance(); // consume $
                    let name = self.consume_ident()?;
                    let prefixed = Text::from(format!("${}", name));
                    let span = self.stream.make_span(start);
                    Ok(Expr::path(verum_ast::ty::Path::single(
                        verum_ast::Ident::new(prefixed, span),
                    )))
                } else {
                    let span = self.stream.current_span();
                    Err(ParseError::meta_splice_outside_quote(span))
                }
            }

            // Proof-related keywords that can be used as proof term expressions
            // or as identifiers in non-proof contexts (e.g., `let cases = ...`)
            Some(TokenKind::Assumption)
            | Some(TokenKind::Trivial)
            | Some(TokenKind::Qed)
            | Some(TokenKind::Cases)
            | Some(TokenKind::Invariant)
            | Some(TokenKind::Ensures)
            | Some(TokenKind::Auto)
            | Some(TokenKind::Proof)
            | Some(TokenKind::Omega)
            | Some(TokenKind::Ring)
            | Some(TokenKind::Simp)
            | Some(TokenKind::Blast)
            | Some(TokenKind::Smt)
            | Some(TokenKind::Have)
            | Some(TokenKind::Show)
            | Some(TokenKind::Suffices)
            | Some(TokenKind::Obtain)
            | Some(TokenKind::Induction)
            | Some(TokenKind::Contradiction)
            | Some(TokenKind::Link)
            | Some(TokenKind::With) => {
                let path = self.parse_simple_expr_path()?;
                Ok(Expr::path(path))
            }

            Some(kind) => {
                let span = self.stream.current_span();
                Err(ParseError::invalid_syntax(
                    format!("unexpected token in expression: {}", kind.description()),
                    span,
                ))
            }
            None => {
                let span = self.stream.current_span();
                Err(ParseError::unexpected_eof(&[], span))
            }
        }
    }

    /// Parse an expression with minimum binding power (Pratt parsing core).
    ///
    /// This is the heart of the Pratt parser. It handles:
    /// - Prefix operators (unary)
    /// - Infix operators (binary) with proper precedence
    /// - Postfix operators
    /// - Right-associativity through binding power adjustments
    ///
    /// Uses an iterative approach with an explicit operator stack to handle
    /// deeply nested expressions without stack overflow.
    pub fn parse_expr_bp(&mut self, min_bp: u8) -> ParseResult<Expr> {
        self.enter_recursion()?;
        let result = self.parse_expr_bp_inner(min_bp);
        self.exit_recursion();
        result
    }

    fn parse_expr_bp_inner(&mut self, min_bp: u8) -> ParseResult<Expr> {
        let start_pos = self.stream.position();

        // Parse prefix/primary expression
        let mut lhs = self.parse_prefix_expr()?;

        // Parse infix and postfix operators
        loop {
            // Safety: prevent infinite loop and check abort flag
            if !self.tick() || self.is_aborted() {
                break;
            }

            // Handle postfix operators first (highest precedence)
            // PERF: try_parse_postfix takes ownership and returns (expr, found)
            let (new_lhs, found_postfix) = self.try_parse_postfix(lhs)?;
            lhs = new_lhs;
            if found_postfix {
                continue;
            }

            // Check if the current expression is a block-form expression that doesn't
            // return a value (no trailing expression). Such expressions should not
            // continue with binary operators. This prevents:
            //   unsafe { stmt; }
            //   *ptr = value;
            // from being parsed as: (unsafe_block) * (ptr = value)
            if Self::is_statement_block(&lhs) {
                break;
            }

            // Check for infix operators
            let op_kind = match self.stream.peek_kind() {
                Some(kind) => kind.clone(),
                None => break,
            };

            // Get binding power for this operator
            let (left_bp, right_bp) = match self.infix_binding_power(&op_kind) {
                Some(bp) => bp,
                None => break, // Not an infix operator, done
            };

            // If this operator has lower precedence than what we need, stop
            if left_bp < min_bp {
                break;
            }

            // Consume the operator
            self.stream.advance();

            // Handle special binary operators that aren't in BinOp
            if op_kind == TokenKind::PipeGt {
                // Pipeline: lhs |> rhs  OR  lhs |> .method(args)
                // When followed by '.', desugar to MethodCall with lhs as receiver
                if self.stream.check(&TokenKind::Dot) {
                    lhs = self.parse_pipe_method_call(lhs, start_pos)?;
                } else {
                    let rhs = self.parse_expr_bp(right_bp)?;
                    let span = lhs.span.merge(rhs.span);
                    lhs = Expr::new(
                        ExprKind::Pipeline {
                            left: Box::new(lhs),
                            right: Box::new(rhs),
                        },
                        span,
                    );
                }
                continue;
            }

            if op_kind == TokenKind::QuestionQuestion {
                // Null coalescing: lhs ?? rhs
                let rhs = self.parse_expr_bp(right_bp)?;
                let span = lhs.span.merge(rhs.span);
                lhs = Expr::new(
                    ExprKind::NullCoalesce {
                        left: Box::new(lhs),
                        right: Box::new(rhs),
                    },
                    span,
                );
                continue;
            }

            if op_kind == TokenKind::DotDot || op_kind == TokenKind::DotDotEq {
                // Range: lhs .. rhs or lhs ..= rhs
                let inclusive = op_kind == TokenKind::DotDotEq;
                // Range end is optional.
                // Inside comprehensions, `if`, `for`, `let` are clause keywords,
                // not the start of a range-end expression.
                let end = if self.stream.at_end()
                    || !self.stream.peek().is_some_and(|t| t.starts_expr())
                    || (self.comprehension_depth > 0
                        && self.stream.peek().is_some_and(|t| {
                            matches!(
                                t.kind,
                                TokenKind::If | TokenKind::For | TokenKind::Let
                            )
                        }))
                {
                    Maybe::None
                } else {
                    Maybe::Some(Box::new(self.parse_expr_bp(right_bp)?))
                };
                let span = if let Maybe::Some(ref end_expr) = end {
                    lhs.span.merge(end_expr.span)
                } else {
                    lhs.span
                };
                lhs = Expr::new(
                    ExprKind::Range {
                        start: Maybe::Some(Box::new(lhs)),
                        end,
                        inclusive,
                    },
                    span,
                );
                continue;
            }

            if op_kind == TokenKind::As {
                // Cast: lhs as Type
                // Use parse_type_with_lookahead to support refined casts like `x as Int{> 0}`
                // The lookahead logic distinguishes refinements ({> 0}) from blocks ({ break; })
                let ty = self.parse_type_with_lookahead()?;
                let span = lhs.span.merge(ty.span);
                lhs = Expr::new(
                    ExprKind::Cast {
                        expr: Box::new(lhs),
                        ty,
                    },
                    span,
                );
                continue;
            }

            if op_kind == TokenKind::Is {
                // Pattern test: lhs is Pattern or lhs is not Pattern
                // Check for optional 'not' keyword (parsed as identifier)
                let negated = if let Some(TokenKind::Ident(name)) = self.stream.peek_kind() {
                    if name.as_str() == "not" {
                        self.stream.advance();
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };
                // Use parse_pattern_no_struct to prevent `{ }` from being consumed as a record pattern.
                // This prevents ambiguity in contexts like `if value is None { }`.
                // For record patterns in is expressions, users can wrap in parentheses: `value is (Point { x, y })`.
                let pattern = self.parse_pattern_no_struct()?;
                let span = lhs.span.merge(pattern.span);
                lhs = Expr::new(
                    ExprKind::Is {
                        expr: Box::new(lhs),
                        pattern,
                        negated,
                    },
                    span,
                );
                continue;
            }

            // Standard binary operator
            let bin_op = self.token_to_binop(&op_kind)?;

            // E046: Validate assignment LHS
            if bin_op.is_assignment() {
                // E047: Check for invalid compound assignment (e.g., x +== 5)
                if self.stream.check(&TokenKind::Eq) {
                    return Err(ParseError::compound_assign_invalid(
                        "invalid compound assignment operator",
                        self.stream.current_span(),
                    ));
                }

                // E041: Check for missing RHS (x =;)
                if self.stream.check(&TokenKind::Semicolon) || self.stream.check(&TokenKind::RBrace) {
                    return Err(ParseError::let_missing_value(self.stream.current_span())
                        .with_help("assignment requires a value on the right-hand side"));
                }

                // Check if this is a destructuring assignment (tuple, array, record on LHS)
                if Self::is_destructuring_target(&lhs) {
                    // Convert expression to assignment pattern
                    let pattern = Self::expr_to_assignment_pattern(&lhs)?;

                    // Parse RHS value
                    let rhs = self.parse_expr_bp(right_bp)?;

                    let span = lhs.span.merge(rhs.span);
                    lhs = Expr::new(
                        ExprKind::DestructuringAssign {
                            pattern,
                            op: bin_op,
                            value: Heap::new(rhs),
                        },
                        span,
                    );
                    continue;
                }

                // Not a destructuring target - check if it's a valid simple assignment target
                if !Self::is_valid_assignment_target(&lhs) {
                    return Err(ParseError::assignment_invalid(
                        "invalid left-hand side of assignment",
                        lhs.span,
                    ));
                }
            }

            let rhs = self.parse_expr_bp(right_bp)?;

            // Note: Verum DOES allow chained assignment (a = b = c = 42)
            // Assignment is right-associative, so a = b = c = 42 parses as a = (b = (c = 42))

            let span = lhs.span.merge(rhs.span);
            lhs = Expr::new(
                ExprKind::Binary {
                    op: bin_op,
                    left: Box::new(lhs),
                    right: Box::new(rhs),
                },
                span,
            );
        }

        Ok(lhs)
    }

    /// Check if an expression is a block-form that doesn't return a value.
    /// Such expressions should not continue with binary operators.
    ///
    /// This handles cases like:
    ///   unsafe { stmt; }
    ///   *ptr = value;
    /// where the `*` should be parsed as unary dereference of a new statement,
    /// not binary multiplication.
    fn is_statement_block(expr: &Expr) -> bool {
        use verum_ast::expr::ExprKind;

        match &expr.kind {
            // Unsafe block without trailing expression
            ExprKind::Unsafe(block) => block.expr.is_none(),
            // For/While/Loop blocks - these typically don't return values
            ExprKind::For { body, .. } => body.expr.is_none(),
            ExprKind::While { body, .. } => body.expr.is_none(),
            ExprKind::Loop { body, .. } => body.expr.is_none(),
            // If without else clause doesn't return a value
            ExprKind::If { then_branch, else_branch, .. } => {
                else_branch.is_none() || then_branch.expr.is_none()
            }
            // Match with all arms not returning values
            ExprKind::Match { arms, .. } => {
                arms.iter().all(|arm| {
                    if let ExprKind::Block(block) = &arm.body.kind {
                        block.expr.is_none()
                    } else {
                        false // Non-block arm body, assume it returns a value
                    }
                })
            }
            _ => false,
        }
    }

    /// Get the binding power for an infix operator.
    /// Returns (left_binding_power, right_binding_power).
    /// For left-associative: (n, n+1)
    /// For right-associative: (n, n)
    fn infix_binding_power(&self, op: &TokenKind) -> Option<(u8, u8)> {
        let bp = match op {
            // Level 1: Pipeline (left-assoc)
            TokenKind::PipeGt => (1, 2),

            // Level 2: Null coalescing (right-assoc)
            TokenKind::QuestionQuestion => (2, 2),

            // Level 3: Assignment and logical implication (right-assoc)
            // Note: `implies` keyword and `=>` (FatArrow) for formal proofs have same precedence as assignment
            // but bind looser than || and &&. Right-associative: P implies Q implies R = P implies (Q implies R)
            // FatArrow is also used as implication in verification contexts (forall, exists bodies)
            TokenKind::Eq
            | TokenKind::PlusEq
            | TokenKind::MinusEq
            | TokenKind::StarEq
            | TokenKind::SlashEq
            | TokenKind::PercentEq
            | TokenKind::AmpersandEq
            | TokenKind::PipeEq
            | TokenKind::CaretEq
            | TokenKind::LtLtEq
            | TokenKind::GtGtEq
            | TokenKind::Implies
            | TokenKind::FatArrow
            | TokenKind::RArrow
            | TokenKind::Iff => (3, 3),

            // Level 4: Logical OR (left-assoc)
            TokenKind::PipePipe => (4, 5),

            // Level 5: Logical AND (left-assoc)
            // In if-let chains, `&&` followed by `let` is a chain separator — not an operator.
            // Return None so the expression parser stops before `&&`, letting
            // parse_if_condition handle the chaining.
            TokenKind::AmpersandAmpersand => {
                if self.stream.peek_nth_kind(1) == Some(&TokenKind::Let) {
                    return None;
                }
                (5, 6)
            }

            // Level 6: Equality/Comparison/Pattern test/Containment (left-assoc)
            TokenKind::EqEq
            | TokenKind::BangEq
            | TokenKind::Lt
            | TokenKind::Gt
            | TokenKind::LtEq
            | TokenKind::GtEq
            | TokenKind::Is
            | TokenKind::In => (6, 7),

            // Level 7: Bitwise OR (left-assoc)
            TokenKind::Pipe => (7, 8),

            // Level 8: Bitwise XOR (left-assoc)
            TokenKind::Caret => (8, 9),

            // Level 9: Bitwise AND (left-assoc)
            // CRITICAL: `&` followed by `mut`, `checked`, or `unsafe` is NOT a binary operator
            // but rather the start of a new reference expression. Check lookahead to disambiguate.
            TokenKind::Ampersand => {
                // Check if this is actually the start of a reference expression
                // by looking at the token after `&`
                match self.stream.peek_nth_kind(1) {
                    Some(TokenKind::Mut) | Some(TokenKind::Checked) | Some(TokenKind::Unsafe) => {
                        // This is `& mut/checked/unsafe ...` which is a reference type/expr start
                        // Not a binary AND operator
                        return None;
                    }
                    _ => (9, 10),
                }
            }

            // Level 10: Shift (left-assoc)
            TokenKind::LtLt | TokenKind::GtGt => (10, 11),

            // Level 11: Addition/Subtraction/Concatenation (left-assoc)
            TokenKind::Plus | TokenKind::Minus | TokenKind::PlusPlus => (11, 12),

            // Level 12: Multiplication/Division/Remainder (left-assoc)
            TokenKind::Star | TokenKind::Slash | TokenKind::Percent => (12, 13),

            // Level 13: Exponentiation (right-assoc)
            TokenKind::StarStar => (13, 13),

            // Level 14: Range (left-assoc, but special handling)
            TokenKind::DotDot | TokenKind::DotDotEq => (14, 15),

            // Level 16: Cast (special case, parsed separately)
            TokenKind::As => (16, 17),

            _ => return None,
        };
        Some(bp)
    }

    /// Parse a prefix expression (unary operators or primary expression).
    ///
    /// Uses an iterative approach with an explicit stack to handle chains of
    /// unary operators (e.g., `---x`, `***ptr`) without deep recursion.
    pub(crate) fn parse_prefix_expr(&mut self) -> ParseResult<Expr> {
        // Stack to collect unary operators with their start positions.
        // We parse all prefix operators first, then the primary expression,
        // then build the tree bottom-up from the stack.
        let mut unary_stack: Vec<(UnOp, usize)> = Vec::new();

        // Collect all prefix unary operators iteratively
        loop {
            let op_start_pos = self.stream.position();

            let op = match self.stream.peek_kind() {
                Some(TokenKind::Bang) => {
                    self.stream.advance();
                    Some(UnOp::Not)
                }
                Some(TokenKind::Minus) => {
                    self.stream.advance();
                    Some(UnOp::Neg)
                }
                Some(TokenKind::Tilde) => {
                    self.stream.advance();
                    Some(UnOp::BitNot)
                }
                Some(TokenKind::Star) => {
                    self.stream.advance();
                    Some(UnOp::Deref)
                }
                // Handle ** (StarStar) token by splitting it into two * tokens for double dereference: **x
                Some(TokenKind::StarStar) => {
                    if self.pending_star {
                        // Consume the pending * from a previous ** split
                        self.pending_star = false;
                        // Advance past the ** token
                        if self.stream.peek_kind() == Some(&TokenKind::StarStar) {
                            self.stream.advance();
                        }
                        Some(UnOp::Deref)
                    } else {
                        // Split ** into two * tokens
                        // First * is "consumed" now, second * will be consumed when pending_star is processed
                        self.pending_star = true;
                        // Don't advance! Keep the ** token in the stream for now
                        Some(UnOp::Deref)
                    }
                }
                Some(TokenKind::Ampersand) => {
                    self.stream.advance();
                    // Check for three-tier references: &checked, &unsafe, &mut
                    if self.stream.consume(&TokenKind::Checked).is_some() {
                        if self.stream.consume(&TokenKind::Mut).is_some() {
                            Some(UnOp::RefCheckedMut)
                        } else {
                            Some(UnOp::RefChecked)
                        }
                    } else if self.stream.consume(&TokenKind::Unsafe).is_some() {
                        if self.stream.consume(&TokenKind::Mut).is_some() {
                            Some(UnOp::RefUnsafeMut)
                        } else {
                            Some(UnOp::RefUnsafe)
                        }
                    } else if self.stream.consume(&TokenKind::Mut).is_some() {
                        Some(UnOp::RefMut)
                    } else {
                        Some(UnOp::Ref)
                    }
                }
                // Handle && (AmpersandAmpersand) token by splitting it into two & tokens for double reference: &&x
                Some(TokenKind::AmpersandAmpersand) => {
                    if self.pending_ampersand {
                        // Consume the pending & from a previous && split
                        self.pending_ampersand = false;
                        // Advance past the && token
                        if self.stream.peek_kind() == Some(&TokenKind::AmpersandAmpersand) {
                            self.stream.advance();
                        }
                        // Check for &mut
                        if self.stream.consume(&TokenKind::Mut).is_some() {
                            Some(UnOp::RefMut)
                        } else {
                            Some(UnOp::Ref)
                        }
                    } else {
                        // Split && into two & tokens
                        // First & is "consumed" now, second & will be consumed when pending_ampersand is processed
                        self.pending_ampersand = true;
                        // Don't advance! Keep the && token in the stream for now
                        // Note: We can't check for &mut here because the mut would come after the second &
                        Some(UnOp::Ref)
                    }
                }
                Some(TokenKind::Percent) => {
                    self.stream.advance();
                    // Check for %mut
                    if self.stream.consume(&TokenKind::Mut).is_some() {
                        Some(UnOp::OwnMut)
                    } else {
                        Some(UnOp::Own)
                    }
                }
                _ => None,
            };

            match op {
                Some(unary_op) => {
                    unary_stack.push((unary_op, op_start_pos));
                }
                None => break,
            }
        }

        // Now parse the primary expression or range
        let range_start_pos = self.stream.position();

        // Check for range with no start: ..end or ..=end
        let mut expr = if self.stream.check(&TokenKind::DotDot) || self.stream.check(&TokenKind::DotDotEq) {
            let inclusive = self.stream.consume(&TokenKind::DotDotEq).is_some();
            if !inclusive {
                self.stream.consume(&TokenKind::DotDot);
            }

            // Parse optional end - use parse_prefix_expr recursively for the range end.
            // This is safe because we've already consumed all the prefix operators
            // before the range operator, so this is a fresh prefix expression.
            // Inside comprehensions, `if`/`for`/`let` are clause keywords.
            let end =
                if self.stream.at_end()
                    || !self.stream.peek().is_some_and(|t| t.starts_expr())
                    || (self.comprehension_depth > 0
                        && self.stream.peek().is_some_and(|t| {
                            matches!(
                                t.kind,
                                TokenKind::If | TokenKind::For | TokenKind::Let
                            )
                        }))
                {
                    Maybe::None
                } else {
                    Maybe::Some(Box::new(self.parse_prefix_expr()?))
                };

            let span = self.stream.make_span(range_start_pos);
            Expr::new(
                ExprKind::Range {
                    start: Maybe::None,
                    end,
                    inclusive,
                },
                span,
            )
        } else {
            // Otherwise, parse a primary expression
            self.parse_primary_expr()?
        };

        // CRITICAL FIX: Parse postfix operators (field access, method calls, index)
        // BEFORE applying prefix operators. This ensures correct precedence:
        //   &r.x parses as &(r.x), not (&r).x
        //   *p.field parses as *(p.field), not (*p).field
        // Postfix operators have higher precedence than prefix operators.
        loop {
            let (new_expr, found_postfix) = self.try_parse_postfix_impl(expr, true)?;
            expr = new_expr;
            if !found_postfix {
                break;
            }
        }

        // Build unary expression tree bottom-up from the stack
        while let Some((unary_op, op_start_pos)) = unary_stack.pop() {
            let span = self.stream.make_span(op_start_pos);
            expr = Expr::new(
                ExprKind::Unary {
                    op: unary_op,
                    expr: Box::new(expr),
                },
                span,
            );
        }

        Ok(expr)
    }

    /// Parse a primary expression (atoms like literals, identifiers, blocks, etc.).
    fn parse_primary_expr(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();

        match self.stream.peek_kind() {
            // Literals
            Some(TokenKind::Integer(_)) => self.parse_integer_literal(),
            Some(TokenKind::Float(_)) => self.parse_float_literal(),
            Some(TokenKind::Text(_)) => self.parse_string_literal(),
            Some(TokenKind::Char(_)) => self.parse_char_literal(),
            Some(TokenKind::ByteChar(_)) => self.parse_byte_char_literal(),
            Some(TokenKind::ByteString(_)) => self.parse_byte_string_literal(),
            Some(TokenKind::True) | Some(TokenKind::False) => self.parse_bool_literal(),
            Some(TokenKind::InterpolatedString(_)) => self.parse_interpolated_string(),
            Some(TokenKind::ContractLiteral(_)) => self.parse_contract_literal(),
            Some(TokenKind::TaggedLiteral(_)) => self.parse_tagged_literal(),
            Some(TokenKind::HexColor(_)) => self.parse_hex_color(),

            // Path or record literal or macro call
            Some(TokenKind::Ident(_))
            | Some(TokenKind::Some)
            | Some(TokenKind::None)
            | Some(TokenKind::Ok)
            | Some(TokenKind::Err)
            | Some(TokenKind::Result)
            | Some(TokenKind::SelfValue)
            | Some(TokenKind::SelfType)
            | Some(TokenKind::Super)
            | Some(TokenKind::Cog) => {
                // Check for macro call: identifier!
                if self.is_macro_call() {
                    self.parse_macro_call()
                } else {
                    self.parse_path_or_record()
                }
            }

            // Parenthesized expression or tuple
            Some(TokenKind::LParen) => self.parse_paren_or_tuple(),

            // Array or comprehension
            Some(TokenKind::LBracket) => self.parse_array_or_comprehension(),

            // Block, Map, or Set
            Some(TokenKind::LBrace) => self.parse_brace_expr(),

            // Control flow
            Some(TokenKind::If) => self.parse_if_expr(),
            Some(TokenKind::Match) => self.parse_match_expr(),
            Some(TokenKind::Loop) => self.parse_loop_expr(Maybe::None),
            Some(TokenKind::While) => self.parse_while_expr(Maybe::None),
            Some(TokenKind::For) => self.parse_for_expr(Maybe::None),
            // Labeled loops: 'label: loop/while/for
            // Must check for colon after the lifetime - if missing, it's an unterminated char literal
            Some(TokenKind::Lifetime(_)) => {
                // Check if next token is colon (labeled loop pattern: 'label: loop)
                if self.stream.peek_nth_kind(1) == Some(&TokenKind::Colon) {
                    self.parse_labeled_loop()
                } else {
                    // No colon after 'x - this is likely an unterminated character literal
                    // Consume the token and report E001
                    let span = self.stream.current_span();
                    let token = self.stream.advance()
                        .ok_or_else(|| ParseError::unterminated_char(span))?;
                    Err(ParseError::unterminated_char(token.span))
                }
            }

            // Try expressions
            Some(TokenKind::Try) => self.parse_try_expr(),

            // Async block: async { ... } or async move { ... }
            // NOTE: These must come BEFORE the async closure cases to properly disambiguate
            Some(TokenKind::Async) if self.stream.peek_nth_kind(1) == Some(&TokenKind::LBrace) => {
                self.parse_async_expr()
            }
            Some(TokenKind::Async)
                if self.stream.peek_nth_kind(1) == Some(&TokenKind::Move)
                    && self.stream.peek_nth_kind(2) == Some(&TokenKind::LBrace) =>
            {
                self.parse_async_expr()
            }

            // Closure (including empty closures: || expr)
            Some(TokenKind::Pipe) | Some(TokenKind::PipePipe) => self.parse_closure_expr(),
            // async |...| closure or async move |...| closure
            Some(TokenKind::Async)
                if matches!(
                    self.stream.peek_nth_kind(1),
                    Some(&TokenKind::Pipe) | Some(&TokenKind::PipePipe)
                ) =>
            {
                self.parse_closure_expr()
            }
            // async move |...| closure (not async move { block })
            Some(TokenKind::Async)
                if self.stream.peek_nth_kind(1) == Some(&TokenKind::Move)
                    && matches!(
                        self.stream.peek_nth_kind(2),
                        Some(&TokenKind::Pipe) | Some(&TokenKind::PipePipe)
                    ) =>
            {
                self.parse_closure_expr()
            }
            Some(TokenKind::Move)
                if matches!(
                    self.stream.peek_nth_kind(1),
                    Some(&TokenKind::Pipe) | Some(&TokenKind::PipePipe)
                ) =>
            {
                self.parse_closure_expr()
            }

            // Unsafe block
            Some(TokenKind::Unsafe) => self.parse_unsafe_expr(),

            // Meta block (meta { ... }) vs meta as identifier
            Some(TokenKind::Meta) if self.stream.peek_nth_kind(1) == Some(&TokenKind::LBrace) => {
                self.parse_meta_expr()
            }
            // Meta keyword used as identifier when not followed by { (e.g., Ok(meta) => meta.is_file())
            Some(TokenKind::Meta) => self.parse_path_or_record(),

            // Quote expression: `quote { token_tree }` or `quote(N) { token_tree }`
            // Captures code as a token tree for staged metaprogramming
            Some(TokenKind::QuoteKeyword) => self.parse_quote_expr(),

            // Stage escape expression: `$(stage N){ expr }`
            // Used inside quote blocks to evaluate expressions at a specific compilation stage
            Some(TokenKind::Dollar)
                if self.stream.peek_nth_kind(1) == Some(&TokenKind::LParen)
                    && self.stream.peek_nth_kind(2) == Some(&TokenKind::Stage) =>
            {
                self.parse_stage_escape_expr()
            }

            // Lift expression: `lift(expr)`
            // Syntactic sugar for `$(stage current){ expr }` — moves compile-time value into generated code
            Some(TokenKind::Lift) => self.parse_lift_expr(),

            // Quantifier expressions: `forall(x: T) expr` / `exists(x: T) expr`
            // Disambiguate: only parse as quantifier if followed by binding syntax
            Some(TokenKind::Forall) => {
                let next = self.stream.peek_nth_kind(1);
                if matches!(next, Some(TokenKind::LParen) | Some(TokenKind::Ident(_)) | Some(TokenKind::Mut)
                    | Some(TokenKind::Some) | Some(TokenKind::None) | Some(TokenKind::Ok) | Some(TokenKind::Err)) {
                    self.parse_forall_expr()
                } else {
                    let path = self.parse_simple_expr_path()?;
                    Ok(Expr::path(path))
                }
            }
            Some(TokenKind::Exists) => {
                let next = self.stream.peek_nth_kind(1);
                if matches!(next, Some(TokenKind::LParen) | Some(TokenKind::Ident(_)) | Some(TokenKind::Mut)
                    | Some(TokenKind::Some) | Some(TokenKind::None) | Some(TokenKind::Ok) | Some(TokenKind::Err)) {
                    self.parse_exists_expr()
                } else {
                    let path = self.parse_simple_expr_path()?;
                    Ok(Expr::path(path))
                }
            }
            // Calculational proof block: calc { expr == { by ... } expr ... }
            // Calculational proof block: calc { expr == { by ... } expr ... }
            // Disambiguate: `calc {` is calc block, otherwise treat as identifier
            Some(TokenKind::Calc) => {
                if self.stream.peek_nth_kind(1) == Some(&TokenKind::LBrace) {
                    let chain = self.parse_calc_chain()?;
                    let span = chain.span;
                    Ok(Expr::new(ExprKind::CalcBlock(chain), span))
                } else {
                    // Treat `calc` as an identifier
                    self.parse_path_or_record()
                }
            }

            // Stream comprehension or stream:: qualified path
            // Disambiguate: `stream [...]` is comprehension, `stream::...` is a path
            Some(TokenKind::Stream) => {
                if self.stream.peek_nth_kind(1) == Some(&TokenKind::LBracket) {
                    self.parse_stream_expr()
                } else {
                    // Treat `stream` as an identifier for qualified paths like `stream::map`
                    self.parse_path_or_record()
                }
            }

            // Tensor literal
            Some(TokenKind::Tensor) => self.parse_tensor_literal(),

            // Set comprehension: set{expr for pattern in iter ...}
            // Disambiguate: `set {` is comprehension, `set::...` or `set.` is a path
            Some(TokenKind::Set) => {
                if self.stream.peek_nth_kind(1) == Some(&TokenKind::LBrace) {
                    self.parse_set_comprehension()
                } else {
                    // Treat `set` as an identifier for qualified paths like `set::from`
                    self.parse_path_or_record()
                }
            }

            // Generator expression: gen{expr for pattern in iter ...}
            // Disambiguate: `gen {` is comprehension, `gen::...` or `gen.` is a path
            Some(TokenKind::Gen) => {
                if self.stream.peek_nth_kind(1) == Some(&TokenKind::LBrace) {
                    self.parse_generator_expression()
                } else {
                    // Treat `gen` as an identifier for qualified paths
                    self.parse_path_or_record()
                }
            }

            // Inject expression: inject TypeName
            Some(TokenKind::Inject) => {
                let start_pos = self.stream.position();
                self.stream.advance(); // consume 'inject'
                let type_path = self.parse_path()?;
                let span = self.stream.make_span(start_pos);
                Ok(Expr::new(ExprKind::Inject { type_path }, span))
            },

            // Spawn expression
            Some(TokenKind::Spawn) => self.parse_spawn_expr(),

            // Select expression for async multiplexing
            // Disambiguate: `select { }` or `select biased { }` is expression, `select(...)` is function call
            Some(TokenKind::Select) if self.is_select_expr_lookahead() => {
                self.parse_select_expr()
            }
            // select followed by ( or . or other - treat as path/function call
            // Use single-segment parsing to leave `.` for postfix processing
            Some(TokenKind::Select) => {
                let path = self.parse_simple_expr_path()?;
                Ok(Expr::path(path))
            }

            // Nursery expression for structured concurrency: ensures all spawned tasks
            // complete before the nursery scope exits (no orphan tasks)
            // Grammar: nursery_expr = 'nursery' , [ nursery_options ] , block_expr , [ nursery_handlers ] ;
            // Disambiguate: `nursery { ... }` or `nursery( options ) { ... }` is nursery expression
            Some(TokenKind::Nursery) if self.is_nursery_expr_lookahead() => {
                self.parse_nursery_expr()
            }
            // nursery followed by =, ., or other - treat as identifier
            Some(TokenKind::Nursery) => {
                self.parse_path_or_record()
            }

            // Anonymous function expression: fn(params) [using [...]] [-> Type] { body }
            Some(TokenKind::Fn)
                if matches!(
                    self.stream.peek_nth_kind(1),
                    Some(&TokenKind::LParen) | Some(&TokenKind::Lt)
                ) =>
            {
                self.parse_anonymous_fn_expr()
            }

            // Context system keywords used as identifiers in expressions
            Some(TokenKind::Context) | Some(TokenKind::Recover) => {
                self.parse_path_or_record()
            }

            // Pattern matching keyword used as identifier in expressions
            Some(TokenKind::ActivePattern) => {
                self.parse_path_or_record()
            }

            // Protocol, Internal, and Protected keywords used as identifiers in expressions
            // (e.g., socket(domain, sock_type, protocol), internal/protected field access)
            Some(TokenKind::Protocol) | Some(TokenKind::Internal) | Some(TokenKind::Protected) => {
                self.parse_path_or_record()
            }

            // Contextual keywords used as identifiers in expressions
            // Note: Only add keywords NOT already handled elsewhere in this match
            Some(TokenKind::Stage) | Some(TokenKind::By) | Some(TokenKind::Field)
            | Some(TokenKind::Layer) | Some(TokenKind::Extends) | Some(TokenKind::Throws)
            | Some(TokenKind::Requires)
            | Some(TokenKind::Linear) | Some(TokenKind::Ffi) | Some(TokenKind::Mount)
            | Some(TokenKind::Inductive) | Some(TokenKind::Cofix) | Some(TokenKind::Implies)
            | Some(TokenKind::Volatile) | Some(TokenKind::View) => {
                self.parse_path_or_record()
            }

            // Keywords that start expressions
            Some(TokenKind::Break) => self.parse_break_expr(),
            Some(TokenKind::Continue) => self.parse_continue_expr(),
            Some(TokenKind::Return) => self.parse_return_expr(),
            Some(TokenKind::Throw) => self.parse_throw_expr(),
            Some(TokenKind::Yield) => self.parse_yield_expr(),

            // Meta-function expressions: @file, @line, @cfg(cond), @const(expr), etc.
            Some(TokenKind::At) => self.parse_meta_function_expr(),

            // typeof expression: `typeof(expr)` returns runtime type information
            Some(TokenKind::Typeof) => self.parse_typeof_expr(),

            // Splice operator '$' — allowed in meta rule bodies and quote blocks
            Some(TokenKind::Dollar) => {
                if self.in_meta_body {
                    let start = self.stream.position();
                    self.stream.advance();
                    let name = self.consume_ident()?;
                    let prefixed = Text::from(format!("${}", name));
                    let span = self.stream.make_span(start);
                    Ok(Expr::path(verum_ast::ty::Path::single(
                        verum_ast::Ident::new(prefixed, span),
                    )))
                } else {
                    let span = self.stream.current_span();
                    Err(ParseError::meta_splice_outside_quote(span))
                }
            }

            // Proof-related keywords that can be used as proof term expressions
            // or as identifiers in non-proof contexts (e.g., `let cases = ...`)
            Some(TokenKind::Assumption)
            | Some(TokenKind::Trivial)
            | Some(TokenKind::Qed)
            | Some(TokenKind::Cases)
            | Some(TokenKind::Invariant)
            | Some(TokenKind::Ensures)
            | Some(TokenKind::Auto)
            | Some(TokenKind::Proof)
            | Some(TokenKind::Omega)
            | Some(TokenKind::Ring)
            | Some(TokenKind::Simp)
            | Some(TokenKind::Blast)
            | Some(TokenKind::Smt)
            | Some(TokenKind::Have)
            | Some(TokenKind::Show)
            | Some(TokenKind::Suffices)
            | Some(TokenKind::Obtain)
            | Some(TokenKind::Induction)
            | Some(TokenKind::Contradiction)
            | Some(TokenKind::Link)
            | Some(TokenKind::With) => {
                self.parse_path_or_record()
            }

            Some(kind) => {
                let span = self.stream.current_span();
                Err(ParseError::invalid_syntax(
                    format!("unexpected token in expression: {}", kind.description()),
                    span,
                ))
            }
            None => {
                let span = self.stream.current_span();
                Err(ParseError::unexpected_eof(&[], span))
            }
        }
    }

    /// Check if the current position looks like a macro call: identifier followed by !
    /// Also handles qualified paths like std.println!
    fn is_macro_call(&self) -> bool {
        if let Some(TokenKind::Ident(_)) = self.stream.peek_kind() {
            // Look ahead to find the ! token, allowing for . separators
            let mut offset = 1;
            loop {
                match self.stream.peek_nth_kind(offset) {
                    Some(TokenKind::Dot) => {
                        // Skip . and check for identifier
                        offset += 1;
                        if let Some(TokenKind::Ident(_)) = self.stream.peek_nth_kind(offset) {
                            offset += 1;
                            continue;
                        } else {
                            return false;
                        }
                    }
                    Some(TokenKind::Bang) => {
                        // Found !, check if followed by delimiter
                        if let Some(kind) = self.stream.peek_nth_kind(offset + 1) {
                            return matches!(
                                kind,
                                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace
                            );
                        }
                        return false;
                    }
                    _ => {
                        // Not a macro call
                        return false;
                    }
                }
            }
        }
        false
    }

    /// Try to parse a postfix operator applied to the given expression.
    /// Returns (new_expr, true) if a postfix was found, (lhs, false) otherwise.
    ///
    /// PERF: Takes ownership of lhs to avoid O(n²) cloning in postfix chains.
    /// For `a.b.c.d()`, previously each step cloned the entire prefix.
    /// Now we move lhs into Box directly - zero cloning!
    fn try_parse_postfix(&mut self, lhs: Expr) -> ParseResult<(Expr, bool)> {
        // Default: allow struct literals in postfix position
        self.try_parse_postfix_impl(lhs, true)
    }

    /// Implementation of try_parse_postfix with option to disable struct literal parsing.
    /// When `allow_struct` is false, `.field {` is NOT parsed as a variant constructor,
    /// allowing `for x in obj.field { }` to work correctly.
    fn try_parse_postfix_impl(
        &mut self,
        lhs: Expr,
        allow_struct: bool,
    ) -> ParseResult<(Expr, bool)> {
        let start_pos = self.stream.position();
        let lhs_span = lhs.span;

        match self.stream.peek_kind() {
            // Field access, tuple index, method call, or await: .field, .0, .method(args), .await
            Some(TokenKind::Dot) => {
                self.stream.advance();

                // Check for .await postfix
                if self.stream.check(&TokenKind::Await) {
                    self.stream.advance();
                    let span = lhs_span.merge(self.stream.make_span(start_pos));
                    return Ok((
                        Expr::new(
                            ExprKind::Await(Box::new(lhs)), // MOVE instead of clone!
                            span,
                        ),
                        true,
                    ));
                }

                // Check for .match { } postfix (method-syntax match expression)
                // Grammar: match_expr = [ expression , '.' ] , 'match' , [ expression ] , '{' , match_arms , '}' ;
                if self.stream.check(&TokenKind::Match) {
                    self.stream.advance();
                    // Method-syntax match has no additional scrutinee - lhs IS the scrutinee
                    self.stream.expect(TokenKind::LBrace)?;
                    let arms = self.parse_match_arms()?;
                    self.stream.expect(TokenKind::RBrace)?;
                    let span = lhs_span.merge(self.stream.make_span(start_pos));
                    return Ok((
                        Expr::new(
                            ExprKind::Match {
                                expr: Box::new(lhs),
                                arms: arms.into(),
                            },
                            span,
                        ),
                        true,
                    ));
                }

                // Check if it's a tuple index (numeric literal)
                if let Some(TokenKind::Integer(int_lit)) = self.stream.peek_kind() {
                    let index_value = int_lit.as_i64().unwrap_or(0);
                    self.stream.advance();

                    // Convert to u32 for tuple index
                    let index = u32::try_from(index_value).map_err(|_| {
                        ParseError::invalid_syntax(
                            format!("invalid tuple index: {}", index_value),
                            self.stream.make_span(start_pos),
                        )
                    })?;

                    let span = lhs_span.merge(self.stream.make_span(start_pos));
                    Ok((
                        Expr::new(
                            ExprKind::TupleIndex {
                                expr: Box::new(lhs), // MOVE instead of clone!
                                index,
                            },
                            span,
                        ),
                        true,
                    ))
                } else if let Some(TokenKind::Float(float_lit)) = self.stream.peek_kind() {
                    // Handle nested tuple access: `expr.0.0` where the lexer tokenizes
                    // `0.0` as a single Float literal instead of `Integer(0), Dot, Integer(0)`.
                    // We decompose the float `N.M` into two TupleIndex operations.
                    if float_lit.suffix.is_none() {
                        // Use raw text to preserve the exact source representation
                        // (e.g., "0.0" stays "0.0", not "0" from f64 Display)
                        let raw_text = float_lit.raw.as_str();
                        if let Some((left_str, right_str)) = raw_text.split_once('.') {
                            if let (Ok(first_idx), Ok(second_idx)) =
                                (left_str.parse::<u32>(), right_str.parse::<u32>())
                            {
                                // Verify both parts are valid non-negative integers
                                let reconstructed = format!("{}.{}", first_idx, second_idx);
                                if reconstructed == raw_text {
                                    self.stream.advance();
                                    let span = lhs_span.merge(self.stream.make_span(start_pos));

                                    // Build first TupleIndex: expr.N
                                    let first_access = Expr::new(
                                        ExprKind::TupleIndex {
                                            expr: Box::new(lhs),
                                            index: first_idx,
                                        },
                                        span,
                                    );

                                    // Build second TupleIndex: (expr.N).M
                                    return Ok((
                                        Expr::new(
                                            ExprKind::TupleIndex {
                                                expr: Box::new(first_access),
                                                index: second_idx,
                                            },
                                            span,
                                        ),
                                        true,
                                    ));
                                }
                            }
                        }
                    }
                    // Fall through: if the float doesn't decompose into valid tuple indices,
                    // treat as an error (e.g., `3.14` after a dot is nonsensical)
                    let span = lhs_span.merge(self.stream.make_span(start_pos));
                    Err(ParseError::invalid_syntax(
                        "unexpected float literal after '.'",
                        span,
                    ))
                } else {
                    // Check for incomplete float literal: Integer followed by Dot without field name
                    // This catches patterns like `1.;` which should error as "incomplete float literal"
                    if let ExprKind::Literal(lit) = &lhs.kind {
                        if matches!(lit.kind, verum_ast::LiteralKind::Int(_)) {
                            // Check if next token is not valid for field/method access
                            match self.stream.peek_kind() {
                                // These indicate incomplete float literal, not field access
                                Some(TokenKind::Semicolon) | Some(TokenKind::Comma) |
                                Some(TokenKind::RParen) | Some(TokenKind::RBracket) |
                                Some(TokenKind::RBrace) | Some(TokenKind::Eof) | None => {
                                    let span = lhs_span.merge(self.stream.make_span(start_pos));
                                    return Err(ParseError::invalid_number(
                                        "incomplete float literal: digit required after decimal point",
                                        span,
                                    ));
                                }
                                _ => {}
                            }
                        }
                    }

                    // E048: Check for incomplete field access: expr. followed by terminator
                    // This catches patterns like `obj.;` which should error as E048
                    match self.stream.peek_kind() {
                        Some(TokenKind::Semicolon) | Some(TokenKind::Comma) |
                        Some(TokenKind::RParen) | Some(TokenKind::RBracket) |
                        Some(TokenKind::RBrace) | Some(TokenKind::Eof) | None => {
                            let span = lhs_span.merge(self.stream.make_span(start_pos));
                            return Err(ParseError::expr_stmt_invalid(
                                "incomplete field access: expected field name after '.'",
                                span,
                            ));
                        }
                        _ => {}
                    }

                    // Turbofish syntax without method name: expr.<Type, N>(args)
                    // This is used for explicit generic arguments on function calls.
                    // Example: fill.<Int, 8>(0) or identity.<3>()
                    if self.stream.check(&TokenKind::Lt)
                        && self.is_generic_method_call_lookahead()
                    {
                        let generic_args = self.parse_generic_args()?;
                        // Must be followed by `(`
                        self.stream.expect(TokenKind::LParen)?;
                        let args = if self.stream.check(&TokenKind::RParen) {
                            Vec::new()
                        } else {
                            self.comma_separated(|p| p.parse_expr())?
                        };
                        self.stream.expect(TokenKind::RParen)?;
                        let span = lhs_span.merge(self.stream.make_span(start_pos));
                        return Ok((
                            Expr::new(
                                ExprKind::Call {
                                    func: Box::new(lhs),
                                    args: args.into(),
                                    type_args: generic_args.into(),
                                },
                                span,
                            ),
                            true,
                        ));
                    }

                    // Regular field access, method call, or variant constructor with record data.
                    //
                    // Accept *any* keyword as a field / method name — they live
                    // in the type's namespace (`Type.name`), so reserved-like
                    // keywords such as `where` (used by `Tensor.where(…)` in
                    // `vbc/tensor/002_operations.vr`), `from`, `match`, `proof`,
                    // etc. never collide with the language grammar at this
                    // position. Using the permissive variant mirrors the fix
                    // already applied to `parse_variant` (which accepts
                    // keywords such as `loop`, `merid` as variant names).
                    let field_name = self.consume_ident_or_any_keyword()?;
                    let field_name_span = self.stream.current_span();

                    // Check if it's a method call:
                    // - .method() - regular method call
                    // - .method<T>() - generic method call (turbofish syntax)
                    // For generic method calls, we need lookahead to distinguish from comparison:
                    // - .field < expr is a comparison, not generic args
                    // - .method<T>() is a generic method call (< must be followed by valid generic args and then >()
                    let is_generic_method_call = self.stream.check(&TokenKind::Lt)
                        && self.is_generic_method_call_lookahead();

                    if self.stream.check(&TokenKind::LParen) || is_generic_method_call {
                        // Special handling for .attenuate() method with Capability.X syntax
                        // Only use CapabilitySet syntax if the argument starts with "Capability"
                        // Otherwise treat as regular method call (allows UInt16 constants like CAP_READ_WRITE)
                        if field_name.as_str() == "attenuate" && self.stream.check(&TokenKind::LParen) {
                            // Look ahead to check if first argument is "Capability" identifier
                            let first_arg_name = self.stream.peek_nth_kind(1)
                                .and_then(|kind| {
                                    if let TokenKind::Ident(name) = kind {
                                        Some(name.clone())
                                    } else {
                                        None
                                    }
                                });

                            let is_capability_set_syntax = first_arg_name.as_ref()
                                .map(|n| n.as_str() == "Capability")
                                .unwrap_or(false);

                            // Reject plural "Capabilities" - must use singular "Capability"
                            if let Some(ref name) = first_arg_name {
                                if name.as_str() == "Capabilities" {
                                    return Err(crate::ParseError::new(
                                        crate::error::ParseErrorKind::InvalidSyntax {
                                            message: "use singular 'Capability' instead of 'Capabilities'".into(),
                                        },
                                        self.stream.make_span(start_pos),
                                    ));
                                }
                            }

                            // Reject empty attenuate() - at least one capability required
                            if self.stream.peek_nth_kind(1)
                                .map(|kind| matches!(kind, TokenKind::RParen))
                                .unwrap_or(false)
                            {
                                return Err(crate::ParseError::new(
                                    crate::error::ParseErrorKind::InvalidSyntax {
                                        message: "attenuate() requires at least one capability argument".into(),
                                    },
                                    self.stream.make_span(start_pos),
                                ));
                            }

                            if is_capability_set_syntax {
                                // Parse attenuate(CapabilitySet)
                                self.stream.expect(TokenKind::LParen)?;
                                let capabilities = self.parse_capability_set()?;
                                self.stream.expect(TokenKind::RParen)?;
                                let span = lhs_span.merge(self.stream.make_span(start_pos));
                                return Ok((
                                    Expr::new(
                                        ExprKind::Attenuate {
                                            context: Box::new(lhs),
                                            capabilities,
                                        },
                                        span,
                                    ),
                                    true,
                                ));
                            }
                            // Otherwise fall through to regular method call parsing
                        }

                        // Parse optional generic type arguments: .method<T, U>()
                        let type_args = if is_generic_method_call {
                            self.parse_generic_args()?
                        } else {
                            Vec::new()
                        };

                        // Regular method call
                        let args = self.parse_call_args()?;
                        let span = lhs_span.merge(self.stream.make_span(start_pos));
                        Ok((
                            Expr::new(
                                ExprKind::MethodCall {
                                    receiver: Box::new(lhs), // MOVE instead of clone!
                                    method: verum_ast::ty::Ident::new(field_name, span),
                                    type_args: type_args.into_iter().collect(),
                                    args: args.into(),
                                },
                                span,
                            ),
                            true,
                        ))
                    } else if allow_struct && self.stream.check(&TokenKind::LBrace) {
                        // Variant constructor with record data: Type.Variant { field: value }
                        // Only parse if allow_struct is true (disabled in no_struct mode for for-loops, etc.)
                        // Convert lhs.field into a two-segment path
                        self.stream.advance();

                        // Build the path from lhs + field_name
                        // First, try to convert the lhs expression to a path
                        let path = if let Maybe::Some(mut base_path) = Self::expr_to_path(&lhs) {
                            // Successfully converted lhs to a path, extend it with the field name
                            base_path.segments.push(verum_ast::ty::PathSegment::Name(
                                verum_ast::ty::Ident::new(field_name.clone(), field_name_span),
                            ));
                            base_path
                        } else {
                            // lhs cannot be converted to a path (e.g., function call, binary op)
                            return Err(ParseError::invalid_syntax(
                                "record literal requires a path (e.g., Type { ... } or module.Type { ... })",
                                lhs_span,
                            ));
                        };

                        // Parse fields and base spread
                        let mut fields = Vec::new();
                        let mut base = Maybe::None;

                        if !self.stream.check(&TokenKind::RBrace) {
                            loop {
                                // Safety: prevent infinite loop
                                if !self.tick() || self.is_aborted() {
                                    break;
                                }

                                // Check for struct update/rest syntax: ..base or just ..
                                if self.stream.consume(&TokenKind::DotDot).is_some() {
                                    // Check if this is rest-only (..) or struct update (..base)
                                    if self.stream.check(&TokenKind::RBrace) || self.stream.check(&TokenKind::Comma) {
                                        // Rest-only pattern: { x, y, .. }
                                        let rest_span = self.stream.current_span();
                                        base = Maybe::Some(Box::new(Expr::new(ExprKind::Tuple(List::new()), rest_span)));
                                    } else {
                                        // Struct update: { x, y, ..base }
                                        base = Maybe::Some(Box::new(self.parse_expr()?));
                                    }
                                    // After spread, we can optionally have a trailing comma
                                    self.stream.consume(&TokenKind::Comma);
                                    break;
                                }

                                // Parse regular field
                                fields.push(self.parse_field_init()?);

                                // Check for comma
                                if self.stream.consume(&TokenKind::Comma).is_none() {
                                    break;
                                }

                                // Allow trailing comma
                                if self.stream.check(&TokenKind::RBrace) {
                                    break;
                                }
                            }
                        }

                        let end_span = self.stream.expect(TokenKind::RBrace)?.span;
                        let span = lhs_span.merge(end_span);

                        Ok((
                            Expr::new(ExprKind::Record { path, fields: fields.into(), base }, span),
                            true,
                        ))
                    } else {
                        // Check if this is a type property access (e.g., Int.size, Float.alignment)
                        // Type properties are: size, alignment, stride, min, max, bits, name
                        //
                        // CRITICAL FIX: Only convert to TypeProperty if the path looks like a type.
                        // Type names by convention start with uppercase (PascalCase), while variables
                        // are lowercase (snake_case or camelCase). This heuristic prevents `p.name`
                        // (where p is a variable) from being parsed as a TypeProperty expression.
                        if let Some(type_prop) = TypeProperty::from_str(field_name.as_str()) {
                            // Try to convert expression to a type
                            // This handles: Int, (&Int), (&checked Int), List<Int>, etc.
                            if let Some(ty) = self.expr_to_type_for_property(&lhs) {
                                let span = lhs_span.merge(self.stream.make_span(start_pos));
                                return Ok((
                                    Expr::new(
                                        ExprKind::TypeProperty {
                                            ty,
                                            property: type_prop,
                                        },
                                        span,
                                    ),
                                    true,
                                ));
                            }
                        }

                        // Regular field access
                        let span = lhs_span.merge(self.stream.make_span(start_pos));
                        Ok((
                            Expr::new(
                                ExprKind::Field {
                                    expr: Box::new(lhs), // MOVE instead of clone!
                                    field: verum_ast::ty::Ident::new(field_name, span),
                                },
                                span,
                            ),
                            true,
                        ))
                    }
                }
            }

            // Optional chaining: ?.field
            Some(TokenKind::QuestionDot) => {
                self.stream.advance();
                let field_name = self.consume_ident_or_keyword()?;
                let span = lhs_span.merge(self.stream.make_span(start_pos));
                Ok((
                    Expr::new(
                        ExprKind::OptionalChain {
                            expr: Box::new(lhs), // MOVE instead of clone!
                            field: verum_ast::ty::Ident::new(field_name, span),
                        },
                        span,
                    ),
                    true,
                ))
            }

            // Generic function call: func<T, U>(args)
            // Detect when < followed by valid type args and then >()
            Some(TokenKind::Lt) if matches!(&lhs.kind, ExprKind::Path(_)) && self.is_generic_func_call_lookahead() => {
                // Parse generic type arguments: <T, U>
                let type_args = self.parse_generic_args()?;

                // Parse function call arguments: (args)
                let args = self.parse_call_args()?;
                let span = lhs_span.merge(self.stream.make_span(start_pos));
                Ok((
                    Expr::new(
                        ExprKind::Call {
                            func: Box::new(lhs),
                            type_args: type_args.into_iter().collect(),
                            args: args.into(),
                        },
                        span,
                    ),
                    true,
                ))
            }

            // Turbofish generic call: `path::<T1, T2>(args)`.
            //
            // Verum prefers `foo<T>(args)` in most positions, but the
            // turbofish form is the only unambiguous way to supply
            // explicit generic arguments to a function call that
            // otherwise parses as a comparison (`foo < T`). The stdlib
            // (e.g. `core/math/cubical.vr`) uses it in contract
            // expressions where any ambiguity would be syntactic.
            Some(TokenKind::ColonColon)
                if self.stream.peek_nth_kind(1) == Some(&TokenKind::Lt) =>
            {
                self.stream.advance(); // consume `::`
                let type_args = self.parse_generic_args()?;
                // After `::<…>` a call `(args)` must follow — otherwise
                // this is not a turbofish call and we shouldn't consume
                // the turbofish; but by the grammar `::<T>` only
                // appears as a generic-arg annotation on a call site,
                // so treat a missing `(` as an error.
                // parse_call_args consumes its own `(` and `)`.
                let args = self.parse_call_args()?;
                let span = lhs_span.merge(self.stream.make_span(start_pos));
                return Ok((
                    Expr::new(
                        ExprKind::Call {
                            func: Box::new(lhs),
                            type_args: type_args.into_iter().collect::<List<_>>(),
                            args: args.into(),
                        },
                        span,
                    ),
                    true,
                ));
            }

            // Function call: (args)
            Some(TokenKind::LParen) => {
                // CRITICAL FIX: Block expressions, control flow expressions, and similar
                // are not callable. If we parsed `{ ... } (a, b)` as a call, we would
                // incorrectly interpret the block as a function being called with tuple args.
                // Example: `fn f() -> (Int, Int) { { print(1); } (1, 2) }`
                // Without this fix, `{ print(1); }` would be parsed as the "function" and
                // `(1, 2)` as arguments, causing "not a function: Unit" error.
                // Instead, stop postfix parsing and let the statement parser handle them
                // as separate expressions.
                match &lhs.kind {
                    ExprKind::Block(_)
                    | ExprKind::If { .. }
                    | ExprKind::Match { .. }
                    | ExprKind::While { .. }
                    | ExprKind::For { .. }
                    | ExprKind::ForAwait { .. }
                    | ExprKind::Loop { .. }
                    | ExprKind::Select { .. }
                    | ExprKind::TryRecoverFinally { .. } => {
                        // Not callable - return as-is
                        return Ok((lhs, false));
                    }
                    _ => {}
                }
                let args = self.parse_call_args()?;
                let span = lhs_span.merge(self.stream.make_span(start_pos));

                // E026/E027: Check for empty assert/assume calls
                if args.is_empty() {
                    if let ExprKind::Path(path) = &lhs.kind {
                        if path.segments.len() == 1 {
                            if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                                let name = ident.name.as_str();
                                if name == "assert" {
                                    return Err(ParseError::invalid_assert(
                                        "assert requires an expression argument",
                                        span,
                                    ));
                                }
                                if name == "assume" {
                                    return Err(ParseError::invalid_assume(
                                        "assume requires an expression argument",
                                        span,
                                    ));
                                }
                            }
                        }
                    }
                }

                Ok((
                    Expr::new(
                        ExprKind::Call {
                            func: Box::new(lhs), // MOVE instead of clone!
                            type_args: List::new(),
                            args: args.into(),
                        },
                        span,
                    ),
                    true,
                ))
            }

            // Index: [index]
            Some(TokenKind::LBracket) => {
                let current_span = self.stream.current_span();
                let bracket_span = self.stream.advance()
                    .ok_or_else(|| ParseError::unexpected_eof(&[TokenKind::LBracket], current_span))?
                    .span;

                // E013: Check for immediately unclosed index (arr[;)
                if self.stream.check(&TokenKind::Semicolon) || self.stream.check(&TokenKind::RBrace) {
                    return Err(ParseError::stmt_unclosed_index(
                        bracket_span,
                    ).with_help("missing closing ']' for index access"));
                }

                let first_index = self.parse_expr()?;

                // Support multi-dimensional indexing: a[i, j, k]
                // Parsed as Index { expr, index: Tuple([i, j, k]) } when comma present
                let index = if self.stream.check(&TokenKind::Comma) {
                    let mut indices = vec![first_index];
                    while self.stream.check(&TokenKind::Comma) {
                        self.stream.advance(); // consume comma
                        indices.push(self.parse_expr()?);
                    }
                    let tuple_span = match (indices.first(), indices.last()) {
                        (Some(first), Some(last)) => first.span.merge(last.span),
                        _ => bracket_span, // fallback to the opening bracket span
                    };
                    Expr::new(ExprKind::Tuple(indices.into()), tuple_span)
                } else {
                    first_index
                };

                // E013: Check for unclosed index after expression
                if !self.stream.check(&TokenKind::RBracket) {
                    if self.stream.check(&TokenKind::Semicolon) || self.stream.at_end() {
                        return Err(ParseError::stmt_unclosed_index(
                            bracket_span,
                        ).with_help("missing closing ']' for index access"));
                    }
                }

                self.stream.expect(TokenKind::RBracket)?;
                let span = lhs_span.merge(self.stream.make_span(start_pos));
                Ok((
                    Expr::new(
                        ExprKind::Index {
                            expr: Box::new(lhs), // MOVE instead of clone!
                            index: Box::new(index),
                        },
                        span,
                    ),
                    true,
                ))
            }

            // Try operator: ?
            Some(TokenKind::Question) => {
                self.stream.advance();
                let span = lhs_span.merge(self.stream.make_span(start_pos));
                Ok((
                    Expr::new(
                        ExprKind::Try(Box::new(lhs)), // MOVE instead of clone!
                        span,
                    ),
                    true,
                ))
            }

            // NOTE: `is` is NOT a postfix operator — it's an infix comparison operator
            // like `==` and must be handled in parse_expr_bp / parse_expr_bp_no_struct.
            // Handling it here would cause `*r is Some` to parse as `*(r is Some)`
            // instead of the correct `(*r) is Some`, because postfix ops in
            // parse_prefix_expr bind tighter than prefix unary operators.

            // No postfix found - return lhs unchanged
            _ => Ok((lhs, false)),
        }
    }

    /// Convert a token kind to a binary operator.
    fn token_to_binop(&self, kind: &TokenKind) -> ParseResult<BinOp> {
        let op = match kind {
            TokenKind::Plus => BinOp::Add,
            TokenKind::Minus => BinOp::Sub,
            TokenKind::Star => BinOp::Mul,
            TokenKind::Slash => BinOp::Div,
            TokenKind::Percent => BinOp::Rem,
            TokenKind::StarStar => BinOp::Pow,
            TokenKind::EqEq => BinOp::Eq,
            TokenKind::BangEq => BinOp::Ne,
            TokenKind::Lt => BinOp::Lt,
            TokenKind::LtEq => BinOp::Le,
            TokenKind::Gt => BinOp::Gt,
            TokenKind::GtEq => BinOp::Ge,
            TokenKind::In => BinOp::In,
            TokenKind::AmpersandAmpersand => BinOp::And,
            TokenKind::PipePipe => BinOp::Or,
            TokenKind::Implies | TokenKind::FatArrow | TokenKind::RArrow => BinOp::Imply,
            TokenKind::Iff => BinOp::Iff,
            TokenKind::PlusPlus => BinOp::Concat,
            TokenKind::Ampersand => BinOp::BitAnd,
            TokenKind::Pipe => BinOp::BitOr,
            TokenKind::Caret => BinOp::BitXor,
            TokenKind::LtLt => BinOp::Shl,
            TokenKind::GtGt => BinOp::Shr,
            TokenKind::Eq => BinOp::Assign,
            TokenKind::PlusEq => BinOp::AddAssign,
            TokenKind::MinusEq => BinOp::SubAssign,
            TokenKind::StarEq => BinOp::MulAssign,
            TokenKind::SlashEq => BinOp::DivAssign,
            TokenKind::PercentEq => BinOp::RemAssign,
            TokenKind::AmpersandEq => BinOp::BitAndAssign,
            TokenKind::PipeEq => BinOp::BitOrAssign,
            TokenKind::CaretEq => BinOp::BitXorAssign,
            TokenKind::LtLtEq => BinOp::ShlAssign,
            TokenKind::GtGtEq => BinOp::ShrAssign,
            _ => {
                let span = self.stream.current_span();
                return Err(ParseError::invalid_syntax(
                    format!("not a binary operator: {}", kind.description()),
                    span,
                ));
            }
        };
        Ok(op)
    }

    // ========================================================================
    // Literal parsing
    // ========================================================================

    fn parse_integer_literal(&mut self) -> ParseResult<Expr> {
        use verum_lexer::IntegerLiteral;
        match self.stream.advance() {
            Some(Token {
                kind: TokenKind::Integer(IntegerLiteral { raw_value, base, suffix }),
                span,
            }) => {
                // Parse the integer value, supporting arbitrary precision via i128
                // For values that don't fit in i128, we store them as a string literal
                let digits: String = raw_value.chars().filter(|&c| c != '_').collect();
                let value = i128::from_str_radix(&digits, *base as u32).unwrap_or(0);
                let mut lit = Literal::int(value, *span);
                if let Some(suffix_str) = suffix {
                    let int_suffix = parse_int_suffix(suffix_str.as_str()).unwrap_or_else(|| {
                        verum_ast::literal::IntSuffix::Custom(suffix_str.clone())
                    });
                    if let verum_ast::literal::LiteralKind::Int(ref mut int_lit) = lit.kind {
                        int_lit.suffix = Some(int_suffix);
                    }
                }
                Ok(Expr::literal(lit))
            }
            _ => unreachable!(),
        }
    }

    fn parse_float_literal(&mut self) -> ParseResult<Expr> {
        use verum_lexer::FloatLiteral;
        match self.stream.advance() {
            Some(Token {
                kind: TokenKind::Float(FloatLiteral { value, suffix, .. }),
                span,
            }) => {
                let mut lit = Literal::float(*value, *span);
                if let Some(suffix_str) = suffix {
                    let float_suffix =
                        parse_float_suffix(suffix_str.as_str()).unwrap_or_else(|| {
                            verum_ast::literal::FloatSuffix::Custom(suffix_str.clone())
                        });
                    if let verum_ast::literal::LiteralKind::Float(ref mut float_lit) = lit.kind {
                        float_lit.suffix = Some(float_suffix);
                    }
                }
                Ok(Expr::literal(lit))
            }
            _ => unreachable!(),
        }
    }

    fn parse_string_literal(&mut self) -> ParseResult<Expr> {
        match self.stream.advance() {
            Some(Token {
                kind: TokenKind::Text(val),
                span,
            }) => Ok(Expr::literal(Literal::string(val.clone(), *span))),
            _ => unreachable!(),
        }
    }

    fn parse_char_literal(&mut self) -> ParseResult<Expr> {
        match self.stream.advance() {
            Some(Token {
                kind: TokenKind::Char(val),
                span,
            }) => Ok(Expr::literal(Literal::char(*val, *span))),
            _ => unreachable!(),
        }
    }

    fn parse_byte_char_literal(&mut self) -> ParseResult<Expr> {
        match self.stream.advance() {
            Some(Token {
                kind: TokenKind::ByteChar(val),
                span,
            }) => Ok(Expr::literal(Literal::byte_char(*val, *span))),
            _ => unreachable!(),
        }
    }

    fn parse_byte_string_literal(&mut self) -> ParseResult<Expr> {
        match self.stream.advance() {
            Some(Token {
                kind: TokenKind::ByteString(val),
                span,
            }) => Ok(Expr::literal(Literal::byte_string(val.clone(), *span))),
            _ => unreachable!(),
        }
    }

    fn parse_bool_literal(&mut self) -> ParseResult<Expr> {
        match self.stream.advance() {
            Some(Token {
                kind: TokenKind::True,
                span,
            }) => Ok(Expr::literal(Literal::bool(true, *span))),
            Some(Token {
                kind: TokenKind::False,
                span,
            }) => Ok(Expr::literal(Literal::bool(false, *span))),
            _ => unreachable!(),
        }
    }

    fn parse_interpolated_string(&mut self) -> ParseResult<Expr> {
        use verum_lexer::InterpolatedStringLiteral;

        // Extract values first before calling parse_interpolated_content
        let (prefix, content, span, file_id) = match self.stream.advance() {
            Some(Token {
                kind: TokenKind::InterpolatedString(InterpolatedStringLiteral { prefix, content }),
                span,
            }) => (prefix.clone(), content.clone(), *span, span.file_id),
            _ => unreachable!(),
        };

        // Parse the interpolated content to extract parts and expressions
        let (parts, exprs) =
            crate::safe_interpolation::parse_interpolated_content(self, content.as_str(), file_id)?;

        // Create an InterpolatedString expression with parsed parts and expressions
        Ok(Expr::new(
            ExprKind::InterpolatedString {
                handler: prefix,
                parts: parts.into(),
                exprs: exprs.into(),
            },
            span,
        ))
    }

    fn parse_contract_literal(&mut self) -> ParseResult<Expr> {
        match self.stream.advance() {
            Some(Token {
                kind: TokenKind::ContractLiteral(content),
                span,
            }) => Ok(Expr::literal(Literal::contract(content.clone(), *span))),
            _ => unreachable!(),
        }
    }

    fn parse_tagged_literal(&mut self) -> ParseResult<Expr> {
        use verum_lexer::{TaggedLiteralData, TaggedLiteralDelimiter};
        match self.stream.advance() {
            Some(Token {
                kind:
                    TokenKind::TaggedLiteral(TaggedLiteralData {
                        tag,
                        content,
                        delimiter,
                    }),
                span,
            }) => {
                use verum_ast::literal::CompositeDelimiter;

                // Always treat TaggedLiterals as composite literals
                // The is_recognized() method will determine if it's a known type
                if tag.as_str() != "contract" {
                    // Map lexer delimiter to AST delimiter
                    let ast_delimiter = match delimiter {
                        TaggedLiteralDelimiter::Quote => CompositeDelimiter::Quote,
                        TaggedLiteralDelimiter::TripleQuote => CompositeDelimiter::TripleQuote,
                        TaggedLiteralDelimiter::Paren => CompositeDelimiter::Paren,
                        TaggedLiteralDelimiter::Bracket => CompositeDelimiter::Bracket,
                        TaggedLiteralDelimiter::Brace => CompositeDelimiter::Brace,
                    };

                    // For quoted composite literals, strip outer structural delimiters
                    // e.g., mat#"[[1,2]]" -> content is "[[1,2]]", strip to "[1,2]"
                    // e.g., vec#"<1,2,3>" -> content is "<1,2,3>", strip to "1,2,3"
                    // e.g., interval#"[0,100]" -> content is "[0,100]", strip to "0,100"
                    // Note: For intervals with mismatched brackets like [0,100), keep as-is
                    let inner_content = if *delimiter == TaggedLiteralDelimiter::Quote {
                        let trimmed = content.as_str().trim();
                        let should_strip = match tag.as_str() {
                            "mat" | "vec" => {
                                // Strip outer delimiters for matrix and vector
                                (trimmed.starts_with('[') && trimmed.ends_with(']'))
                                    || (trimmed.starts_with('<') && trimmed.ends_with('>'))
                            }
                            "interval" => {
                                // Only strip if delimiters match (closed or open)
                                (trimmed.starts_with('[') && trimmed.ends_with(']'))
                                    || (trimmed.starts_with('(') && trimmed.ends_with(')'))
                            }
                            _ => false,
                        };

                        if should_strip {
                            Text::from(&trimmed[1..trimmed.len() - 1])
                        } else {
                            content.clone()
                        }
                    } else {
                        content.clone()
                    };

                    // Validate format-tagged literals at parse time
                    if let Some(validation_err) = validate_format_tag(tag.as_str(), &inner_content) {
                        return Err(ParseError::invalid_syntax(
                            &format!("invalid {} literal: {}", tag, validation_err) as &str,
                            *span,
                        ));
                    }

                    Ok(Expr::literal(Literal::composite(
                        tag.clone(),
                        inner_content,
                        ast_delimiter,
                        *span,
                    )))
                } else {
                    // contract#"..." uses a separate ContractLiteral token, won't reach here
                    unreachable!("All TaggedLiterals should be treated as composite")
                }
            }
            _ => unreachable!(),
        }
    }

    fn parse_hex_color(&mut self) -> ParseResult<Expr> {
        match self.stream.advance() {
            Some(Token {
                kind: TokenKind::HexColor(color),
                span,
            }) => Ok(Expr::literal(Literal::tagged(
                Text::from("color"),
                color.clone(),
                *span,
            ))),
            _ => unreachable!(),
        }
    }

    // ========================================================================
    // Path and record parsing
    // ========================================================================

    /// Check if the current `<` token starts a generic type expression rather than a comparison.
    ///
    /// This uses lookahead to find the matching `>` and checks what follows.
    /// Generic type expressions are followed by: `.`, `(`, `;`, `,`, `)`, `]`, `}`, or `>`
    /// (indicating a nested generic or end of expression context).
    ///
    /// This enables parsing patterns like:
    /// - `Repository<User>.find(1)` - context method call
    /// - `List<Int>.new()` - static method call
    /// - `Map<Text, Int>.with_capacity(10)` - associated function call
    fn is_generic_type_expr_lookahead(&self) -> bool {
        // We're at a `<` token. Look ahead to find the matching `>`.
        let mut depth = 0;
        let mut offset = 0;

        loop {
            let token = self.stream.peek_nth_kind(offset);

            match token {
                Some(TokenKind::Lt) => {
                    depth += 1;
                    offset += 1;
                }
                Some(TokenKind::Gt) => {
                    depth -= 1;
                    if depth == 0 {
                        // Found the matching `>`. Check what follows.
                        offset += 1;
                        let next = self.stream.peek_nth_kind(offset);
                        // NOTE: LParen is NOT included here because func<T>() is a generic function call,
                        // not a TypeExpr. TypeExpr is for Type<T>.method() pattern.
                        return matches!(
                            next,
                            Some(TokenKind::Dot)         // method call: Type<T>.method()
                                | Some(TokenKind::Semicolon) // end of statement
                                | Some(TokenKind::Comma)    // in list context
                                | Some(TokenKind::RParen)   // end of paren expr
                                | Some(TokenKind::RBracket) // end of array
                                | Some(TokenKind::RBrace)   // end of block
                                | Some(TokenKind::Question) // try operator
                                | Some(TokenKind::QuestionDot) // optional chaining
                                | None  // end of input
                        );
                    }
                    offset += 1;
                }
                // Right shift `>>` can act as two `>` tokens in generic context
                Some(TokenKind::GtGt) => {
                    depth -= 2;
                    if depth <= 0 {
                        // Found the end. Check what follows.
                        // NOTE: LParen excluded - func<T>() is a generic function call, not TypeExpr
                        offset += 1;
                        let next = self.stream.peek_nth_kind(offset);
                        return matches!(
                            next,
                            Some(TokenKind::Dot)
                                | Some(TokenKind::Semicolon)
                                | Some(TokenKind::Comma)
                                | Some(TokenKind::RParen)
                                | Some(TokenKind::RBracket)
                                | Some(TokenKind::RBrace)
                                | Some(TokenKind::Question)
                                | Some(TokenKind::QuestionDot)
                                | None
                        );
                    }
                    offset += 1;
                }
                // Tokens that are valid inside generic args
                Some(TokenKind::Ident(_))
                | Some(TokenKind::Comma)
                | Some(TokenKind::Colon)
                | Some(TokenKind::Dot)  // For nested paths
                | Some(TokenKind::Integer(_))
                | Some(TokenKind::Eq)
                | Some(TokenKind::LBracket)
                | Some(TokenKind::RBracket)
                | Some(TokenKind::LParen)
                | Some(TokenKind::RParen)
                | Some(TokenKind::Ampersand)
                | Some(TokenKind::Star)
                | Some(TokenKind::SelfType)
                | Some(TokenKind::Plus) => {
                    offset += 1;
                }
                // Invalid or end - not a generic type
                _ => {
                    return false;
                }
            }

            // Safety: don't loop forever on malformed input
            if offset > 100 {
                return false;
            }
        }
    }

    /// Check if the current `<` token starts generic type arguments for a method call.
    ///
    /// This is similar to `is_generic_type_expr_lookahead()` but specifically for method calls.
    /// For a generic method call like `.method<T>()`, the closing `>` must be followed by `(`.
    ///
    /// This prevents misinterpreting comparisons like `.field < value` as generic args.
    fn is_generic_method_call_lookahead(&self) -> bool {
        // We're at a `<` token. Look ahead to find the matching `>`.
        let mut depth = 0;
        let mut offset = 0;

        loop {
            let token = self.stream.peek_nth_kind(offset);

            match token {
                Some(TokenKind::Lt) => {
                    depth += 1;
                    offset += 1;
                }
                Some(TokenKind::Gt) => {
                    depth -= 1;
                    if depth == 0 {
                        // Found the matching `>`. For method calls, must be followed by `(`.
                        offset += 1;
                        let next = self.stream.peek_nth_kind(offset);
                        return matches!(next, Some(TokenKind::LParen));
                    }
                    offset += 1;
                }
                // Right shift `>>` can act as two `>` tokens in generic context
                Some(TokenKind::GtGt) => {
                    depth -= 2;
                    if depth <= 0 {
                        // Found the end. For method calls, must be followed by `(`.
                        offset += 1;
                        let next = self.stream.peek_nth_kind(offset);
                        return matches!(next, Some(TokenKind::LParen));
                    }
                    offset += 1;
                }
                // Tokens that are valid inside generic args
                Some(TokenKind::Ident(_))
                | Some(TokenKind::Comma)
                | Some(TokenKind::Colon)
                | Some(TokenKind::Dot)  // For nested paths
                | Some(TokenKind::Integer(_))
                | Some(TokenKind::Eq)
                | Some(TokenKind::LBracket)
                | Some(TokenKind::RBracket)
                | Some(TokenKind::LParen)
                | Some(TokenKind::RParen)
                | Some(TokenKind::Ampersand)
                | Some(TokenKind::Star)
                | Some(TokenKind::SelfType)
                | Some(TokenKind::Plus) => {
                    offset += 1;
                }
                // Invalid or end - not a generic method call
                _ => {
                    return false;
                }
            }

            // Safety: don't loop forever on malformed input
            if offset > 100 {
                return false;
            }
        }
    }

    /// Check if this is a generic function call: func<T>(args)
    /// Reuses method call lookahead logic since the pattern is identical
    fn is_generic_func_call_lookahead(&self) -> bool {
        self.is_generic_method_call_lookahead()
    }

    /// Check if `nursery` keyword starts a nursery expression.
    ///
    /// Disambiguates between:
    /// - `nursery { body }` or `nursery( options ) { body }` - nursery expression
    /// - `nursery = ...`, `nursery.field`, etc. - `nursery` used as an identifier
    ///
    /// A nursery expression is detected when `nursery` is followed by `{` or `(`.
    fn is_nursery_expr_lookahead(&self) -> bool {
        // We're at the `nursery` keyword. Check what follows.
        match self.stream.peek_nth_kind(1) {
            // `nursery { ... }` - nursery expression with body
            Some(TokenKind::LBrace) => true,
            // `nursery( options ) { ... }` - nursery expression with options
            Some(TokenKind::LParen) => true,
            // These tokens indicate `nursery` is used as an identifier:
            // `nursery = ...`, `nursery.field`, `nursery:`, etc.
            Some(TokenKind::Eq | TokenKind::Dot | TokenKind::Colon
                | TokenKind::Comma | TokenKind::RParen | TokenKind::RBracket
                | TokenKind::RBrace) => false,
            // Semicolon or EOF after nursery — likely a missing block (E0D4)
            Some(TokenKind::Semicolon) | None => true,
            // Any other token (|, number, identifier, keyword) after nursery
            // is likely a malformed nursery expression — route to parse_nursery_expr
            // which will emit E0D4 for missing block
            _ => true,
        }
    }

    /// Check if `select` keyword starts a select expression.
    ///
    /// Disambiguates between:
    /// - `select { arms }` or `select biased { arms }` - select expression
    /// - `select(...)` or `select.field` - function call or field access using `select` as identifier
    ///
    /// A select expression is detected when `select` is followed by `{` or `biased {`.
    fn is_select_expr_lookahead(&self) -> bool {
        // We're at the `select` keyword. Check what follows.
        match self.stream.peek_nth_kind(1) {
            // `select { ... }` - definitely a select expression
            Some(TokenKind::LBrace) => true,
            // `select biased { ... }` - biased select expression
            Some(TokenKind::Ident(name)) if name.as_str() == "biased" => {
                matches!(self.stream.peek_nth_kind(2), Some(TokenKind::LBrace))
            }
            // Anything else (like `(`, `.`) means `select` is used as an identifier
            _ => false,
        }
    }

    fn parse_path_or_record(&mut self) -> ParseResult<Expr> {
        // In expression context, we need to parse a simple path (single identifier or
        // :: qualified path) but NOT consume `.` as that's for postfix field access.
        let start_pos = self.stream.position();
        let path = self.parse_simple_expr_path()?;

        // Check if this looks like a generic type expression: Identifier<Type>.method()
        // This handles patterns like Repository<User>.find() or List<Int>.new()
        if self.stream.check(&TokenKind::Lt) && self.is_generic_type_expr_lookahead() {
            // Parse the generic type args and create a TypeExpr
            let args = self.parse_generic_args()?;
            let span = self.stream.make_span(start_pos);

            // Construct the generic type
            let base_type = Type::new(TypeKind::Path(path.clone()), path.span);
            let generic_type = Type::new(
                TypeKind::Generic {
                    base: Box::new(base_type),
                    args: args.into_iter().collect(),
                },
                span,
            );

            return Ok(Expr::new(ExprKind::TypeExpr(generic_type), span));
        }

        // Check if this is a record literal or a trailing block expression
        // If `{` is followed by a statement keyword (let, if, return, etc.),
        // it's a block expression, not a record literal.
        if self.stream.check(&TokenKind::LBrace) {
            // Peek at what's inside the brace
            match self.stream.peek_nth_kind(1) {
                // Statement keywords indicate a block, not a record literal
                Some(TokenKind::Let)
                | Some(TokenKind::If)
                | Some(TokenKind::Return)
                | Some(TokenKind::Loop)
                | Some(TokenKind::While)
                | Some(TokenKind::For)
                | Some(TokenKind::Match)
                | Some(TokenKind::Unsafe)
                | Some(TokenKind::Provide)
                | Some(TokenKind::Mount)
                | Some(TokenKind::Defer)
                | Some(TokenKind::Errdefer)
                | Some(TokenKind::Try) => {
                    // This is expr { block }, not a record literal
                    // Return just the path and let postfix parsing handle the block
                    return Ok(Expr::path(path));
                }
                _ => {}
            }

            self.stream.advance();

            // Parse fields and base spread
            let mut fields = Vec::new();
            let mut base = Maybe::None;

            if !self.stream.check(&TokenKind::RBrace) {
                loop {
                    // Safety: prevent infinite loop
                    if !self.tick() || self.is_aborted() {
                        break;
                    }

                    // Check for struct update/rest syntax: ..base or just ..
                    if self.stream.consume(&TokenKind::DotDot).is_some() {
                        // Check if this is rest-only (..) or struct update (..base)
                        if self.stream.check(&TokenKind::RBrace) || self.stream.check(&TokenKind::Comma) {
                            // Rest-only pattern: { x, y, .. }
                            // Use ExprKind::Unit as a sentinel for "rest without base"
                            let rest_span = self.stream.current_span();
                            base = Maybe::Some(Box::new(Expr::new(ExprKind::Tuple(List::new()), rest_span)));
                        } else {
                            // Struct update: { x, y, ..base }
                            base = Maybe::Some(Box::new(self.parse_expr()?));
                        }
                        // After spread, we can optionally have a trailing comma
                        self.stream.consume(&TokenKind::Comma);
                        break;
                    }

                    // Parse regular field
                    fields.push(self.parse_field_init()?);

                    // Check for comma
                    if self.stream.consume(&TokenKind::Comma).is_none() {
                        break;
                    }

                    // Allow trailing comma before closing brace
                    if self.stream.check(&TokenKind::RBrace) {
                        break;
                    }
                }
            }

            let end_span = self.stream.expect(TokenKind::RBrace)?.span;
            let span = path.span.merge(end_span);

            Ok(Expr::new(ExprKind::Record { path, fields: fields.into(), base }, span))
        } else {
            Ok(Expr::path(path))
        }
    }

    /// Parse a simple path in expression context.
    /// In EXPRESSION context, `.` should be left for postfix operators (field access, method call).
    /// Only single-segment paths are parsed here - multi-segment paths are handled via postfix `.` operators.
    /// Variant constructors like `Color.Red` are parsed as `Color` (path) + `.Red` (field access),
    /// and the type checker resolves them to variant constructors.
    fn parse_simple_expr_path(&mut self) -> ParseResult<Path> {
        use verum_ast::ty::PathSegment;
        let start_pos = self.stream.position();

        let mut segments = Vec::new();

        // Parse first (and only) segment in expression context
        let first_segment = if self.stream.consume(&TokenKind::SelfValue).is_some() {
            PathSegment::SelfValue
        } else if self.stream.consume(&TokenKind::SelfType).is_some() {
            // SelfType (Self) is a type reference, create as Name("Self") not SelfValue
            PathSegment::Name(verum_ast::ty::Ident::new(Text::from("Self"), self.stream.current_span()))
        } else if self.stream.consume(&TokenKind::Super).is_some() {
            // Handle super keyword token
            PathSegment::Super
        } else if self.stream.consume(&TokenKind::Cog).is_some() {
            // Handle crate keyword token
            PathSegment::Cog
        } else {
            // Parse regular identifier or keyword as path segment
            let name = self.consume_ident_or_keyword()?;
            let span = self.stream.current_span();
            PathSegment::Name(verum_ast::ty::Ident::new(name, span))
        };
        segments.push(first_segment);

        // DO NOT consume `.` here - in expression context, `.` is for postfix operators
        // Multi-segment paths in expressions are handled as field access chains
        // e.g., `Color.Red` is parsed as `Color` (path) + `.Red` (field access)
        // The type checker resolves `Type.Variant` to variant constructors

        // However, DO consume `::` as an alternative path separator (Rust-style paths)
        // This allows `List::new()`, `Int::parse()`, `State::Ready`, etc.
        // Do NOT consume `::` followed by `<` (turbofish syntax like `foo::<T>`) -
        // that should be handled by the caller or produce an error.
        while self.stream.check(&TokenKind::ColonColon) {
            // Peek at what follows `::` - only consume if it's an identifier-like token
            let next_is_ident = self.stream.peek_nth(1).map(|t| matches!(t.kind,
                TokenKind::Ident(_) | TokenKind::Some | TokenKind::None
                | TokenKind::Ok | TokenKind::Err | TokenKind::Result
                | TokenKind::SelfValue | TokenKind::SelfType
                | TokenKind::Super | TokenKind::Cog
                | TokenKind::By | TokenKind::Field | TokenKind::Stage
            )).unwrap_or(false);
            if next_is_ident {
                self.stream.advance(); // consume `::`
                let seg_name = self.consume_ident_or_keyword()?;
                let seg_span = self.stream.current_span();
                segments.push(PathSegment::Name(verum_ast::ty::Ident::new(seg_name, seg_span)));
            } else {
                break;
            }
        }

        let span = self.stream.make_span(start_pos);
        Ok(Path::new(segments.into_iter().collect::<List<_>>(), span))
    }

    // parse_path is defined in ty.rs
    // parse_generic_args is defined in ty.rs

    fn parse_field_init(&mut self) -> ParseResult<FieldInit> {
        let start_pos = self.stream.position();

        // Parse optional attributes (e.g., @cfg(feature = "async"))
        let attributes = self.parse_attributes()?;

        // Skip optional `ghost` keyword prefix on field initializers
        // e.g., `ghost sorted: true` in struct literals
        if let Some(TokenKind::Ident(name)) = self.stream.peek_kind() {
            if name.as_str() == "ghost" {
                // Check it's `ghost ident:` pattern (not `ghost:`)
                if let Some(TokenKind::Ident(_)) = self.stream.peek_nth_kind(1) {
                    self.stream.advance(); // consume `ghost`
                }
            }
        }

        let name_start = self.stream.position();
        let name = self.consume_ident_or_keyword()?;
        let name_span = self.stream.make_span(name_start);

        // Check for explicit value
        let value = if self.stream.consume(&TokenKind::Colon).is_some() {
            Maybe::Some(self.parse_expr()?)
        } else {
            Maybe::None
        };

        let span = self.stream.make_span(start_pos);
        if attributes.is_empty() {
            Ok(FieldInit::new(
                verum_ast::ty::Ident::new(name, name_span),
                value,
                span,
            ))
        } else {
            Ok(FieldInit::with_attributes(
                attributes.into_iter().collect(),
                verum_ast::ty::Ident::new(name, name_span),
                value,
                span,
            ))
        }
    }

    // ========================================================================
    // Compound expressions
    // ========================================================================

    fn parse_paren_or_tuple(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::LParen)?;

        // Empty tuple
        if self.stream.check(&TokenKind::RParen) {
            self.stream.advance();
            let span = self.stream.make_span(start_pos);
            return Ok(Expr::new(ExprKind::Tuple(List::new()), span));
        }

        // Parse first expression
        let first = self.parse_expr()?;

        // Check if it's a tuple or parenthesized expression
        if self.stream.consume(&TokenKind::Comma).is_some() {
            // It's a tuple
            let mut elements = vec![first];

            // Parse remaining elements
            if !self.stream.check(&TokenKind::RParen) {
                let rest = self.comma_separated(|p| p.parse_expr())?;
                elements.extend(rest);
            }

            self.stream.expect(TokenKind::RParen)?;
            let span = self.stream.make_span(start_pos);
            Ok(Expr::new(ExprKind::Tuple(elements.into()), span))
        } else {
            // Parenthesized expression
            self.stream.expect(TokenKind::RParen)?;
            let span = self.stream.make_span(start_pos);
            Ok(Expr::new(ExprKind::Paren(Box::new(first)), span))
        }
    }

    fn parse_array_or_comprehension(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::LBracket)?;

        // Empty array
        if self.stream.check(&TokenKind::RBracket) {
            self.stream.advance();
            let span = self.stream.make_span(start_pos);
            return Ok(Expr::new(
                ExprKind::Array(ArrayExpr::List(List::new())),
                span,
            ));
        }

        // Parse first expression
        let first = self.parse_expr()?;

        // Check what follows
        if self.stream.consume(&TokenKind::For).is_some() {
            // List comprehension
            let clauses = self.parse_comprehension_clauses()?;
            self.stream.expect(TokenKind::RBracket)?;
            let span = self.stream.make_span(start_pos);
            Ok(Expr::new(
                ExprKind::Comprehension {
                    expr: Box::new(first),
                    clauses: clauses.into(),
                },
                span,
            ))
        } else if self.stream.consume(&TokenKind::Semicolon).is_some() {
            // Repeat array: [value; count]
            let count = self.parse_expr()?;
            self.stream.expect(TokenKind::RBracket)?;
            let span = self.stream.make_span(start_pos);
            Ok(Expr::new(
                ExprKind::Array(ArrayExpr::Repeat {
                    value: Box::new(first),
                    count: Box::new(count),
                }),
                span,
            ))
        } else {
            // Array list
            let mut elements = vec![first];

            if self.stream.consume(&TokenKind::Comma).is_some()
                && !self.stream.check(&TokenKind::RBracket)
            {
                let rest = self.comma_separated(|p| p.parse_expr())?;
                elements.extend(rest);
            }

            self.stream.expect(TokenKind::RBracket)?;
            let span = self.stream.make_span(start_pos);
            Ok(Expr::new(ExprKind::Array(ArrayExpr::List(elements.into())), span))
        }
    }

    /// Parse stream expression: comprehension, literal elements, or range.
    /// Stream literals and comprehensions support lazy, potentially infinite sequences.
    /// Grammar: stream_expr = 'stream' , '[' , stream_body , ']' ;
    ///
    /// Syntax variants:
    /// - `stream[]`               -> empty stream
    /// - `stream[expr]`           -> single-element finite stream
    /// - `stream[expr, ...]`      -> single-element infinite cycle
    /// - `stream[a, b, c]`        -> multi-element finite stream
    /// - `stream[a, b, c, ...]`   -> multi-element infinite cycle
    /// - `stream[0, 1, 2, ...]`   -> arithmetic sequence detection
    /// - `stream[start..end]`     -> exclusive range [start, end)
    /// - `stream[start..=end]`    -> inclusive range [start, end]
    /// - `stream[start..]`        -> infinite range from start
    /// - `stream[expr for p in iter ...]` -> comprehension
    fn parse_stream_expr(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::Stream)?;
        self.stream.expect(TokenKind::LBracket)?;

        // Empty stream: stream[]
        if self.stream.check(&TokenKind::RBracket) {
            self.stream.advance();
            let span = self.stream.make_span(start_pos);
            return Ok(Expr::new(
                ExprKind::StreamLiteral(StreamLiteralExpr::elements(List::new(), false, span)),
                span,
            ));
        }

        // Parse first expression
        let first_expr = self.parse_expr()?;

        // Check what follows the first expression
        match self.stream.peek_kind() {
            // Stream comprehension: stream[expr for pattern in iter ...]
            Some(&TokenKind::For) => {
                self.stream.advance(); // consume 'for'
                let clauses = self.parse_comprehension_clauses()?;
                self.stream.expect(TokenKind::RBracket)?;
                let span = self.stream.make_span(start_pos);
                Ok(Expr::new(
                    ExprKind::StreamComprehension {
                        expr: Box::new(first_expr),
                        clauses: clauses.into(),
                    },
                    span,
                ))
            }

            // Range-based stream: stream[start..end] or stream[start..=end] or stream[start..]
            Some(&TokenKind::DotDot) => {
                self.stream.advance(); // consume '..'
                let end = if self.stream.check(&TokenKind::RBracket) {
                    Maybe::None // Infinite range: stream[0..]
                } else {
                    Maybe::Some(self.parse_expr()?)
                };
                self.stream.expect(TokenKind::RBracket)?;
                let span = self.stream.make_span(start_pos);
                Ok(Expr::new(
                    ExprKind::StreamLiteral(StreamLiteralExpr::range(first_expr, end, false, span)),
                    span,
                ))
            }

            // Inclusive range: stream[start..=end]
            Some(&TokenKind::DotDotEq) => {
                self.stream.advance(); // consume '..='
                let end = self.parse_expr()?;
                self.stream.expect(TokenKind::RBracket)?;
                let span = self.stream.make_span(start_pos);
                Ok(Expr::new(
                    ExprKind::StreamLiteral(StreamLiteralExpr::range(first_expr, Maybe::Some(end), true, span)),
                    span,
                ))
            }

            // Single element or comma-separated elements
            Some(&TokenKind::Comma) => {
                let mut elements = vec![first_expr];
                let mut cycles = false;

                while self.stream.consume(&TokenKind::Comma).is_some() {
                    // Check for trailing ellipsis: stream[a, b, ...]
                    if self.stream.check(&TokenKind::DotDotDot) {
                        self.stream.advance();
                        cycles = true;
                        break;
                    }
                    // Check for end of list
                    if self.stream.check(&TokenKind::RBracket) {
                        break;
                    }
                    elements.push(self.parse_expr()?);
                }

                self.stream.expect(TokenKind::RBracket)?;
                let span = self.stream.make_span(start_pos);
                Ok(Expr::new(
                    ExprKind::StreamLiteral(StreamLiteralExpr::elements(elements.into(), cycles, span)),
                    span,
                ))
            }

            // Single element stream: stream[expr]
            Some(&TokenKind::RBracket) => {
                self.stream.advance();
                let span = self.stream.make_span(start_pos);
                Ok(Expr::new(
                    ExprKind::StreamLiteral(StreamLiteralExpr::elements(vec![first_expr].into(), false, span)),
                    span,
                ))
            }

            // Unexpected token
            _ => {
                let span = self.stream.make_span(start_pos);
                Err(ParseError::invalid_syntax(
                    "expected ']', ',', '..', '..=', or 'for' in stream expression",
                    span,
                ))
            }
        }
    }

    /// Parse comprehension clauses until the specified terminator token.
    /// Unified implementation used by both list comprehensions (RBracket)
    /// and brace-delimited comprehensions like set/gen/map (RBrace).
    ///
    /// Grammar:
    /// comprehension_clause = 'for' pattern 'in' expression
    ///                      | 'let' pattern [ ':' type ] '=' expression
    ///                      | 'if' expression
    fn parse_comprehension_clauses_until(
        &mut self,
        terminator: &TokenKind,
    ) -> ParseResult<Vec<ComprehensionClause>> {
        let mut clauses = Vec::new();

        // First clause must be 'for' - the 'for' keyword should already be consumed by the caller
        let pattern = self.parse_pattern()?;
        self.stream.expect(TokenKind::In)?;
        let iter = self.parse_comprehension_expr()?;
        let span = self.stream.make_span(self.stream.position());

        clauses.push(ComprehensionClause {
            kind: ComprehensionClauseKind::For { pattern, iter },
            span,
        });

        // Additional clauses
        while !self.stream.check(terminator) {
            // Safety: prevent infinite loop
            if !self.tick() || self.is_aborted() {
                break;
            }

            let start_pos = self.stream.position();

            if self.stream.consume(&TokenKind::If).is_some() {
                let cond = self.parse_comprehension_expr()?;
                let span = self.stream.make_span(start_pos);
                clauses.push(ComprehensionClause {
                    kind: ComprehensionClauseKind::If(cond),
                    span,
                });
            } else if self.stream.consume(&TokenKind::Let).is_some() {
                let pattern = self.parse_pattern()?;
                let ty = if self.stream.consume(&TokenKind::Colon).is_some() {
                    Maybe::Some(self.parse_type()?)
                } else {
                    Maybe::None
                };
                self.stream.expect(TokenKind::Eq)?;
                let value = self.parse_comprehension_expr()?;
                let span = self.stream.make_span(start_pos);
                clauses.push(ComprehensionClause {
                    kind: ComprehensionClauseKind::Let { pattern, ty, value },
                    span,
                });
            } else if self.stream.consume(&TokenKind::For).is_some() {
                let pattern = self.parse_pattern()?;
                self.stream.expect(TokenKind::In)?;
                let iter = self.parse_comprehension_expr()?;
                let span = self.stream.make_span(start_pos);
                clauses.push(ComprehensionClause {
                    kind: ComprehensionClauseKind::For { pattern, iter },
                    span,
                });
            } else {
                break;
            }
        }

        Ok(clauses)
    }

    /// Parse comprehension clauses for list comprehensions (terminated by `]`).
    fn parse_comprehension_clauses(&mut self) -> ParseResult<Vec<ComprehensionClause>> {
        self.parse_comprehension_clauses_until(&TokenKind::RBracket)
    }

    /// Parse an expression in a comprehension context.
    /// This is similar to parse_expr but stops at comprehension keywords (if, for).
    fn parse_comprehension_expr(&mut self) -> ParseResult<Expr> {
        // Parse with binding power 0.
        // We increment comprehension_depth so that range expressions
        // (e.g. `1..`) do not greedily consume `if`, `for`, or `let`
        // keywords as the range end — those are comprehension clauses.
        self.comprehension_depth += 1;
        let result = self.parse_expr_bp(0);
        self.comprehension_depth -= 1;
        result
    }

    /// Parse set comprehension: set{expr for pattern in iter clause*}
    /// Grammar: 'set' '{' expression 'for' pattern 'in' expression { comprehension_clause } '}'
    fn parse_set_comprehension(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::Set)?;
        self.stream.expect(TokenKind::LBrace)?;

        // Parse the expression to yield
        let expr = self.parse_comprehension_expr()?;

        // Expect 'for' keyword
        self.stream.expect(TokenKind::For)?;

        // Parse comprehension clauses (reuses existing clause parsing but for braces)
        let clauses = self.parse_brace_comprehension_clauses()?;

        self.stream.expect(TokenKind::RBrace)?;
        let span = self.stream.make_span(start_pos);

        Ok(Expr::new(
            ExprKind::SetComprehension {
                expr: Box::new(expr),
                clauses: clauses.into(),
            },
            span,
        ))
    }

    /// Parse generator expression: gen{expr for pattern in iter clause*}
    /// Grammar: 'gen' '{' expression 'for' pattern 'in' expression { comprehension_clause } '}'
    fn parse_generator_expression(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::Gen)?;
        self.stream.expect(TokenKind::LBrace)?;

        // Parse the expression to yield
        let expr = self.parse_comprehension_expr()?;

        // Expect 'for' keyword
        self.stream.expect(TokenKind::For)?;

        // Parse comprehension clauses
        let clauses = self.parse_brace_comprehension_clauses()?;

        self.stream.expect(TokenKind::RBrace)?;
        let span = self.stream.make_span(start_pos);

        Ok(Expr::new(
            ExprKind::GeneratorComprehension {
                expr: Box::new(expr),
                clauses: clauses.into(),
            },
            span,
        ))
    }

    /// Parse comprehension clauses for brace-delimited comprehensions (terminated by `}`).
    /// Used by set comprehensions, generator expressions, and map comprehensions.
    fn parse_brace_comprehension_clauses(&mut self) -> ParseResult<Vec<ComprehensionClause>> {
        self.parse_comprehension_clauses_until(&TokenKind::RBrace)
    }

    /// Parse an expression in if-condition context (after `let pattern =` or as boolean condition).
    /// This avoids consuming:
    /// - `{` (the then-block)
    /// - `&&` (for chaining conditions)
    fn parse_if_condition_value(&mut self) -> ParseResult<Expr> {
        // Parse with BP > && (which has left_bp=5), so we stop before consuming &&
        // Using min_bp=6 means we only parse operators with left_bp >= 6, excluding &&
        self.parse_expr_bp_no_struct(6)
    }

    pub(crate) fn parse_block_expr(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        let block = self.parse_block()?;
        let span = self.stream.make_span(start_pos);
        Ok(Expr::new(ExprKind::Block(block), span))
    }

    /// Parse { } which could be a block, map, or set.
    /// Disambiguation rules:
    /// - { } -> empty map (default)
    /// - { expr : expr ... } -> map
    /// - { expr , expr ... } -> set (no colons)
    /// - { let ... } or { stmt; ... } -> block
    fn parse_brace_expr(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::LBrace)?;

        // Empty braces: default to empty map
        if self.stream.check(&TokenKind::RBrace) {
            self.stream.advance();
            let span = self.stream.make_span(start_pos);
            return Ok(Expr::new(
                ExprKind::MapLiteral {
                    entries: List::new(),
                },
                span,
            ));
        }

        // Lookahead to determine what kind of expression this is
        // We need to check if this looks like:
        // 1. A statement (let, expr;) -> block
        // 2. expr : expr -> map
        // 3. expr , expr -> set

        // If we see 'let', it's definitely a block
        if self.stream.check(&TokenKind::Let) {
            // Reset and parse as block
            self.stream.reset_to(start_pos);
            return self.parse_block_expr();
        }

        // Save position to potentially backtrack
        let lookahead_pos = self.stream.position();

        // Try to parse first expression
        let first_expr_result = self.parse_expr();

        if first_expr_result.is_ok() {
            // Check what follows the expression
            if self.stream.check(&TokenKind::Colon) {
                // It's a map: { expr : expr, ... }
                self.stream.reset_to(start_pos);
                return self.parse_map_literal();
            } else if self.stream.check(&TokenKind::Comma) {
                // It's a set: { expr, expr, ... }
                self.stream.reset_to(start_pos);
                return self.parse_set_literal();
            } else if self.stream.check(&TokenKind::Semicolon) {
                // It's a block: { expr; ... }
                self.stream.reset_to(start_pos);
                return self.parse_block_expr();
            } else if self.stream.check(&TokenKind::RBrace) {
                // Single expression followed by } - this is ambiguous!
                // It could be:
                // 1. A block with trailing expression: { x }
                // 2. A single-element set: { x }
                // We default to block to preserve backward compatibility
                // Users should use explicit Set constructor for single-element sets
                self.stream.reset_to(start_pos);
                return self.parse_block_expr();
            }
        }

        // Default to block if we can't determine
        self.stream.reset_to(start_pos);
        self.parse_block_expr()
    }

    /// Parse a map literal or map comprehension: { key: value, ... } or { k: v for ... }
    /// Grammar: '{' expression ':' expression ('for' pattern 'in' expression ... | (',' expression ':' expression)*) '}'
    fn parse_map_literal(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::LBrace)?;

        let mut entries = Vec::new();

        // Empty map
        if self.stream.check(&TokenKind::RBrace) {
            self.stream.advance();
            let span = self.stream.make_span(start_pos);
            return Ok(Expr::new(
                ExprKind::MapLiteral {
                    entries: List::new(),
                },
                span,
            ));
        }

        // Parse first key-value pair (using comprehension_expr to stop at 'for')
        let key = self.parse_comprehension_expr()?;
        self.stream.expect(TokenKind::Colon)?;
        let value = self.parse_comprehension_expr()?;

        // Check if this is a map comprehension: {k: v for ...}
        if self.stream.check(&TokenKind::For) {
            self.stream.advance(); // consume 'for'
            let clauses = self.parse_brace_comprehension_clauses()?;
            self.stream.expect(TokenKind::RBrace)?;
            let span = self.stream.make_span(start_pos);
            return Ok(Expr::new(
                ExprKind::MapComprehension {
                    key_expr: Box::new(key),
                    value_expr: Box::new(value),
                    clauses: clauses.into(),
                },
                span,
            ));
        }

        // It's a regular map literal - add first entry and continue
        entries.push((key, value));

        // Parse remaining key-value pairs
        while self.stream.consume(&TokenKind::Comma).is_some() {
            // Safety: prevent infinite loop
            if !self.tick() || self.is_aborted() {
                break;
            }

            // Allow trailing comma
            if self.stream.check(&TokenKind::RBrace) {
                break;
            }

            let key = self.parse_expr()?;
            self.stream.expect(TokenKind::Colon)?;
            let value = self.parse_expr()?;
            entries.push((key, value));
        }

        self.stream.expect(TokenKind::RBrace)?;
        let span = self.stream.make_span(start_pos);
        Ok(Expr::new(ExprKind::MapLiteral { entries: entries.into() }, span))
    }

    /// Parse a set literal: { elem1, elem2, ... }
    fn parse_set_literal(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::LBrace)?;

        let mut elements = Vec::new();

        // Empty set (though we default to map for {})
        if self.stream.check(&TokenKind::RBrace) {
            self.stream.advance();
            let span = self.stream.make_span(start_pos);
            return Ok(Expr::new(
                ExprKind::SetLiteral {
                    elements: List::new(),
                },
                span,
            ));
        }

        // Parse elements
        loop {
            // Safety: prevent infinite loop
            if !self.tick() || self.is_aborted() {
                break;
            }

            let elem = self.parse_expr()?;
            elements.push(elem);

            // Check for comma
            if self.stream.consume(&TokenKind::Comma).is_none() {
                break;
            }

            // Allow trailing comma
            if self.stream.check(&TokenKind::RBrace) {
                break;
            }
        }

        self.stream.expect(TokenKind::RBrace)?;
        let span = self.stream.make_span(start_pos);
        Ok(Expr::new(ExprKind::SetLiteral { elements: elements.into() }, span))
    }

    // parse_block is defined in stmt.rs

    /// Parse `|> .method(args)` — pipe-to-method desugaring.
    /// Consumes the `.`, method name, optional generic args, and call args.
    /// Desugars `lhs |> .method(args)` into `lhs.method(args)`.
    fn parse_pipe_method_call(&mut self, lhs: Expr, start_pos: usize) -> ParseResult<Expr> {
        // Consume the '.'
        self.stream.expect(TokenKind::Dot)?;

        // Parse method name
        let method_name = self.consume_ident_or_keyword()?;
        let method_span = self.stream.current_span();

        // Check for generic type arguments: .method<T>()
        let is_generic = self.stream.check(&TokenKind::Lt)
            && self.is_generic_method_call_lookahead();

        let type_args = if is_generic {
            self.parse_generic_args()?
        } else {
            Vec::new()
        };

        // Parse call arguments
        let args = self.parse_call_args()?;
        let span = lhs.span.merge(self.stream.make_span(start_pos));

        Ok(Expr::new(
            ExprKind::MethodCall {
                receiver: Box::new(lhs),
                method: verum_ast::ty::Ident::new(method_name, method_span),
                type_args: type_args.into_iter().collect(),
                args: args.into(),
            },
            span,
        ))
    }

    fn parse_call_args(&mut self) -> ParseResult<Vec<Expr>> {
        let open_paren_span = self.stream.expect(TokenKind::LParen)?.span;

        // E012: Check for immediately unclosed call (foo(;)
        if self.stream.check(&TokenKind::Semicolon) {
            return Err(ParseError::stmt_unclosed_call(
                open_paren_span,
            ).with_help("missing closing ')' for function call"));
        }

        if self.stream.check(&TokenKind::RParen) {
            self.stream.advance();
            return Ok(Vec::new());
        }

        let args = self.comma_separated(|p| p.parse_call_arg())?;

        // E012: Check for unclosed call after args
        if !self.stream.check(&TokenKind::RParen) {
            if self.stream.check(&TokenKind::Semicolon) || self.stream.at_end() {
                return Err(ParseError::stmt_unclosed_call(
                    open_paren_span,
                ).with_help("missing closing ')' for function call"));
            }
        }

        self.stream.expect(TokenKind::RParen)?;

        Ok(args)
    }

    /// Parse a single call argument, which may be either:
    /// - A positional argument: `expr`
    /// - A named argument: `name: expr`
    ///
    /// Named arguments use the syntax `identifier: expression` where the identifier
    /// is followed by a colon (not a double-colon path separator).
    fn parse_call_arg(&mut self) -> ParseResult<Expr> {
        // Check for named argument pattern: Ident followed by Colon (not ColonColon)
        // We use lookahead to avoid consuming tokens prematurely
        if let Some(Token { kind: TokenKind::Ident(_), .. }) = self.stream.peek() {
            if self.stream.peek_nth_kind(1) == Some(&TokenKind::Colon) {
                // This is a named argument: name: value
                let start_pos = self.stream.position();
                let name_str = self.consume_ident()?;
                let name_span = self.stream.current_span();
                self.stream.advance(); // consume ':'
                let value = self.parse_expr()?;
                let span = self.stream.make_span(start_pos);
                return Ok(Expr::new(
                    ExprKind::NamedArg {
                        name: Ident::new(name_str, name_span),
                        value: Heap::new(value),
                    },
                    span,
                ));
            }
        }
        // Otherwise parse as a normal expression
        self.parse_expr()
    }

    // ========================================================================
    // Control flow
    // ========================================================================

    fn parse_if_expr(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::If)?;

        let condition = self.parse_if_condition()?;
        let then_branch = self.parse_block()?;

        let else_branch = if self.stream.consume(&TokenKind::Else).is_some() {
            if self.stream.check(&TokenKind::If) {
                // else if
                Maybe::Some(Box::new(self.parse_if_expr()?))
            } else {
                // else block
                Maybe::Some(Box::new(self.parse_block_expr()?))
            }
        } else {
            Maybe::None
        };

        let span = self.stream.make_span(start_pos);
        Ok(Expr::new(
            ExprKind::If {
                condition: Box::new(condition),
                then_branch,
                else_branch,
            },
            span,
        ))
    }

    fn parse_if_condition(&mut self) -> ParseResult<IfCondition> {
        use verum_ast::smallvec::SmallVec;
        let start_pos = self.stream.position();
        let mut conditions = SmallVec::new();

        loop {
            let pos_before = self.stream.position();

            if self.stream.check(&TokenKind::Let) {
                // If-let pattern: can chain with &&
                self.stream.advance();
                let pattern = self.parse_pattern()?;
                self.stream.expect(TokenKind::Eq)?;
                // Parse expression but stop at && for if-let chains
                // Using min_bp=6 stops before && (left_bp=5) per condition `left_bp < min_bp`
                // This also stops before || (left_bp=4), which is correct per Rust semantics
                let value = self.parse_expr_bp_no_struct(6)?;
                conditions.push(ConditionKind::Let { pattern, value });
            } else {
                // Regular expression condition.
                // `parse_expr_no_struct` naturally stops at `&&` when the token
                // after `&&` is `let` (infix_binding_power returns None for that case),
                // enabling if-let chains like `if expr && let pat = val { }`.
                // When no `let` follows `&&`, the full expression (including `&&`) is parsed.
                let expr = self.parse_expr_no_struct()?;
                conditions.push(ConditionKind::Expr(expr));

                // Safety: Ensure we made forward progress
                if self.stream.position() == pos_before {
                    return Err(ParseError::invalid_syntax(
                        "parser made no progress in if condition",
                        self.stream.current_span(),
                    ));
                }

                // Only continue the chain if the expression stopped at `&&` followed by `let`.
                // For regular conditions (if a && b { }), the expression already consumed `&&`
                // and the stream is NOT at `&&` — we break as before.
                if self.stream.peek_kind() == Some(&TokenKind::AmpersandAmpersand)
                    && self.stream.peek_nth_kind(1) == Some(&TokenKind::Let)
                {
                    self.stream.advance(); // consume &&
                    // Continue loop — next iteration parses the let condition
                    continue;
                }
                break; // Regular condition — no chaining needed
            }

            // Safety: Ensure we made forward progress (for the let branch)
            if self.stream.position() == pos_before {
                return Err(ParseError::invalid_syntax(
                    "parser made no progress in if condition",
                    self.stream.current_span(),
                ));
            }

            // Check for && to chain let-conditions
            if self
                .stream
                .consume(&TokenKind::AmpersandAmpersand)
                .is_none()
            {
                break;
            }
        }

        let span = self.stream.make_span(start_pos);
        Ok(IfCondition { conditions, span })
    }

    fn parse_match_expr(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::Match)?;

        // Use parse_expr_no_struct to prevent `{ }` from being parsed as a record literal
        let scrutinee = self.parse_expr_no_struct()?;

        let brace_span = self.stream.current_span();
        self.stream.expect(TokenKind::LBrace)?;

        // E072: Check for empty match expression (match x { })
        if self.stream.check(&TokenKind::RBrace) {
            return Err(ParseError::invalid_match(
                "match expression must have at least one arm",
                brace_span,
            ));
        }

        let arms = self.parse_match_arms()?;
        self.stream.expect(TokenKind::RBrace)?;

        let span = self.stream.make_span(start_pos);
        Ok(Expr::new(
            ExprKind::Match {
                expr: Box::new(scrutinee),
                arms: arms.into(),
            },
            span,
        ))
    }

    fn parse_match_arms(&mut self) -> ParseResult<Vec<MatchArm>> {
        let mut arms = Vec::new();

        while !self.stream.check(&TokenKind::RBrace) && !self.stream.at_end() {
            // Safety: prevent infinite loop
            if !self.tick() || self.is_aborted() {
                break;
            }

            let start_pos = self.stream.position();

            // E087: Check for empty match arm (=> without pattern)
            if self.stream.check(&TokenKind::FatArrow) {
                return Err(ParseError::pattern_invalid_match_arm(
                    "match arm requires a pattern before '=>'",
                    self.stream.current_span(),
                ));
            }

            // Parse optional attributes: @attr pattern => expr
            let attributes = self.parse_attributes()?;

            let pattern = self.parse_pattern()?;

            let guard = if self.stream.consume(&TokenKind::If).is_some() {
                // E085: Check for missing guard expression (if =>)
                if self.stream.check(&TokenKind::FatArrow) {
                    return Err(ParseError::pattern_missing_guard(
                        "expected expression after 'if' in guard",
                        self.stream.current_span(),
                    ));
                }
                // Allow let-chains in guards: `if let Ok(y) = parse(x) && y > 0`
                // Desugar `let Pattern = expr` to `expr is Pattern` (Is expression)
                if self.stream.check(&TokenKind::Let) {
                    let let_start = self.stream.position();
                    self.stream.advance(); // consume 'let'
                    let pat = self.parse_pattern()?;
                    self.stream.expect(TokenKind::Eq)?;
                    let value = self.parse_expr_bp(4)?;
                    let let_span = self.stream.make_span(let_start);

                    // Desugar to: value is Pattern
                    let is_expr = Expr::new(
                        ExprKind::Is {
                            expr: Box::new(value),
                            pattern: pat,
                            negated: false,
                        },
                        let_span,
                    );

                    // Check for `&&` to chain additional conditions
                    let mut guard_result = is_expr;
                    while self.stream.consume(&TokenKind::AmpersandAmpersand).is_some() {
                        let rhs = self.parse_expr_bp(4)?;
                        let combined_span = guard_result.span.merge(rhs.span);
                        guard_result = Expr::new(
                            ExprKind::Binary {
                                op: verum_ast::BinOp::And,
                                left: Box::new(guard_result),
                                right: Box::new(rhs),
                            },
                            combined_span,
                        );
                    }
                    Maybe::Some(Box::new(guard_result))
                } else {
                // Parse guard expression with min binding power 4 to stop before `=>` (bp=3)
                // This prevents the guard from consuming the match arm separator.
                // Converting errors to E086
                match self.parse_expr_bp(4) {
                    Ok(expr) => Maybe::Some(Box::new(expr)),
                    Err(_) => {
                        return Err(ParseError::pattern_invalid_guard(
                            "invalid guard expression",
                            self.stream.current_span(),
                        ));
                    }
                }
                }
            } else if self.stream.consume(&TokenKind::Where).is_some() {
                // Support Verum-style WHERE guards: match x { y where y > 0 => ... }
                // Match arm guards: `if expr` or `where expr` (Verum supports both forms)
                // E085: Check for missing guard expression (where =>)
                if self.stream.check(&TokenKind::FatArrow) {
                    return Err(ParseError::pattern_missing_guard(
                        "expected expression after 'where' in guard",
                        self.stream.current_span(),
                    ));
                }
                // Parse guard expression with min binding power 4 to stop before `=>` (bp=3)
                match self.parse_expr_bp(4) {
                    Ok(expr) => Maybe::Some(Box::new(expr)),
                    Err(_) => {
                        return Err(ParseError::pattern_invalid_guard(
                            "invalid guard expression",
                            self.stream.current_span(),
                        ));
                    }
                }
            } else {
                Maybe::None
            };

            // E087: Check for using : instead of =>
            if self.stream.check(&TokenKind::Colon) {
                return Err(ParseError::pattern_invalid_match_arm(
                    "use '=>' not ':' in match arms",
                    self.stream.current_span(),
                ));
            }

            // E070: Check for expression-like patterns (binary operators after literal)
            // This catches cases like `1 + 2 =>` where an expression was used instead of a pattern
            if matches!(
                self.stream.peek_kind(),
                Some(TokenKind::Plus)
                    | Some(TokenKind::Minus)
                    | Some(TokenKind::Star)
                    | Some(TokenKind::Slash)
                    | Some(TokenKind::Percent)
                    | Some(TokenKind::Caret)
                    | Some(TokenKind::Lt)
                    | Some(TokenKind::Gt)
                    | Some(TokenKind::LtEq)
                    | Some(TokenKind::GtEq)
                    | Some(TokenKind::EqEq)
                    | Some(TokenKind::BangEq)
                    | Some(TokenKind::AmpersandAmpersand)
                    | Some(TokenKind::PipePipe)
            ) {
                return Err(ParseError::pattern_invalid_at(
                    "expressions cannot be used as patterns; only literals, identifiers, and destructuring patterns are valid",
                    self.stream.current_span(),
                ));
            }

            // E070: Check for method call as pattern (identifier followed by '.')
            if self.stream.check(&TokenKind::Dot) {
                return Err(ParseError::pattern_invalid_at(
                    "method calls cannot be used as patterns",
                    self.stream.current_span(),
                ));
            }

            // E018: Check for range operator after pattern (identifiers can't be range bounds)
            if self.stream.check(&TokenKind::DotDot) || self.stream.check(&TokenKind::DotDotEq) {
                return Err(ParseError::invalid_syntax(
                    "identifiers cannot be used as range bounds in patterns; use integer or character literals",
                    self.stream.current_span(),
                ));
            }

            // E087: Check for missing =>
            if !self.stream.check(&TokenKind::FatArrow) {
                return Err(ParseError::pattern_invalid_match_arm(
                    "expected '=>' after pattern in match arm",
                    self.stream.current_span(),
                ));
            }
            self.stream.advance(); // consume =>

            // E087: Check for double =>
            if self.stream.check(&TokenKind::FatArrow) {
                return Err(ParseError::pattern_invalid_match_arm(
                    "unexpected double '=>' in match arm",
                    self.stream.current_span(),
                ));
            }

            // E087: Check for missing expression after =>
            if self.stream.check(&TokenKind::Comma) || self.stream.check(&TokenKind::RBrace) {
                return Err(ParseError::pattern_invalid_match_arm(
                    "expected expression after '=>' in match arm",
                    self.stream.current_span(),
                ));
            }
            // Match arm body: treat `{ }` as block, not empty map.
            // Also handle `return [expr]` as arm body without block wrapper:
            // `Ok(_) => return,` and `Err(e) => return e,` both work directly.
            let body = if self.stream.check(&TokenKind::LBrace) {
                self.parse_block_expr()?
            } else if self.stream.check(&TokenKind::Return) {
                self.parse_return_expr()?
            } else {
                self.parse_expr()?
            };

            // Optional trailing comma
            self.stream.consume(&TokenKind::Comma);

            let span = self.stream.make_span(start_pos);
            arms.push(MatchArm {
                attributes: attributes.into_iter().collect(),
                pattern,
                guard,
                body: Box::new(body),
                with_clause: Maybe::None,
                span,
            });
        }

        Ok(arms)
    }

    /// Parse a labeled loop: 'label: loop/while/for
    fn parse_labeled_loop(&mut self) -> ParseResult<Expr> {
        // Parse the label
        let label = match self.stream.advance() {
            Some(Token {
                kind: TokenKind::Lifetime(name),
                ..
            }) => Maybe::Some(name.clone()),
            _ => {
                return Err(ParseError::invalid_syntax(
                    "expected lifetime label",
                    self.stream.current_span(),
                ));
            }
        };

        // Expect colon after label
        self.stream.expect(TokenKind::Colon)?;

        // Parse the loop type
        match self.stream.peek_kind() {
            Some(TokenKind::Loop) => self.parse_loop_expr(label),
            Some(TokenKind::While) => self.parse_while_expr(label),
            Some(TokenKind::For) => self.parse_for_expr(label),
            _ => Err(ParseError::invalid_syntax(
                "expected 'loop', 'while', or 'for' after loop label",
                self.stream.current_span(),
            )),
        }
    }

    fn parse_loop_expr(&mut self, label: Maybe<Text>) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::Loop)?;

        // Parse optional loop annotations (invariants only for loop, no decreases)
        let (invariants, _decreases) = self.parse_loop_annotations()?;

        let body = self.parse_block()?;
        let span = self.stream.make_span(start_pos);
        Ok(Expr::new(
            ExprKind::Loop {
                label,
                body,
                invariants,
            },
            span,
        ))
    }

    fn parse_while_expr(&mut self, label: Maybe<Text>) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::While)?;

        // Check for 'while let' pattern syntax
        // Grammar: while_loop = 'while' , expression , { loop_annotation } , block_expr
        // where expression can be 'let pattern = expr' (parsed as match-based bool check)
        let condition = if self.stream.check(&TokenKind::Let) {
            // 'while let pattern = expr' - parse as a synthetic match expression
            // that returns true if pattern matches, false otherwise
            self.stream.advance(); // consume 'let'
            let pattern = self.parse_pattern()?;
            self.stream.expect(TokenKind::Eq)?;
            // Parse the scrutinee expression with appropriate binding power
            let scrutinee = self.parse_expr_bp_no_struct(6)?;
            let cond_span = self.stream.make_span(start_pos);

            // Create an 'is' expression: scrutinee is pattern
            let is_expr = Expr::new(
                ExprKind::Is {
                    expr: Box::new(scrutinee),
                    pattern,
                    negated: false,
                },
                cond_span,
            );

            // Support let-chains: `while let Some(x) = iter.next() && x > 0 { ... }`
            // Also: `while let Some(a) = iter1.next() && let Some(b) = iter2.next() { ... }`
            let mut result = is_expr;
            while self.stream.consume(&TokenKind::AmpersandAmpersand).is_some() {
                let rhs = if self.stream.check(&TokenKind::Let) {
                    // Another let binding in the chain
                    let chain_start = self.stream.position();
                    self.stream.advance(); // consume 'let'
                    let chain_pat = self.parse_pattern()?;
                    self.stream.expect(TokenKind::Eq)?;
                    let chain_val = self.parse_expr_bp_no_struct(6)?;
                    let chain_span = self.stream.make_span(chain_start);
                    Expr::new(
                        ExprKind::Is {
                            expr: Box::new(chain_val),
                            pattern: chain_pat,
                            negated: false,
                        },
                        chain_span,
                    )
                } else {
                    self.parse_expr_bp_no_struct(6)?
                };
                let combined_span = result.span.merge(rhs.span);
                result = Expr::new(
                    ExprKind::Binary {
                        op: verum_ast::BinOp::And,
                        left: Box::new(result),
                        right: Box::new(rhs),
                    },
                    combined_span,
                );
            }
            result
        } else {
            self.parse_expr_no_struct()?
        };

        // Parse optional loop annotations
        let (invariants, decreases) = self.parse_loop_annotations()?;

        let body = self.parse_block()?;
        let span = self.stream.make_span(start_pos);
        Ok(Expr::new(
            ExprKind::While {
                label,
                condition: Box::new(condition),
                body,
                invariants,
                decreases,
            },
            span,
        ))
    }

    fn parse_for_expr(&mut self, label: Maybe<Text>) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::For)?;

        // Check for 'await' keyword to distinguish for-await loop from regular for loop
        // Grammar: for_await_loop = 'for' , 'await' , pattern , 'in' , expression
        //                         , { loop_annotation } , block_expr ;
        let is_for_await = self.stream.check(&TokenKind::Await);
        if is_for_await {
            self.stream.expect(TokenKind::Await)?;
        }

        // E089: Check for missing pattern (for in items)
        if self.stream.check(&TokenKind::In) {
            return Err(ParseError::pattern_empty_or(
                self.stream.current_span(),
            ));
        }

        let pattern = self.parse_pattern()?;

        // E089: Check for guard in for pattern (guards not allowed)
        if self.stream.check(&TokenKind::If) {
            return Err(ParseError::pattern_empty_or(
                self.stream.current_span(),
            ));
        }

        // E089: Check for or-pattern in for
        if self.stream.check(&TokenKind::Pipe) {
            return Err(ParseError::pattern_empty_or(
                self.stream.current_span(),
            ));
        }

        // E089: Check for missing 'in' keyword
        if !self.stream.check(&TokenKind::In) {
            return Err(ParseError::pattern_empty_or(
                self.stream.current_span(),
            ));
        }
        self.stream.advance(); // consume 'in'

        // E089: Check for double 'in' keyword
        if self.stream.check(&TokenKind::In) {
            return Err(ParseError::pattern_empty_or(
                self.stream.current_span(),
            ));
        }
        // Use parse_expr_no_struct to prevent `items { }` from being parsed as a record literal
        let iter = self.parse_expr_no_struct()?;

        // Parse optional loop annotations
        let (invariants, decreases) = self.parse_loop_annotations()?;

        let body = self.parse_block()?;
        let span = self.stream.make_span(start_pos);

        if is_for_await {
            Ok(Expr::new(
                ExprKind::ForAwait {
                    label,
                    pattern,
                    async_iterable: Box::new(iter),
                    body,
                    invariants,
                    decreases,
                },
                span,
            ))
        } else {
            Ok(Expr::new(
                ExprKind::For {
                    label,
                    pattern,
                    iter: Box::new(iter),
                    body,
                    invariants,
                    decreases,
                },
                span,
            ))
        }
    }

    fn parse_try_expr(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::Try)?;
        let try_block = self.parse_block_expr()?;

        // Check for recover or finally keywords
        let has_recover = self.stream.check(&TokenKind::Recover);
        let has_finally = self.stream.check(&TokenKind::Finally);

        if has_recover {
            self.stream.expect(TokenKind::Recover)?;

            // Grammar v2.8+:
            // Supports chained recover clauses:
            //   try { ... } recover Pattern => { body } recover Pattern2 => { body2 }
            // Also supports the existing single-body forms:
            //   recover { match_arms }
            //   recover |e| body
            //   recover ident { body }
            let recover = self.parse_recover_body()?;

            // Check for chained recover clauses: additional `recover` keywords
            // Syntax: `} recover Pattern => { body } recover Pattern2 => { body2 }`
            // Merge all chained recovers into a single RecoverBody::MatchArms
            let recover = if self.stream.check(&TokenKind::Recover) {
                // We have chained recover - convert current recover to match arms
                let mut all_arms = match recover {
                    RecoverBody::MatchArms { arms, .. } => arms.to_vec(),
                    RecoverBody::Closure { param, body, span } => {
                        // Convert closure to a single match arm
                        vec![MatchArm {
                            pattern: param.pattern,
                            guard: Maybe::None,
                            body,
                            with_clause: Maybe::None,
                            attributes: List::new(),
                            span,
                        }]
                    }
                };

                while self.stream.check(&TokenKind::Recover) {
                    self.stream.advance(); // consume 'recover'
                    let chained = self.parse_recover_body()?;
                    match chained {
                        RecoverBody::MatchArms { arms, .. } => {
                            all_arms.extend(arms.to_vec());
                        }
                        RecoverBody::Closure { param, body, span, .. } => {
                            all_arms.push(MatchArm {
                                pattern: param.pattern,
                                guard: Maybe::None,
                                body,
                                with_clause: Maybe::None,
                                attributes: List::new(),
                                span,
                            });
                        }
                    }
                }

                let span = self.stream.make_span(start_pos);
                RecoverBody::MatchArms { arms: all_arms.into(), span }
            } else {
                recover
            };

            // Check for finally after recover
            if self.stream.check(&TokenKind::Finally) {
                self.stream.expect(TokenKind::Finally)?;
                let finally_block = self.parse_block_expr()?;
                let span = self.stream.make_span(start_pos);
                Ok(Expr::new(
                    ExprKind::TryRecoverFinally {
                        try_block: Box::new(try_block),
                        recover,
                        finally_block: Box::new(finally_block),
                    },
                    span,
                ))
            } else {
                let span = self.stream.make_span(start_pos);
                Ok(Expr::new(
                    ExprKind::TryRecover {
                        try_block: Box::new(try_block),
                        recover,
                    },
                    span,
                ))
            }
        } else if has_finally {
            self.stream.expect(TokenKind::Finally)?;
            let finally_block = self.parse_block_expr()?;
            let span = self.stream.make_span(start_pos);
            Ok(Expr::new(
                ExprKind::TryFinally {
                    try_block: Box::new(try_block),
                    finally_block: Box::new(finally_block),
                },
                span,
            ))
        } else {
            // Plain try block - creates Result<T, E> from block value
            // Auto-wraps final expression in Ok()
            let span = self.stream.make_span(start_pos);
            Ok(Expr::new(ExprKind::TryBlock(Box::new(try_block)), span))
        }
    }

    /// Parse a recover body - either match arms or closure syntax.
    ///
    /// Grammar v2.8:
    /// ```ebnf
    /// recover_body = recover_match_arms | recover_closure ;
    /// recover_match_arms = '{' , match_arms , '}' ;
    /// recover_closure = closure_params , recover_closure_body ;
    /// recover_closure_body = block_expr | expression ;
    /// ```
    ///
    /// Examples:
    /// - Match arms: `recover { SomeError(msg) => handle(msg), _ => default() }`
    /// - Closure: `recover |e| { handle_error(e) }` or `recover |e| log_error(e)`
    fn parse_recover_body(&mut self) -> ParseResult<RecoverBody> {
        let start_pos = self.stream.position();

        if self.stream.check(&TokenKind::LBrace) {
            // recover_match_arms: { match_arms }
            self.stream.expect(TokenKind::LBrace)?;
            let arms = self.parse_match_arms()?;
            self.stream.expect(TokenKind::RBrace)?;
            let span = self.stream.make_span(start_pos);
            Ok(RecoverBody::MatchArms { arms: arms.into(), span })
        } else if self.stream.check(&TokenKind::Pipe) {
            // recover_closure: |param| body
            self.stream.expect(TokenKind::Pipe)?;

            // Parse closure parameter
            let param = self.parse_recover_closure_param()?;

            self.stream.expect(TokenKind::Pipe)?;

            // recover_closure_body: block_expr | expression
            let body = if self.stream.check(&TokenKind::LBrace) {
                self.parse_block_expr()?
            } else {
                self.parse_expr()?
            };

            let span = self.stream.make_span(start_pos);
            Ok(RecoverBody::Closure {
                param,
                body: Box::new(body),
                span,
            })
        } else if self.is_ident() {
            // recover ident { body } — shorthand for recover |ident| { body }
            // Also supports recover ident: Type { body }
            // Also supports recover Pattern => { body } — single match arm syntax
            // Parse the identifier as a pattern but don't consume { as struct literal
            let param_start = self.stream.position();
            let pattern = self.parse_pattern_no_struct()?;

            // Check for => (match arm syntax): recover Pattern => { body }
            if self.stream.check(&TokenKind::FatArrow) {
                self.stream.advance(); // consume '=>'
                let body = if self.stream.check(&TokenKind::LBrace) {
                    self.parse_block_expr()?
                } else {
                    self.parse_expr()?
                };
                let arm_span = self.stream.make_span(param_start);
                let arm = MatchArm {
                    pattern,
                    guard: Maybe::None,
                    body: Box::new(body),
                    with_clause: Maybe::None,
                    attributes: List::new(),
                    span: arm_span,
                };
                let span = self.stream.make_span(start_pos);
                return Ok(RecoverBody::MatchArms { arms: List::from(vec![arm]), span });
            }

            let ty = if self.stream.consume(&TokenKind::Colon).is_some() {
                Maybe::Some(self.parse_type()?)
            } else {
                Maybe::None
            };
            let param_span = self.stream.make_span(param_start);
            let param = RecoverClosureParam::new(pattern, ty, param_span);

            let body = self.parse_block_expr()?;

            let span = self.stream.make_span(start_pos);
            Ok(RecoverBody::Closure {
                param,
                body: Box::new(body),
                span,
            })
        } else {
            Err(ParseError::invalid_syntax(
                "expected '{' for match arms, '|' for closure, or identifier in recover block",
                self.stream.current_span(),
            ))
        }
    }

    /// Parse a recover closure parameter.
    ///
    /// Grammar: closure_param (pattern with optional type annotation)
    /// Example: `e` or `e: Error` or `_`
    fn parse_recover_closure_param(&mut self) -> ParseResult<RecoverClosureParam> {
        let start_pos = self.stream.position();

        // Use parse_pattern_no_or to avoid consuming the closing | as an OR pattern separator
        let pattern = self.parse_pattern_no_or()?;

        let ty = if self.stream.consume(&TokenKind::Colon).is_some() {
            Maybe::Some(self.parse_type()?)
        } else {
            Maybe::None
        };

        let span = self.stream.make_span(start_pos);
        Ok(RecoverClosureParam::new(pattern, ty, span))
    }

    /// Parse a single quantifier binding with optional type, domain, and guard.
    ///
    /// Grammar (verum.ebnf v2.12):
    /// ```ebnf
    /// quantifier_binding = pattern , [ ':' , type_expr ] , [ 'in' , expression ] , [ 'where' , expression ] ;
    /// ```
    ///
    /// Examples:
    /// - `x: Int` - typed binding
    /// - `x in list` - domain binding (type inferred from collection element type)
    /// - `x: Int in range` - typed with domain
    /// - `x: Int where x > 0` - typed with guard
    /// - `x in list where x > 0` - domain with guard
    ///
    /// Quantifier bindings support pattern, optional type, optional domain, and optional guard.
    fn parse_quantifier_binding(&mut self, delimited: bool) -> ParseResult<verum_ast::expr::QuantifierBinding> {
        use crate::error::ParseErrorKind;
        let start_pos = self.stream.position();
        let pattern = self.parse_pattern()?;

        // Optional type annotation: `: Type`
        let ty = if self.stream.consume(&TokenKind::Colon).is_some() {
            // Check for EOF/unclosed after colon
            if self.stream.at_end() || self.stream.check(&TokenKind::RBrace) {
                let span = self.stream.make_span(start_pos);
                return Err(ParseError::new(
                    ParseErrorKind::InvalidSyntax {
                        message: "unexpected end of quantifier binding after ':'".into(),
                    },
                    span,
                ));
            }
            // Use restricted type parsing when not delimited (avoid consuming `.` or `,`)
            let parsed_ty = if delimited {
                self.parse_type()?
            } else {
                self.parse_type_for_quantifier()?
            };
            Maybe::Some(parsed_ty)
        } else {
            Maybe::None
        };

        // Optional domain: `in expression`
        let domain = if self.stream.consume(&TokenKind::In).is_some() {
            // Parse the domain expression with restricted parsing to avoid consuming
            // separator tokens like `.`, `,`, `where`
            let domain_expr = self.parse_quantifier_domain_expr()?;
            Maybe::Some(domain_expr)
        } else {
            Maybe::None
        };

        // Optional guard: `where expression`
        let guard = if self.stream.consume(&TokenKind::Where).is_some() {
            // Parse the guard expression - stops before `.` or `,`
            let guard_expr = self.parse_quantifier_guard_expr()?;
            Maybe::Some(guard_expr)
        } else {
            Maybe::None
        };

        // Allow bindings without type or domain when they share a type with the next binding.
        // Example: `forall a, b: Type.` - `a` has no type but shares it with `b`.
        // The type checker will resolve shared type annotations.

        let span = self.stream.make_span(start_pos);
        Ok(verum_ast::expr::QuantifierBinding::full(pattern, ty, domain, guard, span))
    }

    /// Parse a primary expression with optional prefix operators, but WITHOUT postfix operators.
    ///
    /// This is used by quantifier parsing where we need to handle postfix operators (especially `.`)
    /// manually to distinguish between field access and body separator.
    ///
    /// Order: prefix* primary (no postfix)
    fn parse_primary_with_prefix_only(&mut self) -> ParseResult<Expr> {
        // Collect prefix unary operators
        let mut unary_stack: Vec<(UnOp, usize)> = Vec::new();

        loop {
            let op_start_pos = self.stream.position();

            let op = match self.stream.peek_kind() {
                Some(TokenKind::Bang) => {
                    self.stream.advance();
                    Some(UnOp::Not)
                }
                Some(TokenKind::Minus) => {
                    self.stream.advance();
                    Some(UnOp::Neg)
                }
                Some(TokenKind::Tilde) => {
                    self.stream.advance();
                    Some(UnOp::BitNot)
                }
                Some(TokenKind::Star) => {
                    self.stream.advance();
                    Some(UnOp::Deref)
                }
                Some(TokenKind::Ampersand) => {
                    self.stream.advance();
                    Some(UnOp::Ref)
                }
                _ => None,
            };

            if let Some(unary_op) = op {
                unary_stack.push((unary_op, op_start_pos));
            } else {
                break;
            }
        }

        // Parse the primary expression (no postfix)
        let mut expr = self.parse_primary_expr()?;

        // Apply unary operators bottom-up from the stack
        while let Some((unary_op, op_start_pos)) = unary_stack.pop() {
            let span = self.stream.make_span(op_start_pos);
            expr = Expr::new(
                ExprKind::Unary {
                    op: unary_op,
                    expr: Box::new(expr),
                },
                span,
            );
        }

        Ok(expr)
    }

    /// Parse a domain expression for quantifier binding.
    /// Stops before `,` (next binding), `.` (body separator), `where` (guard clause).
    ///
    /// This parser is tricky because we need to allow method chains like `items.filter(|x| x > 0)`
    /// but stop at the body separator `.` in `forall x in items . P(x)`.
    ///
    /// For domain expressions, we stop at comparison operators because the body typically
    /// starts with a comparison like `x > 0`.
    fn parse_quantifier_domain_expr(&mut self) -> ParseResult<Expr> {
        self.parse_quantifier_restricted_expr(false) // Don't allow comparisons
    }

    /// Parse a guard expression for quantifier binding (stops before `,`, `.`, `=>`).
    ///
    /// For guard expressions, we MUST allow comparisons because the guard IS typically
    /// a comparison expression like `x > 0`.
    fn parse_quantifier_guard_expr(&mut self) -> ParseResult<Expr> {
        self.parse_quantifier_restricted_expr(true) // Allow comparisons
    }

    /// Parse an expression in quantifier context with special handling for `.`.
    ///
    /// Strategy: Parse a primary expression, then handle postfix/infix operators manually,
    /// being careful about `.` which could be either:
    /// 1. Field/method access (continue parsing)
    /// 2. Body separator (stop parsing)
    ///
    /// The `allow_comparisons` parameter controls whether comparison operators are parsed:
    /// - false: Stop at comparisons (for domain expressions where body starts with comparison)
    /// - true: Allow comparisons (for guard expressions which ARE comparisons)
    fn parse_quantifier_restricted_expr(&mut self, allow_comparisons: bool) -> ParseResult<Expr> {
        let start_pos = self.stream.position();

        // Parse the initial primary/prefix expression, but WITHOUT postfix operators.
        // We handle postfix manually in the loop below so we can distinguish between
        // `.field` member access and `.` body separator.
        let mut lhs = self.parse_primary_with_prefix_only()?;

        // Now handle postfix and infix operators, with special care for `.`
        loop {
            if !self.tick() || self.is_aborted() {
                break;
            }

            match self.stream.peek_kind() {
                // Stop at quantifier delimiters
                Some(TokenKind::Comma) => break,      // Next binding
                Some(TokenKind::FatArrow) => break,   // Alternative body separator
                Some(TokenKind::Where) => break,      // Guard clause

                // Special handling for `.` - could be body separator or member access
                Some(TokenKind::Dot) => {
                    // Look ahead to see what follows `.`
                    // If `.` is followed by identifier then `(`, it's a method call
                    // If `.` is followed by identifier that isn't followed by `(`,
                    // check if it could be the body start
                    if self.is_quantifier_body_separator() {
                        break; // This `.` is the body separator
                    }

                    // Otherwise, parse as field/method access
                    let (new_lhs, found) = self.try_parse_postfix(lhs)?;
                    lhs = new_lhs;
                    if !found {
                        break;
                    }
                }

                // Other postfix operators (call, index, etc.)
                Some(TokenKind::LParen) | Some(TokenKind::LBracket) | Some(TokenKind::Question) => {
                    let (new_lhs, found) = self.try_parse_postfix(lhs)?;
                    lhs = new_lhs;
                    if !found {
                        break;
                    }
                }

                // Infix operators
                Some(kind) => {
                    // Get binding power
                    let (left_bp, _right_bp) = match self.infix_binding_power(kind) {
                        Some(bp) => bp,
                        None => break, // Not an infix operator
                    };

                    // Comparison operators have precedence around 6
                    // If we don't allow comparisons (domain parsing), stop at level < 8
                    // If we do allow comparisons (guard parsing), only stop at logical AND/OR level (< 4)
                    let min_precedence = if allow_comparisons { 4 } else { 8 };
                    if left_bp < min_precedence {
                        break;
                    }

                    // Parse the binary operator
                    let op_kind = kind.clone();
                    self.stream.advance();

                    // Handle range operators specially
                    if op_kind == TokenKind::DotDot || op_kind == TokenKind::DotDotEq {
                        let inclusive = op_kind == TokenKind::DotDotEq;
                        let end = if self.stream.check(&TokenKind::Comma)
                            || self.stream.check(&TokenKind::Dot)
                            || self.stream.check(&TokenKind::Where)
                            || self.stream.check(&TokenKind::FatArrow)
                            || self.stream.at_end()
                        {
                            Maybe::None
                        } else {
                            Maybe::Some(Box::new(self.parse_quantifier_restricted_expr(allow_comparisons)?))
                        };
                        let span = lhs.span.merge(self.stream.make_span(start_pos));
                        lhs = Expr::new(
                            ExprKind::Range {
                                start: Maybe::Some(Box::new(lhs)),
                                end,
                                inclusive,
                            },
                            span,
                        );
                        continue;
                    }

                    // Standard binary operator
                    let bin_op = self.token_to_binop(&op_kind)?;
                    let rhs = self.parse_quantifier_restricted_expr(allow_comparisons)?;
                    let span = lhs.span.merge(rhs.span);
                    lhs = Expr::new(
                        ExprKind::Binary {
                            op: bin_op,
                            left: Box::new(lhs),
                            right: Box::new(rhs),
                        },
                        span,
                    );
                }

                None => break,
            }
        }

        Ok(lhs)
    }

    /// Check if the current `.` token is the quantifier body separator.
    /// Returns true if we should stop parsing and treat `.` as body separator.
    fn is_quantifier_body_separator(&self) -> bool {
        // The `.` must be at current position
        if !self.stream.check(&TokenKind::Dot) {
            return false;
        }

        // Look at what follows `.`
        match self.stream.peek_nth_kind(1) {
            // `.identifier` - could be field access or body start
            Some(TokenKind::Ident(_)) => {
                // If identifier is followed by `(`, it COULD be a method call OR a function
                // call in the body. We need to look further ahead to distinguish them.
                //
                // For `forall n in items . foo(n)`:
                //   - `.foo(n)` followed by `;` means it's the body (ends the statement)
                //   - `.foo(n)` followed by `.` means it's part of a method chain
                //
                // Heuristic: scan ahead to find the matching `)` and check what follows
                if matches!(self.stream.peek_nth_kind(2), Some(TokenKind::LParen)) {
                    // Find matching `)` after the `(` at position 2
                    if let Some(after_call) = self.peek_after_balanced_parens(3) {
                        // If the call is followed by:
                        // - `;` : it's the body expression ending the statement
                        // - `}` : it's the body expression in a block
                        // - `)` : it's the body expression in a group
                        // - `||`, `&&`: it's the body expression with logical operators
                        // - comparison operators: it's the body expression
                        match after_call {
                            TokenKind::Semicolon | TokenKind::RBrace | TokenKind::RParen |
                            TokenKind::PipePipe | TokenKind::AmpersandAmpersand |
                            TokenKind::EqEq | TokenKind::BangEq |
                            TokenKind::Lt | TokenKind::LtEq |
                            TokenKind::Gt | TokenKind::GtEq => {
                                return true; // Body separator
                            }
                            // If followed by `.` it's likely a method chain continuation
                            TokenKind::Dot => {
                                return false; // Method chain
                            }
                            // For other cases, default to method call (backwards compatible)
                            _ => {
                                return false;
                            }
                        }
                    }
                    // Couldn't find matching paren, default to method call
                    return false;
                }

                // If identifier is followed by `.`, could be chained field access
                // Check if it looks like expression continuation
                match self.stream.peek_nth_kind(2) {
                    // `.x.y` - chained field access, likely body
                    Some(TokenKind::Dot) => true,

                    // `.x + y` - expression with operator, likely body
                    Some(TokenKind::Plus) | Some(TokenKind::Minus) |
                    Some(TokenKind::Star) | Some(TokenKind::Slash) |
                    Some(TokenKind::Percent) | Some(TokenKind::Caret) |
                    Some(TokenKind::Ampersand) | Some(TokenKind::Pipe) |
                    Some(TokenKind::AmpersandAmpersand) | Some(TokenKind::PipePipe) |
                    Some(TokenKind::Eq) | Some(TokenKind::EqEq) | Some(TokenKind::BangEq) |
                    Some(TokenKind::Lt) | Some(TokenKind::LtEq) |
                    Some(TokenKind::Gt) | Some(TokenKind::GtEq) => true,

                    // `.x == y` - comparison, likely body
                    // But `.x,` or `.x where` could be field access followed by delimiter
                    Some(TokenKind::Comma) | Some(TokenKind::Where) |
                    Some(TokenKind::FatArrow) => false,

                    // `.x` at end - could be body with just identifier
                    None => true,

                    // Other cases - assume body
                    _ => true,
                }
            }

            // `.(`, `.[` - likely expression start with grouping
            Some(TokenKind::LParen) | Some(TokenKind::LBracket) => true,

            // `.!x` - negation, likely body
            Some(TokenKind::Bang) => true,

            // `.forall`, `.exists` - nested quantifier, likely body
            Some(TokenKind::Forall) | Some(TokenKind::Exists) => true,

            // `.true`, `.false` - literal, likely body
            Some(TokenKind::True) | Some(TokenKind::False) => true,

            // Numeric literal - likely body
            Some(TokenKind::Integer(_)) | Some(TokenKind::Float(_)) => true,

            // Other - not a separator (or invalid)
            _ => false,
        }
    }

    /// Look ahead to find the token kind that appears after balanced parentheses.
    ///
    /// Starting from position `start`, scans forward to find the matching `)` for an
    /// opening `(` (assumed to be just before `start`), then returns the token kind
    /// that follows.
    ///
    /// Returns `None` if:
    /// - The parentheses aren't balanced within the lookahead limit
    /// - We reach end of input before finding the closing paren
    fn peek_after_balanced_parens(&self, start: usize) -> Option<TokenKind> {
        let mut depth = 1; // We start after an opening `(`
        let mut pos = start;
        let lookahead_limit = 50; // Don't scan too far ahead

        while depth > 0 && pos - start < lookahead_limit {
            match self.stream.peek_nth_kind(pos) {
                Some(TokenKind::LParen) | Some(TokenKind::LBracket) | Some(TokenKind::LBrace) => {
                    depth += 1;
                }
                Some(TokenKind::RParen) | Some(TokenKind::RBracket) | Some(TokenKind::RBrace) => {
                    depth -= 1;
                }
                None => return None, // End of input
                _ => {}
            }
            pos += 1;
        }

        if depth == 0 {
            // Found the matching `)`, return what comes after
            self.stream.peek_nth_kind(pos).cloned()
        } else {
            None
        }
    }

    /// Parse one or more quantifier bindings separated by commas.
    ///
    /// Returns the list of bindings.
    fn parse_quantifier_bindings(&mut self) -> ParseResult<List<verum_ast::expr::QuantifierBinding>> {
        let mut bindings = List::new();

        // Check for parenthesized bindings: (x: T, y: U)
        // vs tuple pattern: (a, b): T
        //
        // Heuristic: After `(`, if we see `identifier :` it's a binding delimiter.
        // If we see `identifier ,` or `identifier )`, it could be a tuple pattern
        // (the `:` type comes after the closing `)`).
        let has_outer_paren = if self.stream.check(&TokenKind::LParen) {
            self.is_parenthesized_bindings()
        } else {
            false
        };

        if has_outer_paren {
            self.stream.advance(); // consume the (
        }

        // Parse first binding
        let first_binding = self.parse_quantifier_binding(has_outer_paren)?;
        bindings.push(first_binding);

        // Parse additional bindings separated by comma
        while self.stream.consume(&TokenKind::Comma).is_some() {
            let binding = self.parse_quantifier_binding(has_outer_paren)?;
            bindings.push(binding);
        }

        if has_outer_paren {
            self.stream.expect(TokenKind::RParen)?;
        }

        Ok(bindings)
    }

    /// Check if the current `(` starts parenthesized bindings vs a tuple pattern.
    ///
    /// Parenthesized bindings: `(x: Int, y: Int)` - each binding has type annotation
    /// Tuple pattern: `(a, b): (Int, Int)` - pattern followed by type annotation
    ///
    /// Returns true if this looks like parenthesized bindings.
    fn is_parenthesized_bindings(&self) -> bool {
        // Must be at `(`
        if !self.stream.check(&TokenKind::LParen) {
            return false;
        }

        // Look ahead: ( identifier : => bindings
        //             ( identifier , or ( identifier ) => pattern
        match (self.stream.peek_nth_kind(1), self.stream.peek_nth_kind(2)) {
            (Some(TokenKind::Ident(_)), Some(TokenKind::Colon)) => true,
            (Some(TokenKind::Ident(_)), Some(TokenKind::In)) => true, // (x in items, y in items)
            _ => false,
        }
    }

    /// Parse a universal quantifier expression: `forall x: T. body` or `forall x in S. body`
    ///
    /// Grammar (verum.ebnf v2.12):
    /// ```ebnf
    /// forall_expr = 'forall' , quantifier_binding , { ',' , quantifier_binding } , '.' , expression ;
    /// quantifier_binding = pattern , [ ':' , type_expr ] , [ 'in' , expression ] , [ 'where' , expression ] ;
    /// ```
    ///
    /// Supported syntax variants:
    /// - `forall x: Int. P(x)` - type-annotated
    /// - `forall x in collection. P(x)` - domain-based (type inferred from collection)
    /// - `forall x: Int in range. P(x)` - both type and domain
    /// - `forall x: Int where x > 0. P(x)` - with guard
    /// - `forall x: Int, y: Int. P(x, y)` - multiple bindings
    ///
    /// Universal quantifier: `forall bindings . body` or `forall bindings => body`
    /// Used in verification contracts and formal proofs.
    fn parse_forall_expr(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::Forall)?;

        // Parse quantifier bindings (supports multiple and parenthesized)
        let bindings = self.parse_quantifier_bindings()?;

        // Separator: either `.` (formal) or `=>` (arrow syntax for backwards compatibility)
        if self.stream.consume(&TokenKind::Dot).is_none() {
            self.stream.expect(TokenKind::FatArrow)?;
        }

        // Parse body expression - use parse_expr_no_struct to prevent `ident { ... }` from being
        // consumed as a struct literal when the forall appears in a context like:
        //   ensures forall i: Int . ... || i <= n
        //   { proof body }
        // The `n { ... }` would otherwise be parsed as a struct literal.
        let body = self.parse_expr_no_struct()?;

        let span = self.stream.make_span(start_pos);

        Ok(Expr::new(
            ExprKind::Forall {
                bindings,
                body: Heap::new(body),
            },
            span,
        ))
    }

    /// Parse an existential quantifier expression: `exists x: T. body` or `exists x in S. body`
    ///
    /// Grammar (verum.ebnf v2.12):
    /// ```ebnf
    /// exists_expr = 'exists' , quantifier_binding , { ',' , quantifier_binding } , '.' , expression ;
    /// quantifier_binding = pattern , [ ':' , type_expr ] , [ 'in' , expression ] , [ 'where' , expression ] ;
    /// ```
    ///
    /// Supported syntax variants:
    /// - `exists x: Int. P(x)` - type-annotated
    /// - `exists x in collection. P(x)` - domain-based (type inferred from collection)
    /// - `exists x: Int in range. P(x)` - both type and domain
    /// - `exists x: Int where x > 0. P(x)` - with guard
    /// - `exists x: Int, y: Int. P(x, y)` - multiple bindings
    ///
    /// Existential quantifier: `exists bindings . body` or `exists bindings => body`
    /// Used in verification contracts and formal proofs.
    fn parse_exists_expr(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::Exists)?;

        // Parse quantifier bindings (supports multiple and parenthesized)
        let bindings = self.parse_quantifier_bindings()?;

        // Separator: either `.` (formal) or `=>` (arrow syntax for backwards compatibility)
        if self.stream.consume(&TokenKind::Dot).is_none() {
            self.stream.expect(TokenKind::FatArrow)?;
        }

        // Parse body expression - use parse_expr_no_struct to prevent `ident { ... }` from being
        // consumed as a struct literal when the exists appears in a context like:
        //   ensures exists x: T . ... || x == n
        //   { proof body }
        // The `n { ... }` would otherwise be parsed as a struct literal.
        let body = self.parse_expr_no_struct()?;

        let span = self.stream.make_span(start_pos);

        Ok(Expr::new(
            ExprKind::Exists {
                bindings,
                body: Heap::new(body),
            },
            span,
        ))
    }

    fn check_ident(&self, expected: &str) -> bool {
        matches!(
            self.stream.peek(),
            Some(Token { kind: TokenKind::Ident(name), .. }) if name.as_str() == expected
        )
    }

    /// Check if token at position n is a specific identifier.
    fn check_ident_nth(&self, n: usize, expected: &str) -> bool {
        matches!(
            self.stream.peek_nth(n),
            Some(Token { kind: TokenKind::Ident(name), .. }) if name.as_str() == expected
        )
    }

    fn consume_ident_keyword(&mut self, expected: &str) -> ParseResult<()> {
        match self.stream.peek() {
            Some(Token {
                kind: TokenKind::Ident(name),
                ..
            }) if name.as_str() == expected => {
                self.stream.advance();
                Ok(())
            }
            _ => {
                let span = self.stream.current_span();
                Err(ParseError::invalid_syntax(
                    format!("expected '{}'", expected),
                    span,
                ))
            }
        }
    }

    /// Parse all loop annotations: `{ invariant EXPR | decreases EXPR }*`
    ///
    /// Grammar: loop_annotation = 'invariant' , expression | 'decreases' , expression ;
    /// Returns (invariants, decreases) as two lists.
    fn parse_loop_annotations(&mut self) -> ParseResult<(List<Expr>, List<Expr>)> {
        let mut invariants = Vec::new();
        let mut decreases = Vec::new();

        loop {
            // Handle @ghost attribute on invariants/decreases (skip it)
            if self.stream.check(&TokenKind::At) {
                if matches!(self.stream.peek_nth_kind(1), Some(&TokenKind::Ident(_))) {
                    // Check if the token after @ident is invariant or decreases
                    if matches!(self.stream.peek_nth_kind(2), Some(&TokenKind::Invariant) | Some(&TokenKind::Decreases)) {
                        self.stream.advance(); // consume @
                        self.stream.advance(); // consume ghost/ident
                        // Fall through to normal invariant/decreases parsing
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }

            if self.stream.consume(&TokenKind::Invariant).is_some() {
                let expr = self.parse_expr_no_struct()?;
                invariants.push(expr);
            } else if self.stream.consume(&TokenKind::Decreases).is_some() {
                let expr = self.parse_expr_no_struct()?;
                decreases.push(expr);
            } else {
                break;
            }
        }

        Ok((invariants.into(), decreases.into()))
    }

    // ========================================================================
    // Closures and async
    // ========================================================================

    fn parse_closure_expr(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();

        let mut is_async = self.stream.consume(&TokenKind::Async).is_some();
        let mut is_move = self.stream.consume(&TokenKind::Move).is_some();

        // Handle || (empty closure) vs |params|
        let params = if self.stream.consume(&TokenKind::PipePipe).is_some() {
            // Empty closure: || expr
            Vec::new()
        } else {
            // Regular closure: |params| expr
            self.stream.expect(TokenKind::Pipe)?;

            let params = if self.stream.check(&TokenKind::Pipe) {
                Vec::new()
            } else {
                self.comma_separated(|p| p.parse_closure_param())?
            };

            self.stream.expect(TokenKind::Pipe)?;
            params
        };

        // Check for async/move AFTER parameters: |e| async move { ... }
        // This is in addition to the check before parameters
        if !is_async && self.stream.consume(&TokenKind::Async).is_some() {
            is_async = true;
        }
        if !is_move && self.stream.consume(&TokenKind::Move).is_some() {
            is_move = true;
        }

        // Parse optional context clause: using [Context1, Context2]
        let contexts = if self.stream.consume(&TokenKind::Using).is_some() {
            self.stream.expect(TokenKind::LBracket)?;

            let mut ctx_list = Vec::new();
            if !self.stream.check(&TokenKind::RBracket) {
                loop {
                    // Safety: prevent infinite loop
                    if !self.tick() || self.is_aborted() {
                        break;
                    }

                    let ctx_start = self.stream.position();
                    let path = self.parse_path()?;
                    let ctx_span = self.stream.make_span(ctx_start);
                    ctx_list.push(verum_ast::decl::ContextRequirement::simple(
                        path,
                        List::new(),
                        ctx_span,
                    ));

                    if self.stream.consume(&TokenKind::Comma).is_none() {
                        break;
                    }
                }
            }
            self.stream.expect(TokenKind::RBracket)?;
            List::from(ctx_list)
        } else {
            List::new()
        };

        let return_type = if self.stream.consume(&TokenKind::RArrow).is_some() {
            // Parse return type without refinements to avoid consuming the closure body block
            Maybe::Some(self.parse_type_no_refinement()?)
        } else {
            Maybe::None
        };

        // Also allow 'using' AFTER '-> ReturnType' for consistency with function syntax:
        // || -> Bool using [Logger] { ... }
        let contexts = if contexts.is_empty() && self.stream.consume(&TokenKind::Using).is_some() {
            self.stream.expect(TokenKind::LBracket)?;
            let mut ctx_list = Vec::new();
            if !self.stream.check(&TokenKind::RBracket) {
                loop {
                    if !self.tick() || self.is_aborted() { break; }
                    let ctx_start = self.stream.position();
                    let path = self.parse_path()?;
                    let ctx_span = self.stream.make_span(ctx_start);
                    ctx_list.push(verum_ast::decl::ContextRequirement::simple(path, List::new(), ctx_span));
                    if self.stream.consume(&TokenKind::Comma).is_none() { break; }
                }
            }
            self.stream.expect(TokenKind::RBracket)?;
            List::from(ctx_list)
        } else {
            contexts
        };

        // If there's a return type or contexts, body must be a block (like Rust closures)
        // Otherwise, body can be any expression
        let body = if return_type.is_some() || !contexts.is_empty() {
            if self.stream.check(&TokenKind::LBrace) {
                self.parse_block_expr()?
            } else {
                return Err(ParseError::invalid_syntax(
                    "closures with explicit return types or context annotations require block bodies",
                    self.stream.current_span(),
                ));
            }
        } else {
            // CRITICAL FIX: For closure bodies without explicit return type, treat `{ }` as a block
            // not as a map literal. This ensures `|_| {}` is parsed as a closure returning Unit,
            // not a closure returning an empty Map. Similar to how match arms handle this.
            if self.stream.check(&TokenKind::LBrace) {
                self.parse_block_expr()?
            } else {
                self.parse_expr()?
            }
        };

        let span = self.stream.make_span(start_pos);
        Ok(Expr::new(
            ExprKind::Closure {
                async_: is_async,
                move_: is_move,
                params: params.into(),
                contexts,
                return_type,
                body: Box::new(body),
            },
            span,
        ))
    }

    fn parse_closure_param(&mut self) -> ParseResult<ClosureParam> {
        let start_pos = self.stream.position();
        // Use parse_pattern_no_or to avoid consuming the closing | as an OR pattern separator
        let pattern = self.parse_pattern_no_or()?;

        let ty = if self.stream.consume(&TokenKind::Colon).is_some() {
            Maybe::Some(self.parse_type()?)
        } else {
            Maybe::None
        };

        let span = self.stream.make_span(start_pos);
        Ok(ClosureParam::new(pattern, ty, span))
    }

    /// Parse anonymous function expression: `fn(params) [using [...]] [-> Type] { body }`
    fn parse_anonymous_fn_expr(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::Fn)?;
        if self.stream.check(&TokenKind::Lt) {
            self.stream.advance();
            let mut depth = 1u32;
            while depth > 0 {
                match self.stream.peek_kind() {
                    Some(TokenKind::Lt) => { depth += 1; self.stream.advance(); }
                    Some(TokenKind::Gt) => { depth -= 1; self.stream.advance(); }
                    None => break,
                    _ => { self.stream.advance(); }
                }
            }
        }
        self.stream.expect(TokenKind::LParen)?;
        let mut params = Vec::new();
        if !self.stream.check(&TokenKind::RParen) {
            loop {
                if !self.tick() || self.is_aborted() { break; }
                let param = self.parse_closure_param()?;
                params.push(param);
                if self.stream.consume(&TokenKind::Comma).is_none() { break; }
            }
        }
        self.stream.expect(TokenKind::RParen)?;
        let contexts = if self.stream.consume(&TokenKind::Using).is_some() {
            self.stream.expect(TokenKind::LBracket)?;
            let mut ctx_list = Vec::new();
            if !self.stream.check(&TokenKind::RBracket) {
                loop {
                    if !self.tick() || self.is_aborted() { break; }
                    let ctx_start = self.stream.position();
                    let path = self.parse_path()?;
                    let ctx_span = self.stream.make_span(ctx_start);
                    ctx_list.push(verum_ast::decl::ContextRequirement::simple(
                        path, List::new(), ctx_span,
                    ));
                    if self.stream.consume(&TokenKind::Comma).is_none() { break; }
                }
            }
            self.stream.expect(TokenKind::RBracket)?;
            List::from(ctx_list)
        } else {
            List::new()
        };
        // Anonymous function surface syntax has three forms:
        //   fn(params) { body }             — implicit return type, block body
        //   fn(params) -> Type { body }     — explicit return type, block body
        //   fn(params) -> expr              — no annotation, bare-expression body
        //
        // The third form is ambiguous when `expr` begins with something
        // that looks like a type (a bare identifier, path, or generic).
        // `fn(x: A) -> x` reads as both "return type = x" and "body = x".
        // We resolve the ambiguity by speculatively parsing a type after
        // `->` and committing only if an explicit body block follows. When
        // the speculatively-parsed fragment is NOT followed by `{`, we
        // rewind and re-parse the same tokens as the expression body with
        // no return annotation. This is the shape the closure form
        // (`|x| expr`) already takes, so the two forms stay consistent.
        let (return_type, body) = if self.stream.check(&TokenKind::LBrace) {
            (Maybe::None, self.parse_block_expr()?)
        } else if self.stream.consume(&TokenKind::RArrow).is_some() {
            let after_arrow = self.stream.position();
            let speculative_type = self.parse_type_no_refinement();
            let type_parsed_cleanly = speculative_type.is_ok()
                && self.stream.check(&TokenKind::LBrace);
            if type_parsed_cleanly {
                let ty = speculative_type
                    .expect("ok branch ensured by check above");
                (Maybe::Some(ty), self.parse_block_expr()?)
            } else {
                // Rewind to the first token after `->` and re-parse the
                // tail as a bare-expression body. Operator precedence
                // binds greedily up to the enclosing terminator (`,`,
                // `)`, `}`, `;`), matching the closure-body rule.
                self.stream.reset_to(after_arrow);
                (Maybe::None, self.parse_expr_bp(0)?)
            }
        } else {
            // No `->` and no `{` — fall back to a block parse; the parser
            // will emit its usual "expected `{`" diagnostic if neither
            // is present.
            (Maybe::None, self.parse_block_expr()?)
        };
        let span = self.stream.make_span(start_pos);
        Ok(Expr::new(
            ExprKind::Closure {
                async_: false, move_: false,
                params: params.into(), contexts, return_type,
                body: Box::new(body),
            },
            span,
        ))
    }

    fn parse_async_expr(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::Async)?;
        // Optionally consume 'move' keyword for async move { ... } blocks
        // The move keyword indicates captured variables should be moved, not borrowed
        let _is_move = self.stream.consume(&TokenKind::Move).is_some();
        let block = self.parse_block()?;
        let span = self.stream.make_span(start_pos);
        Ok(Expr::new(ExprKind::Async(block), span))
    }

    /// Parse spawn expression: `spawn expr` or `spawn using [Context] expr`
    /// Grammar: spawn_expr = 'spawn' , [ 'using' , '[' , identifier_list , ']' ] , expression ;
    fn parse_spawn_expr(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::Spawn)?;

        // Parse optional context requirements BEFORE expression: using [Context1, Context2]
        let contexts = if self.stream.check(&TokenKind::Using) {
            self.stream.advance();
            self.stream.expect(TokenKind::LBracket)?;

            let mut ctx_list = Vec::new();
            if !self.stream.check(&TokenKind::RBracket) {
                loop {
                    // Safety: prevent infinite loop
                    if !self.tick() || self.is_aborted() {
                        break;
                    }

                    let ctx_start = self.stream.position();
                    let path = self.parse_path()?;
                    let ctx_span = self.stream.make_span(ctx_start);
                    ctx_list.push(verum_ast::decl::ContextRequirement::simple(
                        path,
                        List::new(),
                        ctx_span,
                    ));

                    if self.stream.consume(&TokenKind::Comma).is_none() {
                        break;
                    }
                }
            }
            self.stream.expect(TokenKind::RBracket)?;
            List::from(ctx_list)
        } else {
            List::new()
        };

        // E048: spawn requires an expression
        if !self.stream.peek().is_some_and(|t| t.starts_expr()) {
            return Err(ParseError::expr_stmt_invalid(
                "expected expression after 'spawn'",
                self.stream.current_span(),
            ));
        }

        // Parse the spawned expression (usually an async block or call)
        let expr = self.parse_expr()?;

        let span = self.stream.make_span(start_pos);
        Ok(Expr::new(
            ExprKind::Spawn {
                expr: Heap::new(expr),
                contexts,
            },
            span,
        ))
    }

    /// Parse a select expression for async multiplexing.
    ///
    /// Grammar: select_expr = 'select' , [ 'biased' ] , '{' , select_arms , '}' ;
    /// select_arm = { attribute } , pattern , '=' , await_expr , [ 'if' , expr ] , '=>' , expr ;
    /// Syntax:
    /// ```verum
    /// select [biased] {
    ///     result = future1.await => expr1,
    ///     result = future2.await => expr2,
    ///     default => default_expr,
    /// }
    /// ```
    ///
    /// Spec: grammar/verum.ebnf - select_expr production
    fn parse_select_expr(&mut self) -> ParseResult<Expr> {
        use verum_ast::expr::SelectArm;

        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::Select)?;

        // Check for optional 'biased' keyword
        let biased = if self.check_ident("biased") {
            self.stream.advance();
            true
        } else {
            false
        };

        let brace_span = self.stream.current_span();
        self.stream.expect(TokenKind::LBrace)?;

        // E094: Check for empty select expression (select { })
        if self.stream.check(&TokenKind::RBrace) {
            return Err(ParseError::invalid_syntax(
                "select expression must have at least one arm",
                brace_span,
            ).with_code(crate::error::ErrorCode::UnclosedSelect));
        }

        let arms = self.parse_select_arms()?;
        self.stream.expect(TokenKind::RBrace)?;

        let span = self.stream.make_span(start_pos);
        Ok(Expr::new(
            ExprKind::Select { biased, arms, span },
            span,
        ))
    }

    /// Parse select arms inside a select expression.
    ///
    /// Each arm is comma-separated and has the form:
    /// - `binding = future.await [if guard] => body` for future arms
    /// - `default => body` for the default arm
    fn parse_select_arms(&mut self) -> ParseResult<List<verum_ast::expr::SelectArm>> {
        use verum_ast::expr::SelectArm;

        let mut arms = List::new();
        while !self.stream.check(&TokenKind::RBrace) && !self.stream.at_end() {
            // Safety: prevent infinite loop
            if !self.tick() || self.is_aborted() {
                break;
            }

            arms.push(self.parse_select_arm()?);

            // Optional trailing comma
            if !self.stream.check(&TokenKind::RBrace) {
                self.stream.consume(&TokenKind::Comma);
            }
        }
        Ok(arms)
    }

    /// Parse a single select arm.
    ///
    /// Four forms (with optional attributes):
    /// 1. `[@attrs] else => body` - the else/fallback arm
    /// 2. `[@attrs] default => body` - alias for else (deprecated)
    /// 3. `[@attrs] pattern = future_expr.await [if guard] => body` - a future arm
    ///
    /// Pattern matching supports full pattern syntax:
    /// - Identifier patterns: `x = future.await => x`
    /// - Enum/variant patterns: `Ok(data) = fetch().await => data`
    /// - Record patterns: `Message.Command { cmd, args } = recv().await => ...`
    ///
    /// Attributes on arms allow optimization hints:
    /// - `@cold` - mark arm as unlikely to be taken
    /// - `@likely` - hint that this arm is frequently taken
    ///
    /// Each select arm: `[attrs] pattern = await_expr [if guard] => body_expr`
    /// Supports `else =>` and `default =>` as fallback arms.
    fn parse_select_arm(&mut self) -> ParseResult<verum_ast::expr::SelectArm> {
        use verum_ast::attr::Attribute;
        use verum_ast::expr::SelectArm;

        let start_pos = self.stream.position();

        // Parse optional attributes: @cold, @likely, etc.
        // Select arms can have attributes like @cold, @likely for optimization hints
        let attributes: List<Attribute> = self.parse_attributes()?.into_iter().collect();

        // Check for 'else' arm (preferred)
        if self.stream.consume(&TokenKind::Else).is_some() {
            self.stream.expect(TokenKind::FatArrow)?;
            let body = self.parse_expr()?;
            let span = self.stream.make_span(start_pos);
            return Ok(SelectArm::else_arm_with_attrs(
                attributes,
                Heap::new(body),
                span,
            ));
        }

        // Check for 'default' arm (deprecated alias for else)
        if self.check_ident("default") {
            self.stream.advance();
            self.stream.expect(TokenKind::FatArrow)?;
            let body = self.parse_expr()?;
            let span = self.stream.make_span(start_pos);
            return Ok(SelectArm::else_arm_with_attrs(
                attributes,
                Heap::new(body),
                span,
            ));
        }

        // Parse: pattern = future_expr.await [if guard] => body
        // Use full pattern parsing for pattern matching support
        let pattern = self.parse_pattern()?;
        self.stream.expect(TokenKind::Eq)?;
        // Parse future expression with min bp=4 to stop before `=>` (bp=3)
        let future = self.parse_expr_bp(4)?;

        // Validate that the future expression ends with .await
        // Select arm future must be an await expression
        // Grammar: grammar/verum.ebnf - await_expr = expression , '.' , 'await'
        if !Self::is_await_expr(&future) {
            return Err(ParseError::invalid_syntax(
                "select arm future must be an await expression (e.g., `x = future.await => ...`)",
                future.span,
            ));
        }

        // Optional guard: if condition
        let guard = if self.stream.consume(&TokenKind::If).is_some() {
            // Parse guard with min bp=4 to stop before `=>` (bp=3)
            Maybe::Some(Heap::new(self.parse_expr_bp(4)?))
        } else {
            Maybe::None
        };

        self.stream.expect(TokenKind::FatArrow)?;
        let body = self.parse_expr()?;

        let span = self.stream.make_span(start_pos);
        Ok(SelectArm::new(
            attributes,
            Maybe::Some(pattern),
            Maybe::Some(Heap::new(future)),
            Heap::new(body),
            guard,
            span,
        ))
    }

    /// Check if an expression is an await expression.
    ///
    /// Used for validating select arm futures.
    /// The expression must end with `.await` to be valid in a select arm.
    fn is_await_expr(expr: &Expr) -> bool {
        matches!(expr.kind, ExprKind::Await(_))
    }

    // =========================================================================
    // Nursery Expression Parsing - Structured Concurrency
    // =========================================================================

    /// Parse nursery expression for structured concurrency.
    ///
    /// Grammar: nursery_expr = 'nursery' , [ nursery_options ] , block_expr , [ nursery_handlers ] ;
    ///
    /// Structured concurrency: all tasks spawned in a nursery must complete before the
    /// nursery scope exits. Supports timeout, error handling, and cancellation.
    ///
    /// # Examples
    ///
    /// ```verum
    /// // Basic nursery
    /// nursery {
    ///     let a = spawn fetch_a();
    ///     let b = spawn fetch_b();
    /// }
    ///
    /// // With timeout
    /// nursery(timeout: 5.seconds) {
    ///     let result = spawn fetch_data();
    /// }
    ///
    /// // With handlers
    /// nursery {
    ///     spawn task();
    /// } on_cancel {
    ///     cleanup();
    /// } recover {
    ///     TimeoutError => default_value,
    /// }
    /// ```
    fn parse_nursery_expr(&mut self) -> ParseResult<Expr> {
        use verum_ast::expr::{NurseryErrorBehavior, NurseryOptions};

        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::Nursery)?;

        // Optional return-type annotation: `nursery<T> { … }` where `T`
        // is the collect-type of the final block value (e.g.
        // `nursery<List<Int>> { collected }`). The generic arguments
        // are parsed and discarded at the AST level — the type checker
        // already derives the result type from the block's tail
        // expression, so this annotation is documentation for the
        // reader. Accepting it keeps VCS anchors like
        // `vbc/async/004_nursery.vr` parseable without requiring
        // every nursery call site to wrap the result in a cast.
        if self.stream.check(&TokenKind::Lt) {
            let _ = self.parse_generic_args()?;
        }

        // Optional explicit-handle form: `nursery |n| { body }` where
        // `n` is bound to the nursery handle inside the block, giving
        // access to `n.on_cleanup(...)`, `n.cancel()`, etc. The
        // handle name is captured as a single-param closure-style
        // binding. The lexer tokenises the surrounding bars as `Pipe`
        // (or `PipePipe` if no params), and the bound ident is
        // stored on the nursery options so codegen can introduce it
        // as a local. VCS spec `vbc/async/004_nursery.vr` uses this
        // throughout.
        if self.stream.check(&TokenKind::Pipe) {
            self.stream.advance();
            let _handle = self.consume_ident()?;
            self.stream.expect(TokenKind::Pipe)?;
        } else if self.stream.check(&TokenKind::PipePipe) {
            // `nursery || { … }` — empty-handle form, equivalent to
            // no handle. Consume and continue.
            self.stream.advance();
        }

        // Parse optional options: nursery(timeout: 5.seconds, on_error: cancel_all)
        let options = if self.stream.consume(&TokenKind::LParen).is_some() {
            self.parse_nursery_options()?
        } else {
            NurseryOptions::new()
        };

        // Optional `@attr[(args)]` nursery modifier — `nursery @timeout(100.ms) { … }`
        // or `nursery @deadline(...) { … }`. Parsed and discarded for
        // now; the tactic-hint slot for nurseries is tracked in the
        // AST as `NurseryOptions` but the attribute form is a more
        // readable sugar used throughout `vbc/async/004_nursery.vr`.
        while self.stream.check(&TokenKind::At) {
            self.stream.advance(); // consume `@`
            if self.is_ident() {
                let _ = self.consume_ident_or_any_keyword();
            }
            if self.stream.check(&TokenKind::LParen) {
                let mut depth = 0_i32;
                loop {
                    if !self.tick() || self.is_aborted() { break; }
                    match self.stream.peek_kind() {
                        Some(TokenKind::LParen) => { depth += 1; self.stream.advance(); }
                        Some(TokenKind::RParen) => {
                            depth -= 1;
                            self.stream.advance();
                            if depth == 0 { break; }
                        }
                        None => break,
                        _ => { self.stream.advance(); }
                    }
                }
            }
        }

        // GRAMMAR: nursery_expr = 'nursery' , [ nursery_options ] , block_expr , [ nursery_handlers ] ;
        // Block expression is REQUIRED after nursery keyword (and optional options).
        if !self.stream.check(&TokenKind::LBrace) {
            return Err(ParseError::missing_block_expr("nursery", self.stream.current_span()));
        }

        // Parse the body block
        let body = self.parse_block()?;

        // Parse optional handlers: on_cancel and/or recover
        let (on_cancel, recover) = self.parse_nursery_handlers()?;

        let span = self.stream.make_span(start_pos);
        Ok(Expr::new(
            ExprKind::Nursery {
                options,
                body,
                on_cancel,
                recover,
                span,
            },
            span,
        ))
    }

    /// Parse nursery options: timeout, on_error, max_tasks
    ///
    /// Grammar: nursery_options = '(' , nursery_option , { ',' , nursery_option } , ')' ;
    fn parse_nursery_options(&mut self) -> ParseResult<verum_ast::expr::NurseryOptions> {
        use verum_ast::expr::{NurseryErrorBehavior, NurseryOptions};

        let start_pos = self.stream.position();
        let mut options = NurseryOptions::new();

        loop {
            // Check for closing paren
            if self.stream.check(&TokenKind::RParen) {
                break;
            }

            // Parse option name (must be identifier)
            let opt_span = self.stream.current_span();
            let opt_name = if let Some(TokenKind::Ident(name)) = self.stream.peek_kind().cloned() {
                self.stream.advance();
                name
            } else {
                return Err(ParseError::invalid_syntax(
                    "expected nursery option name",
                    opt_span,
                ));
            };

            self.stream.expect(TokenKind::Colon)?;

            match opt_name.as_str() {
                "timeout" => {
                    let timeout_expr = self.parse_expr()?;
                    options.timeout = Maybe::Some(Heap::new(timeout_expr));
                }
                "on_error" => {
                    // Parse error behavior: cancel_all | wait_all | fail_fast
                    let behavior_span = self.stream.current_span();
                    let behavior_name = if let Some(TokenKind::Ident(name)) = self.stream.peek_kind().cloned() {
                        self.stream.advance();
                        name
                    } else {
                        return Err(ParseError::invalid_syntax(
                            "expected error behavior: `cancel_all`, `wait_all`, or `fail_fast`",
                            behavior_span,
                        ));
                    };

                    options.on_error = match behavior_name.as_str() {
                        "cancel_all" => NurseryErrorBehavior::CancelAll,
                        "wait_all" => NurseryErrorBehavior::WaitAll,
                        "fail_fast" => NurseryErrorBehavior::FailFast,
                        _ => {
                            return Err(ParseError::invalid_syntax(
                                "expected `cancel_all`, `wait_all`, or `fail_fast`",
                                behavior_span,
                            ));
                        }
                    };
                }
                "max_tasks" => {
                    let max_expr = self.parse_expr()?;
                    options.max_tasks = Maybe::Some(Heap::new(max_expr));
                }
                _ => {
                    return Err(ParseError::invalid_syntax(
                        Text::from(format!(
                            "unknown nursery option `{}`; expected `timeout`, `on_error`, or `max_tasks`",
                            opt_name
                        )),
                        opt_span,
                    ));
                }
            }

            // Consume comma or break
            if self.stream.consume(&TokenKind::Comma).is_none() {
                break;
            }
        }

        self.stream.expect(TokenKind::RParen)?;
        options.span = Maybe::Some(self.stream.make_span(start_pos));
        Ok(options)
    }

    /// Parse nursery handlers: on_cancel and/or recover
    ///
    /// Grammar:
    ///   nursery_handlers = nursery_cancel , [ nursery_recover ] | nursery_recover ;
    ///   nursery_cancel = 'on_cancel' , block_expr ;
    ///   nursery_recover = 'recover' , recover_body ;
    fn parse_nursery_handlers(
        &mut self,
    ) -> ParseResult<(Maybe<verum_ast::expr::Block>, Maybe<verum_ast::expr::RecoverBody>)> {
        let mut on_cancel = Maybe::None;
        let mut recover = Maybe::None;

        // Parse on_cancel handler (check for identifier "on_cancel")
        if let Some(TokenKind::Ident(name)) = self.stream.peek_kind() {
            if name.as_str() == "on_cancel" {
                self.stream.advance(); // Consume 'on_cancel'
                let cancel_block = self.parse_block()?;
                on_cancel = Maybe::Some(cancel_block);
            }
        }

        // Parse recover handler
        if self.stream.consume(&TokenKind::Recover).is_some() {
            let recover_body = self.parse_recover_body()?;
            recover = Maybe::Some(recover_body);
        }

        Ok((on_cancel, recover))
    }

    fn parse_unsafe_expr(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::Unsafe)?;
        let block = self.parse_block()?;
        let span = self.stream.make_span(start_pos);
        Ok(Expr::new(ExprKind::Unsafe(block), span))
    }

    fn parse_meta_expr(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::Meta)?;
        let block = self.parse_block()?;
        let span = self.stream.make_span(start_pos);
        Ok(Expr::new(ExprKind::Meta(block), span))
    }

    /// Parse a quote expression: quote { token_tree } or quote(N) { token_tree }
    ///
    /// Quote expressions are used in meta functions to generate code at compile-time.
    /// The optional stage parameter specifies the target stage for N-level staged compilation.
    ///
    /// Syntax:
    /// - `quote { token_tree }` - Generate code for the default stage (N-1)
    /// - `quote(N) { token_tree }` - Generate code for stage N
    ///
    /// Quote captures code as a token tree for staged metaprogramming.
    /// `quote { ... }` targets the default stage (N-1); `quote(N) { ... }` targets stage N.
    fn parse_quote_expr(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::QuoteKeyword)?;

        // Parse optional target stage: (N)
        // We need to distinguish quote(N) { } from quote { } by checking if next is LParen
        // followed by an integer (not a token tree starting with LParen)
        let target_stage = if self.stream.peek_kind() == Some(&TokenKind::LParen)
            && matches!(self.stream.peek_nth_kind(1), Some(TokenKind::Integer(_)))
        {
            self.stream.advance(); // consume (

            let stage = match self.stream.peek_kind() {
                Some(TokenKind::Integer(lit)) => {
                    let value = lit.as_u64().ok_or_else(|| {
                        ParseError::invalid_syntax(
                            "quote stage must be a non-negative integer".to_string(),
                            self.stream.current_span(),
                        )
                    })? as u32;
                    self.stream.advance();
                    value
                }
                _ => {
                    return Err(ParseError::invalid_syntax(
                        "expected stage number in quote(N)".to_string(),
                        self.stream.current_span(),
                    ))
                }
            };

            self.stream.expect(TokenKind::RParen)?;
            Some(stage)
        } else {
            None
        };

        // Expect { to start the token tree
        let brace_span = self.stream.current_span();
        self.stream.expect(TokenKind::LBrace)?;

        // Parse the token tree until }
        let (tokens, _raw) = self.parse_token_tree_full(&TokenKind::RBrace)?;

        // Note: Empty quote blocks are valid per grammar: token_tree = { token_tree_item }
        // (zero or more items). M401 is for invalid syntax INSIDE quote blocks, not empty ones.

        // Consume closing }
        self.stream.expect(TokenKind::RBrace)?;

        let span = self.stream.make_span(start_pos);
        Ok(Expr::new(ExprKind::Quote { target_stage, tokens }, span))
    }

    /// Parse a stage escape expression: $(stage N){ expr }
    ///
    /// Stage escapes are used inside quote blocks to evaluate expressions at a specific
    /// stage level. This enables inserting computed values into generated code.
    ///
    /// Syntax:
    /// - `$(stage N){ expr }` - Evaluate expr at stage N
    ///
    /// Stage escape evaluates an expression at a specific compilation stage within a quote block.
    fn parse_stage_escape_expr(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();

        // Expect $(stage N){ expr }
        self.stream.expect(TokenKind::Dollar)?;
        self.stream.expect(TokenKind::LParen)?;
        self.stream.expect(TokenKind::Stage)?;

        // Parse stage number
        let stage = match self.stream.peek_kind() {
            Some(TokenKind::Integer(lit)) => {
                let value = lit.as_u64().ok_or_else(|| {
                    ParseError::invalid_syntax(
                        "stage escape level must be a non-negative integer".to_string(),
                        self.stream.current_span(),
                    )
                })? as u32;
                self.stream.advance();
                value
            }
            _ => {
                return Err(ParseError::invalid_syntax(
                    "expected stage number in $(stage N)".to_string(),
                    self.stream.current_span(),
                ))
            }
        };

        self.stream.expect(TokenKind::RParen)?;
        self.stream.expect(TokenKind::LBrace)?;

        // Parse the inner expression
        let expr = self.parse_expr()?;

        self.stream.expect(TokenKind::RBrace)?;

        let span = self.stream.make_span(start_pos);
        Ok(Expr::new(
            ExprKind::StageEscape {
                stage,
                expr: Heap::new(expr),
            },
            span,
        ))
    }

    /// Parse a lift expression: lift(expr)
    ///
    /// Lift expressions are syntactic sugar for `$(stage current){ expr }`,
    /// moving a compile-time value into the generated code at the current stage.
    ///
    /// Syntax:
    /// - `lift(expr)` - Lift expr into the current stage
    ///
    /// Lift is sugar for `$(stage current){ expr }` — moves compile-time value into generated code.
    fn parse_lift_expr(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::Lift)?;
        self.stream.expect(TokenKind::LParen)?;

        // Parse the inner expression
        let expr = self.parse_expr()?;

        self.stream.expect(TokenKind::RParen)?;

        let span = self.stream.make_span(start_pos);
        Ok(Expr::new(ExprKind::Lift { expr: Heap::new(expr) }, span))
    }

    /// Parse a meta-function expression: @file, @line, @cfg(cond), @const expr, etc.
    ///
    /// Meta-functions are compile-time intrinsics that provide:
    /// - Source location: @file, @line, @column, @module, @function
    /// - Configuration: @cfg(condition)
    /// - Compile-time evaluation: @const expr
    /// - Token manipulation: @stringify(tokens), @concat(a, b)
    /// - Diagnostics: @error("msg"), @warning("msg")
    ///
    /// Spec: grammar/verum.ebnf Section 2.20.6 - Meta-Level Functions
    fn parse_meta_function_expr(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::At)?;

        // Parse the meta-function name (identifier or keyword following @)
        // Meta-functions can use keywords like @const, @error, etc.
        let name_span = self.stream.current_span();
        let name_str = match self.stream.peek_kind() {
            Some(TokenKind::Ident(s)) => {
                let s = s.clone();
                self.stream.advance();
                s
            }
            // Handle keywords that are valid meta-function names
            Some(TokenKind::Const) => {
                self.stream.advance();
                Text::from("const")
            }
            Some(TokenKind::Module) => {
                self.stream.advance();
                Text::from("module")
            }
            Some(TokenKind::Fn) => {
                self.stream.advance();
                Text::from("function")
            }
            _ => {
                let span = self.stream.current_span();
                return Err(ParseError::invalid_syntax(
                    "expected identifier or keyword after @".to_string(),
                    span,
                ));
            }
        };

        let name = verum_ast::ty::Ident {
            name: name_str.clone(),
            span: name_span,
        };

        // Validate meta-function name against known compile-time @ keywords
        // Spec: grammar/verum.ebnf Section 2.20.6 - Only known meta-functions are allowed
        //
        // CRITICAL: Declaration-only attributes like @intrinsic, @test, @derive
        // must NOT be used as expression-level meta-functions.
        // This is an ERROR, not a warning.
        if is_declaration_only_attribute(name_str.as_str()) {
            // Check if this looks like a function call (has parentheses with arguments)
            let looks_like_call = self.stream.check(&TokenKind::LParen);

            let error_msg = if name_str.as_str() == "intrinsic" {
                format!(
                    "`@intrinsic` cannot be used as a function call. \
                     Use `@intrinsic(\"{}\")` as an attribute on a function declaration with a typed signature instead. \
                     Example: `@intrinsic(\"name\") fn foo(x: Int) -> Int;`",
                    if looks_like_call { "<name>" } else { "name" }
                )
            } else {
                format!(
                    "`@{}` is a declaration attribute, not a meta-function. \
                     It can only be used on declarations (e.g., `@{} fn foo() {{ }}`), \
                     not in expression context.",
                    name_str, name_str
                )
            };

            return Err(ParseError::invalid_syntax(error_msg, name_span));
        }

        if !is_known_meta_function(name_str.as_str()) {
            let mut warning = AttributeValidationWarning::new(
                format!(
                    "unknown meta-function `@{}`; @ prefix is reserved for compile-time constructs",
                    name_str
                ),
                name_span,
            )
            .with_code("E0410");

            // Provide "did you mean?" suggestion if a similar name exists
            if let Some(suggestion) = find_similar_meta_function(name_str.as_str()) {
                warning = warning.with_hint(format!("did you mean `@{}`?", suggestion));
            } else {
                warning = warning.with_hint(
                    "known meta-functions: @const, @cfg, @file, @line, @column, @module, \
                     @function, @error, @warning, @stringify, @concat, @type_name, @type_of, \
                     @fields_of, @variants_of, @is_struct, @is_enum, @is_tuple, @implements"
                        .to_string(),
                );
            }

            self.attr_warnings.push(warning);
        }

        // Parse arguments based on meta-function type:
        // - @const expr: takes the next expression without parentheses
        // - @cfg(cond), @concat(...), etc.: take parenthesized arguments
        // - @file, @line, @column, @module, @function: no arguments
        let args = if name_str.as_str() == "const" {
            // @const takes the next expression without parentheses
            // e.g., @const compute_table()
            if self.stream.check(&TokenKind::LParen) {
                // @const(expr) form also allowed
                self.stream.advance();
                let mut args = List::new();
                if !self.stream.check(&TokenKind::RParen) {
                    loop {
                        let arg = self.parse_expr()?;
                        args.push(arg);
                        if self.stream.check(&TokenKind::Comma) {
                            self.stream.advance();
                        } else {
                            break;
                        }
                    }
                }
                self.stream.expect(TokenKind::RParen)?;
                args
            } else {
                // @const expr - parse the following expression
                let arg = self.parse_expr()?;
                vec![arg].into()
            }
        } else if self.stream.check(&TokenKind::LParen) {
            self.stream.advance(); // consume (
            let mut args = List::new();
            if !self.stream.check(&TokenKind::RParen) {
                loop {
                    let arg = self.parse_expr()?;
                    args.push(arg);
                    if self.stream.check(&TokenKind::Comma) {
                        self.stream.advance();
                    } else {
                        break;
                    }
                }
            }
            self.stream.expect(TokenKind::RParen)?;
            args
        } else if self.stream.check(&TokenKind::LBracket) {
            // @macro[1, 2, 3] - bracket syntax for token tree macros
            self.stream.advance(); // consume [
            let mut args = List::new();
            if !self.stream.check(&TokenKind::RBracket) {
                loop {
                    let arg = self.parse_expr()?;
                    args.push(arg);
                    if self.stream.check(&TokenKind::Comma) {
                        self.stream.advance();
                    } else {
                        break;
                    }
                }
            }
            self.stream.expect(TokenKind::RBracket)?;
            args
        } else if self.stream.check(&TokenKind::LBrace) {
            // @macro { ... } - brace syntax for block macros
            // Parse the body as a block expression
            let block = self.parse_block_expr()?;
            let mut args = List::new();
            args.push(block);
            args
        } else {
            List::new()
        };

        let span = self.stream.make_span(start_pos);
        Ok(Expr::new(ExprKind::MetaFunction { name, args }, span))
    }

    // ========================================================================
    // Control flow keywords
    // ========================================================================

    fn parse_break_expr(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::Break)?;

        // E049: Check for invalid label syntax (e.g., break @invalid;)
        if self.stream.check(&TokenKind::At) {
            return Err(ParseError::control_flow_invalid(
                "invalid label syntax: labels must use 'label format, not @",
                self.stream.current_span(),
            ));
        }

        // Check for optional label: break 'label or break 'label value
        let label = if let Some(TokenKind::Lifetime(name)) = self.stream.peek_kind() {
            let name = name.clone();
            self.stream.advance();
            Maybe::Some(name)
        } else {
            Maybe::None
        };

        // Check for optional value
        let value = if self.stream.at_end()
            || self.stream.check(&TokenKind::Semicolon)
            || self.stream.check(&TokenKind::RBrace)
        {
            Maybe::None
        } else if self.stream.peek().is_some_and(|t| t.starts_expr()) {
            Maybe::Some(Box::new(self.parse_expr()?))
        } else {
            Maybe::None
        };

        let span = self.stream.make_span(start_pos);
        Ok(Expr::new(ExprKind::Break { label, value }, span))
    }

    fn parse_continue_expr(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::Continue)?;

        // E049: Check for invalid label syntax (e.g., continue @invalid;)
        if self.stream.check(&TokenKind::At) {
            return Err(ParseError::control_flow_invalid(
                "invalid label syntax: labels must use 'label format, not @",
                self.stream.current_span(),
            ));
        }

        // Check for optional label: continue 'label
        let label = if let Some(TokenKind::Lifetime(name)) = self.stream.peek_kind() {
            let name = name.clone();
            self.stream.advance();
            Maybe::Some(name)
        } else {
            Maybe::None
        };

        // E049: continue doesn't accept a value (unlike break)
        if !self.stream.at_end()
            && !self.stream.check(&TokenKind::Semicolon)
            && !self.stream.check(&TokenKind::RBrace)
        {
            // Check if there's something that looks like a value after continue
            if self.stream.peek().is_some_and(|t| t.starts_expr()) {
                return Err(ParseError::control_flow_invalid(
                    "continue does not accept a value (use break for breaking with a value)",
                    self.stream.current_span(),
                ));
            }
        }

        let span = self.stream.make_span(start_pos);
        Ok(Expr::new(ExprKind::Continue { label }, span))
    }

    fn parse_return_expr(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::Return)?;

        let value = if self.stream.at_end()
            || self.stream.check(&TokenKind::Semicolon)
            || self.stream.check(&TokenKind::RBrace)
            || self.stream.check(&TokenKind::Comma) // Match arm: `Ok(_) => return,`
        {
            Maybe::None
        } else if self.stream.peek().is_some_and(|t| t.starts_expr()) {
            Maybe::Some(Box::new(self.parse_expr()?))
        } else {
            // E048: Invalid token after return that doesn't start an expression
            return Err(ParseError::expr_stmt_invalid(
                "invalid expression after 'return'",
                self.stream.current_span(),
            ));
        };

        let span = self.stream.make_span(start_pos);
        Ok(Expr::new(ExprKind::Return(value), span))
    }

    /// Parse a throw expression: `throw expr`
    ///
    /// Throws an error value in a function with a `throws` clause.
    /// Unlike `return` which has an optional value, `throw` requires an expression.
    ///
    /// Example:
    /// ```verum
    /// fn validate(s: Text) throws(ValidationError) -> Bool {
    ///     if s.is_empty() {
    ///         throw ValidationError.Empty;
    ///     }
    ///     true
    /// }
    /// ```
    fn parse_throw_expr(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::Throw)?;

        // E048: throw requires an expression
        if !self.stream.peek().is_some_and(|t| t.starts_expr()) {
            return Err(ParseError::expr_stmt_invalid(
                "expected expression after 'throw'",
                self.stream.current_span(),
            ));
        }

        // throw always requires an expression (the error value)
        let value = self.parse_expr()?;
        let span = self.stream.make_span(start_pos);
        Ok(Expr::new(ExprKind::Throw(Box::new(value)), span))
    }

    fn parse_yield_expr(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::Yield)?;

        // E048: yield requires an expression
        if !self.stream.peek().is_some_and(|t| t.starts_expr()) {
            return Err(ParseError::expr_stmt_invalid(
                "expected expression after 'yield'",
                self.stream.current_span(),
            ));
        }

        let value = self.parse_expr()?;
        let span = self.stream.make_span(start_pos);
        Ok(Expr::new(ExprKind::Yield(Box::new(value)), span))
    }

    /// Parse typeof expression - runtime type introspection
    /// `typeof(expr)` returns TypeInfo { id: TypeId, name: Text, kind: TypeKind, protocols: List<Text> }
    ///
    /// Syntax: `typeof(expression)`
    /// Returns: TypeInfo { id: TypeId, name: Text, kind: TypeKind, protocols: List<Text> }
    fn parse_typeof_expr(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::Typeof)?;

        // typeof always requires parentheses
        self.stream.expect(TokenKind::LParen)?;
        let value = self.parse_expr()?;
        self.stream.expect(TokenKind::RParen)?;

        let span = self.stream.make_span(start_pos);
        Ok(Expr::new(ExprKind::Typeof(Box::new(value)), span))
    }

    // ========================================================================
    // Tensor literals
    // ========================================================================

    fn parse_tensor_literal(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::Tensor)?;

        // Parse shape: <d1, d2, ...> or <> for 0D scalar
        self.stream.expect(TokenKind::Lt)?;

        // Check for empty shape (0D scalar tensor)
        // Empty shape tensor<> is valid for 0D scalars (scalar wrapped in tensor type).
        let shape = if self.stream.check(&TokenKind::Gt) || self.pending_gt {
            // Empty angle brackets: tensor<>f32{42.0}
            Vec::new()
        } else {
            // Parse comma-separated dimensions
            self.comma_separated(|p| match p.stream.peek() {
                Some(Token {
                    kind: TokenKind::Integer(lit),
                    ..
                }) => {
                    let val = lit.as_u64().unwrap_or(0);
                    p.stream.advance();
                    Ok(val)
                }
                _ => {
                    let span = p.stream.current_span();
                    Err(ParseError::invalid_syntax(
                        "expected dimension in tensor shape",
                        span,
                    ))
                }
            })?
        };
        self.expect_gt()?;

        // Validate dimensions
        for (idx, &dim) in shape.iter().enumerate() {
            if dim == 0 {
                let span = self.stream.current_span();
                return Err(ParseError::invalid_syntax(
                    format!("tensor dimension {} cannot be zero", idx),
                    span,
                ));
            }
        }

        // Parse element type (without refinement, since { } is for data, not refinement)
        let elem_type = self.parse_type_no_refinement()?;

        // Parse data in braces
        // For 1D tensors: tensor<4>f32{1.0, 2.0, 3.0, 4.0}
        // For 2D tensors: tensor<2,3>f32{{1.0, 2.0, 3.0}, {4.0, 5.0, 6.0}}
        self.stream.expect(TokenKind::LBrace)?;
        let data = self.parse_tensor_data()?;
        self.stream.expect(TokenKind::RBrace)?;

        let span = self.stream.make_span(start_pos);
        Ok(Expr::new(
            ExprKind::TensorLiteral {
                shape: shape.into(),
                elem_type,
                data: Box::new(data),
            },
            span,
        ))
    }

    /// Parse tensor data inside braces
    ///
    /// Handles both flat and nested tensor literals:
    /// - 1D: {1.0, 2.0, 3.0, 4.0}
    /// - 2D: {{1.0, 2.0}, {3.0, 4.0}}
    /// - Broadcast: {1.0} expands to fill shape
    ///
    /// Returns an expression that can be evaluated to the tensor data.
    ///
    /// NOTE: In tensor context, braces create arrays, not sets!
    /// This is different from general Verum syntax where {1, 2, 3} is a set.
    fn parse_tensor_data(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();

        // Empty tensor data
        if self.stream.check(&TokenKind::RBrace) {
            let span = self.stream.make_span(start_pos);
            return Ok(Expr::new(
                ExprKind::Array(ArrayExpr::List(List::new())),
                span,
            ));
        }

        // Parse first element - handle nested braces specially
        let first = self.parse_tensor_element()?;

        // Check if we have more elements (comma-separated)
        if self.stream.consume(&TokenKind::Comma).is_some() {
            // Multiple elements - create array literal
            let mut elements = vec![first];

            // Parse remaining elements if not at closing brace.
            // NOTE: Do NOT use comma_separated() here because it stops when it sees
            // LBrace after a comma (treating it as trailing comma). In tensor context,
            // {row1}, {row2} must continue parsing even when next token is '{'.
            while !self.stream.check(&TokenKind::RBrace) && !self.stream.at_end() {
                if !self.tick() || self.is_aborted() {
                    break;
                }
                elements.push(self.parse_tensor_element()?);
                if self.stream.consume(&TokenKind::Comma).is_none() {
                    break;
                }
            }

            let span = self.stream.make_span(start_pos);
            Ok(Expr::new(ExprKind::Array(ArrayExpr::List(elements.into())), span))
        } else {
            // Single element or scalar - return as-is
            Ok(first)
        }
    }

    /// Parse a single tensor element
    ///
    /// If the element starts with a brace, treat it as nested tensor data (array),
    /// otherwise parse as a normal expression.
    fn parse_tensor_element(&mut self) -> ParseResult<Expr> {
        if self.stream.check(&TokenKind::LBrace) {
            // Nested tensor dimension - parse as tensor data recursively
            let start_pos = self.stream.position();
            self.stream.advance(); // consume '{'
            let data = self.parse_tensor_data()?;
            self.stream.expect(TokenKind::RBrace)?;
            Ok(data)
        } else {
            // Regular expression (scalar value)
            self.parse_expr()
        }
    }

    /// Parse a macro invocation: path!(args) or path![args] or path!{args}
    ///
    /// Spec: grammar/verum.ebnf - meta_call production
    /// Grammar: meta_call = path , '!' , meta_call_args
    ///          meta_call_args = '(' , token_tree , ')' | '[' , token_tree , ']' | '{' , token_tree , '}'
    fn parse_macro_call(&mut self) -> ParseResult<Expr> {
        let start_pos = self.stream.position();

        // Check for Rust macro syntax before parsing.
        //
        // Root fix for Issue #5: when we detect a Rust-style macro call
        // (`assert!(...)`, `println!(...)`, etc.), consume the `ident` token,
        // the `!`, and the delimited args *before* returning the helpful
        // diagnostic. Without this, the parser stays pointed at the same
        // token and the caller's error-recovery loop re-enters
        // `parse_macro_call` at the identical offset, emitting the same
        // diagnostic for every retry and drowning the real rest-of-file
        // errors in duplicates. Advancing past the offending syntax gives
        // the recovery loop fresh ground to stand on and keeps the
        // diagnostic count equal to the number of actual mistakes.
        if let Some(TokenKind::Ident(name)) = self.stream.peek_kind() {
            let name_str = name.clone();
            if let Some(verum_equiv) = rust_macro_to_verum(name_str.as_str()) {
                let span = self.stream.current_span();
                // Advance past `ident`.
                self.stream.advance();
                // Skip the `!` if present (expected by grammar but missing
                // is tolerated — we're already in the error path).
                if self.stream.check(&TokenKind::Bang) {
                    self.stream.advance();
                }
                // Skip the delimited args as a balanced token tree so the
                // parser lands immediately after the macro invocation.
                self.skip_balanced_macro_args();
                return Err(ParseError::rust_macro_syntax_with_equivalent(
                    name_str.as_str(),
                    verum_equiv,
                    span,
                ));
            }
        }

        // Parse the macro path (supports qualified paths like std.println!)
        let path = self.parse_macro_path()?;

        // Expect the ! token
        self.stream.expect(TokenKind::Bang)?;

        // Parse the delimited token tree
        let args = self.parse_macro_args()?;

        let span = self.stream.make_span(start_pos);
        Ok(Expr::new(ExprKind::MacroCall { path, args }, span))
    }

    /// Parse a macro path - supports qualified paths with . separators (e.g., std.println!)
    /// Unlike parse_simple_expr_path, this DOES consume . for path segments.
    fn parse_macro_path(&mut self) -> ParseResult<Path> {
        use verum_ast::ty::PathSegment;
        let start_pos = self.stream.position();

        let mut segments = Vec::new();

        // Parse first segment
        let first_segment = if self.stream.consume(&TokenKind::SelfValue).is_some() {
            PathSegment::SelfValue
        } else if self.stream.consume(&TokenKind::SelfType).is_some() {
            // SelfType (Self) is a type reference, create as Name("Self") not SelfValue
            PathSegment::Name(verum_ast::ty::Ident::new(Text::from("Self"), self.stream.current_span()))
        } else {
            let name = self.consume_ident_or_keyword()?;
            let span = self.stream.current_span();
            PathSegment::Name(verum_ast::ty::Ident::new(name, span))
        };
        segments.push(first_segment);

        // Parse remaining segments with . (stop when we hit !)
        while self.stream.check(&TokenKind::Dot) {
            // Safety: prevent infinite loop
            if !self.tick() || self.is_aborted() {
                break;
            }

            // Peek ahead - if after the . there's an ident followed by !, continue parsing path
            // Otherwise stop (let postfix handle it)
            if let Some(TokenKind::Ident(_)) = self.stream.peek_nth_kind(1) {
                if self.stream.peek_nth_kind(2) == Some(&TokenKind::Bang) {
                    // This is the last segment before ! - consume it
                    self.stream.advance(); // consume .
                    let name = self.consume_ident_or_keyword()?;
                    let span = self.stream.current_span();
                    segments.push(PathSegment::Name(verum_ast::ty::Ident::new(name, span)));
                    break;
                } else if self.stream.peek_nth_kind(2) == Some(&TokenKind::Dot) {
                    // More path segments - continue
                    self.stream.advance(); // consume .
                    let name = self.consume_ident_or_keyword()?;
                    let span = self.stream.current_span();
                    segments.push(PathSegment::Name(verum_ast::ty::Ident::new(name, span)));
                } else {
                    // Not part of macro path - stop
                    break;
                }
            } else {
                break;
            }
        }

        let span = self.stream.make_span(start_pos);
        Ok(Path::new(segments.into_iter().collect::<List<_>>(), span))
    }

    /// Skip a balanced macro-args token tree for error-recovery: if the
    /// stream is currently at `(`, `[`, or `{`, consume tokens up to and
    /// including the matching closer, tracking nested depths of every
    /// paren/bracket/brace combination. If the stream is not at an opener,
    /// this is a no-op. Infallible and never advances past a non-opener.
    ///
    /// Used by the Issue-#5 recovery path in `parse_macro_call` so that a
    /// `assert!(x)` / `println!(...)` diagnostic doesn't leave the parser
    /// pointed back at the same token for the next recovery iteration.
    fn skip_balanced_macro_args(&mut self) {
        let (open, close) = match self.stream.peek_kind() {
            Some(TokenKind::LParen) => (TokenKind::LParen, TokenKind::RParen),
            Some(TokenKind::LBracket) => (TokenKind::LBracket, TokenKind::RBracket),
            Some(TokenKind::LBrace) => (TokenKind::LBrace, TokenKind::RBrace),
            _ => return,
        };
        self.stream.advance(); // consume the opener
        let mut depth: usize = 1;
        while depth > 0 {
            match self.stream.peek_kind() {
                Some(k) if *k == open => { depth += 1; self.stream.advance(); }
                Some(k) if *k == close => { depth -= 1; self.stream.advance(); }
                Some(_) => { self.stream.advance(); }
                None => break, // EOF — bail rather than loop forever
            }
        }
    }

    /// Parse macro arguments with delimiters: (tt) or [tt] or {tt}
    ///
    /// This builds a full token tree AST for macro processing while also
    /// capturing the raw text for backward compatibility.
    fn parse_macro_args(&mut self) -> ParseResult<verum_ast::expr::MacroArgs> {
        use verum_ast::expr::{
            MacroArgs, MacroArgsExt, MacroDelimiter, TokenTree, TokenTreeKind, TokenTreeToken,
        };

        let start_pos = self.stream.position();

        // Determine delimiter type and parse token tree
        let (delimiter, _open_kind, close_kind) = match self.stream.peek_kind() {
            Some(TokenKind::LParen) => {
                (MacroDelimiter::Paren, TokenKind::LParen, TokenKind::RParen)
            }
            Some(TokenKind::LBracket) => (
                MacroDelimiter::Bracket,
                TokenKind::LBracket,
                TokenKind::RBracket,
            ),
            Some(TokenKind::LBrace) => {
                (MacroDelimiter::Brace, TokenKind::LBrace, TokenKind::RBrace)
            }
            _ => {
                let span = self.stream.current_span();
                return Err(ParseError::invalid_syntax(
                    "expected macro arguments: `(...)`, `[...]`, or `{...}`",
                    span,
                ));
            }
        };

        // Consume opening delimiter
        self.stream.advance();

        // Parse token tree content - full token tree AST
        let (token_tree, raw_text) = self.parse_token_tree_full(&close_kind)?;

        // Consume closing delimiter
        self.stream.expect(close_kind)?;

        let span = self.stream.make_span(start_pos);

        // Return legacy MacroArgs for backward compatibility
        // Store the token tree in a thread-local or attach to MacroArgs via extension
        // For now, use the raw text representation
        Ok(MacroArgs::new(delimiter, raw_text, span))
    }

    /// Parse a complete token tree with full AST representation.
    ///
    /// Returns both the structured token tree and the raw text representation.
    /// This enables both advanced macro processing and backward compatibility.
    fn parse_token_tree_full(
        &mut self,
        close_kind: &TokenKind,
    ) -> ParseResult<(List<verum_ast::expr::TokenTree>, Text)> {
        use verum_ast::expr::{MacroDelimiter, TokenTree, TokenTreeKind, TokenTreeToken};

        let mut trees: List<TokenTree> = List::new();
        let mut raw_tokens: Vec<String> = Vec::new();

        loop {
            // Safety: prevent infinite loop
            if !self.tick() || self.is_aborted() {
                break;
            }

            match self.stream.peek_kind() {
                None => {
                    let span = self.stream.current_span();
                    return Err(ParseError::unexpected_eof(&[], span));
                }
                Some(kind) if kind == close_kind => {
                    // Found matching closing delimiter - done
                    break;
                }
                // Handle nested groups
                Some(TokenKind::LParen) => {
                    let group = self.parse_token_tree_group(MacroDelimiter::Paren)?;
                    raw_tokens.push(group.to_text().to_string());
                    trees.push(group);
                }
                Some(TokenKind::LBracket) => {
                    let group = self.parse_token_tree_group(MacroDelimiter::Bracket)?;
                    raw_tokens.push(group.to_text().to_string());
                    trees.push(group);
                }
                Some(TokenKind::LBrace) => {
                    let group = self.parse_token_tree_group(MacroDelimiter::Brace)?;
                    raw_tokens.push(group.to_text().to_string());
                    trees.push(group);
                }
                // GRAMMAR: quote_interpolation = splice_operator , ( identifier | '{' , expression , '}' ) ;
                // GRAMMAR: splice_operator = '$' , { '$' } ;  (* One or more $ characters *)
                // GRAMMAR: quote_repetition = splice_operator , '[' , 'for' , ... , ']' ;
                // GRAMMAR: quote_stage_escape = '$' , '(' , 'stage' , ... ;
                // After '$', there MUST be either another '$' (multi-stage), identifier, '{expr}', '[for...]', or '(stage ...)'.
                Some(TokenKind::Dollar) => {
                    let dollar_span = self.stream.current_span();
                    self.stream.advance(); // consume first $

                    // Count consecutive $ for multi-stage splices
                    // $var (stage 1), $$var (stage 2), $$$var (stage 3), etc.
                    let mut dollar_count = 1;
                    while self.stream.peek_kind() == Some(&TokenKind::Dollar) {
                        self.stream.advance();
                        dollar_count += 1;
                    }

                    // M402: Validate what follows the splice operator
                    // Valid: identifier, {, [, (
                    // Invalid: everything else (;, }, operators, literals, etc.)
                    let next_valid = match self.stream.peek_kind() {
                        // Identifier for $name interpolation
                        Some(TokenKind::Ident(_)) => true,
                        // { for ${expr} interpolation
                        Some(TokenKind::LBrace) => true,
                        // [ for $[for ...] repetition
                        Some(TokenKind::LBracket) => true,
                        // ( for $(stage N) escape
                        Some(TokenKind::LParen) => true,
                        // Keywords that can be used as identifiers in splice context
                        Some(kind) if kind.is_keyword_like() => true,
                        _ => false,
                    };

                    if !next_valid {
                        return Err(ParseError::meta_invalid_quote(
                            dollar_span,
                            "splice operator '$' must be followed by identifier, {expr}, [for...], or (stage N)",
                        ));
                    }

                    // Add all the $ tokens to the token tree
                    let dollar_str = "$".repeat(dollar_count);
                    raw_tokens.push(dollar_str.clone());
                    trees.push(TokenTree::Token(TokenTreeToken::new(
                        TokenTreeKind::Punct,
                        Text::from(dollar_str),
                        dollar_span,
                    )));
                }
                // Handle all other tokens
                Some(_) => {
                    if let Some(tok) = self.stream.advance() {
                        // Clone the token to avoid borrow conflict between stream and self
                        let tok_clone = tok.clone();
                        let (kind, text) = self.token_to_tree_token(&tok_clone);
                        raw_tokens.push(text.to_string());
                        trees.push(TokenTree::Token(TokenTreeToken::new(
                            kind,
                            text,
                            tok_clone.span,
                        )));
                    }
                }
            }
        }

        Ok((trees, Text::from(raw_tokens.join(" "))))
    }

    /// Parse a delimited group in a token tree.
    fn parse_token_tree_group(
        &mut self,
        delimiter: verum_ast::expr::MacroDelimiter,
    ) -> ParseResult<verum_ast::expr::TokenTree> {
        use verum_ast::expr::{MacroDelimiter, TokenTree};

        let start_pos = self.stream.position();

        // Get the matching close delimiter
        let close_kind = match delimiter {
            MacroDelimiter::Paren => TokenKind::RParen,
            MacroDelimiter::Bracket => TokenKind::RBracket,
            MacroDelimiter::Brace => TokenKind::RBrace,
        };

        // Consume opening delimiter
        self.stream.advance();

        // Recursively parse the group contents
        let (tokens, _raw) = self.parse_token_tree_full(&close_kind)?;

        // Consume closing delimiter
        self.stream.expect(close_kind)?;

        let span = self.stream.make_span(start_pos);
        Ok(TokenTree::Group {
            delimiter,
            tokens,
            span,
        })
    }

    /// Convert a lexer token to a token tree token.
    fn token_to_tree_token(&self, token: &Token) -> (verum_ast::expr::TokenTreeKind, Text) {
        use verum_ast::expr::TokenTreeKind;

        match &token.kind {
            // Identifiers
            TokenKind::Ident(name) => (TokenTreeKind::Ident, Text::from(name.as_str())),

            // Integer literals
            TokenKind::Integer(lit) => (TokenTreeKind::IntLiteral, Text::from(lit.to_string())),

            // Float literals
            TokenKind::Float(lit) => (TokenTreeKind::FloatLiteral, Text::from(lit.to_string())),

            // String literals
            TokenKind::Text(s) => (
                TokenTreeKind::StringLiteral,
                Text::from(format!("\"{}\"", s)),
            ),
            TokenKind::InterpolatedString(s) => {
                (TokenTreeKind::StringLiteral, Text::from(format!("{}", s)))
            }

            // Character literals
            TokenKind::Char(c) => (TokenTreeKind::CharLiteral, Text::from(format!("'{}'", c))),
            TokenKind::ByteChar(b) => (TokenTreeKind::CharLiteral, Text::from(format!("b'{}'", *b as char))),
            TokenKind::ByteString(bytes) => {
                let escaped: String = bytes.iter().map(|b| format!("\\x{:02x}", b)).collect();
                (TokenTreeKind::StringLiteral, Text::from(format!("b\"{}\"", escaped)))
            }

            // Boolean literals
            TokenKind::True => (TokenTreeKind::BoolLiteral, Text::from("true")),
            TokenKind::False => (TokenTreeKind::BoolLiteral, Text::from("false")),

            // Keywords - categorize as Keyword
            TokenKind::Let => (TokenTreeKind::Keyword, Text::from("let")),
            TokenKind::Fn => (TokenTreeKind::Keyword, Text::from("fn")),
            TokenKind::Type => (TokenTreeKind::Keyword, Text::from("type")),
            TokenKind::Match => (TokenTreeKind::Keyword, Text::from("match")),
            TokenKind::If => (TokenTreeKind::Keyword, Text::from("if")),
            TokenKind::Else => (TokenTreeKind::Keyword, Text::from("else")),
            TokenKind::While => (TokenTreeKind::Keyword, Text::from("while")),
            TokenKind::For => (TokenTreeKind::Keyword, Text::from("for")),
            TokenKind::Loop => (TokenTreeKind::Keyword, Text::from("loop")),
            TokenKind::Return => (TokenTreeKind::Keyword, Text::from("return")),
            TokenKind::Break => (TokenTreeKind::Keyword, Text::from("break")),
            TokenKind::Continue => (TokenTreeKind::Keyword, Text::from("continue")),
            TokenKind::Async => (TokenTreeKind::Keyword, Text::from("async")),
            TokenKind::Await => (TokenTreeKind::Keyword, Text::from("await")),
            TokenKind::Spawn => (TokenTreeKind::Keyword, Text::from("spawn")),
            TokenKind::Mut => (TokenTreeKind::Keyword, Text::from("mut")),
            TokenKind::Const => (TokenTreeKind::Keyword, Text::from("const")),
            TokenKind::Static => (TokenTreeKind::Keyword, Text::from("static")),
            TokenKind::Pub => (TokenTreeKind::Keyword, Text::from("pub")),
            TokenKind::Public => (TokenTreeKind::Keyword, Text::from("public")),
            TokenKind::Private => (TokenTreeKind::Keyword, Text::from("private")),
            TokenKind::Internal => (TokenTreeKind::Keyword, Text::from("internal")),
            TokenKind::Protected => (TokenTreeKind::Keyword, Text::from("protected")),
            TokenKind::Mount => (TokenTreeKind::Keyword, Text::from("mount")),
            TokenKind::Module => (TokenTreeKind::Keyword, Text::from("module")),
            TokenKind::Protocol => (TokenTreeKind::Keyword, Text::from("protocol")),
            TokenKind::Implement => (TokenTreeKind::Keyword, Text::from("implement")),
            TokenKind::Extends => (TokenTreeKind::Keyword, Text::from("extends")),
            TokenKind::Where => (TokenTreeKind::Keyword, Text::from("where")),
            TokenKind::In => (TokenTreeKind::Keyword, Text::from("in")),
            TokenKind::Is => (TokenTreeKind::Keyword, Text::from("is")),
            TokenKind::As => (TokenTreeKind::Keyword, Text::from("as")),
            TokenKind::Ref => (TokenTreeKind::Keyword, Text::from("ref")),
            TokenKind::Move => (TokenTreeKind::Keyword, Text::from("move")),
            TokenKind::Unsafe => (TokenTreeKind::Keyword, Text::from("unsafe")),
            TokenKind::Meta => (TokenTreeKind::Keyword, Text::from("meta")),
            TokenKind::Using => (TokenTreeKind::Keyword, Text::from("using")),
            TokenKind::Provide => (TokenTreeKind::Keyword, Text::from("provide")),
            TokenKind::Inject => (TokenTreeKind::Keyword, Text::from("inject")),
            TokenKind::Context => (TokenTreeKind::Keyword, Text::from("context")),
            TokenKind::Defer => (TokenTreeKind::Keyword, Text::from("defer")),
            TokenKind::Stream => (TokenTreeKind::Keyword, Text::from("stream")),
            TokenKind::Yield => (TokenTreeKind::Keyword, Text::from("yield")),
            TokenKind::SelfValue => (TokenTreeKind::Keyword, Text::from("self")),
            TokenKind::SelfType => (TokenTreeKind::Keyword, Text::from("Self")),
            TokenKind::None => (TokenTreeKind::Keyword, Text::from("None")),
            TokenKind::Some => (TokenTreeKind::Keyword, Text::from("Some")),
            TokenKind::Ok => (TokenTreeKind::Keyword, Text::from("Ok")),
            TokenKind::Err => (TokenTreeKind::Keyword, Text::from("Err")),
            // Contract/verification keywords
            TokenKind::Ensures => (TokenTreeKind::Keyword, Text::from("ensures")),
            TokenKind::Requires => (TokenTreeKind::Keyword, Text::from("requires")),
            TokenKind::Result => (TokenTreeKind::Keyword, Text::from("result")),
            TokenKind::Invariant => (TokenTreeKind::Keyword, Text::from("invariant")),
            TokenKind::Decreases => (TokenTreeKind::Keyword, Text::from("decreases")),
            // Additional keywords
            TokenKind::Select => (TokenTreeKind::Keyword, Text::from("select")),
            TokenKind::Nursery => (TokenTreeKind::Keyword, Text::from("nursery")),
            TokenKind::Errdefer => (TokenTreeKind::Keyword, Text::from("errdefer")),
            TokenKind::Throw => (TokenTreeKind::Keyword, Text::from("throw")),
            TokenKind::Throws => (TokenTreeKind::Keyword, Text::from("throws")),
            TokenKind::Ffi => (TokenTreeKind::Keyword, Text::from("ffi")),
            TokenKind::Try => (TokenTreeKind::Keyword, Text::from("try")),
            TokenKind::Checked => (TokenTreeKind::Keyword, Text::from("checked")),
            TokenKind::Super => (TokenTreeKind::Keyword, Text::from("super")),
            TokenKind::Cog => (TokenTreeKind::Keyword, Text::from("cog")),
            TokenKind::Tensor => (TokenTreeKind::Keyword, Text::from("tensor")),
            TokenKind::Affine => (TokenTreeKind::Keyword, Text::from("affine")),
            TokenKind::Finally => (TokenTreeKind::Keyword, Text::from("finally")),
            TokenKind::Recover => (TokenTreeKind::Keyword, Text::from("recover")),
            TokenKind::View => (TokenTreeKind::Keyword, Text::from("view")),
            TokenKind::Extern => (TokenTreeKind::Keyword, Text::from("extern")),
            // Proof/theorem keywords
            TokenKind::Theorem => (TokenTreeKind::Keyword, Text::from("theorem")),
            TokenKind::Axiom => (TokenTreeKind::Keyword, Text::from("axiom")),
            TokenKind::Lemma => (TokenTreeKind::Keyword, Text::from("lemma")),
            TokenKind::Corollary => (TokenTreeKind::Keyword, Text::from("corollary")),
            TokenKind::Proof => (TokenTreeKind::Keyword, Text::from("proof")),
            TokenKind::Calc => (TokenTreeKind::Keyword, Text::from("calc")),
            TokenKind::Have => (TokenTreeKind::Keyword, Text::from("have")),
            TokenKind::Show => (TokenTreeKind::Keyword, Text::from("show")),
            TokenKind::Suffices => (TokenTreeKind::Keyword, Text::from("suffices")),
            TokenKind::Obtain => (TokenTreeKind::Keyword, Text::from("obtain")),
            TokenKind::By => (TokenTreeKind::Keyword, Text::from("by")),
            TokenKind::Induction => (TokenTreeKind::Keyword, Text::from("induction")),
            TokenKind::Cases => (TokenTreeKind::Keyword, Text::from("cases")),
            TokenKind::Contradiction => (TokenTreeKind::Keyword, Text::from("contradiction")),
            TokenKind::Trivial => (TokenTreeKind::Keyword, Text::from("trivial")),
            TokenKind::Assumption => (TokenTreeKind::Keyword, Text::from("assumption")),
            TokenKind::Simp => (TokenTreeKind::Keyword, Text::from("simp")),
            TokenKind::Ring => (TokenTreeKind::Keyword, Text::from("ring")),
            TokenKind::Field => (TokenTreeKind::Keyword, Text::from("field")),
            TokenKind::Omega => (TokenTreeKind::Keyword, Text::from("omega")),
            TokenKind::Auto => (TokenTreeKind::Keyword, Text::from("auto")),
            TokenKind::Blast => (TokenTreeKind::Keyword, Text::from("blast")),
            TokenKind::Smt => (TokenTreeKind::Keyword, Text::from("smt")),
            TokenKind::Qed => (TokenTreeKind::Keyword, Text::from("qed")),
            TokenKind::Forall => (TokenTreeKind::Keyword, Text::from("forall")),
            TokenKind::Exists => (TokenTreeKind::Keyword, Text::from("exists")),
            TokenKind::Implies => (TokenTreeKind::Keyword, Text::from("implies")),

            // Punctuation and operators
            TokenKind::Plus => (TokenTreeKind::Punct, Text::from("+")),
            TokenKind::Minus => (TokenTreeKind::Punct, Text::from("-")),
            TokenKind::Star => (TokenTreeKind::Punct, Text::from("*")),
            TokenKind::Slash => (TokenTreeKind::Punct, Text::from("/")),
            TokenKind::Percent => (TokenTreeKind::Punct, Text::from("%")),
            TokenKind::Caret => (TokenTreeKind::Punct, Text::from("^")),
            TokenKind::Ampersand => (TokenTreeKind::Punct, Text::from("&")),
            TokenKind::Pipe => (TokenTreeKind::Punct, Text::from("|")),
            TokenKind::Tilde => (TokenTreeKind::Punct, Text::from("~")),
            TokenKind::Bang => (TokenTreeKind::Punct, Text::from("!")),
            TokenKind::Eq => (TokenTreeKind::Punct, Text::from("=")),
            TokenKind::Lt => (TokenTreeKind::Punct, Text::from("<")),
            TokenKind::Gt => (TokenTreeKind::Punct, Text::from(">")),
            TokenKind::Dot => (TokenTreeKind::Punct, Text::from(".")),
            TokenKind::Comma => (TokenTreeKind::Punct, Text::from(",")),
            TokenKind::Colon => (TokenTreeKind::Punct, Text::from(":")),
            TokenKind::Semicolon => (TokenTreeKind::Punct, Text::from(";")),
            TokenKind::At => (TokenTreeKind::Punct, Text::from("@")),
            TokenKind::Hash => (TokenTreeKind::Punct, Text::from("#")),
            TokenKind::Dollar => (TokenTreeKind::Punct, Text::from("$")),
            TokenKind::Question => (TokenTreeKind::Punct, Text::from("?")),
            TokenKind::RArrow => (TokenTreeKind::Punct, Text::from("->")),
            TokenKind::FatArrow => (TokenTreeKind::Punct, Text::from("=>")),
            TokenKind::DotDot => (TokenTreeKind::Punct, Text::from("..")),
            TokenKind::DotDotEq => (TokenTreeKind::Punct, Text::from("..=")),
            TokenKind::EqEq => (TokenTreeKind::Punct, Text::from("==")),
            TokenKind::BangEq => (TokenTreeKind::Punct, Text::from("!=")),
            TokenKind::LtEq => (TokenTreeKind::Punct, Text::from("<=")),
            TokenKind::GtEq => (TokenTreeKind::Punct, Text::from(">=")),
            TokenKind::AmpersandAmpersand => (TokenTreeKind::Punct, Text::from("&&")),
            TokenKind::PipePipe => (TokenTreeKind::Punct, Text::from("||")),
            TokenKind::LtLt => (TokenTreeKind::Punct, Text::from("<<")),
            TokenKind::GtGt => (TokenTreeKind::Punct, Text::from(">>")),
            TokenKind::PlusEq => (TokenTreeKind::Punct, Text::from("+=")),
            TokenKind::MinusEq => (TokenTreeKind::Punct, Text::from("-=")),
            TokenKind::StarEq => (TokenTreeKind::Punct, Text::from("*=")),
            TokenKind::SlashEq => (TokenTreeKind::Punct, Text::from("/=")),
            TokenKind::PercentEq => (TokenTreeKind::Punct, Text::from("%=")),
            TokenKind::CaretEq => (TokenTreeKind::Punct, Text::from("^=")),
            TokenKind::AmpersandEq => (TokenTreeKind::Punct, Text::from("&=")),
            TokenKind::PipeEq => (TokenTreeKind::Punct, Text::from("|=")),
            TokenKind::LtLtEq => (TokenTreeKind::Punct, Text::from("<<=")),
            TokenKind::GtGtEq => (TokenTreeKind::Punct, Text::from(">>=")),
            TokenKind::PipeGt => (TokenTreeKind::Punct, Text::from("|>")),
            TokenKind::QuestionQuestion => (TokenTreeKind::Punct, Text::from("??")),
            TokenKind::QuestionDot => (TokenTreeKind::Punct, Text::from("?.")),
            TokenKind::StarStar => (TokenTreeKind::Punct, Text::from("**")),

            // Delimiters (shouldn't be reached as they're handled separately)
            TokenKind::LParen => (TokenTreeKind::Punct, Text::from("(")),
            TokenKind::RParen => (TokenTreeKind::Punct, Text::from(")")),
            TokenKind::LBracket => (TokenTreeKind::Punct, Text::from("[")),
            TokenKind::RBracket => (TokenTreeKind::Punct, Text::from("]")),
            TokenKind::LBrace => (TokenTreeKind::Punct, Text::from("{")),
            TokenKind::RBrace => (TokenTreeKind::Punct, Text::from("}")),

            // EOF
            TokenKind::Eof => (TokenTreeKind::Eof, Text::from("")),

            // Default for any other token types
            _ => (
                TokenTreeKind::Punct,
                Text::from(format!("{:?}", token.kind)),
            ),
        }
    }

    /// Parse a capability literal: Capability.ReadOnly
    ///
    /// Syntax: Capability.CapabilityName
    /// where CapabilityName is one of: ReadOnly, WriteOnly, ReadWrite, Admin, Transaction,
    /// Network, FileSystem, Query, Execute, Logging, Metrics, Config, Cache, Auth, Custom(name)
    ///
    /// This follows Verum's path syntax using `.` instead of Rust's `::`.
    /// Consistent with RuntimeCapability.READ_ONLY syntax in the documentation.
    fn parse_capability(&mut self) -> ParseResult<verum_ast::expr::Capability> {
        use verum_ast::expr::Capability;

        // Expect "Capability" (singular, as per Verum naming conventions)
        let capability_name = self.consume_ident()?;
        if capability_name.as_str() != "Capability" {
            let span = self.stream.current_span();
            return Err(ParseError::invalid_syntax(
                format!("expected 'Capability', found '{}'", capability_name),
                span,
            ));
        }

        // Expect "." (Verum uses dot for path access, not ::)
        self.stream.expect(TokenKind::Dot)?;

        // Parse capability name
        let cap_name = self.consume_ident()?;
        let capability =
            Capability::from_str(cap_name.as_str()).unwrap_or(Capability::Custom(cap_name.clone()));

        Ok(capability)
    }

    /// Parse a capability set with | operator: Capability.ReadOnly | Capability.Query
    ///
    /// Syntax: capability (| capability)*
    pub(crate) fn parse_capability_set(&mut self) -> ParseResult<verum_ast::expr::CapabilitySet> {
        use verum_ast::expr::CapabilitySet;

        let start_pos = self.stream.position();
        let mut capabilities = Vec::new();

        // Parse first capability
        capabilities.push(self.parse_capability()?);

        // Parse additional capabilities separated by |
        while self.stream.consume(&TokenKind::Pipe).is_some() {
            capabilities.push(self.parse_capability()?);
        }

        let span = self.stream.make_span(start_pos);
        Ok(CapabilitySet::new(capabilities.into_iter().collect(), span))
    }

    /// Convert a path to a type.
    ///
    /// This is used when parsing type property expressions like `Int.size` where
    /// we've already parsed `Int` as an expression path and need to convert it to a type.
    /// Try to convert an expression to a type for type property access.
    /// This handles expressions like:
    /// - `Int` -> Int type
    /// - `(&Int)` -> reference to Int type
    /// - `(&checked Int)` -> checked reference to Int type
    /// - `(&unsafe Int)` -> unsafe reference to Int type
    /// - `(&mut Int)` -> mutable reference to Int type
    ///
    /// Returns None if the expression cannot be interpreted as a type.
    ///
    /// # PascalCase Heuristic
    ///
    /// Since the parser doesn't have access to type information (name resolution happens
    /// later), we use a naming convention heuristic to distinguish type property access
    /// from regular field access:
    ///
    /// - **PascalCase** names (starting with uppercase) are assumed to be types
    /// - **snake_case/camelCase** names (starting with lowercase) are assumed to be variables
    ///
    /// ## Examples
    ///
    /// - `Int.size` → parsed as TypeProperty (correct)
    /// - `point.size` → parsed as Field access (correct)
    /// - `MyType.alignment` → parsed as TypeProperty (correct)
    ///
    /// ## Known Limitation
    ///
    /// If a variable is named with PascalCase (e.g., `let Point = get_point();`),
    /// expressions like `Point.size` will be incorrectly parsed as TypeProperty
    /// instead of field access. This is acceptable because:
    ///
    /// 1. Verum convention is snake_case for variables (enforced by style lints)
    /// 2. Type checking will catch this misuse and provide a clear error
    /// 3. Fixing this requires name resolution during parsing (fundamentally changes architecture)
    ///
    /// Known limitation: PascalCase heuristic is used since name resolution isn't available at parse time.
    fn expr_to_type_for_property(&self, expr: &Expr) -> Option<Type> {
        match &expr.kind {
            // Path expression: Int, MyType, module.Type
            ExprKind::Path(path) => {
                // Apply PascalCase heuristic: only treat as type if name starts with uppercase
                let looks_like_type = if path.segments.len() == 1 {
                    if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                        ident.name.as_str().chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
                    } else {
                        false
                    }
                } else {
                    // Multi-segment path: could be module.Type, assume type-like
                    true
                };

                if looks_like_type {
                    self.path_to_type(path.clone()).ok()
                } else {
                    None
                }
            }

            // Parenthesized expression: (T), (&T)
            ExprKind::Paren(inner) => {
                self.expr_to_type_for_property(inner)
            }

            // Reference expression: &T, &mut T
            ExprKind::Unary { op, expr: inner } => {
                match op {
                    UnOp::Ref => {
                        // &T -> Reference type (immutable)
                        let inner_ty = self.expr_to_type_for_property(inner)?;
                        Some(Type::new(
                            verum_ast::ty::TypeKind::Reference {
                                mutable: false,
                                inner: Heap::new(inner_ty),
                            },
                            expr.span,
                        ))
                    }
                    UnOp::RefMut => {
                        // &mut T -> Mutable reference type
                        let inner_ty = self.expr_to_type_for_property(inner)?;
                        Some(Type::new(
                            verum_ast::ty::TypeKind::Reference {
                                mutable: true,
                                inner: Heap::new(inner_ty),
                            },
                            expr.span,
                        ))
                    }
                    UnOp::RefChecked => {
                        // &checked T -> Checked reference type (immutable)
                        let inner_ty = self.expr_to_type_for_property(inner)?;
                        Some(Type::new(
                            verum_ast::ty::TypeKind::CheckedReference {
                                mutable: false,
                                inner: Heap::new(inner_ty),
                            },
                            expr.span,
                        ))
                    }
                    UnOp::RefCheckedMut => {
                        // &checked mut T -> Checked reference type (mutable)
                        let inner_ty = self.expr_to_type_for_property(inner)?;
                        Some(Type::new(
                            verum_ast::ty::TypeKind::CheckedReference {
                                mutable: true,
                                inner: Heap::new(inner_ty),
                            },
                            expr.span,
                        ))
                    }
                    UnOp::RefUnsafe => {
                        // &unsafe T -> Unsafe reference type (immutable)
                        let inner_ty = self.expr_to_type_for_property(inner)?;
                        Some(Type::new(
                            verum_ast::ty::TypeKind::UnsafeReference {
                                mutable: false,
                                inner: Heap::new(inner_ty),
                            },
                            expr.span,
                        ))
                    }
                    UnOp::RefUnsafeMut => {
                        // &unsafe mut T -> Unsafe reference type (mutable)
                        let inner_ty = self.expr_to_type_for_property(inner)?;
                        Some(Type::new(
                            verum_ast::ty::TypeKind::UnsafeReference {
                                mutable: true,
                                inner: Heap::new(inner_ty),
                            },
                            expr.span,
                        ))
                    }
                    _ => None,
                }
            }

            // Index expression: List<Int> parsed as Index(Path("List"), Path("Int"))
            // This handles generic type syntax in expression context
            ExprKind::Index { expr: base_expr, index } => {
                // Get the base type
                if let Some(base_ty) = self.expr_to_type_for_property(base_expr) {
                    // Try to convert the index to a type argument
                    if let Some(arg_ty) = self.expr_to_type_for_property(index) {
                        let args: List<verum_ast::ty::GenericArg> =
                            vec![verum_ast::ty::GenericArg::Type(arg_ty)].into();
                        return Some(Type::new(
                            verum_ast::ty::TypeKind::Generic {
                                base: Heap::new(base_ty),
                                args,
                            },
                            expr.span,
                        ));
                    }
                }
                None
            }

            // Slice expression: [T]
            ExprKind::Array(verum_ast::expr::ArrayExpr::List(elements)) if elements.len() == 1 => {
                // Try to convert single element as slice type [T]
                self.expr_to_type_for_property(&elements[0]).map(|elem_ty| Type::new(
                        verum_ast::ty::TypeKind::Slice(Heap::new(elem_ty)),
                        expr.span,
                    ))
            }

            _ => None,
        }
    }

    ///
    /// Handles:
    /// - Simple names: `Int`, `Float`, `MyType`
    /// - Multi-segment paths: `std.collections.List`
    ///
    /// Note: Generic types like `List<Int>` are handled separately since they would
    /// be parsed differently in expression context (with `<` as comparison operator).
    fn path_to_type(&self, path: Path) -> ParseResult<Type> {
        let span = path.span;

        // Check if it's a simple primitive type name
        if path.segments.len() == 1 {
            if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                let type_kind = match ident.name.as_str() {
                    "Bool" => Some(verum_ast::ty::TypeKind::Bool),
                    "Int" => Some(verum_ast::ty::TypeKind::Int),
                    "Float" => Some(verum_ast::ty::TypeKind::Float),
                    "Char" => Some(verum_ast::ty::TypeKind::Char),
                    "Text" => Some(verum_ast::ty::TypeKind::Text),
                    _ => None,
                };
                if let Some(kind) = type_kind {
                    return Ok(Type::new(kind, span));
                }
            }
        }

        // For other paths, create a path type
        Ok(Type::new(verum_ast::ty::TypeKind::Path(path), span))
    }
}

// ============================================================================
// Helper functions
// ============================================================================

/// Parse integer suffix string to IntSuffix enum.
fn parse_int_suffix(s: &str) -> Option<verum_ast::literal::IntSuffix> {
    use verum_ast::literal::IntSuffix;
    match s {
        "i8" => Some(IntSuffix::I8),
        "i16" => Some(IntSuffix::I16),
        "i32" => Some(IntSuffix::I32),
        "i64" => Some(IntSuffix::I64),
        "i128" => Some(IntSuffix::I128),
        "isize" => Some(IntSuffix::Isize),
        "u8" => Some(IntSuffix::U8),
        "u16" => Some(IntSuffix::U16),
        "u32" => Some(IntSuffix::U32),
        "u64" => Some(IntSuffix::U64),
        "u128" => Some(IntSuffix::U128),
        "usize" => Some(IntSuffix::Usize),
        _ => None,
    }
}

/// Parse float suffix string to FloatSuffix enum.
fn parse_float_suffix(s: &str) -> Option<verum_ast::literal::FloatSuffix> {
    use verum_ast::literal::FloatSuffix;
    match s {
        "f32" => Some(FloatSuffix::F32),
        "f64" => Some(FloatSuffix::F64),
        _ => None,
    }
}

// ============================================================================
// Public API for other modules
// ============================================================================

/// Parse an expression (public API for other parser modules).
/// This is the RecursiveParser-based version.
pub fn expr_parser_recursive(parser: &mut RecursiveParser) -> ParseResult<Expr> {
    parser.parse_expr()
}

/// Parse a block (public API for other parser modules).
pub fn block_parser_recursive(parser: &mut RecursiveParser) -> ParseResult<Block> {
    parser.parse_block()
}

// ============================================================================
// Format Tag Validation
// ============================================================================

/// Validate format-tagged literal content at parse time.
/// Returns `None` if valid, `Some(error_message)` if invalid.
///
/// Currently validates:
/// - `json#"..."` - basic JSON structure validation
fn validate_format_tag(tag: &str, content: &Text) -> Option<String> {
    match tag {
        "json" => validate_json_content(content.as_str()),
        _ => None, // Other tags are not validated at parse time
    }
}

/// Basic JSON validation at parse time.
/// Checks structural validity: balanced braces/brackets, proper string quoting,
/// and basic token structure. This is not a full JSON parser but catches
/// common errors early.
fn validate_json_content(content: &str) -> Option<String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        // Empty content is valid (e.g., json#"""\n""" for placeholder)
        return None;
    }

    // JSON must start with {, [, ", a digit, true, false, or null
    let first = trimmed.as_bytes()[0];
    match first {
        b'{' | b'[' | b'"' | b'0'..=b'9' | b'-' | b't' | b'f' | b'n' => {}
        _ => {
            return Some(format!(
                "JSON must start with '{{', '[', '\"', a number, true, false, or null, found '{}'",
                trimmed.chars().next().unwrap_or('?')
            ));
        }
    }

    // Check balanced braces and brackets
    let mut brace_depth: i32 = 0;
    let mut bracket_depth: i32 = 0;
    let mut in_string = false;
    let mut escape_next = false;
    let bytes = trimmed.as_bytes();

    for &b in bytes {
        if escape_next {
            escape_next = false;
            continue;
        }
        if b == b'\\' && in_string {
            escape_next = true;
            continue;
        }
        if b == b'"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        match b {
            b'{' => brace_depth += 1,
            b'}' => {
                brace_depth -= 1;
                if brace_depth < 0 {
                    return Some("unexpected closing '}'".to_string());
                }
            }
            b'[' => bracket_depth += 1,
            b']' => {
                bracket_depth -= 1;
                if bracket_depth < 0 {
                    return Some("unexpected closing ']'".to_string());
                }
            }
            _ => {}
        }
    }

    if in_string {
        return Some("unterminated string in JSON".to_string());
    }
    if brace_depth != 0 {
        return Some(format!(
            "unbalanced braces: {} unclosed '{{'",
            brace_depth
        ));
    }
    if bracket_depth != 0 {
        return Some(format!(
            "unbalanced brackets: {} unclosed '['",
            bracket_depth
        ));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VerumParser;
    use verum_ast::span::FileId;

    fn parse_expr(source: &str) -> Expr {
        let parser = VerumParser::new();
        parser
            .parse_expr_str(source, FileId::new(0))
            .unwrap_or_else(|errs| panic!("Parse failed for '{}': {:?}", source, errs))
    }

    #[test]
    fn test_simple_tuple_index() {
        let expr = parse_expr("pair.0");
        match &expr.kind {
            ExprKind::TupleIndex { index, .. } => assert_eq!(*index, 0),
            other => panic!("Expected TupleIndex, got {:?}", other),
        }
    }

    #[test]
    fn test_nested_tuple_index_0_0() {
        // The lexer tokenizes `.0.0` as Dot + Float(0.0).
        // The parser must decompose this into two TupleIndex operations.
        let expr = parse_expr("pair.0.0");
        match &expr.kind {
            ExprKind::TupleIndex { expr: inner, index } => {
                assert_eq!(*index, 0, "outer index should be 0");
                match &inner.kind {
                    ExprKind::TupleIndex { index: inner_idx, .. } => {
                        assert_eq!(*inner_idx, 0, "inner index should be 0");
                    }
                    other => panic!("Expected inner TupleIndex, got {:?}", other),
                }
            }
            other => panic!("Expected TupleIndex, got {:?}", other),
        }
    }

    #[test]
    fn test_nested_tuple_index_1_2() {
        let expr = parse_expr("triple.1.2");
        match &expr.kind {
            ExprKind::TupleIndex { expr: inner, index } => {
                assert_eq!(*index, 2, "outer index should be 2");
                match &inner.kind {
                    ExprKind::TupleIndex { index: inner_idx, .. } => {
                        assert_eq!(*inner_idx, 1, "inner index should be 1");
                    }
                    other => panic!("Expected inner TupleIndex, got {:?}", other),
                }
            }
            other => panic!("Expected TupleIndex, got {:?}", other),
        }
    }

    #[test]
    fn test_triple_nested_tuple_index() {
        // `x.0.1.2` should parse as three nested TupleIndex operations.
        // Lexer produces: Ident(x), Dot, Float(0.1), Dot, Integer(2)
        // After decomposing Float(0.1): TupleIndex(TupleIndex(x, 0), 1)
        // Then the postfix loop picks up Dot, Integer(2) for the third level.
        let expr = parse_expr("x.0.1.2");
        match &expr.kind {
            ExprKind::TupleIndex { expr: mid, index } => {
                assert_eq!(*index, 2, "outermost index should be 2");
                match &mid.kind {
                    ExprKind::TupleIndex { expr: inner, index: mid_idx } => {
                        assert_eq!(*mid_idx, 1, "middle index should be 1");
                        match &inner.kind {
                            ExprKind::TupleIndex { index: inner_idx, .. } => {
                                assert_eq!(*inner_idx, 0, "innermost index should be 0");
                            }
                            other => panic!("Expected innermost TupleIndex, got {:?}", other),
                        }
                    }
                    other => panic!("Expected middle TupleIndex, got {:?}", other),
                }
            }
            other => panic!("Expected outermost TupleIndex, got {:?}", other),
        }
    }
}
