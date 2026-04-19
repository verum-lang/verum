//! Declaration nodes in the AST.
//!
//! This module defines top-level declarations including:
//! - Functions
//! - Types (records, variants, newtypes, aliases)
//! - Protocols (traits)
//! - Implementations
//! - Modules
//! - Constants and statics
//! - Contexts and context groups

pub use crate::attr::{Attribute, FeatureAttr, ProfileAttr};
use crate::expr::{Block, Expr};
use crate::pattern::Pattern;
use crate::span::{Span, Spanned};
use crate::ty::{GenericParam, Ident, Path, Type, WhereClause};
use serde::{Deserialize, Serialize};
use verum_common::{Heap, List, Maybe, Text};

/// A top-level item/declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub kind: ItemKind,
    pub attributes: List<Attribute>,
    pub span: Span,
}

impl Item {
    pub fn new(kind: ItemKind, span: Span) -> Self {
        Self {
            kind,
            attributes: List::new(),
            span,
        }
    }

    pub fn new_with_attrs(kind: ItemKind, attributes: List<Attribute>, span: Span) -> Self {
        Self {
            kind,
            attributes,
            span,
        }
    }
}

impl Spanned for Item {
    fn span(&self) -> Span {
        self.span
    }
}

/// The kind of top-level item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ItemKind {
    /// Function definition
    Function(FunctionDecl),
    /// Type definition
    Type(TypeDecl),
    /// Protocol definition
    Protocol(ProtocolDecl),
    /// Implementation block
    Impl(ImplDecl),
    /// Module definition
    Module(ModuleDecl),
    /// Const definition
    Const(ConstDecl),
    /// Static definition
    Static(StaticDecl),
    /// Mount statement
    Mount(MountDecl),
    /// Meta definition (macro)
    Meta(MetaDecl),
    /// Predicate definition (named refinement type predicate)
    Predicate(PredicateDecl),
    /// Context declaration
    Context(ContextDecl),
    /// Context group declaration
    ContextGroup(ContextGroupDecl),
    /// Context layer declaration — composable context bundle.
    /// Grammar: layer_def = visibility 'layer' identifier layer_body
    Layer(LayerDecl),
    /// FFI boundary declaration (compile-time specification, not a type).
    /// Formalizes expectations at the boundary between provable Verum code and
    /// unprovable external code. Only C ABI is supported.
    FFIBoundary(crate::ffi::FFIBoundary),

    // ==================== Formal Proofs (v2.0+ extension) ====================
    /// Theorem declaration: `theorem name(params): proposition { proof_term }`.
    /// Produces machine-checkable proofs via induction, tactics, or SMT integration.
    Theorem(TheoremDecl),

    /// Lemma declaration (helper theorem, same syntax as theorem).
    Lemma(TheoremDecl),

    /// Corollary declaration (consequence of theorem, same syntax).
    Corollary(TheoremDecl),

    /// Axiom declaration (unproven assumption)
    /// Used for declaring fundamental assumptions in the system
    Axiom(AxiomDecl),

    /// Tactic declaration: custom proof automation strategy.
    /// Tactics compose proof steps (assumption, intro, split, apply, etc.)
    /// for automated proof search.
    Tactic(TacticDecl),

    /// View declaration: alternative pattern matching interface (v2.0+ planned).
    /// Views provide alternative destructuring of data, e.g., `view Parity : Nat -> Type`
    /// with constructors `Even(n)` and `Odd(n)` for computing parity.
    View(ViewDecl),

    /// Extern block declaration - groups FFI functions with a common ABI.
    /// Only C ABI is supported. Example: `extern "C" { fn foo(); fn bar(); }`
    ExternBlock(ExternBlockDecl),

    /// Active pattern declaration (F#-style custom pattern matchers).
    ///
    /// # Examples
    /// ```verum
    /// // Simple active pattern
    /// pattern Even(n: Int) -> Bool = n % 2 == 0;
    ///
    /// // Parameterized active pattern
    /// pattern InRange(lo: Int, hi: Int)(n: Int) -> Bool = lo <= n <= hi;
    ///
    /// // Partial active pattern
    /// pattern ParseInt(s: Text) -> Maybe<Int> = s.parse_int();
    /// ```
    Pattern(PatternDecl),
}

/// A function declaration.
///
/// # Syntax Order
/// ```text
/// @std(ContextGroup)?
/// fn name<T>(params) -> ReturnType
///     using [Context1, Context2]       // Context clause (optional)
///     where type T: Protocol           // Generic constraints (optional)
///     where meta N > 0                 // Meta constraints (optional)
///     requires EXPR                    // Preconditions (optional, repeatable)
///     ensures EXPR                     // Postconditions (optional, repeatable)
/// {
///     body
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionDecl {
    pub visibility: Visibility,
    pub is_async: bool,

    /// Meta function flag - indicates compile-time execution.
    ///
    /// # Staged Metaprogramming
    ///
    /// Verum supports N-level staged metaprogramming where functions execute
    /// at different compilation stages:
    ///
    /// - **Stage 0**: Runtime execution (normal functions, `is_meta = false`)
    /// - **Stage 1**: Compile-time execution (`meta fn`, most common)
    /// - **Stage N**: N-th level meta (`meta(N) fn`, generates Stage N-1 code)
    ///
    /// # Stage Semantics
    ///
    /// A Stage N function generates code for Stage N-1. This creates a
    /// compilation cascade:
    ///
    /// ```text
    /// meta(3) fn → generates → meta(2) fn → generates → meta fn → generates → runtime fn
    /// Stage 3        →        Stage 2        →        Stage 1      →        Stage 0
    /// ```
    ///
    /// # Examples
    ///
    /// ```verum
    /// // Stage 1: generates runtime code at compile time
    /// meta fn derive_eq<T>() -> TokenStream { ... }
    ///
    /// // Stage 2: generates Stage 1 (meta) functions
    /// meta(2) fn create_derivation_family() -> TokenStream {
    ///     quote {
    ///         meta fn derive_X<T>() { ... }
    ///     }
    /// }
    ///
    /// // Stage 3: meta-meta-programming (rare but powerful)
    /// meta(3) fn domain_compiler() { ... }
    /// ```
    ///
    /// # Stage Coherence Rule
    ///
    /// A Stage N function can only DIRECTLY generate Stage N-1 code.
    /// To generate lower-stage code, the output must contain meta functions
    /// that perform further generation.
    ///
    /// See also: `stage_level` field for the numeric stage.
    pub is_meta: bool,

    /// Stage level for multi-stage metaprogramming.
    ///
    /// # Values
    ///
    /// - `0`: Runtime function (default, `is_meta = false`)
    /// - `1`: Standard meta function (`meta fn`, `is_meta = true`)
    /// - `N`: N-th level meta (`meta(N) fn`, `is_meta = true`, N ≥ 2)
    ///
    /// # Invariants
    ///
    /// - If `is_meta = false`, then `stage_level = 0`
    /// - If `is_meta = true` and no explicit level, then `stage_level = 1`
    /// - If `is_meta = true` with explicit `meta(N)`, then `stage_level = N`
    ///
    /// # Quote Target Stage
    ///
    /// Inside a Stage N function, `quote { ... }` targets Stage N-1 by default.
    /// Use `quote(M) { ... }` to target explicit Stage M where M < N.
    ///
    /// # Type Checking
    ///
    /// The stage checker (`StageChecker`) enforces:
    /// - No cross-stage value leakage
    /// - Proper stage coherence
    /// - Valid quote target stages
    pub stage_level: u32,

    /// Pure function - no side effects, compiler-verified
    /// Spec: grammar/verum.ebnf v2.12 - function_modifiers
    /// Examples: `pure fn add(a: Int, b: Int) -> Int`
    pub is_pure: bool,
    /// Generator function (fn*) - produces values lazily via yield
    /// Spec: grammar/verum.ebnf v2.10 - fn_keyword with optional '*'
    /// Examples: `fn* range()`, `async fn* stream()`
    pub is_generator: bool,
    /// Coinductive function (cofix) - allows infinite productive recursive definitions.
    /// Ensures termination via productivity checking: each recursive call must produce
    /// a constructor before recurring. Used for infinite data structures like streams.
    pub is_cofix: bool,
    /// Unsafe function - bypasses Verum's safety guarantees.
    /// Required for FFI calls, raw pointer manipulation, and other operations where
    /// the compiler cannot verify safety. Examples: `unsafe fn raw_access()`
    pub is_unsafe: bool,

    /// Transparent meta function - disables hygienic macro expansion.
    ///
    /// # Hygiene Semantics
    ///
    /// By default, meta functions (macros) in Verum use **hygienic expansion**:
    /// - Identifiers in `quote { ... }` are gensym'd (renamed with unique suffixes)
    /// - This prevents accidental variable capture from the expansion site
    /// - The macro's internal bindings don't leak to callers
    ///
    /// When `@transparent` is applied to a meta function:
    /// - Identifiers in `quote { ... }` are NOT renamed
    /// - The macro can intentionally capture variables from the expansion site
    /// - M402 (Accidental Capture) errors are enabled for safety
    ///
    /// # Use Cases
    ///
    /// - **Anaphoric macros**: `@aif(cond) { ... }` that bind `it` to the condition result
    /// - **DSL builders**: Where explicit capture is part of the design
    /// - **Code generation**: That needs exact identifier matching
    ///
    /// # Examples
    ///
    /// ```verum
    /// // Hygienic (default) - 'x' is gensym'd, no capture possible
    /// meta fn hygienic_macro() -> TokenStream {
    ///     quote { let x = 1; x + 1 }  // x becomes x_gensym_123
    /// }
    ///
    /// // Transparent - 'x' is NOT gensym'd, captures from expansion site
    /// @transparent
    /// meta fn aif(cond: Expr) -> TokenStream {
    ///     quote {
    ///         let it = $cond;        // 'it' captures into caller scope
    ///         if it { ... }
    ///     }
    /// }
    /// ```
    ///
    /// # Hygiene Checks
    ///
    /// For `@transparent` macros, the compiler checks:
    /// - M402: Bare identifiers that might accidentally capture
    /// - M408: Undeclared captures (meta bindings used without $var or lift())
    ///
    /// # Related
    ///
    /// - `is_meta`: Whether this is a meta function
    /// - `stage_level`: Compilation stage for multi-stage metaprogramming
    pub is_transparent: bool,

    /// FFI external function with optional ABI (e.g., "C", "system").
    /// Only C ABI is stable. Examples: `extern fn foo()`, `extern "C" fn bar()`
    pub extern_abi: Maybe<Text>,
    /// Variadic function - accepts variable number of arguments (FFI only).
    /// Example: `extern "C" fn printf(format: *const c_char, ...) -> c_int`
    pub is_variadic: bool,
    pub name: Ident,
    pub generics: List<GenericParam>,
    pub params: List<FunctionParam>,
    pub return_type: Maybe<Type>,

    /// Throws clause specifying error types the function can throw
    /// Spec: grammar/verum.ebnf v2.8 - throws_clause production
    /// Example: `fn parse(input: Text) throws(ParseError | ValidationError) -> AST`
    pub throws_clause: Maybe<ThrowsClause>,

    /// @std attribute for automatic context provisioning.
    /// Provides a context group automatically for rapid development.
    /// Examples:
    /// - `@std` (uses ApplicationContext)
    /// - `@std(ServerContext)` (uses specified context group)
    pub std_attr: Maybe<crate::attr::StdAttr>,

    /// Context requirements (using clause). Declares required contexts after return type.
    /// Examples:
    /// - `using [Database, Logger]` (multiple contexts - brackets required)
    /// - `using Database` (single context - brackets optional)
    /// - `using WebContext` (context group - brackets optional)
    pub contexts: List<ContextRequirement>,

    /// Generic type constraints (where type clause).
    /// Example: `where type T: Ord, type U: Display`
    pub generic_where_clause: Maybe<WhereClause>,

    /// Meta-parameter constraints (where meta clause).
    /// Example: `where meta N > 0, meta M > 0`
    pub meta_where_clause: Maybe<WhereClause>,

    /// Preconditions (requires clauses)
    /// Example: `requires b != 0`
    pub requires: List<Expr>,

    /// Postconditions (ensures clauses)
    /// Example: `ensures result * b == a`
    pub ensures: List<Expr>,

    pub attributes: List<Attribute>,
    pub body: Maybe<FunctionBody>,
    pub span: Span,
}

impl FunctionDecl {
    pub fn is_method(&self) -> bool {
        self.params.first().is_some_and(|p| p.is_self())
    }

    /// Check if this is an async generator function (async fn*)
    /// Async generators return AsyncIterator<Item = T>
    /// Spec: grammar/verum.ebnf v2.10 - Async Generators
    pub fn is_async_generator(&self) -> bool {
        self.is_async && self.is_generator
    }

    /// Check if this is a sync generator function (fn* without async)
    /// Sync generators return Iterator<Item = T>
    /// Spec: grammar/verum.ebnf v2.10 - Sync Generators
    pub fn is_sync_generator(&self) -> bool {
        !self.is_async && self.is_generator
    }
}

impl Spanned for FunctionDecl {
    fn span(&self) -> Span {
        self.span
    }
}

/// Function parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionParam {
    pub kind: FunctionParamKind,
    #[serde(default)]
    pub attributes: List<Attribute>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FunctionParamKind {
    /// Regular parameter: name: Type [= default_value]
    /// Supports optional default values for parameters
    Regular {
        pattern: Pattern,
        ty: Type,
        /// Optional default value for the parameter
        #[serde(default)]
        default_value: Maybe<Expr>,
    },
    /// Self parameter: self
    SelfValue,
    /// Mutable self parameter: mut self (owned, mutable binding)
    SelfValueMut,
    /// CBGR reference self: &self
    SelfRef,
    /// CBGR mutable reference self: &mut self
    SelfRefMut,
    /// CBGR checked reference self: &checked self (Tier 1, escape-analysis proven safe)
    SelfRefChecked,
    /// CBGR checked mutable reference self: &checked mut self
    SelfRefCheckedMut,
    /// CBGR unsafe reference self: &unsafe self (Tier 2, manual safety proof required)
    SelfRefUnsafe,
    /// CBGR unsafe mutable reference self: &unsafe mut self
    SelfRefUnsafeMut,
    /// Ownership reference self: %self
    SelfOwn,
    /// Ownership mutable reference self: %mut self
    SelfOwnMut,
}

impl FunctionParam {
    /// Create a new function parameter with no attributes.
    pub fn new(kind: FunctionParamKind, span: Span) -> Self {
        Self {
            kind,
            attributes: List::new(),
            span,
        }
    }

    /// Create a new function parameter with attributes.
    pub fn with_attributes(
        kind: FunctionParamKind,
        attributes: List<Attribute>,
        span: Span,
    ) -> Self {
        Self {
            kind,
            attributes,
            span,
        }
    }

    pub fn is_self(&self) -> bool {
        matches!(
            self.kind,
            FunctionParamKind::SelfValue
                | FunctionParamKind::SelfValueMut
                | FunctionParamKind::SelfRef
                | FunctionParamKind::SelfRefMut
                | FunctionParamKind::SelfRefChecked
                | FunctionParamKind::SelfRefCheckedMut
                | FunctionParamKind::SelfRefUnsafe
                | FunctionParamKind::SelfRefUnsafeMut
                | FunctionParamKind::SelfOwn
                | FunctionParamKind::SelfOwnMut
        )
    }
}

impl Spanned for FunctionParam {
    fn span(&self) -> Span {
        self.span
    }
}

/// Function body (block or single expression).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FunctionBody {
    /// Block body: { stmts }
    Block(Block),
    /// Expression body: = expr;
    Expr(Expr),
}

/// A throws clause specifying error types a function can throw.
///
/// Throws clauses declare the error types that a function may propagate.
/// This enables explicit error type tracking and inference of the Fallible
/// computational property.
///
/// # Syntax (Spec: grammar/verum.ebnf v2.8)
/// ```text
/// throws_clause = 'throws' , '(' , error_type_list , ')' ;
/// error_type_list = type_expr , { '|' , type_expr } ;
/// ```
///
/// # Example
/// ```verum
/// fn parse(input: Text) throws(ParseError | ValidationError) -> AST {
///     // function body
/// }
/// ```
///
/// # Computational Properties
///
/// A function with a throws clause has the `Fallible` computational property,
/// meaning it may fail and propagate errors. This is tracked at compile-time
/// for effect inference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThrowsClause {
    /// The error types that can be thrown, separated by `|`
    /// Example: `throws(ParseError | ValidationError)` has two error types
    pub error_types: List<Type>,
    /// Source span
    pub span: Span,
}

impl ThrowsClause {
    /// Create a new throws clause with the given error types
    pub fn new(error_types: List<Type>, span: Span) -> Self {
        Self { error_types, span }
    }

    /// Check if this throws clause has any error types
    pub fn has_errors(&self) -> bool {
        !self.error_types.is_empty()
    }

    /// Get the number of error types
    pub fn error_count(&self) -> usize {
        self.error_types.len()
    }
}

impl Spanned for ThrowsClause {
    fn span(&self) -> Span {
        self.span
    }
}

/// A predicate declaration for named refinement type predicates.
///
/// Predicates are reusable boolean expressions that can be used in refinement types.
/// They define constraints that values of a type must satisfy.
///
/// # Example
/// ```verum
/// predicate NonZero(x: Int) -> Bool { x != 0 }
/// predicate Positive(x: Float) -> Bool { x > 0.0 }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PredicateDecl {
    pub visibility: Visibility,
    pub name: Ident,
    pub generics: List<crate::ty::GenericParam>,
    pub params: List<FunctionParam>,
    pub return_type: Type,
    pub body: Heap<Expr>,
    pub span: Span,
}

impl Spanned for PredicateDecl {
    fn span(&self) -> Span {
        self.span
    }
}

// Context types are defined in the `context` module to avoid circular dependencies
// and allow use in both `decl` and `ty` modules.
pub use crate::context::{ContextList, ContextRequirement, ContextTransform};

/// Resource modifier for type declarations.
///
/// Resource modifiers control how values of a type can be used and ensure
/// compile-time safety for resource management.
///
/// # Specification
///
/// Affine types provide compile-time resource safety guarantees:
/// - Values MUST be consumed at most once
/// - Prevents resource leaks (files, network connections, etc.)
/// - Zero runtime overhead (single-use proven statically)
///
/// Type Checking Rule:
/// ```text
/// Γ, x: τ^affine ⊢ e : U    x used at most once in e
/// ────────────────────────────────────────────────────
/// Γ ⊢ let x: τ^affine = e₁ in e₂ : U
/// ```
///
/// # Examples
///
/// ```verum
/// type affine FileHandle is {
///     fd: Int,
///     path: Path,
/// }
///
/// fn process_file(path: Path) -> Result<Data> {
///     let handle = FileHandle.open(path)?;  // Affine value
///     let data = handle.read_all()?;         // handle consumed
///     // handle.cleanup() called automatically - GUARANTEED
///     Ok(data)
/// }
/// ```
///
/// Error case:
/// ```verum
/// fn leak_file(path: Path) {
///     let handle = FileHandle.open(path)?;
///     let data1 = handle.read()?;  // First use - OK
///     let data2 = handle.read()?;  // ERROR: affine value used more than once
/// }
/// ```
///
/// Affine types can be used at most once (moved or dropped).
/// Linear types must be used exactly once (compile error if dropped unused).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResourceModifier {
    /// Affine type: use at most once
    ///
    /// Values can be:
    /// - Used once (moved/consumed)
    /// - Not used (dropped with cleanup)
    /// - Never used more than once (compile error)
    Affine,

    /// Linear type: use exactly once (future feature)
    ///
    /// Values must be:
    /// - Used exactly once (moved/consumed)
    /// - Never dropped without use (compile error)
    /// - Never used more than once (compile error)
    Linear,
}

impl ResourceModifier {
    /// Check if this modifier allows at most one use
    pub fn is_at_most_once(&self) -> bool {
        matches!(self, ResourceModifier::Affine | ResourceModifier::Linear)
    }

    /// Check if this modifier requires exactly one use
    pub fn is_exactly_once(&self) -> bool {
        matches!(self, ResourceModifier::Linear)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ResourceModifier::Affine => "affine",
            ResourceModifier::Linear => "linear",
        }
    }
}

impl std::fmt::Display for ResourceModifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A type declaration.
///
/// # Unified 'is' Syntax
///
/// All type definitions use the unified `type ... is` pattern:
/// ```text
/// type [affine] Name<T> where meta N > 0 is Body;
/// ```
///
/// # Resource Modifiers
///
/// Type declarations can have resource modifiers that control how values
/// of the type are used and managed:
///
/// - `affine`: Values can be used at most once (prevents double-free, use-after-move)
/// - `linear` (future): Values must be used exactly once
///
/// # Examples
///
/// ```verum
/// // Affine type - use at most once
/// type affine FileHandle is {
///     fd: Int,
/// }
///
/// // Type with meta constraints
/// type Matrix<M: meta usize, N: meta usize>
///     where meta M > 0, meta N > 0
/// is {
///     data: [[Float; N]; M]
/// }
///
/// fn read_file(handle: FileHandle) -> Text {
///     // handle consumed here
/// }
/// ```
///
/// # Specification
///
/// Supports affine types (use at most once) and linear types (use exactly once).
/// Type declarations use unified 'is' syntax: type Name is { fields } or type Name is A | B.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeDecl {
    pub visibility: Visibility,
    pub name: Ident,
    pub generics: List<GenericParam>,
    pub attributes: List<Attribute>,
    pub body: TypeDeclBody,

    /// Resource safety modifier
    /// - None: Normal type (can be used multiple times)
    /// - Some(Affine): Affine type (at most once use)
    /// - Some(Linear): Linear type (exactly once use) - future feature
    pub resource_modifier: Maybe<ResourceModifier>,

    /// Generic constraints (where type clause before 'is')
    /// Unified type definition using the 'is' keyword.
    /// Example: `type DedupIter<I: Iterator> where I.Item: Eq is { ... }`
    pub generic_where_clause: Maybe<WhereClause>,

    /// Meta-parameter constraints (where meta clause)
    /// Unified type definition using the 'is' keyword.
    /// Example: `type Matrix<M, N> where meta M > 0, meta N > 0 is { ... }`
    pub meta_where_clause: Maybe<WhereClause>,

    pub span: Span,
}

impl Spanned for TypeDecl {
    fn span(&self) -> Span {
        self.span
    }
}

/// Type declaration body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TypeDeclBody {
    /// Type alias: type X is Y;
    Alias(Type),

    /// Record type: type Point is { x: Float, y: Float }
    Record(List<RecordField>),

    /// Variant type: type Option<T> is Some(T) | None;
    Variant(List<Variant>),

    /// Protocol type: type Serializable is protocol { fn serialize(&self) -> Bytes; }
    /// Unified protocol syntax: type Name is protocol { methods... }
    /// Supports protocol extension: protocol extends BaseProtocol + OtherProtocol { ... }
    Protocol(ProtocolBody),

    /// Newtype: type UserId is Int;
    Newtype(Type),

    /// Tuple type: type Pair<T> is (T, T);
    Tuple(List<Type>),

    /// Sigma/dependent tuple type: type SizedArray is (n: Int, arr: [Int; n]);
    /// Each element is a sigma type (name: Type) allowing dependent types.
    /// Spec: grammar/verum.ebnf line 443 - sigma_type
    SigmaTuple(List<Type>),

    /// Unit type: type Unit;
    Unit,

    /// Inductive type: type Nat is inductive { | Zero | Succ(Nat) };
    /// Defined by constructors with well-founded recursion.
    /// Enables structural induction and termination checking.
    /// The constructors use the same Variant representation as sum types.
    Inductive(List<Variant>),

    /// Coinductive type: type Stream<A> is coinductive { fn head(&self) -> A; fn tail(&self) -> Stream<A>; };
    /// Defined by destructors (observations) rather than constructors.
    /// Allows infinite data structures with productivity checking via cofix.
    /// The destructors are represented as protocol items (method signatures).
    Coinductive(ProtocolBody),

    /// Quotient type: `type Q is T / R` — T1-T.
    ///
    /// Identifies elements of `base` that are related by the
    /// equivalence relation `relation`. The relation is a lambda
    /// expression of type `fn(base, base) -> Bool` that must be
    /// provably reflexive, symmetric, and transitive (the type
    /// checker emits proof obligations at elaboration time).
    ///
    /// Example:
    /// ```verum
    /// type ZmodN<N: Int{self > 0}> is Int / (|a, b| (a - b) % N == 0);
    /// ```
    ///
    /// Semantically equivalent to the HIT:
    /// ```verum
    /// type Q is
    ///     | of(rep: T)
    ///     | quot: fn(a: T, b: T) -> Path<Q>(of(a), of(b));
    /// ```
    /// The quotient-type parser is the ergonomic surface; the type
    /// system lowers Q into the HIT form for universal-property
    /// purposes (map-out-of-Q requires respecting the equivalence).
    Quotient {
        base: Type,
        relation: Heap<Expr>,
    },
}

/// Protocol body containing optional extends clause, where clause, and items.
/// Spec: grammar/verum.ebnf:289 - protocol_def with extends and where clause support
///
/// # Context Protocol Modifier
///
/// Protocol bodies can be marked as context protocols using the `context` modifier.
/// This is used with the unified `type ... is protocol { ... }` syntax:
///
/// ```verum
/// // Alternative syntax (compatible with existing type declarations)
/// pub context type Database is protocol {
///     async fn query(self, sql: Text) -> Result<Rows, Error>;
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProtocolBody {
    /// Whether this is a context protocol (`context type X is protocol { ... }`)
    ///
    /// Context protocols are used for dependency injection via `using [...]` clauses,
    /// as opposed to constraint protocols which are used in `where T: Protocol` bounds.
    pub is_context: bool,
    /// Base protocols this protocol extends (protocol extends Base1 + Base2)
    /// Supports generic arguments: `extends Converter<A, B>`
    pub extends: List<crate::ty::Type>,
    /// Generic where clause for type constraints (where T: Clone)
    pub generic_where_clause: Maybe<WhereClause>,
    /// Protocol items (methods, associated types, constants)
    pub items: List<ProtocolItem>,
}

impl ProtocolBody {
    /// Create a new protocol body with no extends clause (default: not a context protocol)
    pub fn new(items: List<ProtocolItem>) -> Self {
        Self {
            is_context: false,
            extends: List::new(),
            generic_where_clause: Maybe::None,
            items,
        }
    }

    /// Create a new context protocol body with no extends clause
    pub fn new_context(items: List<ProtocolItem>) -> Self {
        Self {
            is_context: true,
            extends: List::new(),
            generic_where_clause: Maybe::None,
            items,
        }
    }

    /// Create a new protocol body with extends clause (default: not a context protocol)
    pub fn with_extends(extends: List<crate::ty::Type>, items: List<ProtocolItem>) -> Self {
        Self {
            is_context: false,
            extends,
            generic_where_clause: Maybe::None,
            items,
        }
    }

    /// Create a new protocol body with extends clause and where clause (default: not a context protocol)
    pub fn with_extends_and_where(
        extends: List<crate::ty::Type>,
        generic_where_clause: Maybe<WhereClause>,
        items: List<ProtocolItem>,
    ) -> Self {
        Self {
            is_context: false,
            extends,
            generic_where_clause,
            items,
        }
    }

    /// Create a new protocol body with full configuration
    pub fn with_full_config(
        is_context: bool,
        extends: List<crate::ty::Type>,
        generic_where_clause: Maybe<WhereClause>,
        items: List<ProtocolItem>,
    ) -> Self {
        Self {
            is_context,
            extends,
            generic_where_clause,
            items,
        }
    }

    /// Check if this is a context protocol (injectable via `using [...]`)
    pub fn is_context_protocol(&self) -> bool {
        self.is_context
    }
}

/// A field in a record type.
///
/// # Default Values (Builder Pattern)
///
/// Fields can have optional default values for use with @builder:
/// ```verum
/// @builder
/// type HttpRequest is {
///     method: HttpMethod,                    // Required (no default)
///     url: Url,                              // Required (no default)
///     headers: Map<Text, Text> = Map.new(),  // Optional with default
///     timeout: Duration = 30.seconds,        // Optional with default
/// };
/// ```
///
/// # Bitfield Support
///
/// Fields can have bit specifications for packed bitfield types:
/// ```verum
/// @bitfield
/// @endian(big)
/// type IpHeader is {
///     @bits(4) version: U8,
///     @bits(4) ihl: U8,
///     @bits(16) total_length: U16,
/// };
/// ```
///
/// When a field has a `bit_spec`, it represents a bitfield member with:
/// - `width`: Number of bits the field occupies
/// - `offset`: Optional explicit bit offset from container start
///
/// The type system validates that:
/// - Bit width does not exceed the storage type's bit width
/// - No overlapping fields (unless explicitly allowed)
/// - Total bits fit within the container
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordField {
    pub visibility: Visibility,
    pub name: Ident,
    pub ty: Type,
    pub attributes: List<Attribute>,
    /// Optional default value for the field.
    /// Used by @builder derive to determine required vs optional fields.
    #[serde(default)]
    pub default_value: Maybe<Expr>,
    /// Optional bit specification for bitfield members.
    /// Present when field has @bits(N) attribute in a @bitfield type.
    #[serde(default)]
    pub bit_spec: Maybe<crate::bitfield::BitSpec>,
    pub span: Span,
}

impl RecordField {
    /// Create a new record field with no attributes and no default.
    pub fn new(visibility: Visibility, name: Ident, ty: Type, span: Span) -> Self {
        Self {
            visibility,
            name,
            ty,
            attributes: List::new(),
            default_value: Maybe::None,
            bit_spec: Maybe::None,
            span,
        }
    }

    /// Create a new record field with attributes but no default.
    pub fn with_attributes(
        visibility: Visibility,
        name: Ident,
        ty: Type,
        attributes: List<Attribute>,
        span: Span,
    ) -> Self {
        Self {
            visibility,
            name,
            ty,
            attributes,
            default_value: Maybe::None,
            bit_spec: Maybe::None,
            span,
        }
    }

    /// Create a new record field with a default value.
    pub fn with_default(
        visibility: Visibility,
        name: Ident,
        ty: Type,
        default_value: Expr,
        span: Span,
    ) -> Self {
        Self {
            visibility,
            name,
            ty,
            attributes: List::new(),
            default_value: Maybe::Some(default_value),
            bit_spec: Maybe::None,
            span,
        }
    }

    /// Create a new record field with attributes and default value.
    pub fn with_attributes_and_default(
        visibility: Visibility,
        name: Ident,
        ty: Type,
        attributes: List<Attribute>,
        default_value: Maybe<Expr>,
        span: Span,
    ) -> Self {
        Self {
            visibility,
            name,
            ty,
            attributes,
            default_value,
            bit_spec: Maybe::None,
            span,
        }
    }

    /// Create a new bitfield member with bit specification.
    ///
    /// Used for fields in @bitfield types that have @bits(N) attributes.
    ///
    /// # Example
    ///
    /// ```verum
    /// @bitfield
    /// type Flags is {
    ///     @bits(4) version: U8,
    ///     @bits(4) ihl: U8,
    /// };
    /// ```
    pub fn with_bit_spec(
        visibility: Visibility,
        name: Ident,
        ty: Type,
        attributes: List<Attribute>,
        bit_spec: crate::bitfield::BitSpec,
        span: Span,
    ) -> Self {
        Self {
            visibility,
            name,
            ty,
            attributes,
            default_value: Maybe::None,
            bit_spec: Maybe::Some(bit_spec),
            span,
        }
    }

    /// Check if this field has a default value (optional field for @builder).
    pub fn has_default(&self) -> bool {
        matches!(self.default_value, Maybe::Some(_))
    }

    /// Check if this field is a bitfield member (has @bits specification).
    pub fn is_bitfield_member(&self) -> bool {
        matches!(self.bit_spec, Maybe::Some(_))
    }

    /// Get the bit width if this is a bitfield member.
    pub fn bit_width(&self) -> Maybe<u32> {
        match &self.bit_spec {
            Maybe::Some(spec) => Maybe::Some(spec.bit_width()),
            Maybe::None => Maybe::None,
        }
    }

    /// Check if this field is required (no default value for @builder).
    pub fn is_required(&self) -> bool {
        !self.has_default()
    }
}

impl Spanned for RecordField {
    fn span(&self) -> Span {
        self.span
    }
}

/// A variant in a sum type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Variant {
    pub name: Ident,
    /// Optional generic parameters on the variant (GADT constructors).
    /// E.g., `| IntLit<T>(Int) where T == Int`
    pub generic_params: List<crate::ty::GenericParam>,
    pub data: Maybe<VariantData>,
    /// Optional where clause constraining variant (GADT style).
    /// E.g., `| IntLit(Int) where T == Int`
    pub where_clause: Maybe<crate::ty::WhereClause>,
    pub attributes: List<Attribute>,
    /// HIT path-constructor endpoints. Populated when the parser sees
    /// `Foo(args) = from..to` syntax; `None` for ordinary data-type
    /// variants. When set, the lowering to `Type::HigherInductive`
    /// emits a `PathConstructor` with these endpoints instead of a
    /// regular `Constructor` record.
    pub path_endpoints: Maybe<(Heap<crate::expr::Expr>, Heap<crate::expr::Expr>)>,
    pub span: Span,
}

impl Variant {
    /// Create a new variant with no attributes.
    pub fn new(name: Ident, data: Maybe<VariantData>, span: Span) -> Self {
        Self {
            name,
            generic_params: List::new(),
            data,
            where_clause: Maybe::None,
            attributes: List::new(),
            path_endpoints: Maybe::None,
            span,
        }
    }

    /// Create a new variant with attributes.
    pub fn with_attributes(
        name: Ident,
        data: Maybe<VariantData>,
        attributes: List<Attribute>,
        span: Span,
    ) -> Self {
        Self {
            name,
            generic_params: List::new(),
            data,
            where_clause: Maybe::None,
            attributes,
            path_endpoints: Maybe::None,
            span,
        }
    }
}

impl Spanned for Variant {
    fn span(&self) -> Span {
        self.span
    }
}

/// Variant data (tuple or record).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum VariantData {
    /// Tuple variant: Some(T)
    ///
    /// Also represents HIT path-constructors at the AST level — the
    /// parser accepts the `Foo(args) = from..to` syntax and stores
    /// the args as the tuple payload. The path-endpoint metadata
    /// is captured in a separate `PathConstructor` record attached
    /// to the containing type declaration (see verum_types::ty).
    Tuple(List<Type>),
    /// Record variant: Error { code: Int, message: Text }
    Record(List<RecordField>),
}

/// A protocol declaration.
///
/// # Protocol Declaration Syntax
/// ```text
/// protocol Name<T>: BaseProtocol
///     where type T: Ord
///     where meta N > 0
/// {
///     items
/// }
/// ```
///
/// # Context Protocol Modifier
///
/// Protocols can be marked as context protocols using the `context` modifier.
/// This distinguishes between constraint protocols and injectable protocols:
///
/// - **Constraint protocols**: `protocol Comparable { ... }` - used in `where T: Comparable`
/// - **Injectable protocols**: `context protocol Database { ... }` - used in `using [Database]`
///
/// # Examples
/// ```verum
/// // Constraint protocol (default)
/// protocol Comparable {
///     fn compare(&self, other: &Self) -> Ordering;
/// }
///
/// // Context protocol (injectable)
/// context protocol Database {
///     async fn query(self, sql: Text) -> Result<Rows, Error>;
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProtocolDecl {
    pub visibility: Visibility,
    /// Whether this is a context protocol (`context protocol Database { ... }`)
    ///
    /// Context protocols are used for dependency injection via `using [...]` clauses,
    /// as opposed to constraint protocols which are used in `where T: Protocol` bounds.
    pub is_context: bool,
    pub name: Ident,
    pub generics: List<GenericParam>,
    pub bounds: List<Type>,
    pub items: List<ProtocolItem>,

    /// Generic type constraints (where type clause)
    /// Example: `where type T: Ord`
    pub generic_where_clause: Maybe<WhereClause>,

    /// Meta-parameter constraints (where meta clause)
    /// Example: `where meta N > 0`
    pub meta_where_clause: Maybe<WhereClause>,

    pub span: Span,
}

impl Spanned for ProtocolDecl {
    fn span(&self) -> Span {
        self.span
    }
}

impl ProtocolDecl {
    /// Check if this is a context protocol (injectable via `using [...]`)
    ///
    /// Context protocols are used for dependency injection, as opposed to
    /// constraint protocols which are used in `where T: Protocol` bounds.
    pub fn is_context_protocol(&self) -> bool {
        self.is_context
    }
}

/// An item in a protocol definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProtocolItem {
    pub kind: ProtocolItemKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProtocolItemKind {
    /// Function signature
    Function {
        decl: FunctionDecl,
        default_impl: Maybe<FunctionBody>,
    },
    /// Associated type (including GATs)
    /// Generic associated type (GAT): associated types with their own type parameters.
    Type {
        name: Ident,
        /// Type parameters for GATs (empty for regular associated types)
        /// Example: `type Item<T>` has one type parameter
        type_params: List<GenericParam>,
        /// Protocol bounds on the associated type
        bounds: List<Path>,
        /// Where clauses specific to this GAT
        /// Example: `type Item<T> where T: Clone`
        where_clause: Maybe<WhereClause>,
        /// Default type implementation
        /// Example: `default type Item = Heap<u8>;`
        /// Spec: grammar/verum.ebnf lines 416-417, 932-939
        default_type: Maybe<Type>,
    },
    /// Associated constant
    Const { name: Ident, ty: Type },
    /// Protocol-level axiom — T1-R foundation.
    ///
    /// A protocol axiom is a proposition universally quantified over the
    /// protocol's parameters AND the implementing type's associated types.
    /// Every `implement` block for this protocol generates a proof
    /// obligation for each axiom, substituting `Self.T` with the
    /// implementation's concrete definitions. Obligations route through
    /// the SMT backend or can be discharged with explicit `proof name by tactic`
    /// clauses inside the implement block.
    ///
    /// Example:
    /// ```verum
    /// type Group is protocol {
    ///     type Elem;
    ///     fn unit() -> Self.Elem;
    ///     fn mul(a: Self.Elem, b: Self.Elem) -> Self.Elem;
    ///     axiom left_unit(x: Self.Elem)
    ///         ensures Self.mul(Self.unit(), x) == x;
    /// };
    /// ```
    Axiom(AxiomDecl),
}

impl Spanned for ProtocolItem {
    fn span(&self) -> Span {
        self.span
    }
}

/// An implementation block.
///
/// # Implementation Block Syntax
/// ```text
/// implement<T> Protocol for Type
///     where type T: Ord
///     where meta N > 0
/// {
///     items
/// }
/// ```
///
/// # Specialization (v2.0+ planned)
/// ```text
/// @specialize
/// implement Protocol for SpecificType {
///     // More specific implementation
/// }
///
/// @specialize(negative)
/// implement<T: !Clone> Protocol for List<T> { }
///
/// @specialize(rank = 10)
/// implement Protocol for Int { }
///
/// @specialize(when(T: Clone + Send))
/// implement<T> Protocol for Heap<T> { }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImplDecl {
    /// Whether this is an unsafe impl (for unsafe traits like Send, Sync)
    /// Grammar: impl_block = [ attribute ] , [ 'unsafe' ] , 'implement' , ...
    pub is_unsafe: bool,

    pub generics: List<GenericParam>,
    pub kind: ImplKind,

    /// Generic type constraints (where type clause)
    /// Example: `where type T: Ord`
    pub generic_where_clause: Maybe<WhereClause>,

    /// Meta-parameter constraints (where meta clause)
    /// Example: `where meta N > 0`
    pub meta_where_clause: Maybe<WhereClause>,

    /// Specialization attribute
    /// Specialization condition for selecting which impl applies.
    /// Contains specialization metadata (negative, rank, when clause)
    pub specialize_attr: Maybe<crate::attr::SpecializeAttr>,

    pub items: List<ImplItem>,
    pub span: Span,
}

impl Spanned for ImplDecl {
    fn span(&self) -> Span {
        self.span
    }
}

/// The kind of implementation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ImplKind {
    /// Inherent implementation: implement Type { ... }
    Inherent(Type),
    /// Protocol implementation: implement Protocol for Type { ... }
    /// Or with HKT: implement Protocol<TypeCtor> for Type { ... }
    Protocol {
        protocol: Path,
        /// Type constructor arguments for HKT support
        /// Example: For `implement Functor<List> for MyType`, this stores [List]
        protocol_args: List<crate::ty::GenericArg>,
        for_type: Type,
    },
}

/// An item in an implementation block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImplItem {
    /// Attributes on the impl item (e.g., @inject for DI)
    pub attributes: List<Attribute>,
    pub visibility: Visibility,
    pub kind: ImplItemKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ImplItemKind {
    /// Function implementation
    Function(FunctionDecl),
    /// Type alias (with GAT support)
    /// Generic associated type declaration with its own type parameters.
    Type {
        name: Ident,
        /// Type parameters for GATs (empty for regular associated types)
        /// Example: `type Item<T> = List<T>;` has one type parameter
        type_params: List<GenericParam>,
        ty: Type,
    },
    /// Const definition
    Const { name: Ident, ty: Type, value: Expr },
    /// Axiom proof clause — `proof axiom_name by tactic;`
    ///
    /// Inside an `implement P for T { ... }` block, discharges the
    /// named axiom from protocol `P` using the given tactic. The
    /// model-verification phase (T1-R) matches the name against `P`'s
    /// axiom list and runs the tactic against the self-substituted
    /// proposition instead of the default `auto_prove`.
    ///
    /// Example:
    /// ```verum
    /// implement Group for IntGroup {
    ///     type Elem = Int;
    ///     fn unit() -> Int { 0 }
    ///     fn mul(a: Int, b: Int) -> Int { a + b }
    ///     fn inv(a: Int) -> Int { -a }
    ///     proof assoc     by ring;
    ///     proof left_unit by ring;
    ///     proof left_inv  by ring;
    /// }
    /// ```
    Proof {
        axiom_name: Ident,
        tactic: crate::decl::TacticExpr,
    },
}

impl Spanned for ImplItem {
    fn span(&self) -> Span {
        self.span
    }
}

/// A module declaration.
///
/// # Profile Support
///
/// Modules can declare which language profiles they support using the @profile() attribute.
/// This enables fine-grained control over language features within a single project.
///
/// # Examples
///
/// ```verum
/// @profile(application)
/// module web_server { }
///
/// @profile(systems)
/// module low_level { }
///
/// @profile(application)
/// @feature(enable: ["unsafe"])
/// module ffi_bindings { }
/// ```
///
/// # Specification
///
/// Language profiles control which features are available in a module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleDecl {
    pub visibility: Visibility,
    pub name: Ident,
    pub items: Maybe<List<Item>>,

    /// Language profile(s) this module supports
    /// Profile level (e.g., systems, application) controlling available features.
    pub profile: Maybe<ProfileAttr>,

    /// Features enabled beyond base profile.
    /// Custom feature sets can be listed explicitly.
    pub features: Maybe<FeatureAttr>,

    /// Module-level context requirements (from @using attribute)
    /// Example: `@using([Database, Logger])`
    /// All functions within this module implicitly inherit these contexts
    pub contexts: List<ContextRequirement>,

    pub span: Span,
}

impl Spanned for ModuleDecl {
    fn span(&self) -> Span {
        self.span
    }
}

/// A const declaration.
///
/// # Generic Constants
///
/// Constants can have generic parameters:
///
/// ```verum
/// const ZERO<T: Default>: T = T.default();
/// const IDENTITY<T>: fn(T) -> T = |x| x;
/// ```
///
/// Mount statement for importing names into scope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConstDecl {
    pub visibility: Visibility,
    pub name: Ident,
    /// Generic parameters for the constant (e.g., `<T: Default>`)
    pub generics: List<GenericParam>,
    pub ty: Type,
    pub value: Expr,
    pub span: Span,
}

impl Spanned for ConstDecl {
    fn span(&self) -> Span {
        self.span
    }
}

/// A static declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StaticDecl {
    pub visibility: Visibility,
    pub is_mut: bool,
    pub name: Ident,
    pub ty: Type,
    pub value: Expr,
    pub span: Span,
}

impl Spanned for StaticDecl {
    fn span(&self) -> Span {
        self.span
    }
}

/// A mount declaration.
///
/// Supports re-exports with visibility modifiers:
/// - `import std.io.File;` - private import
/// - `public mount std.io.File;` - re-export as public
/// - `public mount std.io.File as MyFile;` - re-export with rename
///
/// Re-export statement for making imported items publicly visible.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MountDecl {
    /// Visibility modifier for re-exports
    /// Whether this re-export is public (visible to importers of this module).
    pub visibility: Visibility,
    pub tree: MountTree,
    pub alias: Maybe<Ident>,
    pub span: Span,
}

impl Spanned for MountDecl {
    fn span(&self) -> Span {
        self.span
    }
}

/// A mount tree (supports nested mounts).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MountTree {
    pub kind: MountTreeKind,
    /// Optional alias for this mount item (e.g., `exit as sys_exit`)
    pub alias: Maybe<Ident>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MountTreeKind {
    /// Simple path: std.io.File
    Path(Path),
    /// Glob mount: std.io.*
    Glob(Path),
    /// Nested mounts: std.io.{File, Read, Write}
    Nested {
        prefix: Path,
        trees: List<MountTree>,
    },
}

impl Spanned for MountTree {
    fn span(&self) -> Span {
        self.span
    }
}

/// A meta (macro) declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetaDecl {
    pub visibility: Visibility,
    pub name: Ident,
    pub params: List<MetaParam>,
    pub rules: List<MetaRule>,
    pub span: Span,
}

impl Spanned for MetaDecl {
    fn span(&self) -> Span {
        self.span
    }
}

/// A meta parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetaParam {
    pub name: Ident,
    pub fragment: Maybe<MetaFragment>,
    pub span: Span,
}

/// Meta fragment specifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MetaFragment {
    Expr,
    Stmt,
    Type,
    Pattern,
    Ident,
    Path,
    TokenTree,
    Item,
    Block,
}

/// A meta rule (pattern => expansion).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetaRule {
    pub pattern: Pattern,
    pub expansion: Expr,
    pub span: Span,
}

impl Spanned for MetaRule {
    fn span(&self) -> Span {
        self.span
    }
}

/// A context declaration.
///
/// Contexts define dependency injection containers that can be used to provide
/// values to functions. This enables better testability and separation of concerns.
///
/// # Example
/// ```verum
/// context Database {
///     fn query(sql: Text) -> Result<Rows>
///     fn execute(sql: Text) -> Result<Unit>
/// }
/// ```
///
/// # Sub-Contexts
///
/// Contexts can define nested sub-contexts for fine-grained capability control:
///
/// ```verum
/// context FileSystem {
///     context Read {
///         fn read(path: Text) -> Result<List<u8>>
///     }
///     context Write {
///         fn write(path: Text, data: List<u8>) -> Result<()>
///     }
/// }
/// ```
///
/// Sub-context declaration: derives a new context from an existing one with restrictions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextDecl {
    pub visibility: Visibility,
    /// Whether this is an async context (`context async Database { ... }`)
    pub is_async: bool,
    pub name: Ident,
    /// Generic parameters for the context (`context State<S> { ... }`)
    pub generics: List<GenericParam>,
    pub methods: List<FunctionDecl>,
    /// Associated types: `type Row;` or `type Error: From<IoError>;`
    /// Grammar: context_type = 'type' identifier [':' bounds] ';'
    #[serde(default)]
    pub associated_types: List<TypeDecl>,
    /// Associated constants: `const MAX_CONNECTIONS: Int;`
    /// Grammar: context_const = 'const' identifier ':' type_expr ';'
    #[serde(default)]
    pub associated_consts: List<ConstDecl>,
    /// Nested sub-contexts for fine-grained capabilities
    /// The parent context this sub-context derives from.
    pub sub_contexts: List<ContextDecl>,
    pub span: Span,
}

impl ContextDecl {
    /// Create a synthetic (empty) context declaration for pre-
    /// registration from the embedded stdlib archive. The methods
    /// and types are not available but the name is valid for
    /// `using [Name]` resolution.
    pub fn synthetic() -> Self {
        Self {
            visibility: Visibility::Public,
            is_async: false,
            name: Ident::new("", Span::default()),
            generics: List::new(),
            methods: List::new(),
            associated_types: List::new(),
            associated_consts: List::new(),
            sub_contexts: List::new(),
            span: Span::default(),
        }
    }
}

impl Spanned for ContextDecl {
    fn span(&self) -> Span {
        self.span
    }
}

/// A context group declaration.
///
/// Context groups allow multiple contexts to be used together as a unit,
/// simplifying function signatures that require multiple contexts.
///
/// # Example
/// ```verum
/// context group WebApp {
///     Database,
///     Logger,
///     Cache
/// }
/// ```
///
/// Context groups can also use extended syntax with negation and type arguments:
/// ```verum
/// using Pure = [!IO, !State<_>, !Random];
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextGroupDecl {
    pub visibility: Visibility,
    pub name: Ident,
    /// Full context requirements supporting negation, type args, transforms, etc.
    pub contexts: List<ContextRequirement>,
    pub span: Span,
}

impl Spanned for ContextGroupDecl {
    fn span(&self) -> Span {
        self.span
    }
}

/// Context layer declaration — composable context bundles.
///
/// Layers group `provide` statements with dependency ordering.
/// Composition via `+` enables modular application assembly.
///
/// Grammar: layer_def = visibility 'layer' identifier layer_body
///          layer_body = '{' { provide_stmt } '}' | '=' layer_expr ';'
///          layer_expr = identifier { '+' identifier }
///
/// # Examples
/// ```verum
/// layer DatabaseLayer {
///     provide ConnectionPool = ConnectionPool.new(Config.get_url());
///     provide QueryExecutor = QueryExecutor.new(ConnectionPool);
/// }
/// layer AppLayer = DatabaseLayer + LoggingLayer;
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayerDecl {
    pub visibility: Visibility,
    pub name: Ident,
    /// The layer definition body.
    pub kind: LayerKind,
    pub span: Span,
}

/// Kind of layer declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LayerKind {
    /// Inline layer with provide statements.
    /// `layer Name { provide Ctx = expr; ... }`
    Inline {
        /// Provide statements (context name + value expression).
        provides: List<(Ident, Expr)>,
    },
    /// Composite layer combining other layers.
    /// `layer Name = Layer1 + Layer2 + Layer3;`
    Composite {
        /// Names of composed layers.
        layers: List<Ident>,
    },
}

impl Spanned for LayerDecl {
    fn span(&self) -> Span {
        self.span
    }
}

/// Visibility modifier.
///
/// Visibility modifiers: public, public(crate), public(super), public(in path), or private (default).
///
/// | Modifier | Visibility |
/// |----------|------------|
/// | `public` | Public to all users |
/// | `public(crate)` | Public within crate only |
/// | `public(super)` | Public to parent module |
/// | `public(in path)` | Public within specified path |
/// | *(none)* *(default)* | Private to current module |
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Visibility {
    /// Public: visible everywhere
    /// Syntax: `public`
    Public,

    /// Crate-local: visible within the current crate only
    /// Syntax: `public(crate)`
    PublicCrate,

    /// Parent-public: visible to parent module only
    /// Syntax: `public(super)`
    PublicSuper,

    /// Path-restricted: visible within specified module path and its submodules
    /// Syntax: `public(in path.to.module)`
    PublicIn(Path),

    /// Internal: visible within the current crate only
    /// Syntax: `internal`
    Internal,

    /// Protected: visible to submodules and implementations
    /// Syntax: `protected`
    Protected,

    /// Private: visible only within the current module (default)
    /// Syntax: (none)
    #[default]
    Private,
}

impl Visibility {
    pub fn is_public(&self) -> bool {
        matches!(self, Visibility::Public)
    }

    /// Check if this visibility is crate-local or more visible
    pub fn is_crate_visible(&self) -> bool {
        matches!(
            self,
            Visibility::Public | Visibility::PublicCrate | Visibility::PublicIn(_)
        )
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Visibility::Public => "public",
            Visibility::PublicCrate => "public(crate)",
            Visibility::PublicSuper => "public(super)",
            Visibility::PublicIn(_) => "public(in ...)",
            Visibility::Private => "private",
            Visibility::Internal => "internal",
            Visibility::Protected => "protected",
        }
    }
}

// Note: Attribute is defined in crate::attr and re-exported at the top of this file

// ==================== Formal Proofs Declarations (v2.0+ extension) ====================

/// A theorem declaration.
///
/// Theorems are named propositions with proofs. They represent mathematical
/// truths that have been verified through formal proof.
///
/// # Theorem/Lemma/Corollary Syntax
/// ```text
/// theorem name<T>(params) -> Type
///     requires precondition1, precondition2
///     ensures postcondition1, postcondition2
/// {
///     proof by tactic
/// }
/// ```
///
/// # Examples
/// ```verum
/// theorem plus_comm(m: Int, n: Int)
///     ensures m + n == n + m
/// {
///     proof by ring
/// }
///
/// theorem division_valid(a: Int, b: Int)
///     requires b != 0
///     ensures a / b * b + a % b == a
/// {
///     proof by auto
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TheoremDecl {
    /// Visibility modifier
    pub visibility: Visibility,

    /// Theorem name
    pub name: Ident,

    /// Generic type parameters
    pub generics: List<GenericParam>,

    /// Theorem parameters (like function parameters)
    pub params: List<FunctionParam>,

    /// Optional return type (-> Type)
    pub return_type: Maybe<Type>,

    /// Preconditions (requires clauses)
    /// Example: `requires b != 0`
    pub requires: List<Expr>,

    /// Postconditions (ensures clauses)
    /// Example: `ensures result == a + b`
    pub ensures: List<Expr>,

    /// The proposition to prove (legacy field - combined from ensures for backwards compat)
    /// This will be synthesized from ensures clauses if not explicitly provided
    pub proposition: Heap<Expr>,

    /// Generic type constraints (where type clause)
    pub generic_where_clause: Maybe<WhereClause>,

    /// Meta-parameter constraints (where meta clause)
    pub meta_where_clause: Maybe<WhereClause>,

    /// The proof body
    pub proof: Maybe<ProofBody>,

    /// Attributes (e.g., @smt, @interactive, @extract)
    pub attributes: List<Attribute>,

    /// Source span
    pub span: Span,
}

impl TheoremDecl {
    /// Create a new theorem declaration
    pub fn new(name: Ident, proposition: Expr, span: Span) -> Self {
        Self {
            visibility: Visibility::Private,
            name,
            generics: List::new(),
            params: List::new(),
            return_type: Maybe::None,
            requires: List::new(),
            ensures: List::new(),
            proposition: Heap::new(proposition),
            generic_where_clause: Maybe::None,
            meta_where_clause: Maybe::None,
            proof: Maybe::None,
            attributes: List::new(),
            span,
        }
    }

    /// Create a new theorem with requires/ensures clauses
    pub fn with_contracts(
        name: Ident,
        return_type: Maybe<Type>,
        requires: List<Expr>,
        ensures: List<Expr>,
        span: Span,
    ) -> Self {
        // Synthesize proposition from ensures clauses
        let proposition = if ensures.is_empty() {
            Expr::new(crate::ExprKind::Literal(crate::Literal::bool(true, span)), span)
        } else if ensures.len() == 1 {
            ensures.iter().next().unwrap().clone()
        } else {
            // Combine multiple ensures with AND
            let mut iter = ensures.iter();
            let first = iter.next().unwrap().clone();
            iter.fold(first, |acc, e| {
                Expr::new(
                    crate::ExprKind::Binary {
                        op: crate::BinOp::And,
                        left: Heap::new(acc),
                        right: Heap::new(e.clone()),
                    },
                    span,
                )
            })
        };
        Self {
            visibility: Visibility::Private,
            name,
            generics: List::new(),
            params: List::new(),
            return_type,
            requires,
            ensures,
            proposition: Heap::new(proposition),
            generic_where_clause: Maybe::None,
            meta_where_clause: Maybe::None,
            proof: Maybe::None,
            attributes: List::new(),
            span,
        }
    }

    /// Check if the theorem has been proven
    pub fn is_proven(&self) -> bool {
        self.proof.is_some()
    }

    /// Get the theorem kind (for display purposes)
    pub fn kind_str(&self) -> &'static str {
        "theorem"
    }
}

impl Spanned for TheoremDecl {
    fn span(&self) -> Span {
        self.span
    }
}

/// An axiom declaration.
///
/// Axioms are unproven propositions that are assumed to be true.
/// They form the foundational assumptions of the proof system.
///
/// # Syntax
/// ```text
/// axiom name<T>(params) -> Type;
/// ```
///
/// # Examples
/// ```verum
/// // Excluded middle (classical logic axiom)
/// axiom excluded_middle(p: Bool) -> Bool;
/// ```
///
/// # Warning
///
/// Axioms should be used sparingly as they introduce unproven assumptions.
/// Inconsistent axioms can lead to proving False.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AxiomDecl {
    /// Visibility modifier
    pub visibility: Visibility,

    /// Axiom name
    pub name: Ident,

    /// Generic type parameters
    pub generics: List<GenericParam>,

    /// Axiom parameters
    pub params: List<FunctionParam>,

    /// Optional return type (-> Type)
    pub return_type: Maybe<Type>,

    /// The proposition assumed to be true (legacy field for backwards compat)
    pub proposition: Heap<Expr>,

    /// Generic type constraints
    pub generic_where_clause: Maybe<WhereClause>,

    /// Meta-parameter constraints
    pub meta_where_clause: Maybe<WhereClause>,

    /// Attributes
    pub attributes: List<Attribute>,

    /// Source span
    pub span: Span,
}

impl AxiomDecl {
    /// Create a new axiom declaration
    pub fn new(name: Ident, proposition: Expr, span: Span) -> Self {
        Self {
            visibility: Visibility::Private,
            name,
            generics: List::new(),
            params: List::new(),
            return_type: Maybe::None,
            proposition: Heap::new(proposition),
            generic_where_clause: Maybe::None,
            meta_where_clause: Maybe::None,
            attributes: List::new(),
            span,
        }
    }
}

impl Spanned for AxiomDecl {
    fn span(&self) -> Span {
        self.span
    }
}

/// A tactic declaration.
///
/// Tactics are proof automation strategies that can be defined by users.
/// They compose basic proof steps into reusable automation.
///
/// # Tactic Declaration Syntax
/// ```text
/// tactic name is {
///     tactic_body
/// }
/// ```
///
/// # Examples
/// ```verum
/// // Automated proof search
/// tactic auto is {
///     first [
///         assumption,
///         reflexivity,
///         { intro; auto },
///         { split; auto },
///         { apply_hypothesis; auto },
///         { unfold_definition; auto }
///     ]
/// }
///
/// // Induction with automation
/// tactic induction_auto is {
///     induction *;
///     all_goals auto
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TacticDecl {
    /// Visibility modifier
    pub visibility: Visibility,

    /// Tactic name
    pub name: Ident,

    /// Generic type parameters (e.g. `tactic category_law<C: Category>()`)
    pub generics: List<GenericParam>,

    /// Tactic parameters
    pub params: List<TacticParam>,

    /// Generic type constraints (`where` clause)
    pub generic_where_clause: Maybe<WhereClause>,

    /// The tactic body
    pub body: TacticBody,

    /// Attributes
    pub attributes: List<Attribute>,

    /// Source span
    pub span: Span,
}

impl Spanned for TacticDecl {
    fn span(&self) -> Span {
        self.span
    }
}

/// A tactic parameter.
///
/// Tactics take typed parameters, much like functions. The `kind` field
/// captures the classical tactic-parameter classification (Expr, Type,
/// Tactic, Hypothesis, Int, Prop); the `ty` field carries the concrete
/// type expression when the parameter is declared with arbitrary typing
/// (e.g. `confidence: Float`, `candidate: Maybe<Proof>`). The optional
/// `default` value lets tactic authors declare default arguments like
/// `oracle(goal: Prop, confidence: Float = 0.9)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TacticParam {
    /// Parameter name
    pub name: Ident,
    /// Parameter kind (expression, type, tactic, etc.)
    pub kind: TacticParamKind,
    /// Concrete type when the parameter is annotated with an arbitrary type
    /// expression (e.g. `x: Maybe<Proof>`). `None` for the classical
    /// tactic-parameter kinds which are fully determined by `kind`.
    pub ty: Maybe<Type>,
    /// Optional default value (for parameters declared like `x: T = expr`)
    pub default: Maybe<Heap<Expr>>,
    /// Source span
    pub span: Span,
}

/// Kind of tactic parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TacticParamKind {
    /// Expression parameter
    Expr,
    /// Type parameter
    Type,
    /// Tactic parameter (for higher-order tactics)
    Tactic,
    /// Hypothesis identifier
    Hypothesis,
    /// Integer parameter (for iteration counts, etc.)
    Int,
    /// Proposition parameter (a specification / formula, first-class in the tactic DSL)
    Prop,
    /// Any other typed parameter — the real type lives in `TacticParam::ty`.
    /// Used for parameters declared with arbitrary types like `Float`, `List<T>`, etc.
    Other,
}

/// A tactic body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TacticBody {
    /// Simple tactic expression
    Simple(TacticExpr),
    /// Block of tactic expressions
    Block(List<TacticExpr>),
}

/// A proof body.
///
/// Proof bodies can be explicit proof terms, tactics, or a combination.
///
/// # Proof Tactics Syntax
/// ```text
/// proof {
///     have h1: P by tactic
///     have h2: Q by assumption
///     show R by apply lemma[h1, h2]
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProofBody {
    /// Explicit proof term (Curry-Howard style)
    Term(Heap<Expr>),

    /// Tactic proof (declarative style)
    Tactic(TacticExpr),

    /// Structured proof with steps
    Structured(ProofStructure),

    /// Proof by specific method (induction, cases, etc.)
    ByMethod(ProofMethod),
}

/// A structured proof with intermediate steps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProofStructure {
    /// Proof steps
    pub steps: List<ProofStep>,
    /// Final conclusion tactic
    pub conclusion: Maybe<TacticExpr>,
    /// Source span
    pub span: Span,
}

/// A step in a structured proof.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProofStep {
    /// Step kind
    pub kind: ProofStepKind,
    /// Source span
    pub span: Span,
}

/// Kind of proof step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProofStepKind {
    /// Introduce hypothesis: `have h: P by tactic`
    Have {
        name: Ident,
        proposition: Heap<Expr>,
        justification: TacticExpr,
    },

    /// Show intermediate goal: `show P by tactic`
    Show {
        proposition: Heap<Expr>,
        justification: TacticExpr,
    },

    /// Suffices to show: `suffices to show P by tactic`
    Suffices {
        proposition: Heap<Expr>,
        justification: TacticExpr,
    },

    /// Let binding in proof: `let x := e`
    Let { pattern: Pattern, value: Heap<Expr> },

    /// Obtain existential witness: `obtain pattern from proof`
    /// Grammar: obtain_step = 'obtain' , pattern , 'from' , expression ;
    Obtain { pattern: Pattern, from: Heap<Expr> },

    /// Calculation chain: `calc { ... }`
    Calc(CalculationChain),

    /// Case analysis: `cases e { ... }`
    Cases {
        scrutinee: Heap<Expr>,
        cases: List<ProofCase>,
    },

    /// Focus on subgoal
    Focus {
        goal_index: usize,
        steps: List<ProofStep>,
    },

    /// Tactic application: `tactic;`
    /// Grammar: tactic_application = tactic_expr , ';' ;
    Tactic(TacticExpr),
}

/// A calculation chain (equational reasoning).
///
/// # Algebraic Structure Syntax
/// ```text
/// calc {
///     op(a, id)
///         = op(a, op(inv(a), a))     by left_inv
///         = op(op(a, inv(a)), a)     by assoc
///         = op(id, a)                by left_inv
///         = a                        by left_id
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalculationChain {
    /// The starting expression
    pub start: Heap<Expr>,
    /// Chain of calculation steps
    pub steps: List<CalculationStep>,
    /// Source span
    pub span: Span,
}

/// A step in a calculation chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalculationStep {
    /// Relation (=, <, ≤, etc.)
    pub relation: CalcRelation,
    /// Target expression
    pub target: Heap<Expr>,
    /// Justification for this step
    pub justification: TacticExpr,
    /// Source span
    pub span: Span,
}

/// Relation in calculation steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CalcRelation {
    /// Equality (=)
    Eq,
    /// Not equal (≠)
    Ne,
    /// Less than (<)
    Lt,
    /// Less than or equal (≤)
    Le,
    /// Greater than (>)
    Gt,
    /// Greater than or equal (≥)
    Ge,
    /// Implies (→)
    Implies,
    /// If and only if (↔)
    Iff,
    /// Subset (⊆)
    Subset,
    /// Superset (⊇)
    Superset,
    /// Divides (|)
    Divides,
    /// Congruent modulo (≡)
    Congruent,
}

/// A case in a proof by cases.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProofCase {
    /// Case pattern
    pub pattern: Pattern,
    /// Proof for this case
    pub proof: List<ProofStep>,
    /// Source span
    pub span: Span,
}

/// Proof method for `by` clauses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProofMethod {
    /// Proof by induction
    Induction {
        /// Variable to induct on (None = automatic)
        on: Maybe<Ident>,
        /// Induction cases
        cases: List<ProofCase>,
    },

    /// Proof by cases
    Cases {
        /// Expression to case split on
        on: Heap<Expr>,
        /// Cases
        cases: List<ProofCase>,
    },

    /// Proof by contradiction
    Contradiction {
        /// Assumption of negation
        assumption: Ident,
        /// Proof deriving False
        proof: List<ProofStep>,
    },

    /// Proof by strong induction
    StrongInduction { on: Ident, cases: List<ProofCase> },

    /// Proof by well-founded induction
    WellFoundedInduction {
        relation: Heap<Expr>,
        on: Ident,
        cases: List<ProofCase>,
    },
}

/// A tactic expression.
///
/// Tactic expressions are the primitive proof automation steps.
///
/// # Proof Tactics Syntax
/// ```text
/// tactic ::= intro | apply expr | simp | ring | omega | ...
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TacticExpr {
    /// Trivially true (`trivial`)
    Trivial,

    /// Use an assumption (`assumption`)
    Assumption,

    /// Reflexivity (`refl`)
    Reflexivity,

    /// Introduction (`intro x` or `intro`)
    Intro(List<Ident>),

    /// Apply a lemma (`apply lemma_name`)
    Apply { lemma: Heap<Expr>, args: List<Expr> },

    /// Rewrite using equality (`rewrite h` or `rewrite h at target`)
    Rewrite {
        hypothesis: Heap<Expr>,
        at_target: Maybe<Ident>,
        rev: bool, // reverse direction
    },

    /// Simplification (`simp` or `simp[lemmas]`)
    Simp {
        lemmas: List<Expr>,
        at_target: Maybe<Ident>,
    },

    /// Ring arithmetic solver (`ring`)
    Ring,

    /// Field arithmetic solver (`field`)
    Field,

    /// Linear integer arithmetic solver (`omega`)
    Omega,

    /// General automation (`auto` or `auto with hints`)
    Auto { with_hints: List<Ident> },

    /// Tableau prover (`blast`)
    Blast,

    /// SMT solver dispatch (`smt` or `smt(solver = "Z3")`)
    Smt {
        solver: Maybe<Text>,
        timeout: Maybe<u64>,
    },

    /// Split conjunction (`split`)
    Split,

    /// Left of disjunction (`left`)
    Left,

    /// Right of disjunction (`right`)
    Right,

    /// Existential witness (`exists e`)
    Exists(Heap<Expr>),

    /// Case analysis on hypothesis (`cases h`)
    CasesOn(Ident),

    /// Induction on variable (`induction x`)
    InductionOn(Ident),

    /// Contradiction from hypothesis
    Exact(Heap<Expr>),

    /// Unfold definition (`unfold def_name`)
    Unfold(List<Ident>),

    /// Compute/normalize (`compute`)
    Compute,

    /// Try tactic, continue if fails (`try tactic`)
    Try(Heap<TacticExpr>),

    /// Try-else: attempt the body tactic; if it fails, execute the
    /// else branch instead. Syntax: `try { body } else { fallback }`
    TryElse {
        body: Heap<TacticExpr>,
        fallback: Heap<TacticExpr>,
    },

    /// Repeat tactic until failure (`repeat tactic`)
    Repeat(Heap<TacticExpr>),

    /// Sequential composition (`tactic1; tactic2`)
    Seq(List<TacticExpr>),

    /// Alternative (`tactic1 <|> tactic2` or `first [...]`)
    Alt(List<TacticExpr>),

    /// Apply tactic to all goals (`all_goals tactic`)
    AllGoals(Heap<TacticExpr>),

    /// Apply tactic to specific goal (`{ tactic }` inside proof)
    Focus(Heap<TacticExpr>),

    /// Named tactic invocation.
    ///
    /// Tactics may be generic (e.g. `tactic category_law<C: Category>()`)
    /// and can therefore be called with explicit type arguments:
    ///
    /// ```verum
    /// category_law<F.Source>();
    /// functor_law<Identity>();
    /// ```
    ///
    /// `generic_args` is empty when no type arguments are supplied.
    Named {
        name: Ident,
        generic_args: List<Type>,
        args: List<Expr>,
    },

    /// Local let-binding inside a tactic body:
    /// `let x: T = expr;` — computes `expr`, binds it to `x`, and makes it
    /// available to the remaining tactic sequence. Enables monadic
    /// composition in tactic DSLs (analogous to Lean's `let _ ← …`).
    Let {
        name: Ident,
        ty: Maybe<Type>,
        value: Heap<Expr>,
    },

    /// Pattern-match on a value inside a tactic body:
    /// `match x { P₁ => t₁, P₂ => t₂, … }`
    ///
    /// Each arm's body is itself a tactic expression, allowing tactics to
    /// branch on the shape of an auxiliary value (e.g. a `Maybe<Proof>`).
    Match {
        scrutinee: Heap<Expr>,
        arms: List<TacticMatchArm>,
    },

    /// Explicit failure with a diagnostic message:
    /// `fail("oracle candidate rejected by SMT backend")`.
    /// Distinct from `Admit`/`Sorry`: `Fail` is a *tactic-local* control-flow
    /// operator that feeds into surrounding `try`/`first` combinators.
    Fail { message: Heap<Expr> },

    /// Conditional tactic execution:
    /// `if cond { t₁ } else { t₂ }` — selects a branch at tactic runtime.
    If {
        cond: Heap<Expr>,
        then_branch: Heap<TacticExpr>,
        else_branch: Maybe<Heap<TacticExpr>>,
    },

    /// Done/QED marker
    Done,

    /// Admit (leave goal unproven - for development)
    Admit,

    /// Sorry (like admit but marks as incomplete)
    Sorry,

    /// Contradiction tactic (proof by contradiction)
    Contradiction,
}

/// An arm of a tactic-level `match` expression: pattern, optional guard,
/// and a tactic-expression body executed when the pattern matches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TacticMatchArm {
    /// The pattern to match against.
    pub pattern: crate::pattern::Pattern,
    /// Optional guard expression (`if cond`).
    pub guard: Maybe<Heap<Expr>>,
    /// Tactic body to execute when this arm matches.
    pub body: Heap<TacticExpr>,
    /// Source span of the arm.
    pub span: Span,
}

impl TacticExpr {
    /// Check if this tactic is a terminating tactic
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TacticExpr::Trivial
                | TacticExpr::Assumption
                | TacticExpr::Reflexivity
                | TacticExpr::Done
                | TacticExpr::Admit
                | TacticExpr::Sorry
        )
    }

    /// Check if this tactic is unsafe (leaves goals unproven)
    pub fn is_unsafe(&self) -> bool {
        matches!(self, TacticExpr::Admit | TacticExpr::Sorry)
    }
}

// ==================== View Patterns (v2.0+ planned) ====================

/// A view declaration.
///
/// Views provide alternative pattern matching interfaces for types.
/// They allow matching on computed properties rather than constructors.
///
/// # View Pattern Syntax (v2.0+ planned)
/// ```text
/// view Name : ParamType -> ReturnType {
///     Constructor1 : (params) -> ReturnType(index1),
///     Constructor2 : (params) -> ReturnType(index2)
/// }
/// ```
///
/// # Examples
/// ```verum
/// view Parity : Nat -> Type {
///     Even : (n: Nat) -> Parity(2 * n),
///     Odd : (n: Nat) -> Parity(2 * n + 1)
/// }
///
/// fn parity(n: Nat) : Parity(n) = {
///     match n {
///         Zero => Even(Zero),
///         Succ(Zero) => Odd(Zero),
///         Succ(Succ(n')) =>
///             match parity(n') {
///                 Even(k) => Even(Succ(k)),
///                 Odd(k) => Odd(Succ(k))
///             }
///     }
/// }
///
/// fn is_even(n: Nat) -> bool =
///     match parity(n) {
///         Even(_) => true,
///         Odd(_) => false
///     }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewDecl {
    /// Visibility modifier
    pub visibility: Visibility,

    /// View name
    pub name: Ident,

    /// Generic type parameters
    pub generics: List<GenericParam>,

    /// Parameter type (input to the view function)
    /// Example: Nat in `view Parity : Nat -> Type`
    pub param_type: Type,

    /// Return type constructor (dependent on parameter)
    /// Example: Type in `view Parity : Nat -> Type`
    /// For indexed types: Type or specific indexed type
    pub return_type: Type,

    /// View constructors
    pub constructors: List<ViewConstructor>,

    /// Generic type constraints (where type clause)
    pub generic_where_clause: Maybe<WhereClause>,

    /// Meta-parameter constraints (where meta clause)
    pub meta_where_clause: Maybe<WhereClause>,

    /// Attributes
    pub attributes: List<Attribute>,

    /// Source span
    pub span: Span,
}

impl ViewDecl {
    /// Create a new view declaration
    pub fn new(
        name: Ident,
        param_type: Type,
        return_type: Type,
        constructors: List<ViewConstructor>,
        span: Span,
    ) -> Self {
        Self {
            visibility: Visibility::Private,
            name,
            generics: List::new(),
            param_type,
            return_type,
            constructors,
            generic_where_clause: Maybe::None,
            meta_where_clause: Maybe::None,
            attributes: List::new(),
            span,
        }
    }

    /// Check if this view has any constructors
    pub fn has_constructors(&self) -> bool {
        !self.constructors.is_empty()
    }

    /// Get the number of constructors
    pub fn num_constructors(&self) -> usize {
        self.constructors.len()
    }
}

impl Spanned for ViewDecl {
    fn span(&self) -> Span {
        self.span
    }
}

/// A constructor in a view declaration.
///
/// View constructors define how to construct values of the view type
/// from the parameter type.
///
/// # Examples
/// ```verum
/// // Even constructor: (n: Nat) -> Parity(2 * n)
/// ViewConstructor {
///     name: "Even",
///     params: [(n, Nat)],
///     result_index: 2 * n  // The index in the dependent type
/// }
///
/// // Odd constructor: (n: Nat) -> Parity(2 * n + 1)
/// ViewConstructor {
///     name: "Odd",
///     params: [(n, Nat)],
///     result_index: 2 * n + 1
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewConstructor {
    /// Constructor name (e.g., "Even", "Odd")
    pub name: Ident,

    /// Type parameters (for polymorphic constructors)
    pub type_params: List<GenericParam>,

    /// Constructor parameters
    /// Example: [(n, Nat)] for `Even : (n: Nat) -> ...`
    pub params: List<FunctionParam>,

    /// Result type (must be the view's return type with appropriate index)
    /// Example: Parity(2 * n) for Even constructor
    pub result_type: Type,

    /// Source span
    pub span: Span,
}

impl ViewConstructor {
    /// Create a new view constructor
    pub fn new(name: Ident, params: List<FunctionParam>, result_type: Type, span: Span) -> Self {
        Self {
            name,
            type_params: List::new(),
            params,
            result_type,
            span,
        }
    }

    /// Check if this constructor has parameters
    pub fn has_params(&self) -> bool {
        !self.params.is_empty()
    }

    /// Get the number of parameters
    pub fn num_params(&self) -> usize {
        self.params.len()
    }
}

impl Spanned for ViewConstructor {
    fn span(&self) -> Span {
        self.span
    }
}

// =============================================================================
// Extern Block Declaration
// =============================================================================

/// Extern block declaration - groups FFI function declarations with a common ABI.
///
/// Extern block groups FFI function declarations under a shared calling convention.
///
/// # Syntax
/// ```verum
/// extern "C" {
///     fn malloc(size: Int) -> &unsafe Byte;
///     fn free(ptr: &unsafe Byte);
/// }
///
/// extern {
///     // Uses default platform ABI
///     fn custom_func();
/// }
/// ```
///
/// Functions inside an extern block are implicitly extern with the block's ABI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternBlockDecl {
    /// The ABI string (e.g., "C", "stdcall").
    /// None means default platform ABI (usually "C" on most platforms).
    pub abi: Maybe<Text>,

    /// Function declarations inside the block (without bodies).
    /// Each function inherits the extern ABI from the block.
    pub functions: List<FunctionDecl>,

    /// Attributes on the extern block
    pub attributes: List<Attribute>,

    /// Source span of the entire block
    pub span: Span,
}

impl ExternBlockDecl {
    /// Create a new extern block declaration
    pub fn new(abi: Maybe<Text>, functions: List<FunctionDecl>, span: Span) -> Self {
        Self {
            abi,
            functions,
            attributes: List::new(),
            span,
        }
    }

    /// Get the ABI as a string (defaults to "C" if not specified)
    pub fn abi_str(&self) -> &str {
        match &self.abi {
            Maybe::Some(abi) if !abi.is_empty() => abi.as_str(),
            _ => "C",
        }
    }

    /// Get the number of functions in this block
    pub fn num_functions(&self) -> usize {
        self.functions.len()
    }
}

impl Spanned for ExternBlockDecl {
    fn span(&self) -> Span {
        self.span
    }
}

// =============================================================================
// ACTIVE PATTERN DECLARATION
// Active pattern declaration for user-defined pattern matchers.
// =============================================================================

/// Active pattern declaration (F#-style custom pattern matcher).
///
/// Active patterns allow defining custom pattern matchers that can be used
/// in match expressions, providing a more expressive pattern matching system.
///
/// # Syntax
/// ```verum
/// // Simple active pattern
/// pattern Even(n: Int) -> Bool = n % 2 == 0;
///
/// // Parameterized active pattern
/// pattern InRange(lo: Int, hi: Int)(n: Int) -> Bool = lo <= n <= hi;
///
/// // Partial active pattern (returns Maybe for extraction)
/// pattern ParseInt(s: Text) -> Maybe<Int> = s.parse_int();
/// ```
///
/// # Usage in Match Expressions
/// ```verum
/// match n {
///     Even() => "even",
///     InRange(0, 100)() => "in valid range",
///     _ => "other",
/// }
/// ```
///
/// # Pattern Combination
/// Active patterns can be combined with `&` for conjunction:
/// ```verum
/// match n {
///     Even() & Positive() => "positive even",
///     _ => "other",
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatternDecl {
    /// Visibility modifier
    pub visibility: Visibility,

    /// Pattern name (e.g., "Even", "InRange")
    pub name: Ident,

    /// Generic type parameters - for generic patterns like First<T>
    /// Example: <T> in `pattern First<T>(list: List<T>) -> Maybe<T>`
    pub generics: List<crate::ty::GenericParam>,

    /// Pattern type parameters - for parameterized patterns like InRange(lo, hi)
    /// These are the parameters before the main pattern parameters.
    /// Example: (lo: Int, hi: Int) in `pattern InRange(lo: Int, hi: Int)(n: Int) -> Bool`
    pub type_params: List<FunctionParam>,

    /// Pattern parameters - the value being matched against
    /// Example: (n: Int) in `pattern Even(n: Int) -> Bool`
    pub params: List<FunctionParam>,

    /// Return type - Bool for total patterns, Maybe<T> for partial patterns
    pub return_type: Type,

    /// Pattern body - the expression that determines if the pattern matches
    pub body: Expr,

    /// Attributes (e.g., @inline)
    pub attributes: List<Attribute>,

    /// Source span
    pub span: Span,
}

impl PatternDecl {
    /// Create a new active pattern declaration.
    pub fn new(
        name: Ident,
        params: List<FunctionParam>,
        return_type: Type,
        body: Expr,
        span: Span,
    ) -> Self {
        Self {
            visibility: Visibility::Private,
            name,
            generics: List::new(),
            type_params: List::new(),
            params,
            return_type,
            body,
            attributes: List::new(),
            span,
        }
    }

    /// Create a parameterized active pattern declaration.
    pub fn parameterized(
        name: Ident,
        type_params: List<FunctionParam>,
        params: List<FunctionParam>,
        return_type: Type,
        body: Expr,
        span: Span,
    ) -> Self {
        Self {
            visibility: Visibility::Private,
            name,
            generics: List::new(),
            type_params,
            params,
            return_type,
            body,
            attributes: List::new(),
            span,
        }
    }

    /// Check if this is a parameterized pattern.
    pub fn is_parameterized(&self) -> bool {
        !self.type_params.is_empty()
    }

    /// Check if this is a partial pattern (returns Maybe<T>).
    pub fn is_partial(&self) -> bool {
        // Check if return type is Maybe<T>
        matches!(&self.return_type.kind, crate::ty::TypeKind::Generic { base, .. }
            if matches!(&base.kind, crate::ty::TypeKind::Path(path)
                if path.as_ident().map(|id| id.name.as_str() == "Maybe").unwrap_or(false)))
    }

    /// Check if this is a total pattern (returns Bool).
    pub fn is_total(&self) -> bool {
        matches!(&self.return_type.kind, crate::ty::TypeKind::Bool)
    }
}

impl Spanned for PatternDecl {
    fn span(&self) -> Span {
        self.span
    }
}
