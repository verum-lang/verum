//! Expression nodes in the AST.
//!
//! This module defines all expression types in Verum, including:
//! - Literals and identifiers
//! - Arithmetic and logical operations
//! - Function calls and method calls
//! - Control flow (if, match, loops)
//! - Closures and async expressions
//! - Stream comprehensions and pipelines

use crate::attr::Attribute;
use crate::literal::Literal;
use crate::pattern::{MatchArm, Pattern};
use crate::span::{Span, Spanned};
use crate::ty::{Ident, Path, Type};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use verum_common::{Heap, List, Maybe, Text};

/// An expression in Verum.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
    /// Reference kind for optimization (CBGR tier selection)
    #[serde(skip)]
    pub ref_kind: Option<ReferenceKind>,
    /// Whether CBGR check has been eliminated for this expression
    #[serde(skip)]
    pub check_eliminated: bool,
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Self {
            kind,
            span,
            ref_kind: None,
            check_eliminated: false,
        }
    }

    pub fn literal(lit: Literal) -> Self {
        let span = lit.span;
        Self::new(ExprKind::Literal(lit), span)
    }

    pub fn path(path: Path) -> Self {
        let span = path.span;
        Self::new(ExprKind::Path(path), span)
    }

    pub fn ident(name: Ident) -> Self {
        let _span = name.span;
        Self::path(Path::single(name))
    }

    /// Get reference kind for optimization
    pub fn reference_kind(&self) -> Option<ReferenceKind> {
        self.ref_kind
    }

    /// Set reference kind for optimization
    pub fn set_reference_kind(&mut self, kind: ReferenceKind) {
        self.ref_kind = Some(kind);
    }

    /// Check if CBGR check has been eliminated
    pub fn is_check_eliminated(&self) -> bool {
        self.check_eliminated
    }

    /// Mark CBGR check as eliminated
    pub fn mark_check_eliminated(&mut self) {
        self.check_eliminated = true;
    }
}

impl Spanned for Expr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Capability set for context attenuation
///
/// Represents a set of capabilities that a context can provide.
/// Used with the `attenuate()` method to create restricted sub-contexts.
///
/// Capability expression for context-based access control.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CapabilitySet {
    /// Set of capability flags
    pub capabilities: List<Capability>,
    /// Source span for error reporting
    pub span: Span,
}

impl CapabilitySet {
    /// Create a new capability set
    pub fn new(capabilities: List<Capability>, span: Span) -> Self {
        Self { capabilities, span }
    }

    /// Create an empty capability set (no capabilities)
    pub fn empty(span: Span) -> Self {
        Self {
            capabilities: List::new(),
            span,
        }
    }

    /// Create a capability set with a single capability
    pub fn single(capability: Capability, span: Span) -> Self {
        Self {
            capabilities: vec![capability].into(),
            span,
        }
    }

    /// Check if this set contains a specific capability
    pub fn contains(&self, capability: &Capability) -> bool {
        self.capabilities.iter().any(|c| c == capability)
    }

    /// Check if this set is a subset of another
    pub fn is_subset_of(&self, other: &CapabilitySet) -> bool {
        self.capabilities
            .iter()
            .all(|c| other.capabilities.contains(c))
    }

    /// Check if this set is empty
    pub fn is_empty(&self) -> bool {
        self.capabilities.is_empty()
    }

    /// Get the number of capabilities
    pub fn len(&self) -> usize {
        self.capabilities.len()
    }

    /// Merge two capability sets (union)
    pub fn merge(&self, other: &CapabilitySet) -> Self {
        let mut caps = self.capabilities.clone();
        for cap in &other.capabilities {
            if !caps.contains(cap) {
                caps.push(cap.clone());
            }
        }
        Self {
            capabilities: caps,
            span: self.span,
        }
    }

    /// Intersect two capability sets
    pub fn intersect(&self, other: &CapabilitySet) -> Self {
        let caps: List<Capability> = self
            .capabilities
            .iter()
            .filter(|c| other.capabilities.contains(c))
            .cloned()
            .collect();
        Self {
            capabilities: caps,
            span: self.span,
        }
    }
}

impl Spanned for CapabilitySet {
    fn span(&self) -> Span {
        self.span
    }
}

/// Individual capability that can be granted or restricted
///
/// Capabilities follow common patterns across different contexts.
/// Context implementations can define custom capabilities beyond these standard ones.
///
/// Capability expression for context-based access control.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
    /// Read-only access (no mutations)
    ReadOnly,

    /// Write-only access (no reads)
    WriteOnly,

    /// Full read and write access
    ReadWrite,

    /// Administrative/elevated privileges
    Admin,

    /// Transaction management capabilities
    Transaction,

    /// Network access capabilities
    Network,

    /// File system access capabilities
    FileSystem,

    /// Database query capabilities
    Query,

    /// Database mutation capabilities
    Execute,

    /// Logging capabilities
    Logging,

    /// Metrics/telemetry capabilities
    Metrics,

    /// Configuration access capabilities
    Config,

    /// Cache access capabilities
    Cache,

    /// Authentication capabilities
    Auth,

    /// Custom named capability
    Custom(Text),
}

impl Capability {
    /// Get the string representation of this capability
    pub fn as_str(&self) -> &str {
        match self {
            Capability::ReadOnly => "ReadOnly",
            Capability::WriteOnly => "WriteOnly",
            Capability::ReadWrite => "ReadWrite",
            Capability::Admin => "Admin",
            Capability::Transaction => "Transaction",
            Capability::Network => "Network",
            Capability::FileSystem => "FileSystem",
            Capability::Query => "Query",
            Capability::Execute => "Execute",
            Capability::Logging => "Logging",
            Capability::Metrics => "Metrics",
            Capability::Config => "Config",
            Capability::Cache => "Cache",
            Capability::Auth => "Auth",
            Capability::Custom(name) => name.as_str(),
        }
    }

    /// Parse a capability from a string
    pub fn from_str(s: &str) -> Maybe<Self> {
        match s {
            "ReadOnly" => Maybe::Some(Capability::ReadOnly),
            "WriteOnly" => Maybe::Some(Capability::WriteOnly),
            "ReadWrite" => Maybe::Some(Capability::ReadWrite),
            "Admin" => Maybe::Some(Capability::Admin),
            "Transaction" => Maybe::Some(Capability::Transaction),
            "Network" => Maybe::Some(Capability::Network),
            "FileSystem" => Maybe::Some(Capability::FileSystem),
            "Query" => Maybe::Some(Capability::Query),
            "Execute" => Maybe::Some(Capability::Execute),
            "Logging" => Maybe::Some(Capability::Logging),
            "Metrics" => Maybe::Some(Capability::Metrics),
            "Config" => Maybe::Some(Capability::Config),
            "Cache" => Maybe::Some(Capability::Cache),
            "Auth" => Maybe::Some(Capability::Auth),
            _ => Maybe::Some(Capability::Custom(Text::from(s))),
        }
    }

    /// Check if this is a standard (non-custom) capability
    pub fn is_standard(&self) -> bool {
        !matches!(self, Capability::Custom(_))
    }
}

/// Type property for compile-time type introspection.
///
/// Type properties provide zero-overhead access to type metadata at compile time.
/// These replace deprecated intrinsic functions like `size_of<T>()` with a cleaner
/// postfix syntax: `T.size` instead of `size_of::<T>()`.
///
/// # Examples
/// ```verum
/// Int.size          // Size of Int in bytes (8)
/// Float.alignment   // Alignment requirement of Float (8)
/// List<Int>.stride  // Memory stride for iteration
/// i8.min            // Minimum value (-128)
/// i8.max            // Maximum value (127)
/// u32.bits          // Bit width (32)
/// T.name            // Type name as string
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TypeProperty {
    /// Size of the type in bytes
    /// Example: `Int.size` -> 8
    Size,

    /// Alignment requirement of the type in bytes
    /// Example: `Float.alignment` -> 8
    Alignment,

    /// Memory stride for arrays/iteration
    /// Example: `[Int; 10].stride` -> 8
    Stride,

    /// Minimum value (for numeric types)
    /// Example: `i8.min` -> -128
    Min,

    /// Maximum value (for numeric types)
    /// Example: `i8.max` -> 127
    Max,

    /// Bit width (for numeric types)
    /// Example: `Int.bits` -> 64
    Bits,

    /// Type name as a string
    /// Example: `Int.name` -> "Int"
    Name,

    /// Unique type identifier (hash of canonical name)
    /// Example: `Int.id` -> 0x1234567890ABCDEF
    Id,
}

impl TypeProperty {
    /// Parse a type property from an identifier string.
    /// Returns None if the identifier is not a recognized type property.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "size" => Some(TypeProperty::Size),
            "alignment" => Some(TypeProperty::Alignment),
            "stride" => Some(TypeProperty::Stride),
            "min" => Some(TypeProperty::Min),
            "max" => Some(TypeProperty::Max),
            "bits" => Some(TypeProperty::Bits),
            "name" => Some(TypeProperty::Name),
            "id" => Some(TypeProperty::Id),
            _ => None,
        }
    }

    /// Get the string representation of this type property.
    pub fn as_str(&self) -> &'static str {
        match self {
            TypeProperty::Size => "size",
            TypeProperty::Alignment => "alignment",
            TypeProperty::Stride => "stride",
            TypeProperty::Min => "min",
            TypeProperty::Max => "max",
            TypeProperty::Bits => "bits",
            TypeProperty::Name => "name",
            TypeProperty::Id => "id",
        }
    }
}

impl std::fmt::Display for TypeProperty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// The kind of expression.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ExprKind {
    /// Literal value: 42, "hello", true
    Literal(Literal),

    /// Path to a value or type: x, std.io.print
    Path(Path),

    /// Binary operation: a + b, x * y
    Binary {
        op: BinOp,
        left: Heap<Expr>,
        right: Heap<Expr>,
    },

    /// Unary operation: !x, -y, *ptr
    Unary { op: UnOp, expr: Heap<Expr> },

    /// Named argument in function calls: name: value
    ///
    /// Used for keyword arguments in function/method calls:
    /// - `foo(x: 1, y: 2)` - named positional args
    /// - `tensor.sum(axis: 0)` - named method args
    /// - `Tensor.rand([100], uniform: (0.0, 1.0))` - distribution params
    NamedArg {
        name: Ident,
        value: Heap<Expr>,
    },

    /// Function call: f(a, b, c) or f<T>(a, b) with explicit type arguments
    Call {
        func: Heap<Expr>,
        /// Optional explicit type arguments: func<T>() - Verum uses direct <T> syntax, not turbofish
        type_args: List<crate::ty::GenericArg>,
        args: List<Expr>,
    },

    /// Method call: obj.method(args) or obj.method<T>(args)
    MethodCall {
        receiver: Heap<Expr>,
        method: Ident,
        /// Optional type arguments for generic method calls: method<T>()
        type_args: List<crate::ty::GenericArg>,
        args: List<Expr>,
    },

    /// Field access: obj.field
    Field { expr: Heap<Expr>, field: Ident },

    /// Optional chaining: obj?.field
    OptionalChain { expr: Heap<Expr>, field: Ident },

    /// Tuple field access: tuple.0
    TupleIndex { expr: Heap<Expr>, index: u32 },

    /// Index operation: arr[i]
    Index { expr: Heap<Expr>, index: Heap<Expr> },

    /// Pipeline operator: x |> f |> g
    Pipeline { left: Heap<Expr>, right: Heap<Expr> },

    /// Null coalescing: a ?? b
    NullCoalesce { left: Heap<Expr>, right: Heap<Expr> },

    /// Type cast: x as T
    Cast { expr: Heap<Expr>, ty: Type },

    /// Error propagation: expr?
    Try(Heap<Expr>),

    /// Plain try block without recover or finally: try { ... }
    ///
    /// Grammar: try_expr = 'try' , block ;
    ///
    /// A plain try block creates a Result<T, E> from the block's value,
    /// auto-wrapping the final expression in Ok() and capturing any errors
    /// from ? operators within the block.
    ///
    /// Example:
    /// ```verum
    /// let result: Result<Int, Text> = try {
    ///     let x = some_fallible_op()?;
    ///     x + 1  // Auto-wrapped in Ok(x + 1)
    /// };
    /// ```
    ///
    /// The error type E is inferred from the ? operators used in the block.
    /// If no ? operators are present, E defaults to Never (infallible).
    ///
    /// Throws clause for typed error boundaries (Swift 6-inspired).
    TryBlock(Heap<Expr>),

    /// Try-recover expression: try { ... } recover { pattern => expr, ... }
    ///
    /// Grammar: try_recovery = 'recover' , '{' , match_arms , '}' ;
    ///          try_recovery = 'recover' , '|' , pattern , '|' , expr ;
    ///
    /// The recover block can use either match arms or closure syntax.
    ///
    /// Match arms example:
    /// ```verum
    /// try {
    ///     // code that may fail
    /// } recover {
    ///     SomeError(msg) => print(f"Error: {msg}"),
    ///     OtherError => print(f"Other error"),
    ///     _ => print(f"Unknown error"),
    /// }
    /// ```
    ///
    /// Closure example:
    /// ```verum
    /// try {
    ///     // code that may fail
    /// } recover |e| {
    ///     log(f"Error occurred: {e}")
    /// }
    /// ```
    ///
    /// try/recover/finally structured error handling with pattern matching on error types.
    TryRecover {
        try_block: Heap<Expr>,
        /// Recovery body - either match arms or closure
        recover: RecoverBody,
    },

    /// Try-finally expression: try { ... } finally { ... }
    TryFinally {
        try_block: Heap<Expr>,
        finally_block: Heap<Expr>,
    },

    /// Try-recover-finally expression: try { ... } recover { pattern => expr, ... } finally { ... }
    ///
    /// Grammar: try_handlers = try_recovery , [ try_finally ]
    ///          try_recovery = 'recover' , '{' , match_arms , '}' ;
    ///          try_recovery = 'recover' , '|' , pattern , '|' , expr ;
    ///
    /// Combines recover (match arms or closure) with finally block for resource cleanup.
    ///
    /// Match arms example:
    /// ```verum
    /// try {
    ///     // code that may fail
    /// } recover {
    ///     FileError(msg) => log(f"File error: {msg}"),
    ///     _ => log("Unknown error"),
    /// } finally {
    ///     cleanup()
    /// }
    /// ```
    ///
    /// Closure example:
    /// ```verum
    /// try {
    ///     // code that may fail
    /// } recover |e| log(f"Error: {e}")
    /// finally {
    ///     cleanup()
    /// }
    /// ```
    ///
    /// try/recover/finally structured error handling with pattern matching on error types.
    TryRecoverFinally {
        try_block: Heap<Expr>,
        /// Recovery body - either match arms or closure
        recover: RecoverBody,
        finally_block: Heap<Expr>,
    },

    /// Tuple expression: (a, b, c)
    Tuple(List<Expr>),

    /// Array expression: [a, b, c] or [x; n]
    Array(ArrayExpr),

    /// List comprehension: [x * 2 for x in list if x > 0]
    Comprehension {
        expr: Heap<Expr>,
        clauses: List<ComprehensionClause>,
    },

    /// Stream comprehension: stream [x * 2 for x in source]
    StreamComprehension {
        expr: Heap<Expr>,
        clauses: List<ComprehensionClause>,
    },

    /// Stream literal: stream[1, 2, 3, ...] or stream[0..100]
    /// Lazy stream comprehension expression for deferred evaluation.
    ///
    /// Creates lazy streams from:
    /// - Element list with optional ellipsis for infinite cycle
    /// - Range expressions for lazy ranges
    ///
    /// Examples:
    /// - stream[1, 2, 3, ...]  -> cycles [1, 2, 3] infinitely
    /// - stream[0, 1, 2, ...]  -> count_from(0) pattern detected
    /// - stream[0..100]        -> lazy range [0, 100)
    /// - stream[0..]           -> infinite range from 0
    StreamLiteral(StreamLiteralExpr),

    /// Record expression: Point { x: 1, y: 2 }
    Record {
        path: Path,
        fields: List<FieldInit>,
        /// Struct update syntax: { ..base }
        base: Maybe<Heap<Expr>>,
    },

    /// Interpolated string: f"Hello {name}" or sql"SELECT * FROM {table}"
    ///
    /// This represents string interpolation with embedded expressions.
    /// The handler specifies how to process the interpolation (e.g., "f" for format, "sql" for SQL injection protection).
    InterpolatedString {
        /// Handler name (e.g., "f", "sql", "html")
        handler: Text,
        /// Template parts (string segments between interpolations)
        parts: List<Text>,
        /// Interpolated expressions
        exprs: List<Expr>,
    },

    /// Tensor literal with compile-time shape validation: tensor<2, 3> Int { [[1, 2, 3], [4, 5, 6]] }
    /// Tensor literal with compile-time shape validation and SIMD acceleration.
    ///
    /// This is the PRIMARY approach (P0) for tensor literals with compile-time shape validation.
    /// Shape is encoded in the type system for SIMD/GPU optimization.
    ///
    /// Examples:
    /// - tensor<4> f32 { 1.0, 2.0, 3.0, 4.0 }  // 1D vector
    /// - tensor<2, 3> Int { [[1, 2, 3], [4, 5, 6]] }  // 2D matrix
    /// - tensor<3, 224, 224> u8 { ... }  // 3D RGB image
    TensorLiteral {
        /// Shape dimensions (compile-time constants)
        shape: List<u64>,
        /// Element type
        elem_type: Type,
        /// Data expression (nested arrays or flat list)
        data: Heap<Expr>,
    },

    /// Map literal: { "key": value, "key2": value2 }
    ///
    /// Distinguished from records by having key-value pairs where keys are expressions (not identifiers).
    MapLiteral {
        /// Key-value pairs
        entries: List<(Expr, Expr)>,
    },

    /// Set literal: { 1, 2, 3 }
    ///
    /// Distinguished from maps by having single expressions (not key-value pairs).
    SetLiteral {
        /// Set elements
        elements: List<Expr>,
    },

    /// Map comprehension: {k: v for (k, v) in pairs if condition}
    /// Pipeline operator (|>) for left-to-right function chaining.
    ///
    /// Produces Map<K, V> from key-value expressions with iteration.
    /// Disambiguated from map literal by 'for' keyword after value expression.
    ///
    /// Examples:
    /// - {x: x * x for x in 1..10}
    /// - {name: len for name in names if name.len() > 0}
    /// - {(i, j): i * j for i in 0..n for j in 0..m}
    MapComprehension {
        /// Key expression
        key_expr: Heap<Expr>,
        /// Value expression
        value_expr: Heap<Expr>,
        /// Comprehension clauses (for, if, let)
        clauses: List<ComprehensionClause>,
    },

    /// Set comprehension: set{x for x in items if condition}
    /// State machine expression with compile-time state transition verification.
    ///
    /// Produces Set<T> with unique elements from iteration.
    /// Uses 'set' keyword prefix for explicit disambiguation.
    ///
    /// Examples:
    /// - set{user.id for user in users}
    /// - set{n for n in 2..100 if is_prime(n)}
    /// - set{tag for post in posts for tag in post.tags}
    SetComprehension {
        /// Element expression
        expr: Heap<Expr>,
        /// Comprehension clauses (for, if, let)
        clauses: List<ComprehensionClause>,
    },

    /// Generator expression: gen{x for x in items if condition}
    /// Label block expression for named break targets in nested control flow.
    ///
    /// Produces lazy Iterator<T> evaluated on demand.
    /// Memory-efficient for large/infinite sequences.
    ///
    /// Examples:
    /// - gen{x * x for x in 0..}  // infinite squares
    /// - gen{expensive(x) for x in items if predicate(x)}
    /// - gen{(x, y) for x in xs for y in ys if x != y}
    GeneratorComprehension {
        /// Element expression
        expr: Heap<Expr>,
        /// Comprehension clauses (for, if, let)
        clauses: List<ComprehensionClause>,
    },

    /// Block expression: { stmt1; stmt2; expr }
    Block(Block),

    /// If expression: if cond { a } else { b }
    If {
        condition: Heap<IfCondition>,
        then_branch: Block,
        else_branch: Maybe<Heap<Expr>>,
    },

    /// Match expression: match x { pattern => expr, ... }
    Match {
        expr: Heap<Expr>,
        arms: List<MatchArm>,
    },

    /// Loop expression: loop { ... }
    Loop {
        /// Optional label: 'label: loop { ... }
        label: Maybe<Text>,
        body: Block,
        /// Loop invariants for verification (zero or more)
        invariants: List<Expr>,
    },

    /// While loop: while cond { ... }
    While {
        /// Optional label: 'label: while cond { ... }
        label: Maybe<Text>,
        condition: Heap<Expr>,
        body: Block,
        /// Loop invariants for verification (zero or more)
        invariants: List<Expr>,
        /// Termination measures (zero or more)
        decreases: List<Expr>,
    },

    /// For loop: for x in iter { ... }
    For {
        /// Optional label: 'label: for x in iter { ... }
        label: Maybe<Text>,
        pattern: Pattern,
        iter: Heap<Expr>,
        body: Block,
        /// Loop invariants for verification (zero or more)
        invariants: List<Expr>,
        /// Termination measures (zero or more)
        decreases: List<Expr>,
    },

    /// For-await loop: for await item in async_stream { ... }
    ///
    /// Asynchronously iterates over an async iterable (stream).
    /// This is only valid in async contexts (async fn, async block).
    /// The async_iterable must implement AsyncIterator.
    ///
    /// Grammar: for_await_loop = 'for' , 'await' , pattern , 'in' , expression
    ///                         , { loop_annotation } , block_expr ;
    ///
    /// Example:
    /// ```verum
    /// async fn process_stream(stream: AsyncStream<Item>) {
    ///     for await item in stream {
    ///         process(item);
    ///     }
    /// }
    /// ```
    ///
    /// Spec: grammar/verum.ebnf - for_await_loop production (v2.10)
    ForAwait {
        /// Optional label: 'label: for await item in stream { ... }
        label: Maybe<Text>,
        /// Pattern to bind each yielded value
        pattern: Pattern,
        /// The async iterable expression (must implement AsyncIterator)
        async_iterable: Heap<Expr>,
        /// Loop body
        body: Block,
        /// Loop invariants for verification (zero or more)
        invariants: List<Expr>,
        /// Termination measures (zero or more)
        decreases: List<Expr>,
    },

    /// Break with optional value and label: break, break 'label, break value, break 'label value
    Break {
        label: Maybe<Text>,
        value: Maybe<Heap<Expr>>,
    },

    /// Continue with optional label: continue or continue 'label
    Continue { label: Maybe<Text> },

    /// Return with optional value: return or return value
    Return(Maybe<Heap<Expr>>),

    /// Throw expression: throw error_value
    ///
    /// Throws an error value in a function with a `throws` clause.
    /// The error value must match one of the declared error types.
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
    Throw(Heap<Expr>),

    /// Yield expression: yield value
    Yield(Heap<Expr>),

    /// Typeof expression - runtime type introspection
    /// Returns TypeInfo for the runtime type of a value.
    /// Type guard (x is T) for runtime type checking and narrowing.
    /// Example: `typeof(value)` returns `TypeInfo { id, name, kind, protocols }`
    Typeof(Heap<Expr>),

    /// Closure: |x, y| x + y
    /// With contexts: |x, y| using [IO, State] -> T { ... }
    Closure {
        async_: bool,
        move_: bool,
        params: List<ClosureParam>,
        contexts: List<crate::decl::ContextRequirement>,
        return_type: Maybe<Type>,
        body: Heap<Expr>,
    },

    /// Async block: async { ... }
    Async(Block),

    /// Await expression: expr.await
    ///
    /// Suspends the current async context until the future completes.
    /// Can only be used within async functions or blocks.
    ///
    /// Example:
    /// ```verum
    /// let result = fetch_data().await
    /// ```
    Await(Heap<Expr>),

    /// Spawn expression: spawn expr with context requirements
    ///
    /// Spawns a new concurrent task with specified context requirements.
    /// The task runs independently and returns a handle that can be awaited.
    /// Contexts provide dependency injection (NOT algebraic effects).
    ///
    /// Example:
    /// ```verum
    /// let handle = spawn { compute_value() } using [Database, Logger]
    /// ```
    Spawn {
        /// The expression to execute in the spawned task
        expr: Heap<Expr>,
        /// Context requirements for the spawned task
        contexts: List<crate::decl::ContextRequirement>,
    },

    /// Inject expression: `inject TypeName`
    ///
    /// Level 1 static dependency injection. Resolves a type from the DI container.
    /// Cost: 0ns for Singleton (field access), ~3ns for Request, ~8ns for Transient.
    ///
    /// Grammar: inject_expr = 'inject' , type_path ;
    /// Spec: docs/detailed/16-context-system.md Section 1A
    Inject {
        /// The type to inject
        type_path: crate::ty::Path,
    },

    /// Async select expression: `select [biased] { arm1 => expr1, arm2 => expr2, default => expr3 }`
    ///
    /// Waits for one of multiple futures to complete and executes the corresponding arm.
    /// Similar to Go's select or Tokio's select! macro, but as a first-class expression.
    ///
    /// Spec: grammar/verum.ebnf - select_expr production
    /// Select expression for multiplexing over channels or async operations.
    ///
    /// Example:
    /// ```verum
    /// let result = select {
    ///     data = fetch_data().await => process(data),
    ///     _ = timeout(1000).await => Err("timeout"),
    ///     default => cached_value,
    /// };
    /// ```
    Select {
        /// Whether the select is biased (left-to-right priority evaluation)
        /// If true, earlier arms have higher priority when multiple futures are ready.
        biased: bool,
        /// The select arms (futures to await with their handlers)
        arms: List<SelectArm>,
        /// Span of the entire select expression
        span: Span,
    },

    /// Nursery expression: structured concurrency with guaranteed task completion
    ///
    /// Implements structured concurrency where all spawned tasks MUST complete
    /// before the nursery scope exits. If any task panics, all others are cancelled.
    ///
    /// Nursery block for structured concurrency with bounded task lifetimes.
    /// Grammar: grammar/verum.ebnf Section 2.12.3 - nursery_expr
    ///
    /// # Semantics
    /// - All tasks spawned within nursery must complete before nursery exits
    /// - If any task panics/errors, all other tasks are cancelled
    /// - Supports optional timeout and error recovery
    /// - Task results are collected and available after nursery
    ///
    /// # Examples
    /// ```verum
    /// // Basic nursery - all tasks complete before scope exits
    /// nursery {
    ///     let a = spawn fetch_a();
    ///     let b = spawn fetch_b();
    /// }
    ///
    /// // Nursery with timeout
    /// nursery(timeout: 5.seconds) {
    ///     let result = spawn fetch_data();
    /// } on_cancel {
    ///     cleanup();
    /// }
    ///
    /// // Nursery with error recovery
    /// nursery {
    ///     let a = spawn fetch_a();
    /// } recover {
    ///     TimeoutError => default_value,
    /// }
    /// ```
    Nursery {
        /// Optional nursery configuration
        options: NurseryOptions,
        /// The body block where tasks are spawned
        body: Block,
        /// Optional cancellation handler
        on_cancel: Maybe<Block>,
        /// Optional error recovery handler
        recover: Maybe<RecoverBody>,
        /// Source span
        span: Span,
    },

    /// Unsafe block: unsafe { ... }
    Unsafe(Block),

    /// Meta expression: meta { ... }
    Meta(Block),

    /// Quote expression for code generation: quote { token_tree } or quote(N) { token_tree }
    ///
    /// Quote expressions are used in meta functions to generate code at compile-time.
    /// The optional stage parameter specifies the target stage for N-level staged compilation.
    ///
    /// Syntax:
    /// - `quote { token_tree }` - Generate code for the default stage (N-1)
    /// - `quote(N) { token_tree }` - Generate code for stage N
    ///
    /// Within the token tree, interpolation is supported:
    /// - `$ident` - Splice the value of identifier
    /// - `${expr}` - Splice the result of expression
    /// - `$[for pattern in expr { ... }]` - Repetition
    /// - `$(stage N){ expr }` - Stage-specific escape
    ///
    /// Examples:
    /// ```verum
    /// meta fn generate_impl<T>() -> TokenStream {
    ///     quote {
    ///         implement Display for $T {
    ///             fn fmt(&self, f: &mut Formatter) -> FormatResult {
    ///                 f.write_str(stringify!($T))
    ///             }
    ///         }
    ///     }
    /// }
    ///
    /// // Stage-specific quote for multi-level staging
    /// meta(2) fn gen_stage_1() -> TokenStream {
    ///     quote(1) { fn generated() -> Int { 42 } }
    /// }
    /// ```
    ///
    /// Spec: grammar/verum.ebnf - quote_expr production
    /// Staged meta-programming expression for compile-time code generation.
    Quote {
        /// Target stage for code generation (None = default stage N-1)
        target_stage: Option<u32>,
        /// The token tree to be interpolated and generated
        tokens: List<TokenTree>,
    },

    /// Stage escape expression: $(stage N){ expr }
    ///
    /// Used within quote blocks to escape to a specific stage and evaluate
    /// an expression at that stage. The result is spliced back into the
    /// surrounding quoted code.
    ///
    /// This enables multi-stage computation where values computed at higher
    /// stages can be injected into lower-stage code generation.
    ///
    /// Syntax: `$(stage N){ expr }`
    ///
    /// Examples:
    /// ```verum
    /// // In a stage-2 function, generate stage-1 code with computed values
    /// meta(2) fn generate_family() -> TokenStream {
    ///     let count = compute_member_count();  // Evaluated at stage 2
    ///     quote {
    ///         meta fn derive_member() -> TokenStream {
    ///             // count is evaluated at stage 2 and injected here
    ///             let field_count = $(stage 2){ count };
    ///             quote { }
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// Spec: grammar/verum.ebnf - quote_stage_escape production
    /// Staged meta-programming expression for compile-time code generation.
    StageEscape {
        /// The stage at which to evaluate the expression
        stage: u32,
        /// The expression to evaluate at the specified stage
        expr: Heap<Expr>,
    },

    /// Lift expression: `lift(expr)` - syntactic sugar for `$(stage current){ expr }`
    ///
    /// Lifts a compile-time value into the generated code at the current stage.
    /// This is a convenience syntax for stage escapes where the stage level is
    /// the current stage (most common use case).
    ///
    /// ## Syntax
    /// ```verum
    /// lift(expr)
    /// ```
    ///
    /// ## Equivalent to
    /// ```verum
    /// $(stage current){ expr }
    /// ```
    ///
    /// ## Example
    /// ```verum
    /// meta fn generate_constant() -> Ast {
    ///     let value = 42;
    ///     quote {
    ///         let x = lift(value);  // Lifts compile-time value into generated code
    ///         x
    ///     }
    /// }
    /// ```
    ///
    /// Spec: grammar/verum.ebnf - quote_lift production
    /// Compile-time evaluation of meta expressions with staging support.
    Lift {
        /// The expression to lift into the current stage
        expr: Heap<Expr>,
    },

    /// Meta-level function expression: @file, @line, @cfg(debug), @const expr, etc.
    ///
    /// These are compile-time intrinsics that provide source location information,
    /// configuration checks, compile-time evaluation, and other meta-level features.
    ///
    /// Syntax: `@name` or `@name(args)`
    ///
    /// Examples:
    /// ```verum
    /// let file = @file;           // Source file name
    /// let line = @line;           // Source line number
    /// let col = @column;          // Source column
    /// if @cfg(debug) { ... }      // Configuration check
    /// const X = @const expr;      // Compile-time evaluation
    /// @stringify(identifier)      // Token stringification
    /// @concat("a", "b")           // Compile-time concatenation
    /// @warning("deprecated")      // Compile-time warning
    /// ```
    ///
    /// Spec: grammar/verum.ebnf Section 2.20.6 - Meta-Level Functions
    MetaFunction {
        /// Name of the meta-function (file, line, column, cfg, const, stringify, etc.)
        name: Ident,
        /// Optional arguments (for @cfg(cond), @const expr, @stringify(tokens), etc.)
        args: List<Expr>,
    },

    /// Macro invocation: path!(args)
    ///
    /// Represents compile-time macro expansion with token tree arguments.
    /// The macro system processes this during parsing/compilation.
    ///
    /// Syntax: `macro_name!(args)` or `path::to::macro!(args)`
    ///
    /// Examples:
    /// ```verum
    /// println!("Hello, world!")
    /// vec![1, 2, 3]
    /// assert_eq!(x, 42)
    /// ```
    ///
    /// Spec: grammar/verum.ebnf - meta_call production
    MacroCall {
        /// Path to the macro (e.g., println, vec, std::assert_eq)
        path: Path,
        /// Token tree arguments (unparsed tokens in delimiters)
        args: MacroArgs,
    },

    /// Context handler binding: use C = handler in expr
    ///
    /// This binds a context handler for context C within the scope of expr.
    /// The handler expression is invoked when the context C is triggered,
    /// and the body expression executes with the bound handler in scope.
    ///
    /// Syntax: `use ContextName = handler_expr in body_expr`
    ///
    /// Example:
    /// ```verum
    /// use State = state_handler in {
    ///     get() + 1
    /// }
    /// ```
    UseContext {
        /// The context being handled (e.g., State, IO, Error)
        context: Path,
        /// The handler expression that implements the context operations
        handler: Heap<Expr>,
        /// The expression in which the handler is active
        body: Heap<Expr>,
    },

    /// Type bound condition for compile-time context requirements: T: Bound
    ///
    /// Used in conditional context requirements like:
    /// `using [Validator if T: Validatable]`
    ///
    /// This is evaluated at compile-time to determine if the context is required.
    TypeBound {
        /// The type parameter (e.g., T)
        type_param: Ident,
        /// The bound type (e.g., Validatable)
        bound: Heap<crate::ty::Type>,
    },

    /// Range expression: a..b or a..=b
    Range {
        start: Maybe<Heap<Expr>>,
        end: Maybe<Heap<Expr>>,
        inclusive: bool,
    },

    /// Universal quantifier: ∀x: T. predicate(x)
    ///
    /// Used in dependent types and formal verification (v2.0+).
    /// Expresses that a predicate holds for all values of a type.
    ///
    /// Example:
    /// ```verum
    /// forall (x: Int) => x + 0 == x
    /// ```
    ///
    /// Type-level computation (v2.0+ planned): types that depend on values.
    /// Universal/existential quantifiers for proof terms (v2.0+ planned).
    Forall {
        /// Quantified variable bindings (supports multiple bindings)
        /// Each binding specifies pattern, optional type, optional domain, optional guard
        bindings: List<QuantifierBinding>,
        /// Predicate body (must be boolean-valued)
        body: Heap<Expr>,
    },

    /// Existential quantifier: ∃x: T. predicate(x) or ∃x ∈ S. predicate(x)
    ///
    /// Used in dependent types and formal verification (v2.0+).
    /// Expresses that there exists at least one value satisfying a predicate.
    ///
    /// Examples:
    /// ```verum
    /// exists (x: Int) => x * x == 4           // type-based
    /// exists x in items. x > 0               // collection-based
    /// exists x in items where x > 0. x < 10  // with guard
    /// ```
    ///
    /// Type-level computation (v2.0+ planned): types that depend on values.
    /// Universal/existential quantifiers for proof terms (v2.0+ planned).
    /// Quantifier expression for dependent types and proofs.
    Exists {
        /// Quantified variable bindings (supports multiple bindings)
        /// Each binding specifies pattern, optional type, optional domain, optional guard
        bindings: List<QuantifierBinding>,
        /// Predicate body (must be boolean-valued)
        body: Heap<Expr>,
    },

    /// Parenthesized expression for clarity
    Paren(Heap<Expr>),

    /// Destructuring assignment expression
    ///
    /// Allows assignment to tuple, record, or array patterns:
    /// - `(a, b) = (b, a);` - tuple swap
    /// - `Point { x, y } = compute_point();` - record destructuring
    /// - `[first, second, ..] = items;` - array destructuring
    /// - `(x, y) += (dx, dy);` - compound destructuring
    ///
    /// The pattern on the LHS is validated to only contain assignable targets:
    /// - Identifiers (must be mutable variables in scope)
    /// - Place expressions (field access, index, deref, tuple index)
    /// - Wildcards (values discarded)
    ///
    /// Nested destructuring is supported: `((a, b), c) = nested_tuple();`
    ///
    /// Algebraic effect handler expression (experimental).
    DestructuringAssign {
        /// The destructuring pattern (tuple, record, array with places)
        pattern: Pattern,
        /// Assignment operator (=, +=, -=, etc.)
        op: BinOp,
        /// Value expression on the right-hand side
        value: Heap<Expr>,
    },

    /// Pattern test expression: `value is Pattern` or `value is not Pattern`
    ///
    /// Tests whether a value matches a pattern, returning a boolean.
    /// This replaces Rust's `matches!(x, P)` macro with cleaner syntax.
    ///
    /// Examples:
    /// ```verum
    /// if x is Some(value) { ... }
    /// if result is not Err(_) { ... }
    /// let valid = response is Ok { status: 200 };
    /// ```
    ///
    /// Built-in function call: print, panic, assert, assert_eq, unreachable, join, select.
    Is {
        /// The expression to test
        expr: Heap<Expr>,
        /// The pattern to match against
        pattern: Pattern,
        /// Whether the test is negated (is not)
        negated: bool,
    },

    /// Context capability attenuation: context.attenuate(capabilities)
    ///
    /// Context access expression: reads from the current task-local context environment.
    ///
    /// Creates a restricted sub-context with reduced capabilities.
    /// The attenuated context can be used with `using [attenuated as OriginalType]`.
    ///
    /// Example:
    /// ```verum
    /// let read_only = Database.attenuate(Capability.ReadOnly);
    /// using [read_only as Database] {
    ///     db.query("SELECT * FROM users")  // OK
    ///     db.execute("DELETE FROM users")  // COMPILE ERROR
    /// }
    /// ```
    Attenuate {
        /// The context expression being attenuated
        context: Heap<Expr>,
        /// The capability set to restrict to
        capabilities: CapabilitySet,
    },

    /// Type property access: T.size, T.alignment, T.stride, T.min, T.max, T.bits, T.name
    ///
    /// Provides compile-time access to type metadata. This is the modern syntax
    /// that replaces the deprecated size_of<T>() function-style syntax.
    ///
    /// Example:
    /// ```verum
    /// let sz = Int.size;       // Size of Int in bytes
    /// let align = f64.alignment;  // Alignment requirement
    /// let name = MyType.name;  // Type name as Text
    /// let bits = u32.bits;     // Bit width (32)
    /// let min_val = i8.min;    // Minimum value (-128)
    /// let max_val = i8.max;    // Maximum value (127)
    /// let stride = [Int; 10].stride;  // Element stride in arrays
    /// ```
    TypeProperty {
        /// The type whose property is being accessed
        ty: Type,
        /// The property being accessed
        property: TypeProperty,
    },

    /// Type expression in expression position: List<Int>, Repository<User>
    ///
    /// This allows using a generic type in expression position, enabling patterns like:
    /// - Context method calls: `Repository<User>.find(id)`
    /// - Static method calls: `List<Int>.new()`
    /// - Associated function calls: `Map<Text, Int>.with_capacity(10)`
    ///
    /// The type checker resolves these to the appropriate method dispatch.
    ///
    /// # Examples
    /// ```verum
    /// // Context method call with generic context
    /// fn uses_user_repo() using [Repository<User>] {
    ///     Repository<User>.find(1)
    /// }
    ///
    /// // Static method on generic type
    /// let list = List<Int>.new();
    /// ```
    TypeExpr(Type),

    /// Inline assembly expression: @asm("template", operands, options)
    ///
    /// Provides escape hatch for platform-specific operations while maintaining
    /// type safety through operand constraints.
    ///
    /// # Syntax
    ///
    /// ```verum
    /// @asm(
    ///     "assembly template with {0}, {1}",
    ///     [
    ///         out("=r", result),
    ///         in("r", input),
    ///         inout("=r", &mut value),
    ///     ],
    ///     volatile, preserves_flags
    /// )
    /// ```
    ///
    /// # Template Syntax
    ///
    /// - `{0}`, `{1}`, etc. - positional operand references
    /// - `{name}` - named operand references
    /// - `{{` / `}}` - literal braces
    ///
    /// # Operand Constraints
    ///
    /// - `in("constraint", expr)` - input operand
    /// - `out("constraint", lvalue)` - output operand
    /// - `inout("constraint", lvalue)` - input/output operand
    /// - `inlateout("constraint", expr, lvalue)` - late output
    /// - `sym(symbol)` - symbolic operand
    /// - `const(expr)` - constant expression
    ///
    /// # Common Constraints
    ///
    /// - `"r"` - general purpose register
    /// - `"m"` - memory location
    /// - `"i"` - immediate integer
    /// - `"x"` - SSE register (x86)
    /// - `"v"` - vector register (ARM)
    ///
    /// Inline assembly expression for direct hardware access in systems programming.
    InlineAsm {
        /// Assembly template string with operand placeholders
        template: Text,
        /// Input and output operands
        operands: List<AsmOperand>,
        /// Options (volatile, preserves_flags, etc.)
        options: AsmOptions,
    },

    /// Calculational proof block: calc { expr == { by justification } expr ... }
    ///
    /// Enables equational reasoning chains in both proof and regular contexts.
    /// Each step relates expressions via a relation (==, <, <=, etc.) with a justification.
    CalcBlock(crate::decl::CalculationChain),

    /// Copattern body: defines a coinductive value by observation.
    ///
    /// Used as the body of a `cofix fn` to specify what each observation/destructor returns.
    /// Each arm maps an observation name to the expression that is returned when the
    /// observation is applied to the coinductive value.
    ///
    /// # Syntax
    /// ```verum
    /// cofix fn nats_from(n: Int) -> Stream<Int> {
    ///     .head => n,
    ///     .tail => nats_from(n + 1),
    /// }
    /// ```
    ///
    /// # Semantics
    /// A copattern body defines an element of a coinductive type by exhaustively
    /// specifying the result of every destructor/observation. The productivity
    /// checker (`verum_types::coinductive_analysis`) verifies that each recursive
    /// call is guarded by at least one observation (ensuring the stream is productive).
    ///
    /// Spec: grammar/verum.ebnf - copattern_body production
    CopatternBody {
        arms: List<CopatternArm>,
        span: Span,
    },
}

/// A single arm in a copattern body.
///
/// Binds an observation name (destructor) to the expression to evaluate when
/// that observation is applied to the coinductive value being defined.
///
/// # Syntax
/// ```text
/// .observation_name => expression
/// ```
///
/// # Example
/// `.head => n` — the `head` observation returns `n`
/// `.tail => nats_from(n + 1)` — the `tail` observation returns `nats_from(n + 1)`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CopatternArm {
    /// The observation/destructor name (without leading `.`)
    pub observation: Ident,
    /// Expression evaluated when this observation is applied
    pub body: Heap<Expr>,
    /// Source span covering `.observation => body`
    pub span: Span,
}

impl Spanned for CopatternArm {
    fn span(&self) -> Span {
        self.span
    }
}

// =============================================================================
// INLINE ASSEMBLY TYPES
// =============================================================================

/// Inline assembly operand.
///
/// Represents a single operand in an @asm expression, specifying how
/// data flows between Verum and the assembly code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AsmOperand {
    /// The operand direction and constraint
    pub kind: AsmOperandKind,
    /// Optional name for the operand (for named references in template)
    pub name: Maybe<Ident>,
    /// Source span
    pub span: Span,
}

/// Kind of assembly operand.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AsmOperandKind {
    /// Input operand: in("constraint", expr)
    In {
        constraint: AsmConstraint,
        expr: Heap<Expr>,
    },
    /// Output operand: out("constraint", lvalue)
    Out {
        constraint: AsmConstraint,
        /// Output location (must be assignable)
        place: Heap<Expr>,
        /// Whether to initialize the output location before use
        late: bool,
    },
    /// Input and output operand: inout("constraint", lvalue)
    InOut {
        constraint: AsmConstraint,
        /// Place to read from and write to
        place: Heap<Expr>,
    },
    /// Split input/output: inlateout("constraint", in_expr, out_place)
    InLateOut {
        constraint: AsmConstraint,
        /// Input expression
        in_expr: Heap<Expr>,
        /// Output place (assigned after all inputs consumed)
        out_place: Heap<Expr>,
    },
    /// Symbolic operand: sym(symbol)
    /// References a symbol (function, static) by address
    Sym {
        path: Path,
    },
    /// Constant expression: const(expr)
    /// Compile-time constant to embed in assembly
    Const {
        expr: Heap<Expr>,
    },
    /// Clobbers: clobber("register")
    /// Indicates registers modified but not used as operands
    Clobber {
        reg: Text,
    },
}

/// Assembly operand constraint.
///
/// Constraints specify which registers or memory locations can be used
/// for an operand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsmConstraint {
    /// The constraint string (e.g., "r", "m", "=r", "+r")
    pub constraint: Text,
    /// Source span for error reporting
    pub span: Span,
}

impl AsmConstraint {
    pub fn new(constraint: impl Into<Text>, span: Span) -> Self {
        Self {
            constraint: constraint.into(),
            span,
        }
    }

    /// Check if this is an output constraint (starts with '=' or '+')
    pub fn is_output(&self) -> bool {
        self.constraint.starts_with("=") || self.constraint.starts_with("+")
    }

    /// Check if this is an input/output constraint (starts with '+')
    pub fn is_inout(&self) -> bool {
        self.constraint.starts_with("+")
    }

    /// Check if this is a memory constraint
    pub fn is_memory(&self) -> bool {
        self.constraint.contains("m")
    }

    /// Check if this is a register constraint
    pub fn is_register(&self) -> bool {
        self.constraint.contains("r")
            || self.constraint.contains("a")  // specific register (eax)
            || self.constraint.contains("b")  // specific register (ebx)
            || self.constraint.contains("c")  // specific register (ecx)
            || self.constraint.contains("d")  // specific register (edx)
    }

    /// Get the base constraint without modifiers
    pub fn base(&self) -> Text {
        let s = self.constraint.as_str();
        let s = s.trim_start_matches('=');
        let s = s.trim_start_matches('+');
        let s = s.trim_start_matches('&');
        Text::from(s)
    }
}

/// Assembly options/flags.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsmOptions {
    /// Assembly may have observable side effects (prevents elimination)
    pub volatile: bool,
    /// Assembly does not modify the condition/flags register
    pub preserves_flags: bool,
    /// Assembly does not read memory
    pub nomem: bool,
    /// Assembly does not write memory (read-only memory access)
    pub readonly: bool,
    /// Assembly may unwind (for exception handling)
    pub may_unwind: bool,
    /// Pure assembly (no side effects, can be CSE'd)
    pub pure_asm: bool,
    /// Noreturn - assembly does not return
    pub noreturn: bool,
    /// Nostack - assembly does not use the stack
    pub nostack: bool,
    /// Assembly uses Intel syntax (default is AT&T on x86)
    pub intel_syntax: bool,
    /// Target-specific options
    pub raw_options: List<Text>,
    /// Source span
    pub span: Span,
}

impl AsmOptions {
    pub fn new(span: Span) -> Self {
        Self {
            span,
            ..Default::default()
        }
    }

    /// Create volatile assembly options (default for most asm)
    pub fn volatile(span: Span) -> Self {
        Self {
            volatile: true,
            span,
            ..Default::default()
        }
    }

    /// Check if assembly is purely computational (can be optimized)
    pub fn is_pure(&self) -> bool {
        self.pure_asm && !self.volatile && self.nomem
    }
}

impl Spanned for AsmOperand {
    fn span(&self) -> Span {
        self.span
    }
}

impl Spanned for AsmConstraint {
    fn span(&self) -> Span {
        self.span
    }
}

impl Spanned for AsmOptions {
    fn span(&self) -> Span {
        self.span
    }
}

// =============================================================================
// BINARY OPERATORS
// =============================================================================

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BinOp {
    // Arithmetic
    Add, // +
    Sub, // -
    Mul, // *
    Div, // /
    Rem, // %
    Pow, // **

    // Comparison
    Eq, // ==
    Ne, // !=
    Lt, // <
    Le, // <=
    Gt, // >
    Ge, // >=
    In, // in (containment check)

    // Logical
    And,   // &&
    Or,    // ||
    Imply, // -> (for formal proofs)
    Iff,   // <-> (biconditional, for formal proofs)

    // Collection
    Concat, // ++ (list/sequence concatenation)

    // Bitwise
    BitAnd, // &
    BitOr,  // |
    BitXor, // ^
    Shl,    // <<
    Shr,    // >>

    // Assignment
    Assign,       // =
    AddAssign,    // +=
    SubAssign,    // -=
    MulAssign,    // *=
    DivAssign,    // /=
    RemAssign,    // %=
    BitAndAssign, // &=
    BitOrAssign,  // |=
    BitXorAssign, // ^=
    ShlAssign,    // <<=
    ShrAssign,    // >>=
}

impl BinOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::Pow => "**",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::In => "in",
            BinOp::Concat => "++",
            BinOp::And => "&&",
            BinOp::Or => "||",
            BinOp::Imply => "->",
            BinOp::BitAnd => "&",
            BinOp::BitOr => "|",
            BinOp::BitXor => "^",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
            BinOp::Assign => "=",
            BinOp::AddAssign => "+=",
            BinOp::SubAssign => "-=",
            BinOp::MulAssign => "*=",
            BinOp::DivAssign => "/=",
            BinOp::RemAssign => "%=",
            BinOp::BitAndAssign => "&=",
            BinOp::BitOrAssign => "|=",
            BinOp::BitXorAssign => "^=",
            BinOp::ShlAssign => "<<=",
            BinOp::ShrAssign => ">>=",
            BinOp::Iff => "<->",
        }
    }

    pub fn is_assignment(&self) -> bool {
        matches!(
            self,
            BinOp::Assign
                | BinOp::AddAssign
                | BinOp::SubAssign
                | BinOp::MulAssign
                | BinOp::DivAssign
                | BinOp::RemAssign
                | BinOp::BitAndAssign
                | BinOp::BitOrAssign
                | BinOp::BitXorAssign
                | BinOp::ShlAssign
                | BinOp::ShrAssign
        )
    }

    pub fn is_comparison(&self) -> bool {
        matches!(
            self,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
        )
    }

    pub fn is_commutative(&self) -> bool {
        matches!(
            self,
            BinOp::Add
                | BinOp::Mul
                | BinOp::Eq
                | BinOp::Ne
                | BinOp::And
                | BinOp::Or
                | BinOp::BitAnd
                | BinOp::BitOr
                | BinOp::BitXor
        )
    }
}

impl std::fmt::Display for BinOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UnOp {
    /// Logical not: !x
    Not,
    /// Arithmetic negation: -x
    Neg,
    /// Bitwise not: ~x
    BitNot,
    /// Dereference: *x
    Deref,
    /// CBGR reference: &x (Tier 0 - managed, ~15ns overhead)
    Ref,
    /// CBGR mutable reference: &mut x
    RefMut,
    /// Checked reference: &checked x (Tier 1 - compiler-verified, 0ns overhead)
    RefChecked,
    /// Checked mutable reference: &checked mut x
    RefCheckedMut,
    /// Unsafe reference: &unsafe x (Tier 2 - manual proof, 0ns overhead)
    RefUnsafe,
    /// Unsafe mutable reference: &unsafe mut x
    RefUnsafeMut,
    /// Ownership reference: %x
    Own,
    /// Ownership mutable reference: %mut x
    OwnMut,
}

impl UnOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            UnOp::Not => "!",
            UnOp::Neg => "-",
            UnOp::BitNot => "~",
            UnOp::Deref => "*",
            UnOp::Ref => "&",
            UnOp::RefMut => "&mut",
            UnOp::RefChecked => "&checked",
            UnOp::RefCheckedMut => "&checked mut",
            UnOp::RefUnsafe => "&unsafe",
            UnOp::RefUnsafeMut => "&unsafe mut",
            UnOp::Own => "%",
            UnOp::OwnMut => "%mut",
        }
    }
}

impl std::fmt::Display for UnOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Array expression variants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ArrayExpr {
    /// List of elements: [a, b, c]
    List(List<Expr>),
    /// Repeated element: [x; n]
    Repeat {
        value: Heap<Expr>,
        count: Heap<Expr>,
    },
}

/// A clause in a comprehension (list or stream).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComprehensionClause {
    pub kind: ComprehensionClauseKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ComprehensionClauseKind {
    /// For clause: for x in iter
    For { pattern: Pattern, iter: Expr },
    /// If clause: if condition
    If(Expr),
    /// Let clause: let x = expr
    Let {
        pattern: Pattern,
        ty: Maybe<Type>,
        value: Expr,
    },
}

/// Quantifier binding for forall/exists expressions.
///
/// Supports three forms of quantification:
/// 1. Type-based:       `x: Int`           - variable ranges over type
/// 2. Collection-based: `x in items`       - variable ranges over collection elements
/// 3. Combined:         `x: Int in 0..100` - explicit type with bounded domain
///
/// Optional guard condition filters the domain: `x in items where x > 0`
///
/// Mathematical correspondence:
/// - `forall x: T. P(x)`              ≡ ∀x:T. P(x)
/// - `forall x in S. P(x)`            ≡ ∀x∈S. P(x)
/// - `forall x in S where Q(x). P(x)` ≡ ∀x∈S. Q(x) → P(x)
///
/// Quantifier expression for dependent types and proofs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuantifierBinding {
    /// The bound variable pattern
    pub pattern: Pattern,
    /// Optional explicit type annotation (`x: T`)
    /// If not provided with `in`, type is inferred from domain's element type
    pub ty: Maybe<Type>,
    /// Optional domain expression (`x in collection`)
    /// For bounded quantification over collections, ranges, etc.
    pub domain: Maybe<Expr>,
    /// Optional guard condition (`where cond`)
    /// Filters the domain: only elements satisfying the guard are quantified
    pub guard: Maybe<Expr>,
    /// Source span
    pub span: Span,
}

impl QuantifierBinding {
    /// Create a type-based quantifier binding: `x: T`
    pub fn typed(pattern: Pattern, ty: Type, span: Span) -> Self {
        Self {
            pattern,
            ty: Some(ty),
            domain: None,
            guard: None,
            span,
        }
    }

    /// Create a domain-based quantifier binding: `x in items`
    pub fn in_domain(pattern: Pattern, domain: Expr, span: Span) -> Self {
        Self {
            pattern,
            ty: None,
            domain: Some(domain),
            guard: None,
            span,
        }
    }

    /// Create a combined quantifier binding: `x: T in items where cond`
    pub fn full(
        pattern: Pattern,
        ty: Maybe<Type>,
        domain: Maybe<Expr>,
        guard: Maybe<Expr>,
        span: Span,
    ) -> Self {
        Self {
            pattern,
            ty,
            domain,
            guard,
            span,
        }
    }

    /// Returns true if this binding has a domain (bounded quantification)
    pub fn is_bounded(&self) -> bool {
        self.domain.is_some()
    }

    /// Returns true if this binding has a guard condition
    pub fn has_guard(&self) -> bool {
        self.guard.is_some()
    }
}

/// Field initialization in record expressions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldInit {
    /// Optional conditional compilation attributes (e.g., @cfg(feature = "async"))
    pub attributes: List<Attribute>,
    pub name: Ident,
    /// Optional value (None means shorthand: { x } = { x: x })
    pub value: Maybe<Expr>,
    pub span: Span,
}

impl FieldInit {
    pub fn new(name: Ident, value: Maybe<Expr>, span: Span) -> Self {
        Self {
            attributes: List::new(),
            name,
            value,
            span,
        }
    }

    pub fn with_attributes(
        attributes: List<Attribute>,
        name: Ident,
        value: Maybe<Expr>,
        span: Span,
    ) -> Self {
        Self {
            attributes,
            name,
            value,
            span,
        }
    }

    pub fn shorthand(name: Ident) -> Self {
        let span = name.span;
        Self {
            attributes: List::new(),
            name,
            value: Maybe::None,
            span,
        }
    }
}

impl Spanned for FieldInit {
    fn span(&self) -> Span {
        self.span
    }
}

/// Stream literal expression for lazy stream construction.
/// Lazy stream comprehension expression for deferred evaluation.
///
/// Creates lazy streams from either:
/// - Elements with optional infinite cycle marker (...)
/// - Range expressions for lazy ranges
///
/// Examples:
/// - `stream[1, 2, 3, ...]` -> cycles [1, 2, 3] infinitely
/// - `stream[0, 1, 2, ...]` -> count_from(0) if arithmetic sequence detected
/// - `stream[0..100]`       -> lazy range [0, 100)
/// - `stream[0..]`          -> infinite range from 0
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamLiteralExpr {
    /// The kind of stream literal
    pub kind: StreamLiteralKind,
    /// Source span
    pub span: Span,
}

/// Kind of stream literal expression
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StreamLiteralKind {
    /// Element list with optional infinite cycle: stream[1, 2, 3, ...]
    Elements {
        /// The elements in the stream
        elements: List<Expr>,
        /// Whether the stream cycles infinitely (has ...)
        cycles: bool,
    },
    /// Range-based stream: stream[0..100] or stream[0..]
    Range {
        /// Start of range
        start: Heap<Expr>,
        /// End of range (None for infinite: stream[0..])
        end: Maybe<Heap<Expr>>,
        /// Whether range is inclusive (..=)
        inclusive: bool,
    },
}

impl StreamLiteralExpr {
    /// Create an element-based stream literal
    pub fn elements(elements: List<Expr>, cycles: bool, span: Span) -> Self {
        Self {
            kind: StreamLiteralKind::Elements { elements, cycles },
            span,
        }
    }

    /// Create a range-based stream literal
    pub fn range(start: Expr, end: Maybe<Expr>, inclusive: bool, span: Span) -> Self {
        Self {
            kind: StreamLiteralKind::Range {
                start: Heap::new(start),
                end: end.map(Heap::new),
                inclusive,
            },
            span,
        }
    }
}

impl Spanned for StreamLiteralExpr {
    fn span(&self) -> Span {
        self.span
    }
}

/// A single arm in a select expression.
///
/// Represents either a future arm (`pattern = future.await [if guard] => body`)
/// or a default arm (`else => body`).
///
/// Select expression for multiplexing over channels or async operations.
/// Grammar: grammar/verum.ebnf v2.9 - select_arm
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectArm {
    /// Optional attributes on this arm (e.g., `@cold`, `@likely`)
    /// Select expression for multiplexing over channels or async operations. - Attributes on select arms
    pub attributes: List<crate::attr::Attribute>,
    /// The pattern to bind the future result (None for else/default arm)
    /// Supports full pattern matching: `Ok(data)`, `Message.Command { cmd, args }`, etc.
    pub pattern: Maybe<Pattern>,
    /// The future expression to await (None for else/default arm)
    pub future: Maybe<Heap<Expr>>,
    /// The body expression to execute when this arm is selected
    pub body: Heap<Expr>,
    /// Optional guard condition: `if condition`
    pub guard: Maybe<Heap<Expr>>,
    /// Source span
    pub span: Span,
}

impl SelectArm {
    /// Create a new future arm: `pattern = future.await [if guard] => body`
    ///
    /// # Arguments
    /// * `attributes` - Optional attributes on the arm (e.g., `@cold`, `@likely`)
    /// * `pattern` - Pattern to bind the awaited value
    /// * `future` - Future expression to await
    /// * `body` - Body expression executed when this arm is selected
    /// * `guard` - Optional guard condition
    /// * `span` - Source span
    pub fn new(
        attributes: List<crate::attr::Attribute>,
        pattern: Maybe<Pattern>,
        future: Maybe<Heap<Expr>>,
        body: Heap<Expr>,
        guard: Maybe<Heap<Expr>>,
        span: Span,
    ) -> Self {
        Self {
            attributes,
            pattern,
            future,
            body,
            guard,
            span,
        }
    }

    /// Create an else/default arm: `else => body`
    ///
    /// The else arm has no attributes, pattern, future, or guard.
    pub fn else_arm(body: Heap<Expr>, span: Span) -> Self {
        Self {
            attributes: List::new(),
            pattern: Maybe::None,
            future: Maybe::None,
            body,
            guard: Maybe::None,
            span,
        }
    }

    /// Create an else arm with attributes: `@attr else => body`
    ///
    /// Allows attributes on else arms for optimization hints.
    pub fn else_arm_with_attrs(
        attributes: List<crate::attr::Attribute>,
        body: Heap<Expr>,
        span: Span,
    ) -> Self {
        Self {
            attributes,
            pattern: Maybe::None,
            future: Maybe::None,
            body,
            guard: Maybe::None,
            span,
        }
    }

    /// Check if this is an else/default arm
    pub fn is_else(&self) -> bool {
        self.pattern.is_none() && self.future.is_none()
    }

    /// Check if this is a default arm (alias for is_else for backward compat)
    pub fn is_default(&self) -> bool {
        self.is_else()
    }
}

impl Spanned for SelectArm {
    fn span(&self) -> Span {
        self.span
    }
}

// ============================================================================
// Nursery Expression Types (Structured Concurrency)
// ============================================================================

/// Options for nursery expression configuration.
///
/// Nursery block for structured concurrency with bounded task lifetimes.
/// Grammar: grammar/verum.ebnf Section 2.12.3 - nursery_options
///
/// # Example
/// ```verum
/// nursery(timeout: 5.seconds, on_error: cancel_all, max_tasks: 10) {
///     // ...
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NurseryOptions {
    /// Optional timeout duration expression
    pub timeout: Maybe<Heap<Expr>>,
    /// Error handling behavior
    pub on_error: NurseryErrorBehavior,
    /// Maximum concurrent tasks (None = unlimited)
    pub max_tasks: Maybe<Heap<Expr>>,
    /// Source span of options
    pub span: Maybe<Span>,
}

impl Default for NurseryOptions {
    fn default() -> Self {
        Self {
            timeout: Maybe::None,
            on_error: NurseryErrorBehavior::CancelAll,
            max_tasks: Maybe::None,
            span: Maybe::None,
        }
    }
}

impl NurseryOptions {
    /// Create empty options (no timeout, cancel_all on error)
    pub fn new() -> Self {
        Self::default()
    }

    /// Create options with timeout
    pub fn with_timeout(timeout: Heap<Expr>, span: Span) -> Self {
        Self {
            timeout: Maybe::Some(timeout),
            on_error: NurseryErrorBehavior::CancelAll,
            max_tasks: Maybe::None,
            span: Maybe::Some(span),
        }
    }

    /// Set error behavior
    pub fn set_on_error(mut self, behavior: NurseryErrorBehavior) -> Self {
        self.on_error = behavior;
        self
    }

    /// Set max tasks
    pub fn set_max_tasks(mut self, max: Heap<Expr>) -> Self {
        self.max_tasks = Maybe::Some(max);
        self
    }

    /// Check if options are empty (all defaults)
    pub fn is_empty(&self) -> bool {
        self.timeout.is_none()
            && self.on_error == NurseryErrorBehavior::CancelAll
            && self.max_tasks.is_none()
    }
}

/// Error handling behavior for nursery.
///
/// Determines what happens when a task within the nursery fails.
///
/// Nursery block for structured concurrency with bounded task lifetimes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[derive(Default)]
pub enum NurseryErrorBehavior {
    /// Cancel all remaining tasks immediately when one fails (default)
    #[default]
    CancelAll,
    /// Wait for all tasks to complete even if some fail
    WaitAll,
    /// Fail fast - propagate error immediately without waiting
    FailFast,
}


/// A block of statements with an optional trailing expression.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub stmts: List<Stmt>,
    pub expr: Maybe<Heap<Expr>>,
    pub span: Span,
}

impl Block {
    pub fn new(stmts: List<Stmt>, expr: Maybe<Heap<Expr>>, span: Span) -> Self {
        Self { stmts, expr, span }
    }

    pub fn empty(span: Span) -> Self {
        Self {
            stmts: List::new(),
            expr: Maybe::None,
            span,
        }
    }
}

impl Spanned for Block {
    fn span(&self) -> Span {
        self.span
    }
}

/// If condition (expression or let-chain).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IfCondition {
    pub conditions: SmallVec<[ConditionKind; 2]>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ConditionKind {
    /// Boolean expression
    Expr(Expr),
    /// Let pattern: let pattern = expr
    Let { pattern: Pattern, value: Expr },
}

/// Closure parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClosureParam {
    pub pattern: Pattern,
    pub ty: Maybe<Type>,
    pub span: Span,
}

impl ClosureParam {
    pub fn new(pattern: Pattern, ty: Maybe<Type>, span: Span) -> Self {
        Self { pattern, ty, span }
    }
}

impl Spanned for ClosureParam {
    fn span(&self) -> Span {
        self.span
    }
}

/// Body of a recover block - either match arms or closure syntax.
///
/// The recover block can use two syntaxes:
/// 1. Match arms: `recover { pattern => expr, ... }`
/// 2. Closure: `recover |e| expr` or `recover |e: ErrorType| { ... }`
///
/// try/recover/finally structured error handling with pattern matching on error types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RecoverBody {
    /// Match arms syntax: `recover { SomeError(msg) => handle(msg), _ => default() }`
    MatchArms {
        arms: List<MatchArm>,
        span: Span,
    },
    /// Closure syntax: `recover |e| handle_error(e)`
    Closure {
        param: RecoverClosureParam,
        body: Heap<Expr>,
        span: Span,
    },
}

impl Spanned for RecoverBody {
    fn span(&self) -> Span {
        match self {
            RecoverBody::MatchArms { span, .. } => *span,
            RecoverBody::Closure { span, .. } => *span,
        }
    }
}

/// Parameter for recover closure syntax: `|e|` or `|e: ErrorType|`
///
/// Similar to `ClosureParam` but specific to recover closures.
/// The pattern can be a simple identifier or a more complex destructuring pattern.
///
/// try/recover/finally structured error handling with pattern matching on error types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecoverClosureParam {
    /// The pattern to bind the error value
    pub pattern: Pattern,
    /// Optional type annotation for the error
    pub ty: Maybe<Type>,
    /// Source span
    pub span: Span,
}

impl RecoverClosureParam {
    pub fn new(pattern: Pattern, ty: Maybe<Type>, span: Span) -> Self {
        Self { pattern, ty, span }
    }
}

impl Spanned for RecoverClosureParam {
    fn span(&self) -> Span {
        self.span
    }
}

/// Reference kind for CBGR tier selection
///
/// CBGR (Capability-Based Generational References) expression for memory safety.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ReferenceKind {
    /// &T - CBGR-managed with full checks (~15ns)
    #[default]
    Managed,
    /// &checked T - Compiler-proven safe (0ns)
    Checked,
    /// &unsafe T - Manual safety proof required (0ns)
    Unsafe,
}

/// Macro invocation arguments with delimiter type.
///
/// Represents the token tree arguments to a macro invocation,
/// along with the delimiter used (parentheses, brackets, or braces).
///
/// Spec: grammar/verum.ebnf - meta_call_args production
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MacroArgs {
    /// The delimiter type used for the arguments
    pub delimiter: MacroDelimiter,
    /// The raw token tree content (unparsed)
    pub tokens: Text,
    /// Span covering the entire argument list including delimiters
    pub span: Span,
}

impl MacroArgs {
    pub fn new(delimiter: MacroDelimiter, tokens: Text, span: Span) -> Self {
        Self {
            delimiter,
            tokens,
            span,
        }
    }
}

impl Spanned for MacroArgs {
    fn span(&self) -> Span {
        self.span
    }
}

/// Delimiter type for macro arguments.
///
/// Spec: grammar/verum.ebnf - meta_call_args production
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MacroDelimiter {
    /// Parentheses: macro!(...)
    Paren,
    /// Square brackets: macro![...]
    Bracket,
    /// Curly braces: macro!{...}
    Brace,
}

/// A single element in a token tree.
///
/// Token trees are the fundamental structure for macro arguments in Verum.
/// They preserve the structure of the source code while allowing macros
/// to process tokens at a lower level than the parsed AST.
///
/// Spec: grammar/verum.ebnf - token_tree production
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TokenTree {
    /// A single token (identifier, literal, operator, etc.)
    Token(TokenTreeToken),
    /// A delimited group of tokens: (...), [...], or {...}
    Group {
        delimiter: MacroDelimiter,
        tokens: List<TokenTree>,
        span: Span,
    },
}

impl TokenTree {
    /// Get the span of this token tree
    pub fn span(&self) -> Span {
        match self {
            TokenTree::Token(tok) => tok.span,
            TokenTree::Group { span, .. } => *span,
        }
    }

    /// Check if this is a single token
    pub fn is_token(&self) -> bool {
        matches!(self, TokenTree::Token(_))
    }

    /// Check if this is a grouped token tree
    pub fn is_group(&self) -> bool {
        matches!(self, TokenTree::Group { .. })
    }

    /// Get the delimiter if this is a group
    pub fn delimiter(&self) -> Option<MacroDelimiter> {
        match self {
            TokenTree::Group { delimiter, .. } => Some(*delimiter),
            _ => None,
        }
    }

    /// Convert token tree to a string representation (for backward compatibility)
    pub fn to_text(&self) -> Text {
        match self {
            TokenTree::Token(tok) => tok.text.clone(),
            TokenTree::Group {
                delimiter, tokens, ..
            } => {
                let (open, close) = match delimiter {
                    MacroDelimiter::Paren => ("(", ")"),
                    MacroDelimiter::Bracket => ("[", "]"),
                    MacroDelimiter::Brace => ("{", "}"),
                };
                let inner: Vec<String> = tokens.iter().map(|t| t.to_text().to_string()).collect();
                Text::from(format!("{}{}{}", open, inner.join(" "), close))
            }
        }
    }
}

/// A single token within a token tree.
///
/// This is a simplified representation of tokens that preserves
/// the essential information needed for macro processing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TokenTreeToken {
    /// The kind of token (identifier, literal, punctuation)
    pub kind: TokenTreeKind,
    /// The textual representation of the token
    pub text: Text,
    /// Source span
    pub span: Span,
}

impl TokenTreeToken {
    pub fn new(kind: TokenTreeKind, text: Text, span: Span) -> Self {
        Self { kind, text, span }
    }
}

/// The kind of token in a token tree.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TokenTreeKind {
    /// An identifier (variable name, keyword, etc.)
    Ident,
    /// An integer literal
    IntLiteral,
    /// A floating-point literal
    FloatLiteral,
    /// A string literal
    StringLiteral,
    /// A character literal
    CharLiteral,
    /// A boolean literal (true/false)
    BoolLiteral,
    /// Punctuation or operator
    Punct,
    /// A keyword
    Keyword,
    /// End of token stream marker
    Eof,
}

/// Extended macro arguments with full token tree support.
///
/// This structure holds both the raw text and the parsed token tree,
/// allowing macros to work with either representation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MacroArgsExt {
    /// The delimiter type used for the arguments
    pub delimiter: MacroDelimiter,
    /// The raw token content as text (for simple macros)
    pub raw_tokens: Text,
    /// The parsed token tree (for advanced macro processing)
    pub token_tree: List<TokenTree>,
    /// Span covering the entire argument list including delimiters
    pub span: Span,
}

impl MacroArgsExt {
    pub fn new(
        delimiter: MacroDelimiter,
        raw_tokens: Text,
        token_tree: List<TokenTree>,
        span: Span,
    ) -> Self {
        Self {
            delimiter,
            raw_tokens,
            token_tree,
            span,
        }
    }

    /// Convert to legacy MacroArgs for backward compatibility
    pub fn to_legacy(&self) -> MacroArgs {
        MacroArgs::new(self.delimiter, self.raw_tokens.clone(), self.span)
    }

    /// Create from legacy MacroArgs (token tree will be empty)
    pub fn from_legacy(args: MacroArgs) -> Self {
        Self {
            delimiter: args.delimiter,
            raw_tokens: args.tokens,
            token_tree: List::new(),
            span: args.span,
        }
    }
}

impl Spanned for MacroArgsExt {
    fn span(&self) -> Span {
        self.span
    }
}

// Forward declaration - actual Stmt is in stmt.rs
pub use crate::stmt::Stmt;
