//! Type representation for the Verum type system.
//!
//! This module defines the internal representation of types used during
//! type checking and inference, including:
//! - Primitive types (Int, Float, Bool, Text, Unit)
//! - Compound types (functions, tuples, arrays, records)
//! - Refinement types (the core innovation!)
//! - Generic types with const parameters
//! - Type variables for inference
//! - References (CBGR and ownership)

use indexmap::IndexMap;
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use verum_ast::{span::Span, ty::Path};
use verum_common::ToText;
use verum_common::well_known_types::WellKnownType as WKT;
use verum_common::{List, Map, Set, Text};

use crate::refinement::RefinementPredicate;

// =============================================================================
// DEPENDENT TYPES INFRASTRUCTURE (Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — )
// =============================================================================

pub use crate::universe_level::UniverseLevel;

/// Identity of one compile-time value binder. Names never establish equality.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ValueId(usize);
static VALUE_ID_COUNTER: AtomicUsize = AtomicUsize::new(0);
impl ValueId {
    pub fn fresh() -> Self {
        Self(VALUE_ID_COUNTER.fetch_update(Ordering::Relaxed, Ordering::Relaxed,
            |id| id.checked_add(1)).expect("value binder identity space exhausted"))
    }
    pub fn id(self) -> usize { self.0 }
}

/// Declaration values are rigid; only explicitly instantiated inference slots bind.
#[derive(Clone, Debug)]
pub enum ValueTerm {
    Parameter(ValueId),
    Inference(ValueId),
    Constant(verum_common::ConstValue),
}
#[path = "value_constant_identity.rs"]
mod value_constant_identity;
pub(crate) use value_constant_identity::same_constant_identity;
impl PartialEq for ValueTerm {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Parameter(left), Self::Parameter(right))
            | (Self::Inference(left), Self::Inference(right)) => left == right,
            (Self::Constant(left), Self::Constant(right)) =>
                value_constant_identity::same_constant_identity(left, right),
            _ => false,
        }
    }
}
impl ValueTerm {
    pub fn id(&self) -> Option<ValueId> {
        match self { Self::Parameter(id) | Self::Inference(id) => Some(*id), Self::Constant(_) => None }
    }
    pub fn as_constant(&self) -> Option<&verum_common::ConstValue> {
        match self { Self::Constant(value) => Some(value), _ => None }
    }
}

#[path = "record_field_types.rs"]
mod record_field_types;
pub use record_field_types::{RecordFieldSlot, RecordReceiver};

#[path = "type_children.rs"]
mod type_children;
#[path = "type_substitution.rs"]
mod type_substitution;

/// Term representation for equality types.
/// Equality types: propositional equality Eq<A, x, y> with reflexivity, symmetry, transitivity, substitution
///
/// Equality types compare terms, which can be variables, constants, or expressions.
#[derive(Debug, Clone, PartialEq)]
pub enum EqTerm {
    /// A variable reference
    Var(Text),
    /// A constant value
    Const(EqConst),
    /// Function application
    App {
        func: Box<EqTerm>,
        args: List<EqTerm>,
    },
    /// Lambda abstraction
    Lambda { param: Text, body: Box<EqTerm> },
    /// Projection from dependent pair
    Proj {
        pair: Box<EqTerm>,
        component: ProjComponent,
    },
    /// Reflexivity proof: refl<a>
    Refl(Box<EqTerm>),
    /// J eliminator for equality (path induction)
    J {
        /// The equality proof
        proof: Box<EqTerm>,
        /// The motive (dependent type family)
        motive: Box<EqTerm>,
        /// The base case (proof that motive holds for refl)
        base: Box<EqTerm>,
    },
}

/// Projection component for dependent pairs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjComponent {
    /// First projection (fst)
    Fst,
    /// Second projection (snd)
    Snd,
}

/// Constants in equality terms
#[derive(Debug, Clone, PartialEq)]
pub enum EqConst {
    /// Integer constant
    Int(i64),
    /// Boolean constant
    Bool(bool),
    /// Natural number constant
    Nat(u64),
    /// Unit constant
    Unit,
    /// Named constant (e.g., zero, true)
    Named(Text),
}

/// Quantity for Quantitative Type Theory
/// Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .4
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Quantity {
    /// Erased at runtime (proof-irrelevant, used 0 times)
    Zero,
    /// Linear (used exactly once)
    One,
    /// Unrestricted (used any number of times)
    Omega,
    /// Range: at most n times (affine when n=1)
    AtMost(u32),
    /// Graded: parameterized by a value
    Graded(u32),
}

impl Quantity {
    /// Zero quantity (erased)
    pub const ERASED: Self = Quantity::Zero;
    /// Linear quantity (exactly once)
    pub const LINEAR: Self = Quantity::One;
    /// Affine quantity (at most once)
    pub const AFFINE: Self = Quantity::AtMost(1);
    /// Unrestricted quantity
    pub const UNRESTRICTED: Self = Quantity::Omega;

    /// Check if this quantity allows at least n uses
    pub fn allows(&self, n: u32) -> bool {
        match self {
            Quantity::Zero => n == 0,
            Quantity::One => n == 1,
            Quantity::Omega => true,
            Quantity::AtMost(max) => n <= *max,
            Quantity::Graded(g) => n <= *g,
        }
    }

    /// Combine two quantities (additive)
    pub fn add(&self, other: &Self) -> Self {
        match (self, other) {
            (Quantity::Zero, q) | (q, Quantity::Zero) => *q,
            (Quantity::One, Quantity::One) => Quantity::AtMost(2),
            (Quantity::Omega, _) | (_, Quantity::Omega) => Quantity::Omega,
            (Quantity::AtMost(a), Quantity::AtMost(b)) => Quantity::AtMost(a + b),
            (Quantity::AtMost(a), Quantity::One) | (Quantity::One, Quantity::AtMost(a)) => {
                Quantity::AtMost(a + 1)
            }
            (Quantity::Graded(a), Quantity::Graded(b)) => Quantity::Graded(a + b),
            _ => Quantity::Omega,
        }
    }

    /// Multiply quantities (scaling)
    pub fn mul(&self, other: &Self) -> Self {
        match (self, other) {
            (Quantity::Zero, _) | (_, Quantity::Zero) => Quantity::Zero,
            (Quantity::One, q) | (q, Quantity::One) => *q,
            (Quantity::Omega, _) | (_, Quantity::Omega) => Quantity::Omega,
            (Quantity::AtMost(a), Quantity::AtMost(b)) => Quantity::AtMost(a * b),
            (Quantity::Graded(a), Quantity::Graded(b)) => Quantity::Graded(a * b),
            _ => Quantity::Omega,
        }
    }

    /// Subsumption ordering on the QTT lattice — `self <: other`
    /// returns true when a binding annotated `self` may be
    /// supplied where one annotated `other` is expected.  Read
    /// at the *type-side* convention: `q` denotes the maximum
    /// number of uses allowed; `0` is the smallest set, `Omega`
    /// the largest.  Lattice shape: `0 <: 1 <: AtMost(n) <: Omega`
    /// + `Graded(a) <: Graded(b)` when `a <= b`.  Omega is top,
    /// Zero is bottom.
    ///
    /// This was previously a 3-variant method on the verum_smt
    /// `dependent::Quantity` duplicate (`0 <: 1 <: ω`); lifted
    /// here as part of the canonical-home consolidation so the
    /// `AtMost` / `Graded` bands also have a defined subsumption
    /// answer.  The 3-variant smt semantics are preserved
    /// bit-exact for the (Zero, One, Omega) cells.
    pub fn subsumed_by(&self, other: &Self) -> bool {
        match (self, other) {
            // Top element: anything subsumed by Omega.
            (_, Quantity::Omega) => true,
            // Bottom element: Zero subsumed by anything — a
            // binding that allows 0 uses is satisfied wherever a
            // larger allowance is offered.  Mirrors the smt
            // 3-variant convention.
            (Quantity::Zero, _) => true,
            // Reflexivity for the basic cells.
            (Quantity::One, Quantity::One) => true,
            // 1 fits in AtMost(n) for any n >= 1.
            (Quantity::One, Quantity::AtMost(n)) => *n >= 1,
            // AtMost(a) fits in AtMost(b) when a <= b.
            (Quantity::AtMost(a), Quantity::AtMost(b)) => a <= b,
            // Graded subsumption is monotone in the grade.
            (Quantity::Graded(a), Quantity::Graded(b)) => a <= b,
            // No other comparable pairs.
            _ => false,
        }
    }
}

impl fmt::Display for Quantity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Quantity::Zero => write!(f, "0"),
            Quantity::One => write!(f, "1"),
            Quantity::Omega => write!(f, "ω"),
            Quantity::AtMost(n) => write!(f, "≤{}", n),
            Quantity::Graded(n) => write!(f, "@{}", n),
        }
    }
}

/// Constructor for inductive types
/// Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .1
#[derive(Debug, Clone, PartialEq)]
pub struct InductiveConstructor {
    /// Constructor name (e.g., "zero", "succ", "cons")
    pub name: Text,
    /// Type parameters (implicit)
    pub type_params: List<(Text, Box<Type>)>,
    /// Argument types
    pub args: List<Box<Type>>,
    /// Return type (must be the inductive type itself, possibly with indices)
    pub return_type: Box<Type>,
}

impl InductiveConstructor {
    /// Create a simple constructor with no arguments
    pub fn unit(name: Text, return_type: Type) -> Self {
        InductiveConstructor {
            name,
            type_params: List::new(),
            args: List::new(),
            return_type: Box::new(return_type),
        }
    }

    /// Create a constructor with arguments
    pub fn with_args(name: Text, args: List<Type>, return_type: Type) -> Self {
        InductiveConstructor {
            name,
            type_params: List::new(),
            args: args.into_iter().map(Box::new).collect(),
            return_type: Box::new(return_type),
        }
    }
}

/// Destructor for coinductive types
/// Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .2
#[derive(Debug, Clone, PartialEq)]
pub struct CoinductiveDestructor {
    /// Destructor name (e.g., "head", "tail")
    pub name: Text,
    /// Type of the observation (result of calling the destructor)
    pub result_type: Box<Type>,
}

impl CoinductiveDestructor {
    /// Create a destructor
    pub fn new(name: Text, result_type: Type) -> Self {
        CoinductiveDestructor {
            name,
            result_type: Box::new(result_type),
        }
    }
}

/// Path constructor for Higher Inductive Types
/// Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .3
#[derive(Debug, Clone, PartialEq)]
pub struct PathConstructor {
    /// Constructor name (e.g., "loop", "relate")
    pub name: Text,
    /// Type parameters (implicit)
    pub type_params: List<(Text, Box<Type>)>,
    /// Arguments to the path constructor
    pub args: List<Box<Type>>,
    /// Endpoints of the path (lhs = rhs)
    pub path_type: PathEndpoints,
}

/// Endpoints for a path in a Higher Inductive Type
#[derive(Debug, Clone, PartialEq)]
pub struct PathEndpoints {
    /// Type of the endpoints
    pub ty: Box<Type>,
    /// Left endpoint
    pub lhs: Box<EqTerm>,
    /// Right endpoint
    pub rhs: Box<EqTerm>,
}

impl PathConstructor {
    /// Create a simple path (loop) from a point to itself
    pub fn loop_at(name: Text, point: EqTerm, ty: Type) -> Self {
        PathConstructor {
            name,
            type_params: List::new(),
            args: List::new(),
            path_type: PathEndpoints {
                ty: Box::new(ty),
                lhs: Box::new(point.clone()),
                rhs: Box::new(point),
            },
        }
    }
}
/// A skolemised rank-2 binder: a rigid constant standing for a
/// universally quantified parameter while the quantifier is being
/// checked. A value that works for EVERY instantiation still unifies
/// against it; a value that needs the parameter to be one PARTICULAR
/// type cannot, which is exactly the promise a rank-2 field makes.
///
/// The constant has to live somewhere in [`Type`], and it is encoded as
/// a [`Type::Named`] carrying a reserved name. Producer and reader sit
/// here TOGETHER on purpose: the one consumer that needed to recognise
/// it — the field-mismatch diagnostic — had no way to ask, so it showed
/// the user the raw constant (`?skolem$TypeVar(3853)`), which names
/// nothing in their program (T1038). A consumer matching the spelling
/// by hand would also go stale the moment the spelling changed.
pub mod skolem {
    /// Reserved prefix. Not writable in Verum source (`?` cannot start
    /// an identifier), so it cannot collide with a user type.
    const PREFIX: &str = "?forall-bound#";

    /// The only producer of a skolem's name.
    pub fn name(var_id: usize) -> String {
        format!("{PREFIX}{var_id}")
    }

    /// The only reader: is this RENDERED type a skolem constant?
    pub fn is_rendered(rendered: &str) -> bool {
        rendered.starts_with(PREFIX)
    }
}


// =============================================================================
// END DEPENDENT TYPES INFRASTRUCTURE
// =============================================================================

/// A type in the Verum type system.
///
/// This is the internal representation used during type checking.
/// It's separate from the AST type representation to allow for
/// type variables, substitutions, and inference artifacts.
///
/// # Affine Type Tracking
///
/// Some types (those declared with `type affine T is ...`) have affine
/// semantics, meaning values can be used at most once. This is tracked
/// per-type-name and enforced during type checking.
#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    /// Unit type: ()
    Unit,

    /// Never type: ! (bottom type for diverging control flow)
    ///
    /// Represents computations that never return normally:
    /// - return statements
    /// - break/continue statements
    /// - infinite loops
    /// - panic/abort
    ///
    /// The Never type is a subtype of all types (can unify with anything).
    Never,

    /// Unknown type: unknown (top type for FFI and external boundaries)
    ///
    /// The dual of Never: any value can be assigned to unknown, but nothing
    /// can be done with it without explicit type narrowing.
    ///
    /// Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 13.2 - Unknown Type
    ///
    /// Subtyping rules:
    /// - T <: unknown (any type is a subtype of unknown)
    /// - unknown <: T only if T == unknown
    ///
    /// Use cases:
    /// - FFI boundaries where the exact type is unknown
    /// - Deserialization from external sources
    /// - Type-safe gradual typing
    Unknown,

    /// Boolean type
    Bool,

    /// Canonical signed 64-bit integer type.
    Int,

    /// Floating-point type (IEEE 754 double)
    Float,

    /// Character type (Unicode scalar value)
    Char,

    /// Text type (UTF-8)
    Text,

    /// Type variable (for inference)
    Var(TypeVar),

    /// A type-valued entry in a nominal record telescope. Before introduction
    /// it is bound by the declaration; after elimination by its actual receiver.
    RecordField {
        slot: Box<RecordFieldSlot>,
        receiver: Option<Box<RecordReceiver>>,
        level: UniverseLevel,
    },

    /// Named type (user-defined or protocol)
    ///
    /// Note: Affine tracking is done in the type context, not here.
    /// The `path` identifies the type, and the context tracks whether
    /// it's affine/linear based on its declaration.
    Named { path: Path, args: List<Type> },

    /// Generic type with simple name (convenience alias for common stdlib types)
    ///
    /// This is a simpler form of Named for standard library generics like
    /// List<T>, Maybe<T>, Map<K,V>, Result<T,E>, Set<T>.
    ///
    /// PERF: Uses Text name directly instead of Path for faster matching.
    Generic { name: Text, args: List<Type> },

    /// Function type: A -> B using [contexts] with properties
    ///
    /// Context type system integration: context requirements tracked in function types, checked at call sites — Type System Integration (DI contexts)
    /// Generic bounds tracking: type parameters carry protocol constraints (e.g., T: Ord) that are checked at instantiation sites
    /// Context system integration: function types carry context requirements ("using [Ctx]") checked at call sites
    /// Computational properties: compile-time tracking of Pure, IO, Async, Fallible, Mutates effects inferred from function bodies — (purity tracking)
    /// Context system: capability-based dependency injection with "context" declarations, "using" requirements, "provide" injection, ~5-30ns runtime overhead via task-local storage — Context requirements (DI)
    ///
    /// Example: `fn foo(x: Int) -> Bool using [Database, Logger]`
    /// Creates Function { params: [Int], return_type: Bool, contexts: Some(Concrete(...)), type_params: [], properties: Pure }
    ///
    /// Example with generics: `fn sort<T: Ord>(list: List<T>) -> List<T>`
    /// Creates Function { params: [List<T>], return_type: List<T>, contexts: None, type_params: [T: Ord], properties: Pure }
    ///
    /// Example with context polymorphism: `fn map<T, U, using C>(iter: I, f: fn(T) -> U using C) -> MapIter<T, U> using C`
    /// Creates Function with contexts: Some(Variable(C)) where C is unified with callback's contexts
    ///
    /// Example with properties: `async fn fetch(url: Text) -> Result<Data>`
    /// Creates Function { params: [Text], return_type: Result<Data>, contexts: None, type_params: [], properties: {Async, Fallible, IO} }
    Function {
        params: List<Type>,
        return_type: Box<Type>,
        /// DI context requirements - either concrete or a type variable for polymorphism
        /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.5 - Context Polymorphism
        contexts: Option<crate::di::requirement::ContextExpr>,
        type_params: List<crate::context::TypeParam>, // Generic type parameters with bounds
        properties: Option<crate::computational_properties::PropertySet>, // Computational properties (None = not yet inferred)
        property_requirement: crate::computational_properties::PropertyRequirement,
    },

    /// Tuple type: (T1, T2, ..., Tn)
    Tuple(List<Type>),

    /// Array type with size: [T; N]
    Array {
        element: Box<Type>,
        size: Option<usize>,
    },

    /// Slice type: [T] (dynamically-sized view)
    Slice { element: Box<Type> },

    /// Record type: { field1: T1, field2: T2, ... }
    Record(IndexMap<Text, Type>),

    /// Extensible record type with row polymorphism: { field1: T1, field2: T2, ... | r }
    /// Pattern matching: exhaustiveness checking, type narrowing in match arms, irrefutable patterns — .1 - Row Polymorphism
    ///
    /// Row polymorphism allows functions to work with records that have at least
    /// certain fields, without requiring an exact match. The row variable `r`
    /// captures the "rest" of the record fields.
    ///
    /// Example:
    /// ```verum
    /// // This function works with any record that has an `x` field of type Int
    /// fn get_x<r>(point: {x: Int | r}) -> Int = point.x
    ///
    /// let p2d = {x: 1, y: 2} // {x: Int, y: Int}
    /// let p3d = {x: 1, y: 2, z: 3} // {x: Int, y: Int, z: Int}
    ///
    /// get_x(p2d) // OK: r = {y: Int}
    /// get_x(p3d) // OK: r = {y: Int, z: Int}
    /// ```
    ExtensibleRecord {
        /// Known fields of the record
        fields: IndexMap<Text, Type>,
        /// Row variable capturing additional fields (or None if closed)
        /// When Some, the record can have additional fields beyond those specified
        row_var: Option<TypeVar>,
    },

    /// Variant type: Tag1(T1) | Tag2(T2) | ...
    Variant(IndexMap<Text, Type>),

    /// CBGR reference: &T or &mut T (~15ns overhead)
    /// ThinRef layout: 16 bytes (pointer + generation counter + epoch capabilities) — Three-Tier Model Tier 1
    Reference { mutable: bool, inner: Box<Type> },

    /// Checked reference: &checked T or &checked mut T
    /// FatRef layout: 24 bytes (pointer + generation counter + epoch capabilities + length) for unsized types — Three-Tier Model Tier 2
    /// Runtime bounds checking, no CBGR overhead
    CheckedReference { mutable: bool, inner: Box<Type> },

    /// Unsafe reference: &unsafe T or &unsafe mut T
    /// Reference safety invariants: managed refs validated at dereference, checked refs proven safe at compile time, unsafe refs unchecked — Three-Tier Model Tier 3
    /// Zero-cost, no checks, maximum performance
    UnsafeReference { mutable: bool, inner: Box<Type> },

    /// Ownership reference (zero-cost): %T or %mut T
    Ownership { mutable: bool, inner: Box<Type> },

    /// Raw pointer: *const T or *mut T
    Pointer { mutable: bool, inner: Box<Type> },

    /// Volatile pointer for MMIO: *volatile T or *volatile mut T
    ///
    /// Volatile pointers guarantee that reads/writes are not optimized away
    /// or reordered by the compiler. Essential for memory-mapped I/O
    /// and hardware register access.
    VolatilePointer { mutable: bool, inner: Box<Type> },

    /// Refinement type: T{predicate}
    /// This is Verum's key innovation!
    Refined {
        base: Box<Type>,
        predicate: RefinementPredicate,
    },

    /// Existential type: ∃α. T
    Exists { var: TypeVar, body: Box<Type> },

    /// Dynamic protocol object type: dyn Display + Debug
    /// Syntax grammar: recursive-descent parseable (LL(k), k<=3), reserved keywords only let/fn/is, unified "type X is" definitions — Dynamic dispatch through protocol objects
    ///
    /// Unlike static protocol bounds, DynProtocol types perform runtime
    /// dynamic dispatch. The value is stored as a fat pointer containing
    /// both the data pointer and vtable pointer.
    ///
    /// Examples:
    /// - `dyn Display` - single protocol object
    /// - `dyn Display + Debug` - multiple protocol bounds
    /// - `dyn Iterator<Item = Int>` - with associated type bindings
    DynProtocol {
        /// Protocol bounds that the dynamic type must satisfy
        bounds: List<Text>,
        /// Associated type bindings (e.g., Item = Int for Iterator)
        bindings: Map<Text, Type>,
    },

    /// Universally quantified type (for type schemes)
    /// Internal use only - not directly writable by users
    Forall {
        vars: List<TypeVar>,
        body: Box<Type>,
    },

    /// Meta parameter: compile-time value parameter
    /// Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Unified compile-time computation
    /// Examples:
    /// - `N: meta usize` - compile-time usize value
    /// - `Shape: meta [usize]` - compile-time usize array
    /// - `N: meta usize{> 0}` - with refinement constraint
    ///
    /// Meta parameters replace const generics with unified meta-system.
    /// All compile-time computation uses `meta` (no `const fn`, no `const N: usize`).
    Meta {
        /// Parameter name (for error messages and debugging).
        /// At declaration sites this holds the user-visible parameter name (e.g. "N").
        /// For constant arguments the name is the concrete
        /// value's canonical Display form (e.g. "2"), preserved for diagnostics.
        name: Text,
        /// Base type of the meta parameter (e.g., usize, [usize], bool)
        ty: Box<Type>,
        /// Optional refinement constraint (e.g., {> 0})
        refinement: Option<RefinementPredicate>,
        /// Exact owned identity or constant; declaration and inference roles are distinct.
        value: ValueTerm,
    },

    /// Future type: Future<T>
    /// Async/await integration: async functions return Future<T>, await extracts T, select for multi-future - Async/Await Integration
    /// Represents an asynchronous computation that yields a value of type T.
    /// Created by async functions and blocks.
    Future {
        /// The output type of the future
        output: Box<Type>,
    },

    /// Generator type: Generator<Yield, Return>
    /// Generator functions: fn* syntax yields values lazily, producing Iterator<Item=T> types
    /// Represents a generator that yields values of type Yield
    /// and returns a final value of type Return.
    Generator {
        /// Type of values yielded by the generator
        yield_ty: Box<Type>,
        /// Final return type of the generator
        return_ty: Box<Type>,
    },

    /// Tensor type with compile-time shape parameters
    /// Tensor types: Tensor<T, Shape: meta [usize]> with compile-time shape tracking for N-dimensional arrays
    ///
    /// Examples:
    /// - Tensor<f32, [4]> // 1D vector (4 elements)
    /// - Tensor<f32, [2, 3]> // 2D matrix (2×3)
    /// - Tensor<u8, [3, 224, 224]> // 3D tensor (RGB image)
    ///
    /// The shape is validated at compile-time using meta parameters,
    /// and strides are computed for efficient row-major indexing.
    Tensor {
        /// Element type (e.g., f32, f64, Int)
        element: Box<Type>,

        /// Shape dimensions as compile-time meta parameters
        /// Example: [4] for 1D, [2, 3] for 2D matrix
        /// Stored as ConstValue for compile-time evaluation
        shape: List<verum_common::ConstValue>,

        /// Strides for row-major layout
        /// Computed as: strides[i] = product(shape[i+1..])
        /// Used for efficient multi-dimensional indexing
        strides: List<usize>,

        span: Span,
    },

    /// Lifetime type parameter
    /// Subtyping: structural subtyping for records, refinement subtyping (T{P} <: T when P holds), protocol-based nominal subtyping — .2 - Lifetime parameters
    ///
    /// Verum uses CBGR for memory safety, but lifetimes can be explicitly
    /// specified for complex cases or interop with systems that use them.
    /// Common lifetimes:
    /// - 'a, 'b, etc. - Named lifetimes for tracking reference relationships
    /// - 'static - Lifetime of the entire program
    ///
    /// Note: Unlike Rust, Verum lifetimes are optional annotations.
    /// CBGR handles most memory safety concerns at runtime.
    Lifetime {
        /// Name of the lifetime (e.g., "a" for 'a, "static" for 'static)
        name: Text,
    },

    /// Generation-aware reference: GenRef<T>
    /// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .2 lines 143-193, Section 2.3 lines 533-547
    ///
    /// GenRef wraps a CBGR reference with explicit generation tracking,
    /// enabling lending iterators and self-referential types without lifetime annotations.
    ///
    /// # Memory Layout
    ///
    /// ```text
    /// GenRef<T> {
    ///  ptr: *const T // 8 bytes
    ///  generation: u64 // 8 bytes
    /// }
    /// Total: 16 bytes (overhead: ~20ns = 15ns CBGR + 5ns generation check)
    /// ```
    ///
    /// # Example
    ///
    /// ```verum
    /// type WindowIterator<T> {
    ///  data: GenRef<List<T>>,
    ///  window_size: usize,
    ///  position: usize
    /// }
    ///
    /// implement<T> Iterator for WindowIterator<T> {
    ///  type Item is [T]
    ///
    ///  fn next(&mut self) -> Maybe<GenRef<&[T]>> {
    ///  let data = self.data.deref()?;
    ///  if self.position + self.window_size <= data.len() {
    ///  let slice = &data[self.position..self.position + self.window_size];
    ///  self.position += 1;
    ///  Some(GenRef.borrow(slice))
    ///  } else {
    ///  None
    ///  }
    ///  }
    /// }
    /// ```
    GenRef {
        /// Inner type being referenced with generation tracking
        inner: Box<Type>,
    },

    /// Type constructor for higher-kinded types: F<_>
    /// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .3 lines 410-437
    ///
    /// Represents a type constructor (a type-level function) that takes type arguments.
    /// Examples:
    /// - `List` has kind `* -> *` (takes one type, produces a type)
    /// - `Map` has kind `* -> * -> *` (takes two types, produces a type)
    /// - `Functor` protocol requires `type F<_>` (a type constructor)
    ///
    /// # Example
    ///
    /// ```verum
    /// protocol Functor {
    ///  type F<_> // Type constructor with arity 1
    ///
    ///  fn map<A, B>(self: Self.F<A>, f: fn(A) -> B) -> Self.F<B>
    /// }
    ///
    /// // List is a type constructor
    /// implement Functor for ListFunctor {
    ///  type F<T> is List<T> // TypeConstructor { name: "List", arity: 1, kind: * -> * }
    ///
    ///  fn map<A, B>(self: List<A>, f: fn(A) -> B) -> List<B> {
    ///  self.iter().map(f).collect()
    ///  }
    /// }
    /// ```
    TypeConstructor {
        /// Name of the type constructor (e.g., "List", "Map", "Maybe")
        name: Text,

        /// Number of type parameters this constructor takes
        arity: usize,

        /// Kind of this type constructor (e.g., * -> *, * -> * -> *)
        kind: crate::advanced_protocols::Kind,
    },

    /// Type application for higher-kinded types: F<T>
    /// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .3 lines 410-437
    ///
    /// Applies a type constructor to type arguments to produce a concrete type.
    /// This is separate from Named types because it preserves the higher-kinded
    /// structure needed for protocol resolution and type inference.
    ///
    /// # Example
    ///
    /// ```verum
    /// // Given: type F<_> is a type constructor
    /// // TypeApp applies F to Int to get F<Int>
    ///
    /// protocol Monad {
    ///  type M<_> // Type constructor
    ///
    ///  fn pure<T>(value: T) -> Self.M<T>
    ///  fn bind<T, U>(self: Self.M<T>, f: fn(T) -> Self.M<U>) -> Self.M<U>
    /// }
    ///
    /// // TypeApp { constructor: M, args: [Int] } represents M<Int>
    /// // TypeApp { constructor: M, args: [Bool] } represents M<Bool>
    /// ```
    TypeApp {
        /// The type constructor being applied
        constructor: Box<Type>,

        /// Type arguments to apply to the constructor
        args: List<Type>,
    },

    // ==========================================================================
    // DEPENDENT TYPES (Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — )
    // ==========================================================================
    /// Pi Type (Dependent Function): (x: A) -> B(x)
    /// Pi types (dependent functions): (x: A) -> B(x) where return type depends on input value, non-dependent functions are special case
    ///
    /// The return type B depends on the input value x.
    /// This is the foundation for dependent function types.
    ///
    /// # Examples
    /// ```text
    /// // Simple function type is special case
    /// i32 -> bool ≡ (_: i32) -> bool
    ///
    /// // Dependent function type
    /// fn replicate<T>(n: Nat) -> List<T, n> // Return type depends on n
    /// fn printf(fmt: Text) -> ParseFormat(fmt) -> Text
    /// ```
    ///
    /// # Type Theory
    /// Pi types are introduced by lambda abstraction and eliminated by function application.
    /// ```text
    /// Γ, x: A ⊢ b: B[x]
    /// ─────────────────────── (Π-Intro)
    /// Γ ⊢ λx. b : (x: A) → B
    ///
    /// Γ ⊢ f : (x: A) → B Γ ⊢ a : A
    /// ──────────────────────────────── (Π-Elim)
    /// Γ ⊢ f a : B[a/x]
    /// ```
    Pi {
        /// Name of the dependent parameter (for display and substitution)
        param_name: Text,
        /// Type of the parameter
        param_type: Box<Type>,
        /// Return type (may reference param_name)
        return_type: Box<Type>,
    },

    /// Sigma Type (Dependent Pair): (x: A, B(x))
    /// Sigma types (dependent pairs): (x: A, B(x)) where second component type depends on first value, refinement types desugar to Sigma
    ///
    /// The type of the second component depends on the value of the first.
    /// Sigma types are used for existential quantification and dependent records.
    ///
    /// # Examples
    /// ```text
    /// // Dependent pair
    /// type BoundedInt is n: i32 where n >= 0 && n <= 100
    /// // Desugars to: Sigma<i32, \n -> Proof(n >= 0 && n <= 100)>
    ///
    /// // Parse result where success type depends on bool
    /// type ParseResult is (success: bool, if success then AST else Error)
    ///
    /// // Subset types
    /// type PositiveInt is n: i32 where n > 0
    /// // Desugars to: Sigma<i32, \n -> Proof(n > 0)>
    /// ```
    ///
    /// # Type Theory
    /// Sigma types are introduced by dependent pairs and eliminated by projection.
    /// ```text
    /// Γ ⊢ a : A Γ ⊢ b : B[a/x]
    /// ──────────────────────────────── (Σ-Intro)
    /// Γ ⊢ (a, b) : (x: A) × B
    ///
    /// Γ ⊢ p : (x: A) × B
    /// ────────────────── (Σ-Elim₁)
    /// Γ ⊢ fst p : A
    ///
    /// Γ ⊢ p : (x: A) × B
    /// ──────────────────────── (Σ-Elim₂)
    /// Γ ⊢ snd p : B[fst p/x]
    /// ```
    Sigma {
        /// Name of the first component (for display and substitution)
        fst_name: Text,
        /// Type of the first component
        fst_type: Box<Type>,
        /// Type of the second component (may reference fst_name)
        snd_type: Box<Type>,
    },

    /// Equality Type: Eq<A, x, y> or x = y
    /// Equality types: propositional equality Eq<A, x, y> with reflexivity, symmetry, transitivity, substitution
    ///
    /// Propositional equality between two values of the same type.
    ///
    /// # Examples
    /// ```text
    /// // Reflexivity
    /// refl<A, x: A> : Eq<A, x, x>
    ///
    /// // Symmetry (derivable)
    /// fn sym<A, x: A, y: A>(eq: x = y) -> y = x
    ///
    /// // Transitivity (derivable)
    /// fn trans<A, x: A, y: A, z: A>(eq1: x = y, eq2: y = z) -> x = z
    ///
    /// // Substitution principle
    /// fn subst<A, P: A -> Type, x: A, y: A>(eq: x = y, px: P(x)) -> P(y)
    /// ```
    ///
    /// # Type Theory (Identity Type)
    /// ```text
    /// Γ ⊢ a : A
    /// ─────────────────── (Id-Intro / refl)
    /// Γ ⊢ refl : a = a
    ///
    /// Γ ⊢ p : a = b Γ, x: A, h: a = x ⊢ C : Type Γ ⊢ c : C[a/x, refl/h]
    /// ─────────────────────────────────────────────────────────────────────── (J-Elim)
    /// Γ ⊢ J(p, c) : C[b/x, p/h]
    /// ```
    Eq {
        /// The type of the values being compared
        ty: Box<Type>,
        /// Left-hand side of equality
        lhs: Box<EqTerm>,
        /// Right-hand side of equality
        rhs: Box<EqTerm>,
    },

    /// Cubical Path Type: Path<A>(a, b)
    ///
    /// The type of paths (equalities with computational content) in cubical
    /// type theory (Cohen–Coquand–Huber–Mörtberg 2015). A path from `a` to `b`
    /// in type `A` is a function from the abstract interval `I` to `A` that
    /// computes to `a` at `i0` and to `b` at `i1`.
    ///
    /// # Examples
    /// ```verum
    /// // Path between two values
    /// let p: Path<Int>(3, 3) = refl(3);
    ///
    /// // Path in a function type (function extensionality)
    /// let q: Path<fn(Int) -> Int>(f, g) = funext(h);
    ///
    /// // Path in a universe (univalence)
    /// let r: Path<Type>(A, B) = ua(equiv);
    /// ```
    ///
    /// # Relation to Eq
    /// `Path<A>(a, b)` is the cubical refinement of `Eq<A, a, b>`. While `Eq`
    /// uses J-elimination (path induction), `Path` has direct computational
    /// content via transport and hcomp. The cubical normalizer in `cubical.rs`
    /// provides reduction rules for Path terms.
    ///
    /// # Type Theory
    /// ```text
    /// Γ, i: I ⊢ e : A e[i0/i] ≡ a e[i1/i] ≡ b
    /// ─────────────────────────────────────────────────── (Path-Intro)
    /// Γ ⊢ λ(i). e : Path<A>(a, b)
    ///
    /// Γ ⊢ p : Path<A>(a, b) Γ ⊢ r : I
    /// ───────────────────────────────────── (Path-Elim)
    /// Γ ⊢ p @ r : A
    /// ```
    PathType {
        /// The type of values connected by the path
        space: Box<Type>,
        /// Left endpoint (at i0)
        left: Box<crate::cubical::CubicalTerm>,
        /// Right endpoint (at i1)
        right: Box<crate::cubical::CubicalTerm>,
    },

    /// Abstract Interval Type: I
    ///
    /// The abstract interval with two endpoints `i0` and `i1`. Not a regular
    /// type in the universe hierarchy — it is a "cofibrant" object used only
    /// for constructing paths. Variables of type `I` are dimension variables.
    ///
    /// # Properties
    /// - `I` is not in any universe (`I : ☐` where ☐ is outside the hierarchy)
    /// - De Morgan algebra: meets, joins, reversals on interval expressions
    /// - Functions `I → A` represent paths in `A`
    ///
    /// # Examples
    /// ```verum
    /// // Dimension variable
    /// fn my_path(i: I) -> A { ... }
    ///
    /// // Interval endpoints
    /// let start: I = i0;
    /// let end: I = i1;
    /// ```
    Interval,

    /// Partial Element Type: Partial<A>(φ)
    ///
    /// A partial element of type A defined on the extent where the face
    /// formula φ holds. Used in homogeneous composition (hcomp) to
    /// specify the boundary data of a cube filling.
    ///
    /// # Examples
    /// ```verum
    /// // A partial element defined when i = i0 or i = i1
    /// let walls: Partial<A>(φ) = ...;
    ///
    /// // hcomp uses partial elements for its side faces
    /// hcomp<A>(φ, walls, base) : A
    /// ```
    ///
    /// # Type Theory
    /// ```text
    /// Γ ⊢ A : Type Γ ⊢ φ : I Γ, (φ = i1) ⊢ u : A
    /// ───────────────────────────────────────────────────── (Partial-Intro)
    /// Γ ⊢ [φ ↦ u] : Partial<A>(φ)
    /// ```
    Partial {
        /// The type of the partial element.
        element_type: Box<Type>,
        /// The face formula (encoded as a cubical term over interval variables).
        face: Box<crate::cubical::CubicalTerm>,
    },

    /// Universe Level: Type_n
    /// Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter
    ///
    /// Types have types (called universes) forming a hierarchy that prevents paradoxes.
    /// ```text
    /// Type : Type₁
    /// Type₁ : Type₂
    /// Type₂ : Type₃
    /// ... (infinite hierarchy)
    /// ```
    ///
    /// # Cumulative Universes
    /// Types at level n are implicitly at level n+1:
    /// ```text
    /// If A : Type_n, then A : Type_{n+1} (cumulativity)
    /// ```
    ///
    /// # Universe Polymorphism
    /// Functions can be universe-polymorphic:
    /// ```text
    /// fn identity<u: Level>(T: Type u, x: T) -> T = x
    /// ```
    Universe {
        /// Universe level (0 = Type, 1 = Type₁, etc.)
        level: UniverseLevel,
    },

    /// Proposition universe: Prop
    /// Inductive types: recursive type definitions with structural recursion, termination checking — .1
    ///
    /// Proof-irrelevant propositions. All proofs of a proposition are equal.
    /// Used for logical reasoning without computational content.
    ///
    /// # Properties
    /// - Proof irrelevance: ∀ P: Prop, p1 p2: P → p1 = p2
    /// - Impredicativity: ∀ P: Prop, (∀ x: A. P) : Prop
    /// - Squash: |A| : Prop (erases computational content)
    ///
    /// # Examples
    /// ```verum
    /// // All proofs of a proposition are equal
    /// axiom proof_irrelevance:
    ///  [P: Prop] -> (p1: P) -> (p2: P) -> p1 = p2
    ///
    /// // Squash types into propositions
    /// type Squash<A: Type> : Prop is ∃(_: A). True
    ///
    /// // Subset types with irrelevant proofs
    /// type BoundedInt is n: i32 where 0 <= n && n <= 100 :~ Prop
    /// ```
    Prop,

    /// Inductive Type Definition
    /// Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .1
    ///
    /// Inductive types are defined by constructors and eliminated by pattern matching.
    /// They form the basis for algebraic data types with dependent indices.
    ///
    /// # Examples
    /// ```verum
    /// // Natural numbers
    /// inductive Nat : Type {
    ///  zero : Nat,
    ///  succ : Nat -> Nat
    /// }
    ///
    /// // Indexed list (length-tracked)
    /// inductive List<A: Type> : Nat -> Type {
    ///  nil : List<A, zero>,
    ///  cons : <n> -> A -> List<A, n> -> List<A, succ(n)>
    /// }
    ///
    /// // Finite sets
    /// inductive Fin : Nat -> Type {
    ///  FZero : <n> -> Fin<succ(n)>,
    ///  FSucc : <n> -> Fin<n> -> Fin<succ(n)>
    /// }
    /// ```
    Inductive {
        /// Name of the inductive type
        name: Text,
        /// Type parameters (non-dependent)
        params: List<(Text, Box<Type>)>,
        /// Index parameters (dependent)
        indices: List<(Text, Box<Type>)>,
        /// Target universe
        universe: UniverseLevel,
        /// Constructors
        constructors: List<InductiveConstructor>,
    },

    /// Coinductive Type Definition
    /// Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .2
    ///
    /// Coinductive types are defined by destructors and support infinite structures.
    /// They are the dual of inductive types.
    ///
    /// # Examples
    /// ```verum
    /// // Infinite streams
    /// coinductive Stream<A: Type> : Type {
    ///  head : Stream<A> -> A,
    ///  tail : Stream<A> -> Stream<A>
    /// }
    ///
    /// // Infinite stream of naturals
    /// fn nats_from(n: Nat) : Stream<Nat> = {
    ///  head = n,
    ///  tail = nats_from(succ(n))
    /// }
    ///
    /// // Productivity must be checked (no infinite unfolding)
    /// fn map<A, B>(f: A -> B, s: Stream<A>) : Stream<B> = {
    ///  head = f(s.head),
    ///  tail = map(f, s.tail) // Productive recursive call
    /// }
    /// ```
    Coinductive {
        /// Name of the coinductive type
        name: Text,
        /// Type parameters
        params: List<(Text, Box<Type>)>,
        /// Destructors (observations)
        destructors: List<CoinductiveDestructor>,
    },

    /// Higher Inductive Type (HIT)
    /// Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .3
    ///
    /// Higher inductive types extend inductive types with path constructors
    /// that generate equality proofs. Used in Homotopy Type Theory.
    ///
    /// # Examples
    /// ```verum
    /// // Circle with a loop
    /// hott inductive Circle : Type {
    ///  base : Circle,
    ///  loop : base = base // Path constructor
    /// }
    ///
    /// // Torus with paths and surface
    /// hott inductive Torus : Type {
    ///  point : Torus,
    ///  meridian : point = point,
    ///  longitude : point = point,
    ///  surface : meridian · longitude = longitude · meridian // 2-path
    /// }
    ///
    /// // Quotient types
    /// hott inductive Quotient<A: Type, R: A -> A -> Type> : Type {
    ///  class : A -> Quotient<A, R>,
    ///  relate : (x, y: A) -> R(x, y) -> class(x) = class(y)
    /// }
    /// ```
    HigherInductive {
        /// Name of the HIT
        name: Text,
        /// Type parameters
        params: List<(Text, Box<Type>)>,
        /// Point constructors (value-level)
        point_constructors: List<InductiveConstructor>,
        /// Path constructors (equality-level)
        path_constructors: List<PathConstructor>,
    },

    /// Quantified Type (Quantitative Type Theory)
    /// Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .4
    ///
    /// Track resource usage with quantities (0, 1, ω).
    /// Enables linear and affine type semantics.
    ///
    /// # Quantities
    /// - 0: Erased (not used at runtime)
    /// - 1: Linear (used exactly once)
    /// - ω: Unrestricted (used any number of times)
    ///
    /// # Examples
    /// ```verum
    /// // Linear types (use exactly once)
    /// fn linear_use(x: Text @1) -> Text @1 = x
    ///
    /// // Affine types (use at most once)
    /// fn affine_use(x: File @0..1) -> Maybe<Text> @0..1
    ///
    /// // Unrestricted (use any number of times)
    /// fn normal_use(x: i32 @ω) -> i32 @ω = x + x
    ///
    /// // Graded modalities
    /// fn graded<T, n: Nat>(x: Resource @n) -> Result<T> @n
    /// ```
    Quantified {
        /// Inner type
        inner: Box<Type>,
        /// Usage quantity
        quantity: Quantity,
    },

    /// Placeholder type for order-independent (two-pass) type resolution.
    ///
    /// During the first pass of type checking, type names are registered as
    /// placeholders before their definitions are fully resolved. This allows
    /// forward references between types:
    ///
    /// ```verum
    /// type SearchRequest is {
    ///  sort_by: SortOrder, // SortOrder referenced before definition
    /// };
    /// type SortOrder is Relevance | Downloads;
    /// ```
    ///
    /// In the first pass, `SortOrder` is registered as `Placeholder { name: "SortOrder", span }`.
    /// In the second pass, it is resolved to the actual variant type.
    ///
    /// Any `Placeholder` types remaining after the second pass indicate
    /// unresolved type references, which should be reported as errors.
    Placeholder {
        /// The name of the type being referenced
        name: Text,
        /// Source location for error reporting
        span: Span,
    },

    /// Capability-restricted type: T with [Capabilities]
    /// Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 12 - Capability Attenuation as Types
    ///
    /// Represents a type with a restricted set of capabilities. This is used
    /// for capability-based method filtering where:
    /// - T with [A, B, C] <: T with [A, B] (more caps = subtype of fewer caps)
    /// - Method calls requiring capability C are only valid if C is in the set
    ///
    /// # Examples
    /// ```verum
    /// // Database context with read-only capability
    /// fn read_data(db: Database with [Query]) -> Result<Data>
    ///
    /// // Attenuated context in function parameter
    /// fn process(ctx: AppContext with [ReadOnly, Logging]) -> ()
    /// ```
    CapabilityRestricted {
        /// Base type being capability-restricted
        base: Box<Type>,
        /// Structured set of capabilities for compile-time verification.
        /// Uses TypeCapabilitySet for proper set operations (subset, superset, intersection)
        /// enabling correct capability attenuation subtyping:
        /// T with [A, B, C] <: T with [A, B] (more capabilities => subtype)
        capabilities: crate::capability::TypeCapabilitySet,
    },
}

impl Type {
    /// Create a unit type
    pub fn unit() -> Self {
        Type::Unit
    }

    /// Create a never type (bottom type for diverging control flow)
    pub fn never() -> Self {
        Type::Never
    }

    /// Recognize the primitive bottom type. Aliases require resolution by
    /// their declaration owner; a nominal type's spelling is not evidence.
    pub fn is_never(&self) -> bool {
        matches!(self, Type::Never)
    }

    /// Returns the display name for primitive type variants.
    ///
    /// This is the single source of truth for converting primitive Type variants
    /// to their string representation. All crates should use this instead of
    /// duplicating match arms.
    ///
    /// Returns `None` for non-primitive variants (Named, Var, Function, etc.)
    /// which require more context to format.
    pub fn primitive_name(&self) -> Option<&'static str> {
        match self {
            Type::Unit => Some("Unit"),
            Type::Bool => Some("Bool"),
            Type::Int => Some("Int"),
            Type::Float => Some("Float"),
            Type::Char => Some("Char"),
            Type::Text => Some("Text"),
            Type::Never => Some("Never"),
            Type::Unknown => Some("unknown"),
            _ => None,
        }
    }

    /// Create a bool type
    pub fn bool() -> Self {
        Type::Bool
    }

    /// Create an int type
    pub fn int() -> Self {
        Type::Int
    }

    /// Canonical full-width unsigned scalar, including opaque type identities.
    pub fn uint64() -> Self {
        Self::Named {
            path: Path::single(verum_ast::Ident::new(
                verum_common::well_known_types::type_names::UINT64, Span::default(),
            )),
            args: List::new(),
        }
    }

    /// Create a float type
    pub fn float() -> Self {
        Type::Float
    }

    /// Create a text type
    pub fn text() -> Self {
        Type::Text
    }

    /// Create a char type
    pub fn char() -> Self {
        Type::Char
    }

    /// Create a List<T> type
    /// Core semantics: value semantics by default, explicit reference/heap allocation, no implicit copying — Semantic types
    pub fn list(element: Type) -> Self {
        Type::Generic {
            name: Text::from(WKT::List.as_str()),
            args: List::from(vec![element]),
        }
    }

    /// Create a Maybe<T> type (Option equivalent)
    /// Core semantics: value semantics by default, explicit reference/heap allocation, no implicit copying — Semantic types
    pub fn maybe(inner: Type) -> Self {
        Type::Generic {
            name: Text::from(WKT::Maybe.as_str()),
            args: List::from(vec![inner]),
        }
    }

    /// Apply surface type arguments to a resolved base type.
    ///
    /// THE rule for `Base<A, B, …>`, in one place.  Four sites used to
    /// carry their own copy of it — `infer/types.rs` twice,
    /// `type_exporter.rs`, `method_resolution.rs` — and each spelled it
    /// as "take the base's path, throw its arguments away, put mine
    /// there".  That is how `IoResult<()>` became `Result<Unit>`: the
    /// base had already been resolved through the alias to
    /// `Result<T, StreamError>`, and replacing its arguments dropped
    /// `StreamError`, producing a one-argument `Result` — a type the
    /// language does not have.  Nothing rejected it; it surfaced three
    /// layers later as "cannot use `?` in a function returning
    /// Result<Unit>".
    ///
    /// THE INVARIANT: applying arguments never SHORTENS the argument
    /// list.  N surface arguments replace the first N of the base's and
    /// the rest are kept, so a single-parameter alias over a
    /// two-parameter type resolves correctly (`IoResult<Unit>` ->
    /// `Result<Unit, StreamError>`) and no site can lose an argument by
    /// re-deriving the rule slightly differently.
    ///
    /// Callers that know the alias's parameter names should still
    /// substitute through the unifier (`try_expand_alias`), which is
    /// exact; this is the rule for everyone else, and the floor under
    /// all of them.
    /// Substitute a parameter map into this type.
    ///
    /// THE substitution.  It used to live on `Unifier` as
    /// `apply_alias_subst`, which meant every other subsystem that needed
    /// it either reached through a unifier it did not otherwise want or
    /// grew its own copy — and a second copy of a substitution rule is
    /// how the malformed `Result<Unit>` class was born.  The body never
    /// read unifier state, so it belongs here, beside `apply_arguments`.
    pub fn substitute_params(&self, subst: &Map<Text, Type>) -> Type {
        // Compatibility callers retain the previous depth-limit behavior. Source
        // declaration application uses the fallible boundary below.
        self.try_substitute_params(subst).unwrap_or_else(|error| {
            tracing::warn!("{error}");
            self.clone()
        })
    }

    /// Substitute named declaration parameters without returning a partial type
    /// when traversal or capture-avoiding binder renaming fails.
    pub fn try_substitute_params(&self, subst: &Map<Text, Type>) -> Result<Type, Text> {
        // Root plus the historically admitted 100 nested edges. Resource
        // admission remains explicit; this limit no longer budgets Rust frames.
        const MAX_NAMED_SUBSTITUTION_DEPTH: usize = 101;
        type_substitution::apply_named(self, subst, MAX_NAMED_SUBSTITUTION_DEPTH)
    }

    fn named_substitution_under_binders(
        binders: &[TypeVar],
        body: &Type,
        substitution: &Map<Text, Type>,
        remaining_depth: usize,
    ) -> Result<(List<TypeVar>, Map<Text, Type>), Text> {
        let mut scoped = substitution.clone();
        for binder in binders {
            scoped.remove(&Text::from(format!("{binder}")));
        }
        let mut ranges = Set::new();
        for replacement in scoped.values() {
            replacement.collect_type_variables(&mut ranges, false, Some(remaining_depth))?;
        }
        let mut occupied = ranges.clone();
        body.collect_type_variables(&mut occupied, true, Some(remaining_depth))?;
        let mut renamed = List::new();
        for binder in binders {
            let variable = if ranges.contains(binder) {
                let mut fresh = TypeVar::fresh();
                while occupied.contains(&fresh) {
                    fresh = TypeVar::fresh();
                }
                occupied.insert(fresh);
                scoped.insert(format!("{binder}").into(), Type::Var(fresh));
                fresh
            } else {
                *binder
            };
            renamed.push(variable);
        }
        Ok((renamed, scoped))
    }

    pub fn bind_params(head: &Type, recv: &Type) -> Map<Text, Type> {
        fn args_of(t: &Type) -> Option<&List<Type>> {
            match t {
                Type::Named { args, .. } | Type::Generic { args, .. } => Some(args),
                _ => None,
            }
        }
        fn bare_name(t: &Type) -> Option<Text> {
            match t {
                Type::Named { path, args } if args.is_empty() => {
                    path.as_ident().map(|id| id.name.clone())
                }
                Type::Generic { name, args } if args.is_empty() => Some(name.clone()),
                // A generic impl's head parameters are stored as type
                // VARIABLES, not as named types — `implement<T> Deref for
                // G<T>` reaches us as `G<_>`.  `apply_alias_subst` keys its
                // map on a variable's display form, so key on the same
                // thing here; keying on anything else would silently miss
                // every generic impl, which is exactly what a first
                // attempt at this did (`MutexGuard` unchanged, target
                // returned as an unbound `_`).
                Type::Var(tv) => Some(Text::from(format!("{}", tv))),
                _ => None,
            }
        }
        let mut subst: Map<Text, Type> = Map::new();
        if let (Some(params), Some(actuals)) = (args_of(head), args_of(recv)) {
            for (p, a) in params.iter().zip(actuals.iter()) {
                if let Some(n) = bare_name(p) {
                    subst.insert(n, a.clone());
                }
            }
        }
        subst
    }

    /// Is this a wrapper the VBC layer peels natively?
    ///
    /// Deref steps through such a carrier must NOT be materialised as
    /// `.deref()` calls — the machine already peels them, and an
    /// explicit call mis-dispatches.  Delegates to the shared
    /// well-known-type vocabulary.
    pub fn is_transparent_carrier(&self) -> bool {
        let name = match self {
            Type::Generic { name, .. } => name.as_str(),
            Type::Named { path, .. } => match path.as_ident() {
                Some(id) => id.name.as_str(),
                None => return false,
            },
            _ => return false,
        };
        WKT::name_is_transparent_carrier(name)
    }

    pub fn apply_arguments(self, args: List<Type>) -> Self {
        fn merge(base_args: &List<Type>, args: List<Type>) -> List<Type> {
            if base_args.len() <= args.len() {
                return args;
            }
            let mut merged: Vec<Type> = args.iter().cloned().collect();
            merged.extend(base_args.iter().skip(merged.len()).cloned());
            List::from(merged)
        }
        match self {
            Type::Named { path, args: base_args } => Type::Named {
                path,
                args: merge(&base_args, args),
            },
            // An associated-type projection applied to arguments is a
            // type application, not a re-parameterisation.
            Type::Generic { name, args: base_args } if name.starts_with("::") => Type::TypeApp {
                constructor: Box::new(Type::Generic { name, args: base_args }),
                args,
            },
            Type::Generic { name, args: base_args } => Type::Generic {
                name,
                args: merge(&base_args, args),
            },
            // A type VARIABLE or constructor applied to arguments is an
            // application (HKT): `F<A>` where `F` is still unbound.
            other @ (Type::Var(_) | Type::TypeConstructor { .. }) => Type::TypeApp {
                constructor: Box::new(other),
                args,
            },
            other => other,
        }
    }

    /// Create a Result<T, E> type
    /// Core semantics: value semantics by default, explicit reference/heap allocation, no implicit copying — Semantic types
    pub fn result(ok: Type, err: Type) -> Self {
        Type::Generic {
            name: Text::from(WKT::Result.as_str()),
            args: List::from(vec![ok, err]),
        }
    }

    /// Create a Set<T> type
    /// Core semantics: value semantics by default, explicit reference/heap allocation, no implicit copying — Semantic types
    pub fn set(element: Type) -> Self {
        Type::Generic {
            name: Text::from(WKT::Set.as_str()),
            args: List::from(vec![element]),
        }
    }

    /// Create a Map<K, V> type
    /// Core semantics: value semantics by default, explicit reference/heap allocation, no implicit copying — Semantic types
    pub fn map(key: Type, value: Type) -> Self {
        Type::Generic {
            name: Text::from(WKT::Map.as_str()),
            args: List::from(vec![key, value]),
        }
    }

    /// Create a Stream<T> type (lazy collection)
    /// Expression grammar: precedence levels, associativity rules, all constructs are expressions — .11 - Stream Processing Syntax
    pub fn stream(element: Type) -> Self {
        Type::Generic {
            name: Text::from("Stream"),
            args: List::from(vec![element]),
        }
    }

    /// Create an Iterator<T> type
    /// Spec: core/base/iterator.vr - Iterator protocol
    pub fn iterator(item: Type) -> Self {
        Type::Generic {
            name: Text::from("Iterator"),
            args: List::from(vec![item]),
        }
    }

    /// Create an AsyncIterator<T> type
    /// Spec: core/async/async_iterator.vr - AsyncIterator protocol
    pub fn async_iterator(item: Type) -> Self {
        Type::Generic {
            name: Text::from("AsyncIterator"),
            args: List::from(vec![item]),
        }
    }

    /// Create a Heap<T> type (heap-allocated smart pointer)
    /// Spec: core/base/heap.vr - Heap allocation
    pub fn heap(inner: Type) -> Self {
        Type::Generic {
            name: Text::from(WKT::Heap.as_str()),
            args: List::from(vec![inner]),
        }
    }

    /// Create a Shared<T> type (shared reference-counted pointer)
    /// Spec: core/base/shared.vr - Shared ownership
    pub fn shared(inner: Type) -> Self {
        Type::Generic {
            name: Text::from(WKT::Shared.as_str()),
            args: List::from(vec![inner]),
        }
    }

    /// Structural check: does this type name match a given well-known type name?
    /// Works for both `Type::Named { path, args }` and `Type::Generic { name, args }`.
    /// Returns `Some(&args)` if this type has the given nominal name, else `None`.
    pub fn match_named_type<'a>(&'a self, expected_name: &str) -> Option<&'a List<Type>> {
        match self {
            Type::Named { path, args } => {
                if let Some(ident) = path.as_ident() {
                    if ident.as_str() == expected_name {
                        return Some(args);
                    }
                }
                None
            }
            Type::Generic { name, args } => {
                if name.as_str() == expected_name {
                    Some(args)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Structural check: is this a `Result<T, E>` type?
    /// Returns `Some((T, E))` if so. Checks both nominal Result and variant
    /// types with `Ok`/`Err` constructors (structural match).
    pub fn as_result(&self) -> Option<(Type, Type)> {
        if let Some(args) = self.match_named_type(WKT::Result.as_str()) {
            if args.len() == 2 {
                return Some((args[0].clone(), args[1].clone()));
            }
        }
        // Structural fallback: shape-driven recognition via the canonical
        // RESULT_VARIANT_LAYOUT (centralized in verum_common). The pair is
        // returned in canonical order (Ok, Err), independent of map iteration.
        if let Type::Variant(variants) = self {
            use verum_common::well_known_types::variant_tags::{
                extract_result_pair, is_result_shape,
            };
            if is_result_shape(variants.keys().map(|k| k.as_str())) {
                return extract_result_pair(|name| variants.get(name).cloned());
            }
        }
        None
    }

    /// Structural check: is this a `Maybe<T>` type?
    /// Returns `Some(T)` if so. Checks both nominal Maybe and variant types
    /// with `Some`/`None` (or Haskell-alias `Just`/`Nothing`) constructors.
    pub fn as_maybe(&self) -> Option<Type> {
        if let Some(args) = self.match_named_type(WKT::Maybe.as_str()) {
            if args.len() == 1 {
                return Some(args[0].clone());
            }
        }
        // Structural fallback: shape-driven recognition via the canonical
        // MAYBE_VARIANT_LAYOUT (centralized in verum_common). Recognises
        // both `Some`/`None` and the Haskell-style `Just`/`Nothing` aliases.
        if let Type::Variant(variants) = self {
            use verum_common::well_known_types::variant_tags::{
                extract_maybe_inner, is_maybe_shape,
            };
            if is_maybe_shape(variants.keys().map(|k| k.as_str())) {
                return extract_maybe_inner(|name| variants.get(name).cloned());
            }
        }
        None
    }

    /// Check if this type is nominally `Result<_, _>`.
    pub fn is_result(&self) -> bool {
        self.as_result().is_some()
    }

    /// Check if this type is nominally `Maybe<_>`.
    pub fn is_maybe(&self) -> bool {
        self.as_maybe().is_some()
    }

    /// Construct a function shape without claiming a body footprint.
    pub fn function(params: List<Type>, return_type: Type) -> Self {
        Self::function_with_requirement(params, return_type,
            crate::computational_properties::PropertyRequirement::Unrestricted)
    }

    /// Construct a callable whose body footprint is not known.
    pub fn function_with_requirement(params: List<Type>, return_type: Type, property_requirement: crate::computational_properties::PropertyRequirement) -> Self {
        Type::Function { params, return_type: Box::new(return_type), contexts: None,
            type_params: List::new(), properties: None, property_requirement }
    }

    /// Construct a source callable annotation without changing its eager output.
    pub fn function_with_async(params: List<Type>, return_type: Type, is_async: bool) -> Self {
        Self::function_with_requirement(params, return_type,
            crate::computational_properties::PropertyRequirement::source_callable(is_async))
    }

    /// Retain a declaration's async property alongside other inferred properties.
    pub fn with_async_property(mut self, is_async: bool) -> Self {
        if let Type::Function { properties, property_requirement, .. } = &mut self {
            *property_requirement = crate::computational_properties::PropertyRequirement::source_callable(is_async);
            if is_async && let Some(known) = properties {
                *known = known.union(&crate::computational_properties::PropertySet::single(
                    crate::computational_properties::ComputationalProperty::Async));
            }
        }
        self
    }

    /// Transfer only the checked computational contract; declaration binders,
    /// receiver substitution, contexts and return types stay with their scheme.
    pub(crate) fn copy_callable_contract(&mut self, checked: &Self) {
        if let (
            Self::Function { property_requirement, properties, .. },
            Self::Function { property_requirement: bound, properties: footprint, .. },
        ) = (self, checked) {
            *property_requirement = bound.clone();
            *properties = footprint.clone();
        }
    }

    /// Construct a function with DI contexts and an unknown body footprint.
    pub fn function_with_contexts(
        params: List<Type>,
        return_type: Type,
        contexts: crate::di::requirement::ContextRequirement,
    ) -> Self {
        Type::Function {
            property_requirement: crate::computational_properties::PropertyRequirement::Unrestricted,
            params,
            return_type: Box::new(return_type),
            type_params: List::new(),
            contexts: Some(crate::di::requirement::ContextExpr::Concrete(contexts)),
            properties: None,
        }
    }

    /// Create a function type with a context variable (for context polymorphism)
    pub fn function_with_context_var(
        params: List<Type>,
        return_type: Type,
        context_var: TypeVar,
    ) -> Self {
        Type::Function {
            property_requirement: crate::computational_properties::PropertyRequirement::Unrestricted,
            params,
            return_type: Box::new(return_type),
            type_params: List::new(),
            contexts: Some(crate::di::requirement::ContextExpr::Variable(context_var)),
            properties: None,
        }
    }

    /// Create a function type with computational properties
    pub fn function_with_properties(
        params: List<Type>,
        return_type: Type,
        properties: crate::computational_properties::PropertySet,
    ) -> Self {
        Type::Function {
            property_requirement: crate::computational_properties::PropertyRequirement::AtMost(properties.clone()),
            params,
            return_type: Box::new(return_type),
            type_params: List::new(),
            contexts: None,
            properties: Some(properties),
        }
    }

    /// Create a tuple type
    /// Note: An empty tuple is canonicalized to Unit type
    pub fn tuple(types: List<Type>) -> Self {
        if types.is_empty() {
            Type::Unit
        } else {
            Type::Tuple(types)
        }
    }

    /// Create an array type
    pub fn array(element: Type, size: Option<usize>) -> Self {
        Type::Array {
            element: Box::new(element),
            size,
        }
    }

    /// Create a slice type
    pub fn slice(element: Type) -> Self {
        Type::Slice {
            element: Box::new(element),
        }
    }

    /// Create a reference type (&T or &mut T)
    /// Memory model: three-tier references (&T managed, &checked T verified, &unsafe T raw) with CBGR runtime checking — CBGR reference with ~15ns overhead
    pub fn reference(mutable: bool, inner: Type) -> Self {
        Type::Reference {
            mutable,
            inner: Box::new(inner),
        }
    }

    /// Create a checked reference type (&checked T or &checked mut T)
    /// Memory model: three-tier references (&T managed, &checked T verified, &unsafe T raw) with CBGR runtime checking — Runtime bounds checking, no CBGR overhead
    pub fn checked_reference(mutable: bool, inner: Type) -> Self {
        Type::CheckedReference {
            mutable,
            inner: Box::new(inner),
        }
    }

    /// Create an unsafe reference type (&unsafe T or &unsafe mut T)
    /// Memory model: three-tier references (&T managed, &checked T verified, &unsafe T raw) with CBGR runtime checking — Zero-cost, no checks
    pub fn unsafe_reference(mutable: bool, inner: Type) -> Self {
        Type::UnsafeReference {
            mutable,
            inner: Box::new(inner),
        }
    }

    /// Pointee, mutability and safety tier of a Verum reference.
    pub(crate) fn reference_parts(&self) -> Option<(&Type, bool, verum_ast::expr::ReferenceKind)> {
        use verum_ast::expr::ReferenceKind;
        match self {
            Type::Reference { inner, mutable } => Some((inner, *mutable, ReferenceKind::Managed)),
            Type::CheckedReference { inner, mutable } => Some((inner, *mutable, ReferenceKind::Checked)),
            Type::UnsafeReference { inner, mutable } => Some((inner, *mutable, ReferenceKind::Unsafe)),
            _ => None,
        }
    }

    /// Create a refinement type
    pub fn refined(base: Type, predicate: RefinementPredicate) -> Self {
        Type::Refined {
            base: Box::new(base),
            predicate,
        }
    }

    /// Reading a value binder in a term observes its declared carrier and
    /// refinement. Type applications still use the binder's exact identity.
    pub(crate) fn into_expression_type(self) -> Self {
        match self {
            Type::Meta { ty, refinement: Some(predicate), .. } => Self::refined(*ty, predicate),
            Type::Meta { ty, .. } => *ty,
            other => other,
        }
    }

    pub(crate) fn sequence_domain(&self) -> Option<(&Type, Option<usize>)> {
        match self {
            Self::Array { element, size } => Some((element, *size)),
            Self::Slice { element } => Some((element, None)),
            _ => None,
        }
    }

    /// Create a meta parameter type (declaration-site or unresolved).
    /// Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Unified meta-system for compile-time computation
    pub fn meta(name: Text, ty: Type, refinement: Option<RefinementPredicate>) -> Self {
        Type::Meta {
            name,
            ty: Box::new(ty),
            refinement,
            value: ValueTerm::Parameter(ValueId::fresh()),
        }
    }

    /// Create a meta parameter type carrying a concrete compile-time value.
    /// Used at instantiation sites (e.g. the `7` in `Matrix<Float, 7, 7>`).
    /// The `name` is derived from the value's canonical form for diagnostics.
    pub fn meta_value(value: verum_common::ConstValue, base_ty: Type) -> Self {
        let name: Text = format!("{}", value).into();
        Type::Meta {
            name,
            ty: Box::new(base_ty),
            refinement: None,
            value: ValueTerm::Constant(value),
        }
    }

    /// Create a Future type
    /// Async/await integration: async functions return Future<T>, await extracts T, select for multi-future - Async/Await Integration
    pub fn future(output: Type) -> Self {
        Type::Future {
            output: Box::new(output),
        }
    }

    /// Create a Generator type
    /// Generator functions: fn* syntax yields values lazily, producing Iterator<Item=T> types
    pub fn generator(yield_ty: Type, return_ty: Type) -> Self {
        Type::Generator {
            yield_ty: Box::new(yield_ty),
            return_ty: Box::new(return_ty),
        }
    }

    /// Create a Lifetime type
    /// Subtyping: structural subtyping for records, refinement subtyping (T{P} <: T when P holds), protocol-based nominal subtyping — .2 - Lifetime parameters
    ///
    /// Represents a lifetime parameter like 'a, 'b, or 'static.
    /// While Verum uses CBGR for memory safety, lifetimes can be
    /// explicitly tracked for complex cases or FFI interop.
    ///
    /// # Examples
    /// ```
    /// use verum_types::Type;
    /// use verum_common::Text;
    ///
    /// // Named lifetime 'a
    /// let lifetime_a = Type::lifetime(Text::from("a"));
    ///
    /// // Static lifetime 'static
    /// let lifetime_static = Type::lifetime(Text::from("static"));
    /// ```
    pub fn lifetime(name: Text) -> Self {
        Type::Lifetime { name }
    }

    /// Create the 'static lifetime
    /// Convenience constructor for the 'static lifetime.
    pub fn lifetime_static() -> Self {
        Type::Lifetime {
            name: Text::from("static"),
        }
    }

    /// Create a GenRef type (generation-aware reference)
    /// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .2 lines 143-193
    ///
    /// GenRef provides explicit generation tracking for CBGR, enabling
    /// lending iterators and self-referential types without lifetimes.
    ///
    /// # Examples
    /// ```
    /// use verum_types::Type;
    ///
    /// // GenRef<Int>
    /// let genref_int = Type::genref(Type::int());
    ///
    /// // GenRef<&List<T>>
    /// let genref_ref = Type::genref(Type::reference(false, Type::text()));
    /// ```
    pub fn genref(inner: Type) -> Self {
        Type::GenRef {
            inner: Box::new(inner),
        }
    }

    /// Create a type constructor for higher-kinded types
    /// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .3 lines 410-437
    ///
    /// Type constructors are type-level functions that take type arguments.
    /// They enable higher-kinded polymorphism in protocols like Functor and Monad.
    ///
    /// # Examples
    /// ```
    /// use verum_types::Type;
    /// use verum_types::advanced_protocols::Kind;
    /// use verum_common::Text;
    ///
    /// // List has kind * -> * (unary type constructor)
    /// let list_ctor = Type::type_constructor(
    ///  Text::from("List"),
    ///  1,
    ///  Kind::unary_constructor()
    /// );
    ///
    /// // Map has kind * -> * -> * (binary type constructor)
    /// let map_ctor = Type::type_constructor(
    ///  Text::from("Map"),
    ///  2,
    ///  Kind::binary_constructor()
    /// );
    /// ```
    pub fn type_constructor(
        name: Text,
        arity: usize,
        kind: crate::advanced_protocols::Kind,
    ) -> Self {
        Type::TypeConstructor { name, arity, kind }
    }

    /// Create a type application (apply type constructor to arguments)
    /// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .3 lines 410-437
    ///
    /// Applies a type constructor to concrete type arguments, producing
    /// a fully-applied type. This preserves higher-kinded structure for
    /// protocol resolution and type inference.
    ///
    /// # Examples
    /// ```ignore
    /// use verum_types::Type;
    ///
    /// // Given a type constructor F<_>
    /// let f_ctor = Type::type_constructor(...);
    ///
    /// // Apply F to Int to get F<Int>
    /// let f_int = Type::type_app(f_ctor, vec![Type::int()]);
    ///
    /// // Apply F to Bool to get F<Bool>
    /// let f_bool = Type::type_app(f_ctor, vec![Type::bool()]);
    /// ```
    pub fn type_app(constructor: Type, args: List<Type>) -> Self {
        Type::TypeApp {
            constructor: Box::new(constructor),
            args,
        }
    }

    // ==================== Dependent Type Constructors ====================
    // Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking

    /// Create a Pi type (dependent function): (x: A) -> B(x)
    /// Pi types (dependent functions): (x: A) -> B(x) where return type depends on input value, non-dependent functions are special case
    ///
    /// # Examples
    /// ```
    /// use verum_types::Type;
    /// use verum_common::Text;
    ///
    /// // Simple identity function: (n: Nat) -> Nat
    /// let pi = Type::pi(Text::from("n"), Type::int(), Type::int());
    ///
    /// // Dependent function: (n: Nat) -> List<T, n>
    /// // (return type references n)
    /// ```
    pub fn pi(param_name: Text, param_type: Type, return_type: Type) -> Self {
        Type::Pi {
            param_name,
            param_type: Box::new(param_type),
            return_type: Box::new(return_type),
        }
    }

    /// Create a Sigma type (dependent pair): (x: A, B(x))
    /// Sigma types (dependent pairs): (x: A, B(x)) where second component type depends on first value, refinement types desugar to Sigma
    ///
    /// # Examples
    /// ```
    /// use verum_types::Type;
    /// use verum_common::Text;
    ///
    /// // Non-dependent pair: (Int, Bool)
    /// let sigma = Type::sigma(Text::from("_"), Type::int(), Type::bool());
    ///
    /// // Dependent pair: (n: Nat, List<T, n>)
    /// // (second type references n)
    /// ```
    pub fn sigma(fst_name: Text, fst_type: Type, snd_type: Type) -> Self {
        Type::Sigma {
            fst_name,
            fst_type: Box::new(fst_type),
            snd_type: Box::new(snd_type),
        }
    }

    /// Create an equality type: Eq<A, x, y> or x = y
    /// Equality types: propositional equality Eq<A, x, y> with reflexivity, symmetry, transitivity, substitution
    pub fn eq(ty: Type, lhs: EqTerm, rhs: EqTerm) -> Self {
        Type::Eq {
            ty: Box::new(ty),
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        }
    }

    /// Create a cubical path type: Path<A>(a, b)
    pub fn path_type(
        space: Type,
        left: crate::cubical::CubicalTerm,
        right: crate::cubical::CubicalTerm,
    ) -> Self {
        Type::PathType {
            space: Box::new(space),
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    /// Create the abstract interval type I
    pub fn interval() -> Self {
        Type::Interval
    }

    /// Create a partial element type: Partial<A>(φ)
    pub fn partial(element_type: Type, face: crate::cubical::CubicalTerm) -> Self {
        Type::Partial {
            element_type: Box::new(element_type),
            face: Box::new(face),
        }
    }

    /// Create a universe type at a specific level
    /// Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter
    pub fn universe(level: UniverseLevel) -> Self {
        Type::Universe { level }
    }

    /// Create the base universe Type (Type₀)
    pub fn type_universe() -> Self {
        Type::Universe {
            level: UniverseLevel::TYPE,
        }
    }

    /// Create the Prop universe (proof-irrelevant propositions)
    /// Inductive types: recursive type definitions with structural recursion, termination checking — .1
    pub fn prop() -> Self {
        Type::Prop
    }

    /// Create an inductive type
    /// Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .1
    pub fn inductive(
        name: Text,
        params: List<(Text, Type)>,
        indices: List<(Text, Type)>,
        universe: UniverseLevel,
        constructors: List<InductiveConstructor>,
    ) -> Self {
        Type::Inductive {
            name,
            params: params.into_iter().map(|(n, t)| (n, Box::new(t))).collect(),
            indices: indices.into_iter().map(|(n, t)| (n, Box::new(t))).collect(),
            universe,
            constructors,
        }
    }

    /// Create a coinductive type
    /// Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .2
    pub fn coinductive(
        name: Text,
        params: List<(Text, Type)>,
        destructors: List<CoinductiveDestructor>,
    ) -> Self {
        Type::Coinductive {
            name,
            params: params.into_iter().map(|(n, t)| (n, Box::new(t))).collect(),
            destructors,
        }
    }

    /// Create a Higher Inductive Type (HIT)
    /// Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .3
    pub fn higher_inductive(
        name: Text,
        params: List<(Text, Type)>,
        point_constructors: List<InductiveConstructor>,
        path_constructors: List<PathConstructor>,
    ) -> Self {
        Type::HigherInductive {
            name,
            params: params.into_iter().map(|(n, t)| (n, Box::new(t))).collect(),
            point_constructors,
            path_constructors,
        }
    }

    /// Create a quantified type with usage annotation
    /// Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .4
    pub fn quantified(inner: Type, quantity: Quantity) -> Self {
        Type::Quantified {
            inner: Box::new(inner),
            quantity,
        }
    }

    /// Create a linear type (used exactly once)
    pub fn linear(inner: Type) -> Self {
        Type::Quantified {
            inner: Box::new(inner),
            quantity: Quantity::LINEAR,
        }
    }

    /// Create an affine type (used at most once)
    pub fn affine_quantified(inner: Type) -> Self {
        Type::Quantified {
            inner: Box::new(inner),
            quantity: Quantity::AFFINE,
        }
    }

    /// Create an erased type (proof-irrelevant at runtime)
    pub fn erased(inner: Type) -> Self {
        Type::Quantified {
            inner: Box::new(inner),
            quantity: Quantity::ERASED,
        }
    }

    // ==================== Dependent Type Queries ====================

    /// Check if this type is a dependent type (Pi, Sigma, Eq, etc.)
    pub fn is_dependent(&self) -> bool {
        match self {
            Type::Pi { .. }
            | Type::Sigma { .. }
            | Type::Eq { .. }
            | Type::PathType { .. }
            | Type::Partial { .. } => true,
            Type::Inductive { indices, .. } => !indices.is_empty(),
            _ => false,
        }
    }

    /// Check if this type is a universe
    pub fn is_universe(&self) -> bool {
        matches!(self, Type::Universe { .. } | Type::Prop)
    }

    /// Check if this type is proof-irrelevant (Prop or erased)
    pub fn is_proof_irrelevant(&self) -> bool {
        matches!(
            self,
            Type::Prop
                | Type::Quantified {
                    quantity: Quantity::Zero,
                    ..
                }
        )
    }

    /// Get the universe level of a type (if applicable)
    pub fn universe_level(&self) -> Option<UniverseLevel> {
        match self {
            Type::Universe { level } => Some(level.clone()),
            Type::Prop => Some(UniverseLevel::Concrete(0)), // Prop is at level 0
            _ => None,
        }
    }

    /// Compute the type of a type (its universe).
    /// Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter — , Section 6.1
    ///
    /// This implements the universe rules:
    /// - Type₀ : Type₁
    /// - Type₁ : Type₂
    /// - Prop : Type₁ (proof-irrelevant propositions live in Type₁)
    /// - Most types : Type₀
    ///
    /// # Examples
    /// ```ignore
    /// Type::Bool.type_of() // => Type::Universe { level: 0 } (Type₀)
    /// Type::Prop.type_of() // => Type::Universe { level: 1 } (Type₁)
    /// Type::Universe { level: 0 }.type_of() // => Type::Universe { level: 1 } (Type₁)
    /// ```
    pub fn type_of(&self) -> Type {
        match self {
            // Prop : Type₁ (proof irrelevance)
            // Inductive types: recursive type definitions with structural recursion, termination checking — .1
            Type::Prop => Type::Universe {
                level: UniverseLevel::TYPE1,
            },

            // Type_n : Type_{n+1} (universe hierarchy)
            // Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter
            Type::Universe { level } => Type::Universe {
                level: level.succ(),
            },

            Type::RecordField { level, .. } => Type::Universe { level: level.clone() },

            // Pi types: (x: A) -> B
            // The universe is the maximum of A's universe and B's universe
            // Pi types (dependent functions): (x: A) -> B(x) where return type depends on input value, non-dependent functions are special case
            Type::Pi {
                param_type,
                return_type,
                ..
            } => {
                let param_univ = param_type.type_of();
                let return_univ = return_type.type_of();
                match (param_univ, return_univ) {
                    (Type::Universe { level: l1 }, Type::Universe { level: l2 }) => {
                        Type::Universe { level: l1.max(&l2) }
                    }
                    _ => Type::Universe {
                        level: UniverseLevel::TYPE,
                    },
                }
            }

            // Sigma types: (x: A, B(x))
            // The universe is the maximum of A's universe and B's universe
            // Sigma types (dependent pairs): (x: A, B(x)) where second component type depends on first value, refinement types desugar to Sigma
            Type::Sigma {
                fst_type, snd_type, ..
            } => {
                let fst_univ = fst_type.type_of();
                let snd_univ = snd_type.type_of();
                match (fst_univ, snd_univ) {
                    (Type::Universe { level: l1 }, Type::Universe { level: l2 }) => {
                        Type::Universe { level: l1.max(&l2) }
                    }
                    _ => Type::Universe {
                        level: UniverseLevel::TYPE,
                    },
                }
            }

            // Equality types: Eq<A, x, y>
            // Lives in the same universe as A
            // Equality types: propositional equality Eq<A, x, y> with reflexivity, symmetry, transitivity, substitution
            Type::Eq { ty, .. } => ty.type_of(),

            // Path types: Path<A>(a, b)
            // Lives in the same universe as A (the space)
            Type::PathType { space, .. } => space.type_of(),

            // Interval: I is not in any universe — it is a special cofibrant object
            // We place it in Type₀ for pragmatic reasons (it acts like a type in
            // the surface language even though categorically it is outside the hierarchy)
            Type::Interval => Type::Universe {
                level: UniverseLevel::TYPE,
            },

            // Partial types: Partial<A>(φ) lives in the same universe as A
            Type::Partial { element_type, .. } => element_type.type_of(),

            // Inductive types live in their declared universe
            // Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .1
            Type::Inductive { universe, .. } => Type::Universe { level: universe.clone() },

            // Most other types live in Type₀
            _ => Type::Universe {
                level: UniverseLevel::TYPE,
            },
        }
    }

    /// Create a Tensor type with compile-time shape parameters
    /// Tensor types: Tensor<T, Shape: meta [usize]> with compile-time shape tracking for N-dimensional arrays
    ///
    /// Computes row-major strides automatically for efficient indexing.
    ///
    /// # Examples
    /// ```ignore
    /// // 1D vector: Tensor<f32, [4]>
    /// let vec_ty = Type::tensor(Type::float(), vec![ConstValue::UInt(4)], span);
    ///
    /// // 2D matrix: Tensor<f32, [2, 3]>
    /// let mat_ty = Type::tensor(
    ///  Type::float(),
    ///  vec![ConstValue::UInt(2), ConstValue::UInt(3)],
    ///  span
    /// );
    /// ```
    pub fn tensor(element: Type, shape: List<verum_common::ConstValue>, span: Span) -> Self {
        // Compute strides for row-major layout
        let strides = compute_strides(&shape);

        Type::Tensor {
            element: Box::new(element),
            shape,
            strides,
            span,
        }
    }

    // ==================== Integer Type Helpers ====================
    // Integer type hierarchy: all fixed-size integers (i8..i128, u8..u128) are refinement types of Int with range predicates — .2 lines 143-162
    // These create refined types with exact value constraints for literals

    // Literal-synthesis helpers emit the canonical UpperCamel type name
    // (`UInt64`, `Int32`, `Float64`, ...) so downstream comparison,
    // unification, and codegen never have to reconcile lowercase
    // Rust-style aliases (`u64`, `i32`, `f64`) with their canonical
    // forms. The grammar still accepts both spellings at parse time —
    // they converge here before any type-level reasoning runs.
    fn primitive_named(canonical: &str) -> Self {
        let ident = verum_ast::ty::Ident::new(canonical, Span::dummy());
        Type::Named {
            path: Path::single(ident),
            args: List::new(),
        }
    }

    /// Collapse Rust-style lowercase numeric aliases (`u64`, `i32`,
    /// `f64`, `u8`) onto their canonical UpperCamel spellings
    /// (`UInt64`, `Int32`, `Float64`, `Byte`). Non-primitive names
    /// pass through unchanged so nominal types keep their exact
    /// identity.
    ///
    /// Used by every type-compatibility / dispatch path that
    /// receives types spelled either way (the grammar accepts both,
    /// but only canonical forms make it out of `*_refined` literal
    /// synthesis — see `primitive_named`). Central here so all three
    /// consumers (`infer::Checker::types_compatible`,
    /// `protocol::types_compatible`,
    /// `specialization::types_compatible`) stay coherent with any
    /// future extension of the numeric set.
    pub fn canonical_primitive(name: &str) -> &str {
        // Canonical-name set lives in
        // `verum_common::well_known_types::NUMERIC_ALIAS_MATRIX`; this
        // function normalises every legacy uppercase-short and Rust-
        // style lowercase alias to the canonical Verum form. Drift
        // between the registry and this normaliser surfaces as type-
        // dispatch misses (e.g. `Usize` not matching `USize` impl
        // because the normaliser left it as-is).
        match name {
            "i8" | "I8" => "Int8",
            "i16" | "I16" => "Int16",
            "i32" | "I32" => "Int32",
            "i64" | "I64" => "Int64",
            "i128" | "I128" => "Int128",
            "isize" | "Isize" | "ISize" => "IntSize",
            "u8" | "U8" | "UInt8" => "Byte",
            "u16" | "U16" => "UInt16",
            "u32" | "U32" => "UInt32",
            "u64" | "U64" => "UInt64",
            "u128" | "U128" => "UInt128",
            "usize" | "Usize" | "USize" => "UIntSize",
            "f32" | "F32" => "Float32",
            "f64" | "F64" => "Float64",
            other => other,
        }
    }

    pub fn i8_refined(_value: i8) -> Self {
        Self::primitive_named("Int8")
    }
    pub fn i16_refined(_value: i16) -> Self {
        Self::primitive_named("Int16")
    }
    pub fn i32_refined(_value: i32) -> Self {
        Self::primitive_named("Int32")
    }
    pub fn i64_refined(_value: i64) -> Self {
        Self::primitive_named("Int64")
    }
    pub fn i128_refined(_value: i128) -> Self {
        Self::primitive_named("Int128")
    }
    pub fn isize_refined(_value: isize) -> Self {
        Self::primitive_named("IntSize")
    }

    /// `Byte` is the canonical 8-bit unsigned type in Verum; `UInt8`
    /// is a synonym but `Byte` is preferred in semantic-typing
    /// contexts, so the literal synthesizer keeps emitting it.
    pub fn u8_refined(_value: u8) -> Self {
        Self::primitive_named("Byte")
    }
    pub fn u16_refined(_value: u16) -> Self {
        Self::primitive_named("UInt16")
    }
    pub fn u32_refined(_value: u32) -> Self {
        Self::primitive_named("UInt32")
    }
    pub fn u64_refined(_value: u64) -> Self {
        Self::primitive_named("UInt64")
    }
    pub fn u128_refined(_value: u128) -> Self {
        Self::primitive_named("UInt128")
    }
    pub fn usize_refined(_value: usize) -> Self {
        Self::primitive_named("UIntSize")
    }

    pub fn f32_refined(_value: f32) -> Self {
        Self::primitive_named("Float32")
    }
    pub fn f64_refined(_value: f64) -> Self {
        Self::primitive_named("Float64")
    }

    /// Get the free type variables in this type
    pub fn free_vars(&self) -> Set<TypeVar> {
        let mut vars = Set::new();
        self.collect_type_variables(&mut vars, false, None)
            .expect("unbounded variable collection has no depth limit");
        vars
    }

    /// Shared exhaustive traversal for free-variable queries and fresh binder allocation.
    fn collect_type_variables(
        &self,
        vars: &mut Set<TypeVar>,
        include_bound: bool,
        remaining_depth: Option<usize>,
    ) -> Result<(), Text> {
        let remaining_depth = remaining_depth
            .map(|remaining| {
                remaining.checked_sub(1).ok_or_else(|| {
                    Text::from("type substitution exceeds its structural depth budget")
                })
            })
            .transpose()?;
        match self {
            Type::Var(v) => {
                vars.insert(*v);
            }
            Type::Function {
                params,
                return_type,
                ..
            } => {
                for param in params {
                    param.collect_type_variables(vars, include_bound, remaining_depth)?;
                }
                return_type.collect_type_variables(vars, include_bound, remaining_depth)?;
            }
            Type::Tuple(types) => {
                for ty in types {
                    ty.collect_type_variables(vars, include_bound, remaining_depth)?;
                }
            }
            Type::Array { element, .. } => {
                element.collect_type_variables(vars, include_bound, remaining_depth)?;
            }
            Type::Record(fields) => {
                for ty in fields.values() {
                    ty.collect_type_variables(vars, include_bound, remaining_depth)?;
                }
            }
            Type::Variant(variants) => {
                for ty in variants.values() {
                    ty.collect_type_variables(vars, include_bound, remaining_depth)?;
                }
            }
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. }
            | Type::Ownership { inner, .. }
            | Type::Pointer { inner, .. }
            | Type::VolatilePointer { inner, .. } => {
                inner.collect_type_variables(vars, include_bound, remaining_depth)?;
            }
            Type::Refined { base, .. } => {
                // Note: We don't track free vars in predicates
                // as they're value-level, not type-level
                base.collect_type_variables(vars, include_bound, remaining_depth)?;
            }
            Type::Exists { var, body } => {
                let mut body_vars = Set::new();
                body.collect_type_variables(&mut body_vars, include_bound, remaining_depth)?;
                if include_bound {
                    body_vars.insert(*var);
                } else {
                    body_vars.remove(var);
                }
                for v in body_vars {
                    vars.insert(v);
                }
            }
            Type::Forall {
                vars: bound_vars,
                body,
            } => {
                let mut body_vars = Set::new();
                body.collect_type_variables(&mut body_vars, include_bound, remaining_depth)?;
                for v in bound_vars {
                    if include_bound {
                        body_vars.insert(*v);
                    } else {
                        body_vars.remove(v);
                    }
                }
                for v in body_vars {
                    vars.insert(v);
                }
            }
            Type::Named { args, .. } => {
                for arg in args {
                    arg.collect_type_variables(vars, include_bound, remaining_depth)?;
                }
            }
            Type::Generic { args, .. } => {
                for arg in args {
                    arg.collect_type_variables(vars, include_bound, remaining_depth)?;
                }
            }
            Type::Meta { ty, .. } => {
                // Meta parameters can contain type variables in their base type
                ty.collect_type_variables(vars, include_bound, remaining_depth)?;
            }
            Type::Future { output } => {
                output.collect_type_variables(vars, include_bound, remaining_depth)?;
            }
            Type::Generator {
                yield_ty,
                return_ty,
            } => {
                yield_ty.collect_type_variables(vars, include_bound, remaining_depth)?;
                return_ty.collect_type_variables(vars, include_bound, remaining_depth)?;
            }
            Type::Tensor { element, .. } => {
                // Collect free vars from element type
                // Shape dimensions are compile-time constants (no type vars)
                element.collect_type_variables(vars, include_bound, remaining_depth)?;
            }
            Type::GenRef { inner } => {
                // GenRef wraps a type, so collect from inner type
                inner.collect_type_variables(vars, include_bound, remaining_depth)?;
            }
            Type::TypeConstructor { .. } => {
                // Type constructors don't contain type variables themselves
                // They're just names with arity/kind metadata
            }
            Type::TypeApp { constructor, args } => {
                // Collect from both constructor and arguments
                constructor.collect_type_variables(vars, include_bound, remaining_depth)?;
                for arg in args {
                    arg.collect_type_variables(vars, include_bound, remaining_depth)?;
                }
            }
            Type::PathType { space, .. } => {
                // Collect type vars from the space type
                // CubicalTerm endpoints are value-level, no type vars
                space.collect_type_variables(vars, include_bound, remaining_depth)?;
            }
            Type::Partial { element_type, .. } => {
                // Collect type vars from the element type
                element_type.collect_type_variables(vars, include_bound, remaining_depth)?;
            }
            Type::Interval => {
                // Interval is a primitive type, no free vars
            }

            // ── Types that CARRY types and were previously answered by the
            // catch-all below ────────────────────────────────────────────────
            //
            // A missing arm here does not read as "unknown", it reads as "this
            // type has no type variables" — and two things consume that answer:
            // generalization (a var it cannot see is never quantified, so the
            // scheme is shared UNFRESHENED across all call sites) and the
            // occurs check in `bind_var` (a cycle running through such a type
            // is invisible).
            //
            // Measured for `Slice` (T0742): `slice_len<T>(slice: &[T])` and its
            // five siblings in `core/intrinsics/memory.vr` had no quantified
            // vars, so every call site unified against ONE shared var. Calling
            // any of them from a method of `implement<T> List<T>` pinned that
            // block's `T` for every later method in the block.
            Type::Slice { element } => {
                element.collect_type_variables(vars, include_bound, remaining_depth)?;
            }
            Type::ExtensibleRecord { fields, row_var } => {
                for ty in fields.values() {
                    ty.collect_type_variables(vars, include_bound, remaining_depth)?;
                }
                if let Some(row) = row_var {
                    vars.insert(*row);
                }
            }
            Type::DynProtocol { bindings, .. } => {
                // `bounds` are protocol NAMES; the associated-type bindings
                // are real types and can mention variables.
                for ty in bindings.values() {
                    ty.collect_type_variables(vars, include_bound, remaining_depth)?;
                }
            }
            Type::Pi {
                param_type,
                return_type,
                ..
            } => {
                param_type.collect_type_variables(vars, include_bound, remaining_depth)?;
                return_type.collect_type_variables(vars, include_bound, remaining_depth)?;
            }
            Type::Sigma {
                fst_type, snd_type, ..
            } => {
                fst_type.collect_type_variables(vars, include_bound, remaining_depth)?;
                snd_type.collect_type_variables(vars, include_bound, remaining_depth)?;
            }
            Type::Eq { ty, .. } => {
                // The two sides are VALUE-level terms (`EqTerm`), same reason
                // `Refined` skips its predicate; the carried type is not.
                ty.collect_type_variables(vars, include_bound, remaining_depth)?;
            }
            Type::Inductive {
                params, indices, ..
            } => {
                for (_, ty) in params.iter().chain(indices.iter()) {
                    ty.collect_type_variables(vars, include_bound, remaining_depth)?;
                }
            }
            Type::Coinductive { params, .. } => {
                for (_, ty) in params.iter() {
                    ty.collect_type_variables(vars, include_bound, remaining_depth)?;
                }
            }
            Type::HigherInductive { params, .. } => {
                for (_, ty) in params.iter() {
                    ty.collect_type_variables(vars, include_bound, remaining_depth)?;
                }
            }
            Type::Quantified { inner, .. } => {
                inner.collect_type_variables(vars, include_bound, remaining_depth)?;
            }
            Type::CapabilityRestricted { base, .. } => {
                base.collect_type_variables(vars, include_bound, remaining_depth)?;
            }

            // ── Leaves: no component that could hold a type variable ─────────
            // Spelled out rather than left to a catch-all, so that adding a
            // variant to `Type` fails to compile here instead of silently
            // answering "no free variables" for it.
            Type::Unit
            | Type::Never
            | Type::Unknown
            | Type::Bool
            | Type::Int
            | Type::Float
            | Type::Char
            | Type::Text
            | Type::Lifetime { .. }
            | Type::Universe { .. }
            | Type::RecordField { .. }
            | Type::Prop
            | Type::Placeholder { .. } => {}
        }
        Ok(())
    }

    /// Apply an already validated substitution at an internal invariant boundary.
    /// Source checking must use `try_apply_subst` to return normal diagnostics.
    pub fn apply_subst(&self, subst: &Substitution) -> Type {
        self.try_apply_subst(subst)
            .expect("internal type substitution must have been validated")
    }

    /// Apply a substitution without losing failed universe constraints or a
    /// partially visited type when the structural budget is exceeded.
    pub fn try_apply_subst(&self, subst: &Substitution) -> Result<Type, Text> {
        const MAX_SUBST_DEPTH: usize = 30;
        type_substitution::apply(self, subst, MAX_SUBST_DEPTH)
    }

    /// Restrict a simultaneous substitution to a binder's scope. Only a free
    /// variable actually replaced in this body can cause capture. Renaming is
    /// added to the same substitution, so replacement ranges are not rewritten.
    fn substitution_under_binders(
        &self,
        binders: &[TypeVar],
        substitution: &Substitution,
        remaining_depth: usize,
    ) -> Result<(List<TypeVar>, Substitution), Text> {
        let mut scoped = substitution.clone();
        for binder in binders {
            scoped.shift_remove(binder);
        }
        if binders.is_empty() || (scoped.types.is_empty() && scoped.value_bindings.is_empty()) {
            return Ok((binders.iter().copied().collect(), scoped));
        }
        let mut free_body = Set::new();
        self.collect_type_variables(&mut free_body, false, Some(remaining_depth))?;
        let mut free_ranges = Set::new();
        for (variable, replacement) in &scoped {
            if free_body.contains(variable) {
                replacement.collect_type_variables(
                    &mut free_ranges,
                    false,
                    Some(remaining_depth),
                )?;
            }
        }
        let value_ids = self.value_ids();
        for (id, replacement) in &scoped.value_bindings {
            if value_ids.contains(id) {
                replacement.collect_type_variables(&mut free_ranges, false, Some(remaining_depth))?;
            }
        }
        if !binders.iter().any(|binder| free_ranges.contains(binder)) {
            return Ok((binders.iter().copied().collect(), scoped));
        }

        // TypeVar::with_id/new can introduce IDs ahead of the fresh counter.
        // Avoid nested bound IDs too; a fresh outer binder must not shadow them.
        let mut occupied = Set::new();
        self.collect_type_variables(&mut occupied, true, Some(remaining_depth))?;
        occupied.extend(binders.iter().copied());
        occupied.extend(substitution.keys().copied());
        for replacement in substitution.values().chain(substitution.value_bindings.values()) {
            replacement.collect_type_variables(&mut occupied, true, Some(remaining_depth))?;
        }
        let mut renamed = List::new();
        for binder in binders {
            let replacement = if free_ranges.contains(binder) {
                let fresh = loop {
                    let candidate = TypeVar::fresh();
                    if occupied.insert(candidate) {
                        break candidate;
                    }
                };
                scoped.insert(*binder, Type::Var(fresh));
                fresh
            } else {
                *binder
            };
            renamed.push(replacement);
        }
        Ok((renamed, scoped))
    }

    /// Check if this type is a monotype (no type variables)
    pub fn is_monotype(&self) -> bool {
        self.free_vars().is_empty()
    }

    /// Get the base type (unwrap refinement if present)
    pub fn base(&self) -> &Type {
        match self {
            Type::Refined { base, .. } => base.base(),
            ty => ty,
        }
    }

    /// Check if type is a function
    pub fn is_function(&self) -> bool {
        matches!(self.base(), Type::Function { .. })
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Unit => write!(f, "Unit"),
            Type::Never => write!(f, "!"),
            Type::Bool => write!(f, "Bool"),
            Type::Int => write!(f, "Int"),
            Type::Float => write!(f, "Float"),
            Type::Char => write!(f, "Char"),
            Type::Text => write!(f, "Text"),
            Type::Var(v) => write!(f, "{}", v),
            Type::RecordField { slot, receiver, .. } => {
                if let Some(receiver) = receiver {
                    write!(f, "{receiver}.")?;
                }
                write!(f, "{}[{}]", slot.owner, slot.ordinal)
            }
            Type::Named { path, args } => {
                write!(f, "{}", format_path(path))?;
                if !args.is_empty() {
                    write!(f, "<")?;
                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", arg)?;
                    }
                    write!(f, ">")?;
                }
                Ok(())
            }
            Type::Generic { name, args } => {
                // Strip internal :: prefix (used for HKT parameters in protocols)
                let display_name = if let Some(stripped) = name.as_str().strip_prefix("::") {
                    stripped
                } else {
                    name.as_str()
                };
                write!(f, "{}", display_name)?;
                if !args.is_empty() {
                    write!(f, "<")?;
                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", arg)?;
                    }
                    write!(f, ">")?;
                }
                Ok(())
            }
            Type::Function {
                property_requirement,
                params,
                return_type,
                type_params: _,
                contexts,
                properties,
            } => {
                if property_requirement.may_be_async() {
                    write!(f, "async ")?;
                }
                write!(f, "fn(")?;
                for (i, param) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", param)?;
                }
                write!(f, ") -> {}", return_type)?;
                if let Some(req) = contexts
                    && !req.is_empty()
                {
                    write!(f, " using [")?;
                    for (i, ctx_ref) in req.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", ctx_ref.name)?;
                    }
                    write!(f, "]")?;
                }
                Ok(())
            }
            Type::Tuple(types) => {
                write!(f, "(")?;
                for (i, ty) in types.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", ty)?;
                }
                write!(f, ")")
            }
            Type::Array { element, size } => {
                if let Some(n) = size {
                    write!(f, "[{}; {}]", element, n)
                } else {
                    write!(f, "[{}]", element)
                }
            }
            // Slice represents an unsized array type [T], NOT a reference &[T]
            // References to slices are represented as Reference { inner: Slice }
            Type::Slice { element } => write!(f, "[{}]", element),
            Type::Record(fields) => {
                write!(f, "{{ ")?;
                for (i, (name, ty)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", name, ty)?;
                }
                write!(f, " }}")
            }
            Type::Variant(variants) => {
                for (i, (name, ty)) in variants.iter().enumerate() {
                    if i > 0 {
                        write!(f, " | ")?;
                    }
                    write!(f, "{}({})", name, ty)?;
                }
                Ok(())
            }
            Type::Reference { mutable, inner } => {
                if *mutable {
                    write!(f, "&mut {}", inner)
                } else {
                    write!(f, "&{}", inner)
                }
            }
            Type::CheckedReference { mutable, inner } => {
                if *mutable {
                    write!(f, "&checked mut {}", inner)
                } else {
                    write!(f, "&checked {}", inner)
                }
            }
            Type::UnsafeReference { mutable, inner } => {
                if *mutable {
                    write!(f, "&unsafe mut {}", inner)
                } else {
                    write!(f, "&unsafe {}", inner)
                }
            }
            Type::Ownership { mutable, inner } => {
                // Display owned types with user-friendly syntax
                if *mutable {
                    write!(f, "owned mut {}", inner)
                } else {
                    write!(f, "owned {}", inner)
                }
            }
            Type::Pointer { mutable, inner } => {
                if *mutable {
                    write!(f, "*mut {}", inner)
                } else {
                    write!(f, "*const {}", inner)
                }
            }
            Type::VolatilePointer { mutable, inner } => {
                if *mutable {
                    write!(f, "*volatile mut {}", inner)
                } else {
                    write!(f, "*volatile {}", inner)
                }
            }
            Type::Refined { base, predicate } => {
                write!(f, "{}{{{}}}", base, predicate)
            }
            Type::Exists { var, body } => {
                write!(f, "∃{}. {}", var, body)
            }
            Type::Forall { vars, body } => {
                write!(f, "∀")?;
                for (i, v) in vars.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, ". {}", body)
            }
            Type::Meta {
                name,
                ty,
                refinement,
                value,
            } => {
                // If we carry a concrete value, show just the value (e.g. "7")
                // — matches what the user wrote at the call site.
                if let ValueTerm::Constant(v) = value {
                    return write!(f, "{}", v);
                }
                write!(f, "{}: meta {}", name, ty)?;
                if let Some(pred) = refinement {
                    write!(f, "{{{}}}", pred)?;
                }
                Ok(())
            }
            Type::Future { output } => {
                write!(f, "Future<{}>", output)
            }
            Type::Generator {
                yield_ty,
                return_ty,
            } => {
                write!(f, "Generator<{}, {}>", yield_ty, return_ty)
            }
            Type::Tensor { element, shape, .. } => {
                write!(f, "Tensor<{}, [", element)?;
                for (i, dim) in shape.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", dim)?;
                }
                write!(f, "]>")
            }
            Type::Lifetime { name } => {
                write!(f, "'{}", name)
            }
            Type::GenRef { inner } => {
                write!(f, "GenRef<{}>", inner)
            }
            Type::TypeConstructor { name, arity, .. } => {
                // Display type constructor with placeholders for arity
                write!(f, "{}", name)?;
                if *arity > 0 {
                    write!(f, "<")?;
                    for i in 0..*arity {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "_")?;
                    }
                    write!(f, ">")?;
                }
                Ok(())
            }
            Type::TypeApp { constructor, args } => {
                // Display type application: F<T1, T2, ...>
                // Extract just the constructor name, not its full display (which includes <_> placeholders)
                match constructor.as_ref() {
                    Type::TypeConstructor { name, .. } => write!(f, "{}", name)?,
                    _ => write!(f, "{}", constructor)?,
                }
                if !args.is_empty() {
                    write!(f, "<")?;
                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", arg)?;
                    }
                    write!(f, ">")?;
                }
                Ok(())
            }

            // Dependent Type Display (Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — )
            Type::Pi {
                param_name,
                param_type,
                return_type,
            } => {
                write!(f, "({}: {}) → {}", param_name, param_type, return_type)
            }

            Type::Sigma {
                fst_name,
                fst_type,
                snd_type,
            } => {
                write!(f, "({}: {}, {})", fst_name, fst_type, snd_type)
            }

            Type::Eq { ty, lhs, rhs } => {
                write!(f, "Eq<{}, lhs, rhs>", ty)
            }

            Type::PathType { space, left, right } => {
                write!(f, "Path<{}>({:?}, {:?})", space, left, right)
            }

            Type::Interval => write!(f, "I"),

            Type::Partial { element_type, face } => {
                write!(f, "Partial<{}>({:?})", element_type, face)
            }

            Type::Universe { level } => {
                write!(f, "{}", level)
            }

            Type::Prop => write!(f, "Prop"),

            Type::Inductive {
                name,
                params,
                indices,
                ..
            } => {
                write!(f, "inductive {}", name)?;
                if !params.is_empty() || !indices.is_empty() {
                    write!(f, "<")?;
                    let mut first = true;
                    for (pname, pty) in params.iter() {
                        if !first {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}: {}", pname, pty)?;
                        first = false;
                    }
                    for (iname, ity) in indices.iter() {
                        if !first {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}: {}", iname, ity)?;
                        first = false;
                    }
                    write!(f, ">")?;
                }
                Ok(())
            }

            Type::Coinductive { name, params, .. } => {
                write!(f, "coinductive {}", name)?;
                if !params.is_empty() {
                    write!(f, "<")?;
                    for (i, (pname, pty)) in params.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}: {}", pname, pty)?;
                    }
                    write!(f, ">")?;
                }
                Ok(())
            }

            Type::HigherInductive { name, params, .. } => {
                write!(f, "hott inductive {}", name)?;
                if !params.is_empty() {
                    write!(f, "<")?;
                    for (i, (pname, pty)) in params.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}: {}", pname, pty)?;
                    }
                    write!(f, ">")?;
                }
                Ok(())
            }

            Type::Quantified { inner, quantity } => {
                write!(f, "{} @{}", inner, quantity)
            }

            Type::Placeholder { name, .. } => {
                write!(f, "<placeholder:{}>", name)
            }

            Type::ExtensibleRecord { fields, row_var } => {
                write!(f, "{{")?;
                for (i, (name, ty)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", name, ty)?;
                }
                if let Some(rv) = row_var {
                    write!(f, " | {}", rv)?;
                }
                write!(f, "}}")
            }

            Type::CapabilityRestricted { base, capabilities } => {
                write!(f, "{} with [", base)?;
                let names = capabilities.names();
                for (i, name) in names.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", name)?;
                }
                write!(f, "]")
            }

            // Unknown type - a safe top type (like `any` in TypeScript but safe)
            Type::Unknown => write!(f, "Unknown"),

            // DynProtocol - dynamic protocol object (dyn Display + Debug)
            Type::DynProtocol { bounds, bindings } => {
                write!(f, "dyn ")?;
                for (i, bound) in bounds.iter().enumerate() {
                    if i > 0 {
                        write!(f, " + ")?;
                    }
                    write!(f, "{}", bound)?;
                }
                if !bindings.is_empty() {
                    write!(f, "<")?;
                    for (i, (name, ty)) in bindings.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{} = {}", name, ty)?;
                    }
                    write!(f, ">")?;
                }
                Ok(())
            }
        }
    }
}

/// A resolved declaration path can be stored as source segments or as the
/// dotted key used in archive signatures. Relative syntax has no such key.
pub(crate) fn nominal_path_name(path: &Path) -> Option<std::borrow::Cow<'_, str>> {
    if let Some(ident) = path.as_ident() {
        return Some(ident.name.as_str().into());
    }
    let mut name = String::new();
    for segment in &path.segments {
        let verum_ast::ty::PathSegment::Name(ident) = segment else { return None };
        if !name.is_empty() {
            name.push('.');
        }
        name.push_str(ident.name.as_str());
    }
    (!name.is_empty()).then(|| name.into())
}

pub(crate) fn same_type_path(left: &Path, right: &Path) -> bool {
    use verum_ast::ty::PathSegment;
    if left.segments.len() == right.segments.len()
        && left.segments.iter().zip(&right.segments).all(|(a, b)| match (a, b) {
            (PathSegment::Name(a), PathSegment::Name(b)) => a.name == b.name,
            _ => a == b,
        })
    {
        return true;
    }
    nominal_path_name(left).is_some_and(|name| nominal_path_name(right).as_ref() == Some(&name))
}

fn format_path(path: &Path) -> Text {
    use verum_ast::ty::PathSegment;
    let parts: List<&str> = path
        .segments
        .iter()
        .map(|seg| match seg {
            PathSegment::Name(id) => id.name.as_str(),
            PathSegment::SelfValue => "self",
            PathSegment::Super => "super",
            PathSegment::Cog => "cog",
            PathSegment::Relative => ".",
        })
        .collect();
    parts.join(".")
}

/// Compute row-major strides from tensor shape dimensions
/// Tensor types: Tensor<T, Shape: meta [usize]> with compile-time shape tracking for N-dimensional arrays
///
/// Strides are computed as: strides[i] = product(shape[i+1..])
/// This enables efficient multi-dimensional indexing in row-major layout.
///
/// # Examples
/// ```ignore
/// // Shape [2, 3, 4] -> Strides [12, 4, 1]
/// // Shape [4] -> Strides [1]
/// ```
fn compute_strides(shape: &[verum_common::ConstValue]) -> List<usize> {
    let mut strides = List::with_capacity(shape.len());
    let mut stride = 1;

    // Compute strides in reverse order (row-major)
    for i in (0..shape.len()).rev() {
        strides.push(stride);

        // Extract dimension size and update stride
        if let Some(dim) = shape[i].as_u128() {
            stride *= dim as usize;
        } else if let Some(dim) = shape[i].as_i128() {
            stride *= dim as usize;
        }
        // If dimension can't be extracted, stride remains unchanged
        // (error will be caught during type checking)
    }

    // Reverse to get row-major order
    strides.reverse();
    strides
}

/// A type variable for type inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeVar(usize);

static TYPEVAR_COUNTER: AtomicUsize = AtomicUsize::new(0);

impl TypeVar {
    /// Create a fresh type variable
    pub fn fresh() -> Self {
        TypeVar(TYPEVAR_COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// Current value of the fresh counter WITHOUT allocating — diagnosis
    /// instrument for id-collision investigations (a `with_id`-minted id
    /// that the counter later crosses aliases an unrelated live var).
    pub fn peek_counter() -> usize {
        TYPEVAR_COUNTER.load(Ordering::Relaxed)
    }

    /// Create a type variable with a specific ID (for testing)
    pub fn with_id(id: usize) -> Self {
        TypeVar(id)
    }

    /// Alias for with_id - creates a type variable with a specific ID
    pub fn new(id: usize) -> Self {
        TypeVar(id)
    }

    pub fn id(&self) -> usize {
        self.0
    }
}

impl fmt::Display for TypeVar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Use Greek letters for first 8 type variables (common case)
        let letters = ['α', 'β', 'γ', 'δ', 'ε', 'ζ', 'η', 'θ'];
        if self.0 < letters.len() {
            write!(f, "{}", letters[self.0])
        } else {
            // For larger IDs, show as inferred type placeholder
            // This is more user-friendly than internal IDs like τ7610
            write!(f, "_")
        }
    }
}

// ToText implementation for Type - converts Display output to Text
impl ToText for Type {
    fn to_text(&self) -> Text {
        Text::from(self.to_string())
    }
}

// ToText implementation for TypeVar
impl ToText for TypeVar {
    fn to_text(&self) -> Text {
        Text::from(self.to_string())
    }
}

/// A substitution maps type variables to types.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Substitution {
    types: IndexMap<TypeVar, Type>,
    pub(crate) value_bindings: IndexMap<ValueId, Type>,
    pub(crate) universes: crate::universe_inference::UniverseInference,
}

impl Substitution {
    pub fn new() -> Self { Self::default() }
    pub fn is_empty(&self) -> bool { self.types.is_empty() && self.value_bindings.is_empty() && self.universes.is_empty() }

    fn check_value_cycles(&self) -> Result<(), Text> {
        let mut completed = Set::new();
        let mut visiting = Set::new();
        let mut pending = List::new();
        for root in self.value_bindings.keys() {
            if completed.contains(root) { continue; }
            pending.push((*root, false));
            while let Some((id, leaving)) = pending.pop() {
                if leaving {
                    visiting.remove(&id);
                    completed.insert(id);
                    continue;
                }
                if completed.contains(&id) { continue; }
                if !visiting.insert(id) {
                    return Err("cyclic compile-time value substitution".into());
                }
                pending.push((id, true));
                if let Some(value) = self.value_bindings.get(&id) {
                    for dependency in value.value_ids() {
                        if self.value_bindings.contains_key(&dependency) {
                            if visiting.contains(&dependency) {
                                return Err("cyclic compile-time value substitution".into());
                            }
                            if !completed.contains(&dependency) {
                                pending.push((dependency, false));
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

impl std::ops::Deref for Substitution {
    type Target = IndexMap<TypeVar, Type>;
    fn deref(&self) -> &Self::Target { &self.types }
}
impl std::ops::DerefMut for Substitution {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.types }
}
impl FromIterator<(TypeVar, Type)> for Substitution {
    fn from_iter<I: IntoIterator<Item = (TypeVar, Type)>>(items: I) -> Self {
        Self { types: items.into_iter().collect(), value_bindings: Default::default(), universes: Default::default() }
    }
}
impl<'a> IntoIterator for &'a Substitution {
    type Item = (&'a TypeVar, &'a Type);
    type IntoIter = indexmap::map::Iter<'a, TypeVar, Type>;
    fn into_iter(self) -> Self::IntoIter { self.types.iter() }
}

/// Extension trait for Substitution
pub trait SubstitutionExt {
    /// Compose two substitutions: apply `other` after `self`, checking levels.
    fn compose(&self, other: &Substitution) -> Result<Substitution, Text>;

    /// Get the domain (set of variables being substituted)
    fn domain(&self) -> Set<TypeVar>;
}

impl SubstitutionExt for Substitution {
    fn compose(&self, other: &Substitution) -> Result<Substitution, Text> {
        let mut result = Substitution::new();
        result.universes = self.universes.compose(&other.universes)?;
        for (id, value) in &self.value_bindings {
            result.value_bindings.insert(*id, value.try_apply_subst(other)?);
        }
        for (id, value) in &other.value_bindings {
            if let Some(previous) = result.value_bindings.get(id) {
                if previous != value {
                    return Err("conflicting substitutions for one compile-time value binder".into());
                }
            } else {
                result.value_bindings.insert(*id, value.clone());
            }
        }
        result.check_value_cycles()?;

        // Apply other to all types in self (s1.compose(s2) means apply s2 after s1)
        // So for each v -> t in s1, we get v -> s2(t)
        for (var, ty) in self {
            result.insert(*var, ty.try_apply_subst(other)?);
        }

        // Add bindings from other that aren't in self
        for (var, ty) in other {
            if !result.contains_key(var) {
                result.insert(*var, ty.clone());
            }
        }

        Ok(result)
    }

    fn domain(&self) -> Set<TypeVar> {
        let mut set = Set::new();
        for key in self.keys() {
            set.insert(*key);
        }
        set
    }
}
#[cfg(test)]
mod free_vars_covers_every_carrier {
    //! `collect_free_vars` must descend into EVERY type that carries a type.
    //!
    //! A missing arm does not read as "unknown" — it answers "no free
    //! variables", and two consumers act on that: generalization (a var it
    //! cannot see is never quantified, so the scheme is reused unfreshened at
    //! every call site) and the occurs check in `Unifier::bind_var` (a cycle
    //! through such a type is invisible). `Slice` was missing; the six
    //! `slice_*` intrinsics in `core/intrinsics/memory.vr` therefore shared one
    //! var across all call sites (T0742).
    use super::*;

    fn v() -> (TypeVar, Type) {
        let tv = TypeVar::fresh();
        (tv, Type::Var(tv))
    }

    /// The case that was actually broken, stated on its own so a regression
    /// names itself.
    #[test]
    fn slice_element_is_reachable() {
        let (tv, ty) = v();
        let slice = Type::Slice {
            element: Box::new(ty),
        };
        assert!(
            slice.free_vars().contains(&tv),
            "a var inside `&[T]` must be visible to generalization and to the \
             occurs check"
        );
    }

    /// One case per carrier. Each wraps a fresh var and asserts it comes back
    /// out; a variant added to `Type` without an arm now fails to compile in
    /// `collect_free_vars` (the catch-all is gone), and a variant added
    /// WITHOUT a case here is caught by the count assertion at the end.
    #[test]
    fn every_carrying_variant_reports_its_var() {
        let mut checked = 0;
        let mut assert_reaches = |ty: Type, tv: TypeVar, what: &str| {
            assert!(
                ty.free_vars().contains(&tv),
                "{what}: the carried var is invisible to collect_free_vars"
            );
            checked += 1;
        };

        let (tv, t) = v();
        assert_reaches(
            Type::Slice {
                element: Box::new(t.clone()),
            },
            tv,
            "Slice",
        );

        let (tv, t) = v();
        let mut fields = indexmap::IndexMap::new();
        fields.insert(Text::from("f"), t);
        assert_reaches(
            Type::ExtensibleRecord {
                fields,
                row_var: None,
            },
            tv,
            "ExtensibleRecord.fields",
        );

        let row = TypeVar::fresh();
        assert_reaches(
            Type::ExtensibleRecord {
                fields: indexmap::IndexMap::new(),
                row_var: Some(row),
            },
            row,
            "ExtensibleRecord.row_var",
        );

        let (tv, t) = v();
        let mut bindings = Map::new();
        bindings.insert(Text::from("Item"), t);
        assert_reaches(
            Type::DynProtocol {
                bounds: List::new(),
                bindings,
            },
            tv,
            "DynProtocol.bindings",
        );

        let (tv, t) = v();
        assert_reaches(
            Type::Pi {
                param_name: Text::from("x"),
                param_type: Box::new(t),
                return_type: Box::new(Type::Unit),
            },
            tv,
            "Pi.param_type",
        );

        let (tv, t) = v();
        assert_reaches(
            Type::Pi {
                param_name: Text::from("x"),
                param_type: Box::new(Type::Unit),
                return_type: Box::new(t),
            },
            tv,
            "Pi.return_type",
        );

        let (tv, t) = v();
        assert_reaches(
            Type::Sigma {
                fst_name: Text::from("x"),
                fst_type: Box::new(t),
                snd_type: Box::new(Type::Unit),
            },
            tv,
            "Sigma.fst_type",
        );

        let (tv, t) = v();
        assert_reaches(
            Type::Sigma {
                fst_name: Text::from("x"),
                fst_type: Box::new(Type::Unit),
                snd_type: Box::new(t),
            },
            tv,
            "Sigma.snd_type",
        );

        let (tv, t) = v();
        assert_reaches(
            Type::Quantified {
                inner: Box::new(t),
                quantity: Quantity::Omega,
            },
            tv,
            "Quantified.inner",
        );

        let (tv, t) = v();
        assert_reaches(
            Type::CapabilityRestricted {
                base: Box::new(t),
                capabilities: Default::default(),
            },
            tv,
            "CapabilityRestricted.base",
        );

        assert_eq!(
            checked, 10,
            "every carrier case must run — a short count means one was skipped"
        );
    }

    /// The occurs check reads `free_vars`, so a cycle through a slice must be
    /// visible: without this, `T := &[T]` is accepted and later expansion runs
    /// away.
    #[test]
    fn occurs_through_a_slice_is_visible() {
        let (tv, t) = v();
        let cyclic = Type::Reference {
            inner: Box::new(Type::Slice {
                element: Box::new(t),
            }),
            mutable: false,
        };
        assert!(cyclic.free_vars().contains(&tv));
    }
}

// Tests moved to tests/ty_tests.rs

#[cfg(test)]
#[path = "../tests/unit/value_substitution.rs"]
mod value_substitution_laws;
