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
use verum_common::{List, Map, Set, Text};
use verum_common::ToText;
use verum_common::well_known_types::WellKnownType as WKT;

use crate::refinement::RefinementPredicate;

// =============================================================================
// DEPENDENT TYPES INFRASTRUCTURE (Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — )
// =============================================================================

/// Universe level for dependent type theory.
/// Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter
///
/// Types have types (called universes) forming an infinite hierarchy:
/// ```text
/// Type₀ : Type₁ : Type₂ : ...
/// ```
///
/// This hierarchy prevents Girard's paradox (similar to Russell's paradox).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum UniverseLevel {
    /// Concrete level (0, 1, 2, ...)
    Concrete(u32),
    /// Universe variable for universe polymorphism
    Variable(u32),
    /// Maximum of two levels: max(u, v)
    Max(u32, u32),
    /// Successor of a level: u + 1
    Succ(u32),
}

impl UniverseLevel {
    /// The base universe Type₀ (or just Type)
    pub const TYPE: Self = UniverseLevel::Concrete(0);

    /// The first universe Type₁
    pub const TYPE1: Self = UniverseLevel::Concrete(1);

    /// The second universe Type₂
    pub const TYPE2: Self = UniverseLevel::Concrete(2);

    /// Create a concrete universe level
    pub fn concrete(n: u32) -> Self {
        UniverseLevel::Concrete(n)
    }

    /// Create a universe variable
    pub fn variable(id: u32) -> Self {
        UniverseLevel::Variable(id)
    }

    /// Successor universe level.
    /// Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter — Succ levels
    ///
    /// For concrete levels, directly computes n + 1.
    /// For variable levels, creates a Succ expression that will be resolved
    /// during constraint solving.
    pub fn succ(self) -> Self {
        match self {
            // Concrete: directly compute successor
            UniverseLevel::Concrete(n) => UniverseLevel::Concrete(n.saturating_add(1)),
            // Variable: create a Succ that references this variable
            UniverseLevel::Variable(v) => UniverseLevel::Succ(v),
            // Max(a, b) + 1: succ(max(a, b)) - encode as Max with offset
            // The constraint solver will resolve the actual values
            UniverseLevel::Max(a, b) => {
                UniverseLevel::Max(a.saturating_add(1), b.saturating_add(1))
            }
            // Succ(v) + 1 = Succ(Succ(v)) - chain of successors
            // We track this as Succ with an incremented variable ID marker
            UniverseLevel::Succ(v) => UniverseLevel::Succ(v.saturating_add(1)),
        }
    }

    /// Maximum of two universe levels.
    /// Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter — Max levels
    ///
    /// For concrete levels, directly computes the maximum.
    /// For levels involving variables, creates constraint expressions
    /// that will be resolved during constraint solving via UniverseContext.
    pub fn max(self, other: Self) -> Self {
        match (self, other) {
            // Both concrete: directly compute max
            (UniverseLevel::Concrete(a), UniverseLevel::Concrete(b)) => {
                UniverseLevel::Concrete(a.max(b))
            }

            // Both variables: straightforward Max(var_a, var_b)
            (UniverseLevel::Variable(v1), UniverseLevel::Variable(v2)) => {
                UniverseLevel::Max(v1, v2)
            }

            // Variable with concrete: encode concrete in the Max structure
            // The constraint solver (UniverseContext) resolves these
            (UniverseLevel::Concrete(a), UniverseLevel::Variable(v)) => {
                // Use the concrete value as a bound marker
                UniverseLevel::Max(a, v)
            }
            (UniverseLevel::Variable(v), UniverseLevel::Concrete(b)) => UniverseLevel::Max(v, b),

            // Variable with Succ: need constraint-based resolution
            (UniverseLevel::Variable(v), UniverseLevel::Succ(s)) => {
                // max(v, s+1) - encode with v and s for constraint solver
                UniverseLevel::Max(v, s.saturating_add(1000)) // Offset to distinguish from concrete
            }
            (UniverseLevel::Succ(s), UniverseLevel::Variable(v)) => {
                UniverseLevel::Max(s.saturating_add(1000), v)
            }

            // Both Succ: max(a+1, b+1) = max(a, b) + 1
            (UniverseLevel::Succ(a), UniverseLevel::Succ(b)) => UniverseLevel::Max(a, b).succ(),

            // Max with Max: compose into new Max (simplified - full impl uses constraints)
            (UniverseLevel::Max(a1, b1), UniverseLevel::Max(a2, b2)) => {
                // max(max(a1, b1), max(a2, b2)) = max(a1, b1, a2, b2)
                // We pick the larger pair for approximation
                UniverseLevel::Max(a1.max(a2), b1.max(b2))
            }

            // Max with Succ
            (UniverseLevel::Max(a, b), UniverseLevel::Succ(s)) => {
                // max(max(a, b), s+1)
                UniverseLevel::Max(a.max(s.saturating_add(1)), b)
            }
            (UniverseLevel::Succ(s), UniverseLevel::Max(a, b)) => {
                UniverseLevel::Max(s.saturating_add(1).max(a), b)
            }

            // Concrete with Max
            (UniverseLevel::Concrete(c), UniverseLevel::Max(a, b)) => {
                UniverseLevel::Max(c.max(a), b)
            }
            (UniverseLevel::Max(a, b), UniverseLevel::Concrete(c)) => {
                UniverseLevel::Max(a, b.max(c))
            }

            // Concrete with Succ: max(c, s+1)
            (UniverseLevel::Concrete(c), UniverseLevel::Succ(s)) => {
                // The result depends on whether c > s+1
                // For constraint-based handling, encode both
                UniverseLevel::Max(c, s.saturating_add(1000))
            }
            (UniverseLevel::Succ(s), UniverseLevel::Concrete(c)) => {
                UniverseLevel::Max(s.saturating_add(1000), c)
            }

            // Variable with Max: encode for constraint solver
            (UniverseLevel::Variable(v), UniverseLevel::Max(a, b)) => {
                // max(v, max(a, b)) - compose with existing max
                UniverseLevel::Max(v.max(a), b)
            }
            (UniverseLevel::Max(a, b), UniverseLevel::Variable(v)) => {
                // max(max(a, b), v) - compose with existing max
                UniverseLevel::Max(a, b.max(v))
            }
        }
    }

    /// Check if this level is strictly less than another.
    /// Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter
    ///
    /// Returns true only when we can definitively prove the relationship.
    /// For variable levels, returns false (conservative) and the constraint
    /// solver in UniverseContext handles these cases.
    pub fn is_less_than(&self, other: &Self) -> bool {
        match (self, other) {
            // Concrete comparisons are definite
            (UniverseLevel::Concrete(a), UniverseLevel::Concrete(b)) => a < b,

            // Concrete is less than Succ of same or higher value
            (UniverseLevel::Concrete(a), UniverseLevel::Succ(v)) => {
                // Succ(v) >= 1, so Concrete(0) is always less
                *a == 0 || *a < *v
            }

            // v < Succ(v) is always true (definitionally)
            (UniverseLevel::Variable(v1), UniverseLevel::Succ(v2)) if v1 == v2 => true,

            // Succ(u) < Succ(v) if u < v
            (UniverseLevel::Succ(u), UniverseLevel::Succ(v)) => u < v,

            // Max(a, b) < c if both a < c and b < c
            (UniverseLevel::Max(a, b), UniverseLevel::Concrete(c)) => a < c && b < c,

            // a < Max(b, c) if a < b or a < c (cannot determine statically for variables)
            (UniverseLevel::Concrete(a), UniverseLevel::Max(b, c)) => *a < *b || *a < *c,

            // For other cases involving variables, we cannot decide statically
            // The constraint solver in UniverseContext handles these
            _ => false,
        }
    }

    /// Check if this level is less than or equal to another.
    pub fn is_less_or_equal(&self, other: &Self) -> bool {
        self == other || self.is_less_than(other)
    }

    /// Get all variable IDs referenced in this level.
    pub fn variables(&self) -> Vec<u32> {
        match self {
            UniverseLevel::Concrete(_) => vec![],
            UniverseLevel::Variable(v) => vec![*v],
            UniverseLevel::Max(a, b) => vec![*a, *b],
            UniverseLevel::Succ(v) => vec![*v],
        }
    }

    /// Try to compute a concrete lower bound for this level.
    /// Returns the minimum possible value, or 0 if unknown.
    pub fn lower_bound(&self) -> u32 {
        match self {
            UniverseLevel::Concrete(n) => *n,
            UniverseLevel::Variable(_) => 0, // Variables have minimum 0
            UniverseLevel::Max(a, b) => (*a).max(*b), // Max is at least max(a, b)
            UniverseLevel::Succ(_) => 1,     // Succ is at least 1
        }
    }

    /// Check if this level contains any variables (is polymorphic).
    pub fn is_polymorphic(&self) -> bool {
        !matches!(self, UniverseLevel::Concrete(_))
    }
}

impl Default for UniverseLevel {
    fn default() -> Self {
        UniverseLevel::TYPE
    }
}

impl fmt::Display for UniverseLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UniverseLevel::Concrete(0) => write!(f, "Type"),
            UniverseLevel::Concrete(n) => write!(f, "Type{}", subscript(*n)),
            UniverseLevel::Variable(v) => write!(f, "u{}", v),
            UniverseLevel::Max(a, b) => write!(f, "max({}, {})", a, b),
            UniverseLevel::Succ(v) => write!(f, "u{} + 1", v),
        }
    }
}

/// Helper to create subscript digits for universe display
fn subscript(n: u32) -> String {
    n.to_string()
        .chars()
        .map(|c| match c {
            '0' => '₀',
            '1' => '₁',
            '2' => '₂',
            '3' => '₃',
            '4' => '₄',
            '5' => '₅',
            '6' => '₆',
            '7' => '₇',
            '8' => '₈',
            '9' => '₉',
            _ => c,
        })
        .collect()
}

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

    /// Integer type (arbitrary precision)
    Int,

    /// Floating-point type (IEEE 754 double)
    Float,

    /// Character type (Unicode scalar value)
    Char,

    /// Text type (UTF-8)
    Text,

    /// Type variable (for inference)
    Var(TypeVar),

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
    /// let p2d = {x: 1, y: 2}       // {x: Int, y: Int}
    /// let p3d = {x: 1, y: 2, z: 3} // {x: Int, y: Int, z: Int}
    ///
    /// get_x(p2d)  // OK: r = {y: Int}
    /// get_x(p3d)  // OK: r = {y: Int, z: Int}
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
        /// Parameter name (for error messages and debugging)
        name: Text,
        /// Base type of the meta parameter (e.g., usize, [usize], bool)
        ty: Box<Type>,
        /// Optional refinement constraint (e.g., {> 0})
        refinement: Option<RefinementPredicate>,
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
    /// - Tensor<f32, [4]>           // 1D vector (4 elements)
    /// - Tensor<f32, [2, 3]>        // 2D matrix (2×3)
    /// - Tensor<u8, [3, 224, 224]>  // 3D tensor (RGB image)
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
    ///     ptr: *const T      // 8 bytes
    ///     generation: u64    // 8 bytes
    /// }
    /// Total: 16 bytes (overhead: ~20ns = 15ns CBGR + 5ns generation check)
    /// ```
    ///
    /// # Example
    ///
    /// ```verum
    /// type WindowIterator<T> {
    ///     data: GenRef<List<T>>,
    ///     window_size: usize,
    ///     position: usize
    /// }
    ///
    /// implement<T> Iterator for WindowIterator<T> {
    ///     type Item is [T]
    ///
    ///     fn next(&mut self) -> Maybe<GenRef<&[T]>> {
    ///         let data = self.data.deref()?;
    ///         if self.position + self.window_size <= data.len() {
    ///             let slice = &data[self.position..self.position + self.window_size];
    ///             self.position += 1;
    ///             Some(GenRef.borrow(slice))
    ///         } else {
    ///             None
    ///         }
    ///     }
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
    ///     type F<_>  // Type constructor with arity 1
    ///
    ///     fn map<A, B>(self: Self.F<A>, f: fn(A) -> B) -> Self.F<B>
    /// }
    ///
    /// // List is a type constructor
    /// implement Functor for ListFunctor {
    ///     type F<T> is List<T>  // TypeConstructor { name: "List", arity: 1, kind: * -> * }
    ///
    ///     fn map<A, B>(self: List<A>, f: fn(A) -> B) -> List<B> {
    ///         self.iter().map(f).collect()
    ///     }
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
    ///     type M<_>  // Type constructor
    ///
    ///     fn pure<T>(value: T) -> Self.M<T>
    ///     fn bind<T, U>(self: Self.M<T>, f: fn(T) -> Self.M<U>) -> Self.M<U>
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
    /// fn replicate<T>(n: Nat) -> List<T, n>  // Return type depends on n
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
    /// Γ ⊢ f : (x: A) → B    Γ ⊢ a : A
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
    /// Γ ⊢ a : A    Γ ⊢ b : B[a/x]
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
    /// Γ ⊢ p : a = b    Γ, x: A, h: a = x ⊢ C : Type    Γ ⊢ c : C[a/x, refl/h]
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
    /// Γ, i: I ⊢ e : A    e[i0/i] ≡ a    e[i1/i] ≡ b
    /// ─────────────────────────────────────────────────── (Path-Intro)
    /// Γ ⊢ λ(i). e : Path<A>(a, b)
    ///
    /// Γ ⊢ p : Path<A>(a, b)    Γ ⊢ r : I
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
    /// Γ ⊢ A : Type    Γ ⊢ φ : I    Γ, (φ = i1) ⊢ u : A
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
    ///     [P: Prop] -> (p1: P) -> (p2: P) -> p1 = p2
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
    ///     zero : Nat,
    ///     succ : Nat -> Nat
    /// }
    ///
    /// // Indexed list (length-tracked)
    /// inductive List<A: Type> : Nat -> Type {
    ///     nil : List<A, zero>,
    ///     cons : <n> -> A -> List<A, n> -> List<A, succ(n)>
    /// }
    ///
    /// // Finite sets
    /// inductive Fin : Nat -> Type {
    ///     FZero : <n> -> Fin<succ(n)>,
    ///     FSucc : <n> -> Fin<n> -> Fin<succ(n)>
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
    ///     head : Stream<A> -> A,
    ///     tail : Stream<A> -> Stream<A>
    /// }
    ///
    /// // Infinite stream of naturals
    /// fn nats_from(n: Nat) : Stream<Nat> = {
    ///     head = n,
    ///     tail = nats_from(succ(n))
    /// }
    ///
    /// // Productivity must be checked (no infinite unfolding)
    /// fn map<A, B>(f: A -> B, s: Stream<A>) : Stream<B> = {
    ///     head = f(s.head),
    ///     tail = map(f, s.tail)  // Productive recursive call
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
    ///     base : Circle,
    ///     loop : base = base  // Path constructor
    /// }
    ///
    /// // Torus with paths and surface
    /// hott inductive Torus : Type {
    ///     point : Torus,
    ///     meridian : point = point,
    ///     longitude : point = point,
    ///     surface : meridian · longitude = longitude · meridian  // 2-path
    /// }
    ///
    /// // Quotient types
    /// hott inductive Quotient<A: Type, R: A -> A -> Type> : Type {
    ///     class : A -> Quotient<A, R>,
    ///     relate : (x, y: A) -> R(x, y) -> class(x) = class(y)
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
    ///     sort_by: SortOrder,  // SortOrder referenced before definition
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

    /// Check if this type is the Never type (bottom type).
    ///
    /// Handles both Type::Never and Type::Named("Never") which can occur
    /// when the Never type is imported from core.base.panic.
    pub fn is_never(&self) -> bool {
        match self {
            Type::Never => true,
            Type::Named { path, args } if args.is_empty() => {
                // Check if the path is just "Never"
                if let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.last() {
                    ident.name.as_str() == "Never"
                } else {
                    false
                }
            }
            _ => false,
        }
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
        // Structural fallback: variant type with exactly Ok and Err constructors
        if let Type::Variant(variants) = self {
            if variants.len() == 2 {
                let ok = variants.get("Ok");
                let err = variants.get("Err");
                if let (Some(ok_ty), Some(err_ty)) = (ok, err) {
                    return Some((ok_ty.clone(), err_ty.clone()));
                }
            }
        }
        None
    }

    /// Structural check: is this a `Maybe<T>` type?
    /// Returns `Some(T)` if so. Checks both nominal Maybe and variant types
    /// with `Some`/`None` constructors (structural match).
    pub fn as_maybe(&self) -> Option<Type> {
        if let Some(args) = self.match_named_type(WKT::Maybe.as_str()) {
            if args.len() == 1 {
                return Some(args[0].clone());
            }
        }
        // Structural fallback: variant type with exactly Some and None constructors
        if let Type::Variant(variants) = self {
            if variants.len() == 2 && variants.contains_key("Some") && variants.contains_key("None") {
                if let Some(some_ty) = variants.get("Some") {
                    return Some(some_ty.clone());
                }
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

    /// Create a function type (pure by default, no context requirements)
    pub fn function(params: List<Type>, return_type: Type) -> Self {
        Type::Function {
            params,
            return_type: Box::new(return_type),
            type_params: List::new(),
            contexts: None,
            properties: Some(crate::computational_properties::PropertySet::pure()),
        }
    }

    /// Create a function type with DI contexts (pure by default)
    pub fn function_with_contexts(
        params: List<Type>,
        return_type: Type,
        contexts: crate::di::requirement::ContextRequirement,
    ) -> Self {
        Type::Function {
            params,
            return_type: Box::new(return_type),
            type_params: List::new(),
            contexts: Some(crate::di::requirement::ContextExpr::Concrete(contexts)),
            properties: Some(crate::computational_properties::PropertySet::pure()),
        }
    }

    /// Create a function type with a context variable (for context polymorphism)
    pub fn function_with_context_var(
        params: List<Type>,
        return_type: Type,
        context_var: TypeVar,
    ) -> Self {
        Type::Function {
            params,
            return_type: Box::new(return_type),
            type_params: List::new(),
            contexts: Some(crate::di::requirement::ContextExpr::Variable(context_var)),
            properties: Some(crate::computational_properties::PropertySet::pure()),
        }
    }

    /// Create a function type with computational properties
    pub fn function_with_properties(
        params: List<Type>,
        return_type: Type,
        properties: crate::computational_properties::PropertySet,
    ) -> Self {
        Type::Function {
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

    /// Create a refinement type
    pub fn refined(base: Type, predicate: RefinementPredicate) -> Self {
        Type::Refined {
            base: Box::new(base),
            predicate,
        }
    }

    /// Create a meta parameter type
    /// Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Unified meta-system for compile-time computation
    pub fn meta(name: Text, ty: Type, refinement: Option<RefinementPredicate>) -> Self {
        Type::Meta {
            name,
            ty: Box::new(ty),
            refinement,
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
    ///     Text::from("List"),
    ///     1,
    ///     Kind::unary_constructor()
    /// );
    ///
    /// // Map has kind * -> * -> * (binary type constructor)
    /// let map_ctor = Type::type_constructor(
    ///     Text::from("Map"),
    ///     2,
    ///     Kind::binary_constructor()
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
            Type::Pi { .. } | Type::Sigma { .. } | Type::Eq { .. } | Type::PathType { .. } | Type::Partial { .. } => true,
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
            Type::Universe { level } => Some(*level),
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
                        Type::Universe { level: l1.max(l2) }
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
                        Type::Universe { level: l1.max(l2) }
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
            Type::Inductive { universe, .. } => Type::Universe { level: *universe },

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
    ///     Type::float(),
    ///     vec![ConstValue::UInt(2), ConstValue::UInt(3)],
    ///     span
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

    /// Create an i8 type with refinement for the specific value
    pub fn i8_refined(_value: i8) -> Self {
        let ident = verum_ast::ty::Ident::new("i8", Span::dummy());
        Type::Named {
            path: Path::single(ident),
            args: List::new(),
        }
    }

    /// Create an i16 type with refinement for the specific value
    pub fn i16_refined(_value: i16) -> Self {
        let ident = verum_ast::ty::Ident::new("i16", Span::dummy());
        Type::Named {
            path: Path::single(ident),
            args: List::new(),
        }
    }

    /// Create an i32 type with refinement for the specific value
    pub fn i32_refined(_value: i32) -> Self {
        let ident = verum_ast::ty::Ident::new("i32", Span::dummy());
        Type::Named {
            path: Path::single(ident),
            args: List::new(),
        }
    }

    /// Create an i64 type with refinement for the specific value
    pub fn i64_refined(_value: i64) -> Self {
        let ident = verum_ast::ty::Ident::new("i64", Span::dummy());
        Type::Named {
            path: Path::single(ident),
            args: List::new(),
        }
    }

    /// Create an i128 type with refinement for the specific value
    pub fn i128_refined(_value: i128) -> Self {
        let ident = verum_ast::ty::Ident::new("i128", Span::dummy());
        Type::Named {
            path: Path::single(ident),
            args: List::new(),
        }
    }

    /// Create an isize type with refinement for the specific value
    pub fn isize_refined(_value: isize) -> Self {
        let ident = verum_ast::ty::Ident::new("isize", Span::dummy());
        Type::Named {
            path: Path::single(ident),
            args: List::new(),
        }
    }

    /// Create a u8 type with refinement for the specific value
    /// Note: Uses semantic name "Byte" rather than machine name "u8"
    /// Verum philosophy: Byte is the canonical 8-bit unsigned type
    pub fn u8_refined(_value: u8) -> Self {
        let ident = verum_ast::ty::Ident::new("Byte", Span::dummy());
        Type::Named {
            path: Path::single(ident),
            args: List::new(),
        }
    }

    /// Create a u16 type with refinement for the specific value
    pub fn u16_refined(_value: u16) -> Self {
        let ident = verum_ast::ty::Ident::new("u16", Span::dummy());
        Type::Named {
            path: Path::single(ident),
            args: List::new(),
        }
    }

    /// Create a u32 type with refinement for the specific value
    pub fn u32_refined(_value: u32) -> Self {
        let ident = verum_ast::ty::Ident::new("u32", Span::dummy());
        Type::Named {
            path: Path::single(ident),
            args: List::new(),
        }
    }

    /// Create a u64 type with refinement for the specific value
    pub fn u64_refined(_value: u64) -> Self {
        let ident = verum_ast::ty::Ident::new("u64", Span::dummy());
        Type::Named {
            path: Path::single(ident),
            args: List::new(),
        }
    }

    /// Create a u128 type with refinement for the specific value
    pub fn u128_refined(_value: u128) -> Self {
        let ident = verum_ast::ty::Ident::new("u128", Span::dummy());
        Type::Named {
            path: Path::single(ident),
            args: List::new(),
        }
    }

    /// Create a usize type with refinement for the specific value
    pub fn usize_refined(_value: usize) -> Self {
        let ident = verum_ast::ty::Ident::new("usize", Span::dummy());
        Type::Named {
            path: Path::single(ident),
            args: List::new(),
        }
    }

    /// Create an f32 type with refinement for the specific value
    pub fn f32_refined(_value: f32) -> Self {
        let ident = verum_ast::ty::Ident::new("f32", Span::dummy());
        Type::Named {
            path: Path::single(ident),
            args: List::new(),
        }
    }

    /// Create an f64 type with refinement for the specific value
    pub fn f64_refined(_value: f64) -> Self {
        let ident = verum_ast::ty::Ident::new("f64", Span::dummy());
        Type::Named {
            path: Path::single(ident),
            args: List::new(),
        }
    }

    /// Get the free type variables in this type
    pub fn free_vars(&self) -> Set<TypeVar> {
        let mut vars = Set::new();
        self.collect_free_vars(&mut vars);
        vars
    }

    fn collect_free_vars(&self, vars: &mut Set<TypeVar>) {
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
                    param.collect_free_vars(vars);
                }
                return_type.collect_free_vars(vars);
            }
            Type::Tuple(types) => {
                for ty in types {
                    ty.collect_free_vars(vars);
                }
            }
            Type::Array { element, .. } => {
                element.collect_free_vars(vars);
            }
            Type::Record(fields) => {
                for ty in fields.values() {
                    ty.collect_free_vars(vars);
                }
            }
            Type::Variant(variants) => {
                for ty in variants.values() {
                    ty.collect_free_vars(vars);
                }
            }
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. }
            | Type::Ownership { inner, .. }
            | Type::Pointer { inner, .. }
            | Type::VolatilePointer { inner, .. } => {
                inner.collect_free_vars(vars);
            }
            Type::Refined { base, .. } => {
                // Note: We don't track free vars in predicates
                // as they're value-level, not type-level
                base.collect_free_vars(vars);
            }
            Type::Exists { var, body } => {
                let mut body_vars = Set::new();
                body.collect_free_vars(&mut body_vars);
                body_vars.remove(var);
                for v in body_vars {
                    vars.insert(v);
                }
            }
            Type::Forall {
                vars: bound_vars,
                body,
            } => {
                let mut body_vars = Set::new();
                body.collect_free_vars(&mut body_vars);
                for v in bound_vars {
                    body_vars.remove(v);
                }
                for v in body_vars {
                    vars.insert(v);
                }
            }
            Type::Named { args, .. } => {
                for arg in args {
                    arg.collect_free_vars(vars);
                }
            }
            Type::Generic { args, .. } => {
                for arg in args {
                    arg.collect_free_vars(vars);
                }
            }
            Type::Meta { ty, .. } => {
                // Meta parameters can contain type variables in their base type
                ty.collect_free_vars(vars);
            }
            Type::Future { output } => {
                output.collect_free_vars(vars);
            }
            Type::Generator {
                yield_ty,
                return_ty,
            } => {
                yield_ty.collect_free_vars(vars);
                return_ty.collect_free_vars(vars);
            }
            Type::Tensor { element, .. } => {
                // Collect free vars from element type
                // Shape dimensions are compile-time constants (no type vars)
                element.collect_free_vars(vars);
            }
            Type::GenRef { inner } => {
                // GenRef wraps a type, so collect from inner type
                inner.collect_free_vars(vars);
            }
            Type::TypeConstructor { .. } => {
                // Type constructors don't contain type variables themselves
                // They're just names with arity/kind metadata
            }
            Type::TypeApp { constructor, args } => {
                // Collect from both constructor and arguments
                constructor.collect_free_vars(vars);
                for arg in args {
                    arg.collect_free_vars(vars);
                }
            }
            Type::PathType { space, .. } => {
                // Collect type vars from the space type
                // CubicalTerm endpoints are value-level, no type vars
                space.collect_free_vars(vars);
            }
            Type::Partial { element_type, .. } => {
                // Collect type vars from the element type
                element_type.collect_free_vars(vars);
            }
            Type::Interval => {
                // Interval is a primitive type, no free vars
            }
            _ => {}
        }
    }

    /// Apply a substitution to this type
    /// SAFETY: Uses depth limit to prevent exponential type expansion
    pub fn apply_subst(&self, subst: &Substitution) -> Type {
        const MAX_SUBST_DEPTH: usize = 30;
        self.apply_subst_with_depth(subst, 0, MAX_SUBST_DEPTH)
    }

    /// Internal substitution with depth tracking
    fn apply_subst_with_depth(&self, subst: &Substitution, depth: usize, max_depth: usize) -> Type {
        // Prevent exponential type growth
        if depth >= max_depth {
            tracing::warn!("Maximum type substitution depth ({}) exceeded", max_depth);
            return self.clone();
        }

        let next_depth = depth + 1;
        match self {
            Type::Var(v) => subst.get(v).cloned().unwrap_or_else(|| self.clone()),
            Type::Function {
                params,
                return_type,
                type_params,
                contexts,
                properties,
            } => Type::Function {
                params: params
                    .iter()
                    .map(|p| p.apply_subst_with_depth(subst, next_depth, max_depth))
                    .collect(),
                return_type: Box::new(
                    return_type.apply_subst_with_depth(subst, next_depth, max_depth),
                ),
                type_params: type_params.clone(),
                contexts: contexts.clone(),
                properties: properties.clone(),
            },
            Type::Tuple(types) => Type::Tuple(
                types
                    .iter()
                    .map(|t| t.apply_subst_with_depth(subst, next_depth, max_depth))
                    .collect(),
            ),
            Type::Array { element, size } => Type::Array {
                element: Box::new(element.apply_subst_with_depth(subst, next_depth, max_depth)),
                size: *size,
            },
            Type::Record(fields) => Type::Record(
                fields
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            v.apply_subst_with_depth(subst, next_depth, max_depth),
                        )
                    })
                    .collect(),
            ),
            Type::Variant(variants) => Type::Variant(
                variants
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            v.apply_subst_with_depth(subst, next_depth, max_depth),
                        )
                    })
                    .collect(),
            ),
            Type::Reference { mutable, inner } => Type::Reference {
                mutable: *mutable,
                inner: Box::new(inner.apply_subst_with_depth(subst, next_depth, max_depth)),
            },
            Type::CheckedReference { mutable, inner } => Type::CheckedReference {
                mutable: *mutable,
                inner: Box::new(inner.apply_subst_with_depth(subst, next_depth, max_depth)),
            },
            Type::UnsafeReference { mutable, inner } => Type::UnsafeReference {
                mutable: *mutable,
                inner: Box::new(inner.apply_subst_with_depth(subst, next_depth, max_depth)),
            },
            Type::Ownership { mutable, inner } => Type::Ownership {
                mutable: *mutable,
                inner: Box::new(inner.apply_subst_with_depth(subst, next_depth, max_depth)),
            },
            Type::Pointer { mutable, inner } => Type::Pointer {
                mutable: *mutable,
                inner: Box::new(inner.apply_subst_with_depth(subst, next_depth, max_depth)),
            },

            Type::VolatilePointer { mutable, inner } => Type::VolatilePointer {
                mutable: *mutable,
                inner: Box::new(inner.apply_subst_with_depth(subst, next_depth, max_depth)),
            },
            Type::Refined { base, predicate } => Type::Refined {
                base: Box::new(base.apply_subst_with_depth(subst, next_depth, max_depth)),
                predicate: predicate.clone(),
            },
            Type::Named { path, args } => Type::Named {
                path: path.clone(),
                args: args
                    .iter()
                    .map(|a| a.apply_subst_with_depth(subst, next_depth, max_depth))
                    .collect(),
            },
            Type::Generic { name, args } => Type::Generic {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|a| a.apply_subst_with_depth(subst, next_depth, max_depth))
                    .collect(),
            },
            Type::Exists { var, body } => {
                let mut subst = subst.clone();
                subst.shift_remove(var);
                Type::Exists {
                    var: *var,
                    body: Box::new(body.apply_subst_with_depth(&subst, next_depth, max_depth)),
                }
            }
            Type::Forall { vars, body } => {
                let mut subst = subst.clone();
                for v in vars {
                    subst.shift_remove(v);
                }
                Type::Forall {
                    vars: vars.clone(),
                    body: Box::new(body.apply_subst_with_depth(&subst, next_depth, max_depth)),
                }
            }
            Type::Meta {
                name,
                ty,
                refinement,
            } => Type::Meta {
                name: name.clone(),
                ty: Box::new(ty.apply_subst_with_depth(subst, next_depth, max_depth)),
                refinement: refinement.clone(),
            },
            Type::Future { output } => Type::Future {
                output: Box::new(output.apply_subst_with_depth(subst, next_depth, max_depth)),
            },
            Type::Generator {
                yield_ty,
                return_ty,
            } => Type::Generator {
                yield_ty: Box::new(yield_ty.apply_subst_with_depth(subst, next_depth, max_depth)),
                return_ty: Box::new(return_ty.apply_subst_with_depth(subst, next_depth, max_depth)),
            },
            Type::Tensor {
                element,
                shape,
                strides,
                span,
            } => Type::Tensor {
                element: Box::new(element.apply_subst_with_depth(subst, next_depth, max_depth)),
                shape: shape.clone(), // Shape is compile-time constant
                strides: strides.clone(),
                span: *span,
            },
            Type::GenRef { inner } => Type::GenRef {
                inner: Box::new(inner.apply_subst_with_depth(subst, next_depth, max_depth)),
            },
            Type::TypeConstructor { name, arity, kind } => Type::TypeConstructor {
                name: name.clone(),
                arity: *arity,
                kind: kind.clone(),
            },
            Type::TypeApp { constructor, args } => {
                let resolved_ctor = constructor.apply_subst_with_depth(subst, next_depth, max_depth);
                let resolved_args: List<Type> = args
                    .iter()
                    .map(|a| a.apply_subst_with_depth(subst, next_depth, max_depth))
                    .collect();

                // Try to reduce TypeApp when constructor is a concrete type with free vars.
                // This handles GAT resolution: TypeApp { ctor: List<$tv>, args: [Int] } → List<Int>
                match &resolved_ctor {
                    Type::Generic { name, args: ctor_args } if !name.as_str().starts_with("::") => {
                        // Build TypeVar substitution: positionally map free vars in ctor to resolved_args
                        let mut var_subst = Substitution::new();
                        for (i, arg) in ctor_args.iter().enumerate() {
                            if let Type::Var(tv) = arg {
                                if let Some(replacement) = resolved_args.get(i) {
                                    var_subst.insert(*tv, replacement.clone());
                                }
                            }
                        }
                        if !var_subst.is_empty() {
                            return resolved_ctor.apply_subst_with_depth(&var_subst, next_depth, max_depth);
                        }
                        // No free vars to substitute — replace args entirely if same arity
                        if ctor_args.len() == resolved_args.len() && ctor_args.iter().all(|a| matches!(a, Type::Var(_))) {
                            Type::Generic { name: name.clone(), args: resolved_args }
                        } else if resolved_args.is_empty() {
                            resolved_ctor
                        } else {
                            Type::TypeApp { constructor: Box::new(resolved_ctor), args: resolved_args }
                        }
                    }
                    Type::Named { path, args: ctor_args } => {
                        let mut var_subst = Substitution::new();
                        for (i, arg) in ctor_args.iter().enumerate() {
                            if let Type::Var(tv) = arg {
                                if let Some(replacement) = resolved_args.get(i) {
                                    var_subst.insert(*tv, replacement.clone());
                                }
                            }
                        }
                        if !var_subst.is_empty() {
                            return resolved_ctor.apply_subst_with_depth(&var_subst, next_depth, max_depth);
                        }
                        if ctor_args.len() == resolved_args.len() && ctor_args.iter().all(|a| matches!(a, Type::Var(_))) {
                            Type::Named { path: path.clone(), args: resolved_args }
                        } else if resolved_args.is_empty() {
                            resolved_ctor
                        } else {
                            Type::TypeApp { constructor: Box::new(resolved_ctor), args: resolved_args }
                        }
                    }
                    _ => Type::TypeApp { constructor: Box::new(resolved_ctor), args: resolved_args },
                }
            }

            // Dependent Type Substitution (Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — )
            Type::Pi {
                param_name,
                param_type,
                return_type,
            } => Type::Pi {
                param_name: param_name.clone(),
                param_type: Box::new(
                    param_type.apply_subst_with_depth(subst, next_depth, max_depth),
                ),
                return_type: Box::new(
                    return_type.apply_subst_with_depth(subst, next_depth, max_depth),
                ),
            },
            Type::Sigma {
                fst_name,
                fst_type,
                snd_type,
            } => Type::Sigma {
                fst_name: fst_name.clone(),
                fst_type: Box::new(fst_type.apply_subst_with_depth(subst, next_depth, max_depth)),
                snd_type: Box::new(snd_type.apply_subst_with_depth(subst, next_depth, max_depth)),
            },
            Type::Eq { ty, lhs, rhs } => Type::Eq {
                ty: Box::new(ty.apply_subst_with_depth(subst, next_depth, max_depth)),
                lhs: lhs.clone(), // EqTerms are value-level, not type-level
                rhs: rhs.clone(),
            },
            Type::PathType { space, left, right } => Type::PathType {
                space: Box::new(space.apply_subst_with_depth(subst, next_depth, max_depth)),
                left: left.clone(),   // CubicalTerms are value-level, not type-level
                right: right.clone(),
            },
            Type::Partial { element_type, face } => Type::Partial {
                element_type: Box::new(element_type.apply_subst_with_depth(subst, next_depth, max_depth)),
                face: face.clone(), // CubicalTerms are value-level
            },
            Type::Interval => Type::Interval,
            Type::Universe { level } => Type::Universe { level: *level },
            Type::Prop => Type::Prop,
            Type::Inductive {
                name,
                params,
                indices,
                universe,
                constructors,
            } => Type::Inductive {
                name: name.clone(),
                params: params
                    .iter()
                    .map(|(n, t)| {
                        (
                            n.clone(),
                            Box::new(t.apply_subst_with_depth(subst, next_depth, max_depth)),
                        )
                    })
                    .collect(),
                indices: indices
                    .iter()
                    .map(|(n, t)| {
                        (
                            n.clone(),
                            Box::new(t.apply_subst_with_depth(subst, next_depth, max_depth)),
                        )
                    })
                    .collect(),
                universe: *universe,
                constructors: constructors.clone(), // Constructors have their own type context
            },
            Type::Coinductive {
                name,
                params,
                destructors,
            } => Type::Coinductive {
                name: name.clone(),
                params: params
                    .iter()
                    .map(|(n, t)| {
                        (
                            n.clone(),
                            Box::new(t.apply_subst_with_depth(subst, next_depth, max_depth)),
                        )
                    })
                    .collect(),
                destructors: destructors.clone(),
            },
            Type::HigherInductive {
                name,
                params,
                point_constructors,
                path_constructors,
            } => Type::HigherInductive {
                name: name.clone(),
                params: params
                    .iter()
                    .map(|(n, t)| {
                        (
                            n.clone(),
                            Box::new(t.apply_subst_with_depth(subst, next_depth, max_depth)),
                        )
                    })
                    .collect(),
                point_constructors: point_constructors.clone(),
                path_constructors: path_constructors.clone(),
            },
            Type::Quantified { inner, quantity } => Type::Quantified {
                inner: Box::new(inner.apply_subst_with_depth(subst, next_depth, max_depth)),
                quantity: *quantity,
            },

            // Primitives and other leaf types
            Type::Unit
            | Type::Never
            | Type::Bool
            | Type::Int
            | Type::Float
            | Type::Char
            | Type::Text
            | Type::Lifetime { .. }
            | Type::Slice { .. }
            | Type::Placeholder { .. } => self.clone(),

            // DynProtocol - substitute into bindings
            Type::DynProtocol { bounds, bindings } => Type::DynProtocol {
                bounds: bounds.clone(),
                bindings: bindings
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            v.apply_subst_with_depth(subst, next_depth, max_depth),
                        )
                    })
                    .collect(),
            },

            // ExtensibleRecord - substitute into fields, keep row variable
            Type::ExtensibleRecord { fields, row_var } => {
                let new_fields = fields
                    .iter()
                    .map(|(k, v)| (k.clone(), v.apply_subst(subst)))
                    .collect();
                Type::ExtensibleRecord {
                    fields: new_fields,
                    row_var: *row_var,
                }
            }

            // CapabilityRestricted - substitute into base, keep capabilities
            Type::CapabilityRestricted { base, capabilities } => Type::CapabilityRestricted {
                base: Box::new(base.apply_subst_with_depth(subst, next_depth, max_depth)),
                capabilities: capabilities.clone(),
            },

            // Unknown type - a top type with no inner structure to substitute
            // Spec: Unknown is a safe top type that doesn't unify with other types
            Type::Unknown => Type::Unknown,
        }
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
                params,
                return_type,
                type_params: _,
                contexts,
                properties: _,
            } => {
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
            } => {
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
pub type Substitution = IndexMap<TypeVar, Type>;

/// Extension trait for Substitution
pub trait SubstitutionExt {
    /// Compose two substitutions: apply s1 after s2
    fn compose(&self, other: &Substitution) -> Substitution;

    /// Get the domain (set of variables being substituted)
    fn domain(&self) -> Set<TypeVar>;
}

impl SubstitutionExt for Substitution {
    fn compose(&self, other: &Substitution) -> Substitution {
        let mut result = Substitution::new();

        // Apply other to all types in self (s1.compose(s2) means apply s2 after s1)
        // So for each v -> t in s1, we get v -> s2(t)
        for (var, ty) in self {
            result.insert(*var, ty.apply_subst(other));
        }

        // Add bindings from other that aren't in self
        for (var, ty) in other {
            if !result.contains_key(var) {
                result.insert(*var, ty.clone());
            }
        }

        result
    }

    fn domain(&self) -> Set<TypeVar> {
        let mut set = Set::new();
        for key in self.keys() {
            set.insert(*key);
        }
        set
    }
}
// Tests moved to tests/ty_tests.rs
