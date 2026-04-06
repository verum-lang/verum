//! Pattern matching nodes in the AST.
//!
//! Patterns are used in let bindings, function parameters, match expressions,
//! and other contexts where values are destructured or matched.

use crate::attr::Attribute;
use crate::literal::Literal;
use crate::span::{Span, Spanned};
use crate::ty::{Ident, Path};
use serde::{Deserialize, Serialize};
use verum_common::{Heap, List, Maybe};

/// A pattern for destructuring and matching values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pattern {
    pub kind: PatternKind,
    pub span: Span,
}

impl Pattern {
    pub fn new(kind: PatternKind, span: Span) -> Self {
        Self { kind, span }
    }

    pub fn wildcard(span: Span) -> Self {
        Self::new(PatternKind::Wildcard, span)
    }

    pub fn ident(name: Ident, mutable: bool, span: Span) -> Self {
        Self::new(
            PatternKind::Ident {
                by_ref: false,
                mutable,
                name,
                subpattern: Maybe::None,
            },
            span,
        )
    }

    pub fn literal(lit: Literal) -> Self {
        let span = lit.span;
        Self::new(PatternKind::Literal(lit), span)
    }
}

impl Spanned for Pattern {
    fn span(&self) -> Span {
        self.span
    }
}

/// The kind of pattern.
///
/// # Dependent Pattern Matching Extensions (v2.0+ planned)
///
/// In the dependent type system (future extension), patterns can:
/// 1. Refine types based on matched constructors (e.g., matching `Zero` proves `n = 0`)
/// 2. Include view patterns for alternative pattern interfaces via `view` declarations
/// 3. Carry proof obligations about the matched value via `with` clauses
///    These extensions enable compile-time proof of pattern exhaustiveness and type narrowing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PatternKind {
    /// Wildcard pattern: _
    Wildcard,

    /// Rest pattern: ..
    /// Used in slice patterns like [a, .., b]
    Rest,

    /// Identifier pattern: x, mut x, ref x, or ref mut x
    Ident {
        /// Whether the binding is by reference (ref)
        by_ref: bool,
        /// Whether the binding is mutable
        mutable: bool,
        name: Ident,
        /// Optional subpattern with @ binding: x @ Some(y)
        subpattern: Maybe<Heap<Pattern>>,
    },

    /// Literal pattern: 42, "hello", true
    Literal(Literal),

    /// Tuple pattern: (a, b, c)
    Tuple(List<Pattern>),

    /// Array pattern: [a, b, c]
    Array(List<Pattern>),

    /// Slice pattern with rest: [a, .., b]
    Slice {
        before: List<Pattern>,
        rest: Maybe<Heap<Pattern>>,
        after: List<Pattern>,
    },

    /// Record pattern: Point { x, y } or Point { x: px, y: py }
    Record {
        path: Path,
        fields: List<FieldPattern>,
        /// Whether to ignore extra fields with ..
        rest: bool,
    },

    /// Variant pattern: Some(x) or None
    Variant {
        path: Path,
        /// Variant data (tuple or record style)
        data: Maybe<VariantPatternData>,
    },

    /// Or pattern: a | b | c
    Or(List<Pattern>),

    /// Reference pattern: &x or &mut x
    Reference { mutable: bool, inner: Heap<Pattern> },

    /// Range pattern: 1..10 or 1..=10
    Range {
        start: Maybe<Heap<Literal>>,
        end: Maybe<Heap<Literal>>,
        inclusive: bool,
    },

    /// Parenthesized pattern for clarity
    Paren(Heap<Pattern>),

    /// View pattern: match through a transformation function.
    ///
    /// The view function is applied to the scrutinee and the result is matched
    /// against the inner pattern. This enables matching on computed properties
    /// without requiring a separate `let` binding.
    ///
    /// # Syntax
    /// ```verum
    /// view_fn -> inner_pattern
    /// ```
    ///
    /// # Examples
    /// ```verum
    /// // Match through a transformation function
    /// match value {
    ///     parity -> Even(k) => print(f"{k} is even"),
    ///     parity -> Odd(k) => print(f"{k} is odd"),
    /// }
    ///
    /// // Qualified view function (module path)
    /// match response {
    ///     json.parse -> Ok(data) => process(data),
    ///     json.parse -> Err(e) => handle(e),
    /// }
    ///
    /// // Nested view patterns
    /// match input {
    ///     parse_int -> Some(abs -> Positive(n)) => use_positive(n),
    ///     _ => default(),
    /// }
    /// ```
    ///
    /// # Semantics
    /// - The view function is called with the scrutinee as its argument
    /// - The return value is matched against the inner pattern
    /// - Bindings in the inner pattern are available in the match arm body
    /// - View patterns compose: `f -> g -> pat` means apply f, then apply g to result
    View {
        /// The view function expression (identifier or qualified path)
        view_function: Heap<Expr>,
        /// Pattern to match against the view function's return value
        pattern: Heap<Pattern>,
    },

    /// Active pattern invocation: user-defined pattern matchers (F#-style).
    /// Active patterns are declared with `pattern Name(params) -> ReturnType = body;`.
    ///
    /// # Pattern Categories
    ///
    /// ## 1. Total Patterns (Boolean Test)
    /// ```verum
    /// pattern Even(n: Int) -> Bool = n % 2 == 0;
    ///
    /// match n {
    ///     Even() => "even",
    ///     _ => "odd",
    /// }
    /// ```
    ///
    /// ## 2. Parameterized Patterns
    /// ```verum
    /// pattern InRange(lo: Int, hi: Int)(n: Int) -> Bool = lo <= n <= hi;
    ///
    /// match n {
    ///     InRange(0, 100)() => "in range",
    ///     _ => "out of range",
    /// }
    /// ```
    ///
    /// ## 3. Partial Patterns (Extraction with Bindings)
    /// ```verum
    /// pattern ParseInt(s: Text) -> Maybe<Int> = s.parse_int();
    ///
    /// match s {
    ///     ParseInt(n) => use(n),    // n: Int, extracted from Some
    ///     _ => handle_error(),
    /// }
    /// ```
    Active {
        /// Name of the active pattern (e.g., `Even`, `InRange`, `ParseInt`)
        name: Ident,
        /// Pattern parameters - expressions passed to parameterized patterns
        /// For `InRange(0, 100)()`, this contains [0, 100]
        /// For `Even()`, this is empty
        params: List<Expr>,
        /// Extraction bindings for partial patterns returning `Maybe<T>`
        /// For `ParseInt(n)`, this contains [Pattern::Ident("n")]
        /// For total patterns like `Even()`, this is empty
        /// The bindings match against the inner value of `Some(...)`
        bindings: List<Pattern>,
    },

    /// Pattern combination with &: matches when ALL patterns match simultaneously.
    ///
    /// # Example
    /// ```verum
    /// match n {
    ///     Even() & Positive() => "positive even",
    ///     Even() => "non-positive even",
    ///     _ => "odd",
    /// }
    /// ```
    And(List<Pattern>),

    /// Guarded pattern: pattern with inline guard condition
    /// Spec: Rust RFC 3637 - Guard Patterns
    ///
    /// Allows guards to nest within or-patterns, enabling per-alternative conditions.
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
    Guard {
        /// The inner pattern to match
        pattern: Heap<Pattern>,
        /// Guard expression that must evaluate to true for match to succeed
        guard: Heap<Expr>,
    },

    /// Type test pattern for runtime type checking and narrowing.
    ///
    /// This pattern tests if a value has a specific runtime type and binds it
    /// to a variable with the narrowed type. Essential for safe dynamic typing.
    ///
    /// # Example
    /// ```verum
    /// match value {
    ///     x is Int => process(x),   // x has type Int
    ///     x is Text => format(x),   // x has type Text
    ///     _ => other(),
    /// }
    /// ```
    ///
    /// # Semantics
    /// - At runtime, checks if value is of the specified type
    /// - If match succeeds, binding has the narrowed type in that arm
    /// - Essential for working with `unknown` type safely
    TypeTest {
        /// Binding name for the narrowed value
        binding: Ident,
        /// Type to test against
        test_type: crate::ty::Type,
    },

    /// Stream pattern for matching and destructuring lazy streams/iterators.
    ///
    /// Unlike slice patterns which work on fixed collections, stream patterns
    /// consume elements lazily from an iterator.
    ///
    /// # Examples
    /// ```verum
    /// match iterator {
    ///     stream[first, second, ...rest] => {
    ///         // first and second consumed, rest is remaining iterator
    ///     }
    ///     stream[head, ...tail] => {
    ///         // consume one, tail is remaining
    ///     }
    ///     stream[] => {
    ///         // empty stream
    ///     }
    /// }
    /// ```
    ///
    /// # Semantics
    /// - Patterns before `...` are consumed from the iterator
    /// - The `...rest` binding captures the remaining iterator (not a list!)
    /// - Empty pattern `stream[]` matches exhausted iterator
    Stream {
        /// Head elements to consume and match
        head_patterns: List<Pattern>,
        /// Optional binding for remaining iterator (after ...)
        /// If None, remaining elements are discarded
        rest: Maybe<Ident>,
    },

    /// Cons pattern for destructuring stream/list types: `head :: tail`
    ///
    /// Right-associative: `a :: b :: rest` means `Cons(a, Cons(b, rest))`
    ///
    /// # Examples
    /// ```verum
    /// match stream {
    ///     a :: b :: _ => use_two(a, b),
    ///     head :: rest => use_head(head),
    ///     Nil => empty(),
    /// }
    /// ```
    Cons {
        /// Head element pattern
        head: Heap<Pattern>,
        /// Tail pattern (may be another Cons for chaining)
        tail: Heap<Pattern>,
    },
}

/// A field in a record pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldPattern {
    pub name: Ident,
    /// Optional pattern to bind to (None means shorthand: { x } = { x: x })
    pub pattern: Maybe<Pattern>,
    pub span: Span,
}

impl FieldPattern {
    pub fn new(name: Ident, pattern: Maybe<Pattern>, span: Span) -> Self {
        Self {
            name,
            pattern,
            span,
        }
    }

    /// Create a shorthand field pattern: { x }
    pub fn shorthand(name: Ident) -> Self {
        let span = name.span;
        Self {
            name,
            pattern: Maybe::None,
            span,
        }
    }
}

impl Spanned for FieldPattern {
    fn span(&self) -> Span {
        self.span
    }
}

/// Variant pattern data (tuple or record style).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum VariantPatternData {
    /// Tuple-style variant: Some(x, y)
    Tuple(List<Pattern>),
    /// Record-style variant: Error { code, message }
    Record {
        fields: List<FieldPattern>,
        rest: bool,
    },
}

/// A match arm in a match expression.
///
/// # Dependent Pattern Matching (v2.0+ planned)
///
/// Match arms can include with-clauses for proof obligations in dependent pattern matching.
/// The with-clause specifies constraints that are proven when the pattern matches.
/// For example, matching `Zero` in a Nat pattern proves `n = 0` in that arm.
///
/// # Examples
/// ```verum
/// fn is_zero(n: Nat) -> bool with (n = 0) | (n ≠ 0) =
///     match n {
///         Zero => true    // Here we know n = 0
///         Succ(_) => false // Here we know n ≠ 0
///     }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchArm {
    /// The pattern to match against
    pub pattern: Pattern,
    /// Optional guard condition: if x > 0
    pub guard: Maybe<Heap<Expr>>,
    /// The expression to evaluate if matched
    pub body: Heap<Expr>,
    /// With-clause proof obligations (for dependent pattern matching, v2.0+ planned).
    /// Specifies constraints proven when the pattern matches, e.g., `with (n = 0)` when
    /// matching `Zero`. The compiler verifies these obligations via the proof system.
    pub with_clause: Maybe<List<Expr>>,
    /// Attributes on the match arm
    pub attributes: List<Attribute>,
    pub span: Span,
}

impl MatchArm {
    pub fn new(pattern: Pattern, guard: Maybe<Heap<Expr>>, body: Heap<Expr>, span: Span) -> Self {
        Self {
            pattern,
            guard,
            body,
            with_clause: Maybe::None,
            attributes: List::new(),
            span,
        }
    }

    /// Create a match arm with a with-clause for proof obligations
    pub fn with_clause(
        pattern: Pattern,
        guard: Maybe<Heap<Expr>>,
        body: Heap<Expr>,
        with_clause: List<Expr>,
        span: Span,
    ) -> Self {
        Self {
            pattern,
            guard,
            body,
            with_clause: Some(with_clause),
            attributes: List::new(),
            span,
        }
    }
}

impl Spanned for MatchArm {
    fn span(&self) -> Span {
        self.span
    }
}

// Forward declaration - actual Expr is in expr.rs
pub use crate::expr::Expr;
