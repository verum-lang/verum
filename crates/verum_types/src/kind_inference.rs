//! Higher-Kinded Type Inference System
//!
//! Higher-kinded type (HKT) kind inference: infers kinds for type constructors
//! (e.g., List has kind Type -> Type, Map has kind Type -> Type -> Type).
//! Uses constraint-based kind inference with unification.
//!
//! This module implements automatic kind inference and checking for higher-kinded types.
//! Kinds describe the "type of a type":
//! - `*` (Type): Kind of concrete types (Int, Bool, List<Int>)
//! - `* -> *`: Kind of type constructors (List, Maybe, GenRef)
//! - `* -> * -> *`: Kind of binary type constructors (Map, Result)
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────┐
//! │  Kind System                        │
//! │  - Kind representation              │
//! │  - Kind substitution                │
//! │  - Kind unification                 │
//! └─────────────────────────────────────┘
//!           ↓
//! ┌─────────────────────────────────────┐
//! │  Kind Inference Engine              │
//! │  - Generate constraints             │
//! │  - Solve constraint system          │
//! │  - Infer type constructor kinds     │
//! └─────────────────────────────────────┘
//!           ↓
//! ┌─────────────────────────────────────┐
//! │  Kind Checking                      │
//! │  - Check kind correctness           │
//! │  - Validate protocol definitions    │
//! │  - Check type applications          │
//! └─────────────────────────────────────┘
//! ```
//!
//! # Performance Guarantees
//!
//! - Kind inference: <5ms for typical protocols
//! - Kind checking: <1ms per type application
//! - Constraint solving: <10ms for complex kinds
//! - Zero runtime overhead (compile-time only)
//!
//! # Example Usage
//!
//! ```ignore
//! use verum_types::kind_inference::{KindInferer, Kind};
//!
//! let mut inferer = KindInferer::new();
//!
//! // Infer kind of List type constructor
//! let list_kind = inferer.infer_kind(&Type::Named {
//!     path: Path::single(Ident::new("List", Span::default())),
//!     args: List::from(vec![Type::Var(TypeVar::new(0))]),
//! })?;
//!
//! assert_eq!(list_kind, Kind::unary_constructor()); // * -> *
//! ```

use std::fmt;
use verum_ast::span::Span;
use verum_common::well_known_types::WellKnownType as WKT;
use verum_common::{List, Map, Maybe, Set, Text};

use crate::protocol::{Protocol, ProtocolBound};
use crate::ty::{Type, TypeVar};
use crate::{Result, TypeError};

// ==================== Kind System ====================

/// Kind of a type (the "type of a type")
///
/// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .3 lines 410-437
///
/// Kinds form a hierarchy:
/// - `*` (Type): Concrete types like Int, Bool, List<Int>
/// - `* -> *`: Type constructors like List, Maybe, GenRef
/// - `* -> * -> *`: Binary type constructors like Map, Result
/// - `(* -> *) -> *`: Higher-order type constructors (rare)
///
/// # Examples
///
/// ```ignore
/// Int          : *
/// List         : * -> *
/// List<Int>    : *
/// Map          : * -> * -> *
/// Map<Text>    : * -> *
/// Map<Text, Int> : *
/// Functor.F    : * -> * (higher-kinded type parameter)
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Base kind: `*`
    ///
    /// The kind of concrete types that have values.
    /// Examples: Int, Bool, Text, List<Int>, Map<Text, Int>
    Type,

    /// Arrow kind: `k1 -> k2`
    ///
    /// The kind of type constructors that take a type of kind k1
    /// and produce a type of kind k2.
    /// Examples:
    /// - List: * -> *
    /// - Map: * -> (* -> *)
    Arrow(Box<Kind>, Box<Kind>),

    /// Kind variable for inference
    ///
    /// During kind inference, we introduce fresh kind variables
    /// and solve constraints to determine their concrete kinds.
    KindVar(u32),

    /// Constraint kind: the kind of protocols used as generic type bounds.
    /// Example: `type Ord is protocol { ... }` has kind Constraint.
    /// Can be used in `where T: Ord` but NOT in `using [Ord]`.
    Constraint,

    /// Injectable kind: the kind of context types used for dependency injection.
    /// Example: `context Logger { ... }` has kind Injectable.
    /// Can be used in `using [Logger]` but NOT in `where T: Logger`.
    Injectable,

    /// Dual kind: protocols declared with `context protocol` modifier.
    /// Example: `context protocol Serializable { ... }` has kind Constraint & Injectable.
    /// Can be used BOTH as generic bounds AND as injectable contexts.
    /// Subtype of both Constraint and Injectable.
    ConstraintAndInjectable,
}

impl Kind {
    /// Create the base kind `*`
    pub fn type_kind() -> Self {
        Kind::Type
    }

    /// Create a unary constructor kind `* -> *`
    ///
    /// Examples: List, Maybe, GenRef
    pub fn unary_constructor() -> Self {
        Kind::Arrow(Box::new(Kind::Type), Box::new(Kind::Type))
    }

    /// Create a binary constructor kind `* -> * -> *`
    ///
    /// Examples: Map, Result
    pub fn binary_constructor() -> Self {
        Kind::Arrow(
            Box::new(Kind::Type),
            Box::new(Kind::Arrow(Box::new(Kind::Type), Box::new(Kind::Type))),
        )
    }

    /// Create an arrow kind from parameter and result kinds
    pub fn arrow(param: Kind, result: Kind) -> Self {
        Kind::Arrow(Box::new(param), Box::new(result))
    }

    /// Check if this kind is usable as a generic type bound (constraint position).
    pub fn is_constraint(&self) -> bool {
        matches!(self, Kind::Constraint | Kind::ConstraintAndInjectable)
    }

    /// Check if this kind is usable as an injectable context (using [...] position).
    pub fn is_injectable(&self) -> bool {
        matches!(self, Kind::Injectable | Kind::ConstraintAndInjectable)
    }

    /// Check if this kind is a protocol kind (Constraint, Injectable, or both).
    pub fn is_protocol_kind(&self) -> bool {
        matches!(self, Kind::Constraint | Kind::Injectable | Kind::ConstraintAndInjectable)
    }

    /// Apply kind substitution
    pub fn apply(&self, subst: &KindSubstitution) -> Kind {
        match self {
            Kind::Type | Kind::Constraint | Kind::Injectable | Kind::ConstraintAndInjectable => self.clone(),
            Kind::Arrow(param, result) => {
                Kind::Arrow(Box::new(param.apply(subst)), Box::new(result.apply(subst)))
            }
            Kind::KindVar(v) => subst.get(v).cloned().unwrap_or_else(|| self.clone()),
        }
    }

    /// Collect free kind variables
    pub fn free_vars(&self) -> Set<u32> {
        match self {
            Kind::Type | Kind::Constraint | Kind::Injectable | Kind::ConstraintAndInjectable => Set::new(),
            Kind::Arrow(param, result) => {
                let mut vars = param.free_vars();
                for var in result.free_vars() {
                    vars.insert(var);
                }
                vars
            }
            Kind::KindVar(v) => {
                let mut set = Set::new();
                set.insert(*v);
                set
            }
        }
    }

    /// Get the arity of this kind (number of arrows)
    ///
    /// - `*` has arity 0
    /// - `* -> *` has arity 1
    /// - `* -> * -> *` has arity 2
    pub fn arity(&self) -> usize {
        match self {
            Kind::Type | Kind::Constraint | Kind::Injectable | Kind::ConstraintAndInjectable => 0,
            Kind::KindVar(_) => 0,
            Kind::Arrow(_, result) => 1 + result.arity(),
        }
    }

    /// Pretty print kind
    pub fn display(&self) -> Text {
        self.to_string().into()
    }

    /// Check if this is a concrete kind (no kind variables)
    pub fn is_concrete(&self) -> bool {
        self.free_vars().is_empty()
    }

    /// Get the result kind after applying N type arguments
    ///
    /// For example:
    /// - `(* -> * -> *).apply_n(1) = * -> *`
    /// - `(* -> * -> *).apply_n(2) = *`
    pub fn apply_n(&self, n: usize) -> Maybe<Kind> {
        if n == 0 {
            return Maybe::Some(self.clone());
        }

        match self {
            Kind::Arrow(_, result) => result.apply_n(n - 1),
            _ => Maybe::None, // Not enough arrows
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Kind::Type => write!(f, "*"),
            Kind::Constraint => write!(f, "Constraint"),
            Kind::Injectable => write!(f, "Injectable"),
            Kind::ConstraintAndInjectable => write!(f, "Constraint & Injectable"),
            Kind::KindVar(v) => write!(f, "?k{}", v),
            Kind::Arrow(param, result) => {
                match **param {
                    Kind::Arrow(_, _) => write!(f, "({}) -> {}", param, result),
                    _ => write!(f, "{} -> {}", param, result),
                }
            }
        }
    }
}

// ==================== Kind Substitution ====================

/// Substitution for kind variables
///
/// Maps kind variables to their inferred kinds.
#[derive(Debug, Clone, Default)]
pub struct KindSubstitution {
    mapping: Map<u32, Kind>,
}

impl KindSubstitution {
    /// Create an empty substitution
    pub fn new() -> Self {
        Self {
            mapping: Map::new(),
        }
    }

    /// Insert a binding from kind variable to kind
    pub fn insert(&mut self, var: u32, kind: Kind) {
        self.mapping.insert(var, kind);
    }

    /// Get the kind for a variable
    pub fn get(&self, var: &u32) -> Maybe<&Kind> {
        match self.mapping.get(var) {
            Some(k) => Maybe::Some(k),
            None => Maybe::None,
        }
    }

    /// Compose two substitutions
    ///
    /// The result applies s1 first, then s2
    pub fn compose(&self, other: &KindSubstitution) -> KindSubstitution {
        let mut result = KindSubstitution::new();

        // Apply other to all bindings in self
        for (var, kind) in &self.mapping {
            result.insert(*var, kind.apply(other));
        }

        // Add bindings from other that aren't in self
        for (var, kind) in &other.mapping {
            if !self.mapping.contains_key(var) {
                result.insert(*var, kind.clone());
            }
        }

        result
    }
}

// ==================== Kind Constraints ====================

/// A constraint between two kinds that must be satisfied
#[derive(Debug, Clone, PartialEq)]
pub enum KindConstraint {
    /// Two kinds must be equal
    Equal {
        left: Kind,
        right: Kind,
        span: Span,
        reason: Text,
    },

    /// A type constructor must have a specific arity
    Arity {
        name: Text,
        expected: usize,
        span: Span,
    },
}

impl KindConstraint {
    /// Create an equality constraint
    pub fn equal(left: Kind, right: Kind, span: Span, reason: impl Into<Text>) -> Self {
        KindConstraint::Equal {
            left,
            right,
            span,
            reason: reason.into(),
        }
    }

    /// Create an arity constraint
    pub fn arity(name: impl Into<Text>, expected: usize, span: Span) -> Self {
        KindConstraint::Arity {
            name: name.into(),
            expected,
            span,
        }
    }

    /// Apply substitution to this constraint
    pub fn apply(&self, subst: &KindSubstitution) -> KindConstraint {
        match self {
            KindConstraint::Equal {
                left,
                right,
                span,
                reason,
            } => KindConstraint::Equal {
                left: left.apply(subst),
                right: right.apply(subst),
                span: *span,
                reason: reason.clone(),
            },
            KindConstraint::Arity { .. } => self.clone(),
        }
    }
}

// ==================== Kind Errors ====================

/// Kind inference and checking errors
#[derive(Debug, Clone)]
pub enum KindError {
    /// Kind mismatch
    Mismatch {
        expected: Kind,
        found: Kind,
        span: Span,
        reason: Text,
    },

    /// Arity mismatch
    ArityMismatch {
        constructor: Text,
        expected: usize,
        found: usize,
        span: Span,
    },

    /// Infinite kind (occurs check failed)
    InfiniteKind { var: u32, kind: Kind },

    /// Undefined type constructor
    UndefinedTypeConstructor { name: Text, span: Span },

    /// Kind variable not resolved
    UnresolvedKindVar { var: u32, span: Span },

    /// Higher-kinded type used as concrete type
    HigherKindedAsType { name: Text, kind: Kind, span: Span },
}

impl KindError {
    /// Provide helpful suggestions for this error
    pub fn with_suggestion(&self) -> Text {
        match self {
            KindError::Mismatch {
                expected,
                found,
                span: _,
                reason,
            } => format!(
                "Kind mismatch: expected {}, found {}\n\
                 Reason: {}\n\
                 Help: Ensure type applications match the constructor's kind",
                expected, found, reason
            )
            .into(),

            KindError::ArityMismatch {
                constructor,
                expected,
                found,
                span: _,
            } => {
                if found < expected {
                    format!(
                        "Type constructor '{}' expects {} type argument(s), but only {} provided\n\
                         Help: Add {} more type argument(s)",
                        constructor,
                        expected,
                        found,
                        expected - found
                    )
                    .into()
                } else {
                    format!(
                        "Type constructor '{}' expects {} type argument(s), but {} provided\n\
                         Help: Remove {} type argument(s)",
                        constructor,
                        expected,
                        found,
                        found - expected
                    )
                    .into()
                }
            }

            KindError::InfiniteKind { var, kind } => format!(
                "Infinite kind: ?k{} = {}\n\
                 Help: This usually indicates a circular type definition",
                var, kind
            )
            .into(),

            KindError::UndefinedTypeConstructor { name, span: _ } => format!(
                "Undefined type constructor: {}\n\
                 Help: Check if the type is imported or defined",
                name
            )
            .into(),

            KindError::UnresolvedKindVar { var, span: _ } => format!(
                "Could not infer kind for ?k{}\n\
                 Help: Add explicit kind annotations",
                var
            )
            .into(),

            KindError::HigherKindedAsType {
                name,
                kind,
                span: _,
            } => format!(
                "Type constructor '{}' has kind {}, but is used as a concrete type\n\
                 Help: Apply type arguments to make it a concrete type (kind *)",
                name, kind
            )
            .into(),
        }
    }

    /// Convert to TypeError for integration with type system
    pub fn to_type_error(&self) -> TypeError {
        TypeError::Other(self.with_suggestion())
    }
}

// ==================== Kind Inference Engine ====================

/// Kind inference engine
///
/// Implements constraint-based kind inference using unification.
/// Similar to Hindley-Milner type inference, but for kinds.
pub struct KindInferer {
    /// Next fresh kind variable ID
    next_var: u32,

    /// Accumulated kind constraints
    constraints: List<KindConstraint>,

    /// Current kind substitution
    substitution: KindSubstitution,

    /// Known kinds for type constructors
    /// Maps type constructor names to their kinds
    known_kinds: Map<Text, Kind>,
}

impl KindInferer {
    /// Create a new kind inferer with stdlib type constructors pre-registered.
    ///
    /// Production code uses `new_minimal()` — type constructors are registered
    /// dynamically during resolve_type_definition from parsed .vr files.
    /// This method pre-registers stdlib types for tests, benchmarks, and bootstrapping.
    pub fn new() -> Self {
        let mut inferer = Self::new_minimal();
        inferer.register_stdlib_types();
        inferer
    }

    /// Create a minimal kind inferer without stdlib types.
    ///
    /// STDLIB-AGNOSTIC: This constructor creates an empty kind inferer.
    /// Types should be registered via `register_type_constructor()` or
    /// `register_stdlib_types()` based on stdlib metadata.
    ///
    /// Stdlib-agnostic type system: type checker operates without hardcoded knowledge of stdlib types, stdlib types registered from parsed .vr files
    pub fn new_minimal() -> Self {
        Self {
            next_var: 0,
            constraints: List::new(),
            substitution: KindSubstitution::new(),
            known_kinds: Map::new(),
        }
    }

    /// Register standard library type constructors.
    ///
    /// **LEGACY**: Contains hardcoded type names for bootstrapping.
    /// In a fully stdlib-agnostic system, this information would come from
    /// stdlib metadata (e.g., stdlib.vbca or type annotations).
    ///
    /// Types are registered based on their arity:
    /// - Unary (* -> *): Single type parameter (List, Maybe, Set, etc.)
    /// - Binary (* -> * -> *): Two type parameters (Map, Result)
    /// - Reference types: All have single inner type parameter
    pub fn register_stdlib_types(&mut self) {
        // NOTE: These are bootstrapping registrations that allow the compiler
        // to work before stdlib types are fully parsed and analyzed.
        // A fully stdlib-agnostic system would derive this from type definitions.

        // Unary constructors (* -> *)
        for name in &[WKT::List.as_str(), WKT::Maybe.as_str(), WKT::Set.as_str(), "GenRef", WKT::Shared.as_str(), WKT::Heap.as_str()] {
            self.register_type_constructor(*name, Kind::unary_constructor());
        }

        // Binary constructors (* -> * -> *)
        for name in &[WKT::Map.as_str(), WKT::Result.as_str()] {
            self.register_type_constructor(*name, Kind::binary_constructor());
        }

        // Reference types (* -> *)
        for name in &[
            "Ref",
            "RefMut",
            "CheckedRef",
            "CheckedRefMut",
            "UnsafeRef",
            "UnsafeRefMut",
        ] {
            self.register_type_constructor(*name, Kind::unary_constructor());
        }
    }

    /// Register a type constructor with its kind
    pub fn register_type_constructor(&mut self, name: impl Into<Text>, kind: Kind) {
        self.known_kinds.insert(name.into(), kind);
    }

    /// Generate a fresh kind variable
    pub fn fresh_kind_var(&mut self) -> Kind {
        let var = self.next_var;
        self.next_var += 1;
        Kind::KindVar(var)
    }

    /// Infer the kind of a type
    ///
    /// This is the main entry point for kind inference.
    /// It analyzes a type and returns its kind.
    pub fn infer_kind(&mut self, ty: &Type) -> Result<Kind> {
        match ty {
            // Concrete types have kind *
            Type::Unit
            | Type::Never
            | Type::Bool
            | Type::Int
            | Type::Float
            | Type::Char
            | Type::Text => Ok(Kind::Type),

            // Type variables have kind *
            Type::Var(_) => Ok(Kind::Type),

            // Named types: look up constructor and apply arguments
            Type::Named { path, args } => {
                let name: Text = path
                    .segments
                    .last()
                    .map(|seg| match seg {
                        verum_ast::ty::PathSegment::Name(ident) => ident.name.clone(),
                        verum_ast::ty::PathSegment::SelfValue => "Self".into(),
                        verum_ast::ty::PathSegment::Super => "super".into(),
                        verum_ast::ty::PathSegment::Cog => "cog".into(),
                        verum_ast::ty::PathSegment::Relative => ".".into(),
                    })
                    .unwrap_or_else(|| "unknown".into());

                // Look up the kind of the type constructor
                let constructor_kind = self.known_kinds.get(&name).cloned().unwrap_or_else(|| {
                    // Unknown type constructor - create fresh kind variable
                    // and constrain it based on usage
                    if args.is_empty() {
                        Kind::Type
                    } else {
                        // Create kind: * -> * -> ... -> *
                        self.create_constructor_kind(args.len())
                    }
                });

                // Check arity matches
                let expected_arity = constructor_kind.arity();
                let actual_arity = args.len();

                if expected_arity != actual_arity {
                    return Err(KindError::ArityMismatch {
                        constructor: name,
                        expected: expected_arity,
                        found: actual_arity,
                        span: Span::default(),
                    }
                    .to_type_error());
                }

                // Infer kinds of arguments
                for arg in args {
                    let arg_kind = self.infer_kind(arg)?;
                    // All type arguments must have kind *
                    self.add_constraint(KindConstraint::equal(
                        arg_kind,
                        Kind::Type,
                        Span::default(),
                        "Type argument must have kind *",
                    ));
                }

                // Apply the constructor to get result kind
                if let Maybe::Some(result_kind) = constructor_kind.apply_n(actual_arity) {
                    Ok(result_kind)
                } else {
                    Err(KindError::ArityMismatch {
                        constructor: name,
                        expected: expected_arity,
                        found: actual_arity,
                        span: Span::default(),
                    }
                    .to_type_error())
                }
            }

            // Generic types (stdlib like List<T>, Maybe<T>) have kind *
            Type::Generic { name, args } => {
                // Look up constructor kind
                let constructor_kind = self.known_kinds.get(name).cloned().unwrap_or_else(|| {
                    if args.is_empty() {
                        Kind::Type
                    } else {
                        self.create_constructor_kind(args.len())
                    }
                });

                // Infer kinds of arguments
                for arg in args {
                    let arg_kind = self.infer_kind(arg)?;
                    self.add_constraint(KindConstraint::equal(
                        arg_kind,
                        Kind::Type,
                        Span::default(),
                        "Type argument must have kind *",
                    ));
                }

                // Apply constructor
                if let Maybe::Some(result_kind) = constructor_kind.apply_n(args.len()) {
                    Ok(result_kind)
                } else {
                    Ok(Kind::Type) // Fallback
                }
            }

            // Function types have kind *
            Type::Function { .. } => Ok(Kind::Type),

            // Compound types have kind *
            Type::Tuple(_)
            | Type::Array { .. }
            | Type::Slice { .. }
            | Type::Record(_)
            | Type::Variant(_) => Ok(Kind::Type),

            // Reference types
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. }
            | Type::Ownership { inner, .. }
            | Type::Pointer { inner, .. }
            | Type::VolatilePointer { inner, .. } => {
                let inner_kind = self.infer_kind(inner)?;
                self.add_constraint(KindConstraint::equal(
                    inner_kind,
                    Kind::Type,
                    Span::default(),
                    "Reference inner type must have kind *",
                ));
                Ok(Kind::Type)
            }

            // Refined types have the same kind as their base
            Type::Refined { base, .. } => self.infer_kind(base),

            // Existential and universal types have kind *
            Type::Exists { body, .. } | Type::Forall { body, .. } => self.infer_kind(body),

            // Meta parameters have kind * (they're compile-time values, not type constructors)
            Type::Meta { .. } => Ok(Kind::Type),

            // Future and Generator types have kind *
            Type::Future { .. } | Type::Generator { .. } => Ok(Kind::Type),

            // Tensor types have kind *
            Type::Tensor { .. } => Ok(Kind::Type),

            // Lifetime types have kind Lifetime
            Type::Lifetime { .. } => Ok(Kind::Type),

            // Generic references have kind *
            Type::GenRef { inner, .. } => {
                let inner_kind = self.infer_kind(inner)?;
                self.add_constraint(KindConstraint::equal(
                    inner_kind,
                    Kind::Type,
                    Span::default(),
                    "GenRef inner type must have kind *",
                ));
                Ok(Kind::Type)
            }

            // Type constructors and applications
            Type::TypeConstructor { .. } => Ok(Kind::Type),
            Type::TypeApp { constructor, args } => {
                // Infer kind of constructor
                let _constructor_kind = self.infer_kind(constructor)?;
                // Infer kind of all arguments
                for arg in args {
                    let _arg_kind = self.infer_kind(arg)?;
                }
                Ok(Kind::Type)
            }

            // Dependent Types (Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — )

            // Pi types (dependent functions) have kind *
            // (x: A) -> B(x) : *
            Type::Pi {
                param_type,
                return_type,
                ..
            } => {
                let param_kind = self.infer_kind(param_type)?;
                self.add_constraint(KindConstraint::equal(
                    param_kind,
                    Kind::Type,
                    Span::default(),
                    "Pi parameter type must have kind *",
                ));
                let return_kind = self.infer_kind(return_type)?;
                self.add_constraint(KindConstraint::equal(
                    return_kind,
                    Kind::Type,
                    Span::default(),
                    "Pi return type must have kind *",
                ));
                Ok(Kind::Type)
            }

            // Sigma types (dependent pairs) have kind *
            // (x: A, B(x)) : *
            Type::Sigma {
                fst_type, snd_type, ..
            } => {
                let fst_kind = self.infer_kind(fst_type)?;
                self.add_constraint(KindConstraint::equal(
                    fst_kind,
                    Kind::Type,
                    Span::default(),
                    "Sigma first type must have kind *",
                ));
                let snd_kind = self.infer_kind(snd_type)?;
                self.add_constraint(KindConstraint::equal(
                    snd_kind,
                    Kind::Type,
                    Span::default(),
                    "Sigma second type must have kind *",
                ));
                Ok(Kind::Type)
            }

            // Equality types have kind *
            // Eq<A, x, y> : *
            Type::Eq { ty, .. } => {
                let inner_kind = self.infer_kind(ty)?;
                self.add_constraint(KindConstraint::equal(
                    inner_kind,
                    Kind::Type,
                    Span::default(),
                    "Equality type must have kind *",
                ));
                Ok(Kind::Type)
            }

            // Cubical path types have kind *
            // PathType<A, x, y> : *  (like Eq, it's a concrete proposition type)
            Type::PathType { space, .. } => {
                let space_kind = self.infer_kind(space)?;
                self.add_constraint(KindConstraint::equal(
                    space_kind,
                    Kind::Type,
                    Span::default(),
                    "Path type space must have kind *",
                ));
                Ok(Kind::Type)
            }

            // Partial element types have kind *
            // Partial<A>(φ) : *  (a concrete type for partial elements of A on face φ)
            Type::Partial { element_type, .. } => {
                let elem_kind = self.infer_kind(element_type)?;
                self.add_constraint(KindConstraint::equal(
                    elem_kind,
                    Kind::Type,
                    Span::default(),
                    "Partial element type must have kind *",
                ));
                Ok(Kind::Type)
            }

            // Interval type I has kind *
            // It is the abstract interval used as a primitive in cubical type theory
            Type::Interval => Ok(Kind::Type),

            // Universe types have kind based on their level.
            // Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter
            //
            // In dependent type theory: Type_n : Type_{n+1}
            // However, for the HKT kind system (which is separate from the universe hierarchy),
            // all universe types have kind * because:
            // 1. They are proper types that can be used in type positions
            // 2. The universe level tracking is handled by the type system, not the kind system
            // 3. Kind checking for HKT focuses on type constructor arity, not universe levels
            //
            // Example:
            //   Type      : * (can be used as a type parameter)
            //   Type_1    : * (same)
            //   List<Type>: valid because Type : *
            //
            // The universe hierarchy (Type_n : Type_{n+1}) is enforced separately
            // during type checking, not kind checking.
            Type::Universe { .. } => Ok(Kind::Type),

            // Prop universe has kind *
            Type::Prop => Ok(Kind::Type),

            // Inductive types have kind based on indices
            // If no indices: kind *
            // With indices: kind * -> * -> ... -> *
            Type::Inductive {
                params, indices, ..
            } => {
                // Infer kinds of all params
                for (_, param_ty) in params {
                    let param_kind = self.infer_kind(param_ty)?;
                    self.add_constraint(KindConstraint::equal(
                        param_kind,
                        Kind::Type,
                        Span::default(),
                        "Inductive parameter must have kind *",
                    ));
                }
                // Infer kinds of all indices
                for (_, index_ty) in indices {
                    let index_kind = self.infer_kind(index_ty)?;
                    self.add_constraint(KindConstraint::equal(
                        index_kind,
                        Kind::Type,
                        Span::default(),
                        "Inductive index must have kind *",
                    ));
                }
                // Indexed inductive types are type constructors
                if indices.is_empty() {
                    Ok(Kind::Type)
                } else {
                    Ok(self.create_constructor_kind(indices.len()))
                }
            }

            // Coinductive types have kind *
            Type::Coinductive { params, .. } => {
                for (_, param_ty) in params {
                    let param_kind = self.infer_kind(param_ty)?;
                    self.add_constraint(KindConstraint::equal(
                        param_kind,
                        Kind::Type,
                        Span::default(),
                        "Coinductive parameter must have kind *",
                    ));
                }
                Ok(Kind::Type)
            }

            // Higher Inductive Types have kind *
            Type::HigherInductive { params, .. } => {
                for (_, param_ty) in params {
                    let param_kind = self.infer_kind(param_ty)?;
                    self.add_constraint(KindConstraint::equal(
                        param_kind,
                        Kind::Type,
                        Span::default(),
                        "HIT parameter must have kind *",
                    ));
                }
                Ok(Kind::Type)
            }

            // Quantified types (linear/affine) have kind *
            Type::Quantified { inner, .. } => {
                let inner_kind = self.infer_kind(inner)?;
                self.add_constraint(KindConstraint::equal(
                    inner_kind,
                    Kind::Type,
                    Span::default(),
                    "Quantified inner type must have kind *",
                ));
                Ok(Kind::Type)
            }

            // Placeholder types - during resolution, assume kind *
            // These should be resolved before kind checking completes
            Type::Placeholder { .. } => Ok(Kind::Type),

            // Extensible record types have kind *
            // Row polymorphism doesn't change the kind of record types
            Type::ExtensibleRecord { .. } => Ok(Kind::Type),

            // Capability-restricted types have same kind as base type
            Type::CapabilityRestricted { base, .. } => self.infer_kind(base),

            // Unknown type has kind * (Type)
            // It's a proper type that can be used in type positions
            Type::Unknown => Ok(Kind::Type),

            // DynProtocol (dyn Display + Debug) has kind *
            // Dynamic protocol objects are proper types
            Type::DynProtocol { .. } => Ok(Kind::Type),
        }
    }

    /// Create a constructor kind for N parameters: * -> * -> ... -> *
    fn create_constructor_kind(&mut self, arity: usize) -> Kind {
        let mut kind = Kind::Type;
        for _ in 0..arity {
            kind = Kind::arrow(Kind::Type, kind);
        }
        kind
    }

    /// Add a kind constraint
    pub fn add_constraint(&mut self, constraint: KindConstraint) {
        self.constraints.push(constraint);
    }

    /// Generate kind constraints from a protocol definition
    ///
    /// This analyzes a protocol and generates constraints for:
    /// - Associated type kinds
    /// - Method signature kinds
    /// - Default implementation kinds
    pub fn generate_protocol_constraints(
        &mut self,
        protocol: &Protocol,
    ) -> Result<List<KindConstraint>> {
        let mut constraints = List::new();

        // Generate constraints for each associated type
        for (name, assoc_ty) in &protocol.associated_types {
            // GATs have specific kind based on their arity
            let arity = assoc_ty.type_params.len();
            if arity > 0 {
                let kind = self.create_constructor_kind(arity);
                constraints.push(KindConstraint::arity(name.clone(), arity, Span::default()));
                self.register_type_constructor(name.clone(), kind);
            } else {
                // Regular associated types have kind *
                self.register_type_constructor(name.clone(), Kind::Type);
            }
        }

        // Generate constraints for method signatures
        for (_, method) in &protocol.methods {
            let method_kind = self.infer_kind(&method.ty)?;
            constraints.push(KindConstraint::equal(
                method_kind,
                Kind::Type,
                Span::default(),
                "Method type must have kind *",
            ));
        }

        Ok(constraints)
    }

    /// Solve the constraint system
    ///
    /// Uses Robinson's unification algorithm to solve kind constraints.
    /// Returns the final substitution or an error if constraints are unsatisfiable.
    pub fn solve(&mut self) -> Result<KindSubstitution> {
        // Process each constraint
        for constraint in self.constraints.clone() {
            match constraint {
                KindConstraint::Equal {
                    left,
                    right,
                    span,
                    reason,
                } => {
                    let left = left.apply(&self.substitution);
                    let right = right.apply(&self.substitution);

                    let unifier = self.unify(&left, &right, span, reason)?;
                    self.substitution = self.substitution.compose(&unifier);
                }

                KindConstraint::Arity { .. } => {
                    // Arity constraints are checked during inference
                    // No need to solve them here
                }
            }
        }

        Ok(self.substitution.clone())
    }

    /// Unify two kinds
    ///
    /// Robinson's unification algorithm adapted for kinds.
    /// Returns a substitution that makes the two kinds equal.
    pub fn unify(
        &mut self,
        k1: &Kind,
        k2: &Kind,
        span: Span,
        reason: Text,
    ) -> Result<KindSubstitution> {
        match (k1, k2) {
            // Same concrete kinds
            (Kind::Type, Kind::Type) => Ok(KindSubstitution::new()),

            // Same kind variables
            (Kind::KindVar(v1), Kind::KindVar(v2)) if v1 == v2 => Ok(KindSubstitution::new()),

            // Bind kind variable
            (Kind::KindVar(v), k) | (k, Kind::KindVar(v)) => {
                self.bind_kind_var(*v, k.clone(), span)
            }

            // Unify arrows
            (Kind::Arrow(a1, r1), Kind::Arrow(a2, r2)) => {
                let subst1 = self.unify(a1, a2, span, reason.clone())?;
                let r1_subst = r1.apply(&subst1);
                let r2_subst = r2.apply(&subst1);
                let subst2 = self.unify(&r1_subst, &r2_subst, span, reason)?;
                Ok(subst1.compose(&subst2))
            }

            // Mismatch
            _ => Err(KindError::Mismatch {
                expected: k2.clone(),
                found: k1.clone(),
                span,
                reason,
            }
            .to_type_error()),
        }
    }

    /// Bind a kind variable to a kind
    ///
    /// Performs occurs check to prevent infinite kinds.
    pub fn bind_kind_var(&mut self, var: u32, kind: Kind, span: Span) -> Result<KindSubstitution> {
        // Occurs check: ensure var doesn't appear in kind
        if kind.free_vars().contains(&var) {
            return Err(KindError::InfiniteKind { var, kind }.to_type_error());
        }

        let mut subst = KindSubstitution::new();
        subst.insert(var, kind);
        Ok(subst)
    }

    /// Check that a type has the expected kind
    ///
    /// This is the main entry point for kind checking.
    /// It infers the kind of a type and checks it matches the expected kind.
    pub fn check_kind(&mut self, ty: &Type, expected_kind: &Kind) -> Result<()> {
        let inferred_kind = self.infer_kind(ty)?;

        let unifier = self.unify(
            &inferred_kind,
            expected_kind,
            Span::default(),
            "Kind checking".into(),
        )?;

        self.substitution = self.substitution.compose(&unifier);

        Ok(())
    }

    /// Check protocol definition has correct kinds
    ///
    /// Validates:
    /// - All associated types have valid kinds
    /// - All method signatures respect kinds
    /// - Default implementations satisfy kinds
    pub fn check_protocol_kinds(&mut self, protocol: &Protocol) -> Result<()> {
        // Generate constraints
        let constraints = self.generate_protocol_constraints(protocol)?;

        // Add to constraint list
        for constraint in constraints {
            self.add_constraint(constraint);
        }

        // Solve constraints
        self.solve()?;

        Ok(())
    }

    /// Get the final kind for a type after solving constraints
    pub fn get_kind(&mut self, ty: &Type) -> Maybe<Kind> {
        match self.infer_kind(ty) {
            Ok(kind) => Maybe::Some(kind.apply(&self.substitution)),
            Err(_) => Maybe::None,
        }
    }

    /// Check if a type is well-kinded
    ///
    /// A type is well-kinded if:
    /// - All type constructors are applied correctly
    /// - All type applications result in kind *
    pub fn is_well_kinded(&mut self, ty: &Type) -> bool {
        match self.infer_kind(ty) {
            Ok(kind) => {
                let kind = kind.apply(&self.substitution);
                kind == Kind::Type || kind.is_concrete()
            }
            Err(_) => false,
        }
    }
}

impl Default for KindInferer {
    fn default() -> Self {
        Self::new_minimal()
    }
}

// ==================== HKT Instantiation and Application Checking ====================

/// Result of HKT parameter instantiation
#[derive(Debug, Clone)]
pub struct HKTInstantiationResult {
    /// The instantiated kind after applying the type constructor
    pub resulting_kind: Kind,
    /// Whether any protocol bounds need to be checked
    pub protocol_bounds_satisfied: bool,
    /// The substitution applied during instantiation
    pub substitution: KindSubstitution,
}

impl KindInferer {
    /// Check kind compatibility when applying a type constructor to arguments.
    ///
    /// Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — Higher-kinded types
    ///
    /// When applying `F<Int>` where `F: * -> *`, this verifies:
    /// 1. F has the expected constructor kind (* -> *)
    /// 2. Int has kind * (the expected argument kind)
    /// 3. The resulting application F<Int> has kind *
    ///
    /// # Arguments
    ///
    /// * `constructor` - The type constructor being applied (e.g., F, List, Map)
    /// * `args` - The type arguments being applied
    /// * `span` - Source location for error reporting
    ///
    /// # Returns
    ///
    /// * `Ok(Kind)` - The resulting kind after application
    /// * `Err(TypeError)` - If kind mismatch or arity error
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // F<Int> where F: * -> *
    /// let result_kind = inferer.check_type_application_kind(
    ///     &Type::TypeConstructor { name: "F".into(), arity: 1, kind: Kind::unary_constructor() },
    ///     &[Type::Int],
    ///     Span::default()
    /// )?;
    /// assert_eq!(result_kind, Kind::Type);
    ///
    /// // Map<Text, Int> where Map: * -> * -> *
    /// let result_kind = inferer.check_type_application_kind(
    ///     &Type::TypeConstructor { name: "Map".into(), arity: 2, kind: Kind::binary_constructor() },
    ///     &[Type::Text, Type::Int],
    ///     Span::default()
    /// )?;
    /// assert_eq!(result_kind, Kind::Type);
    /// ```
    pub fn check_type_application_kind(
        &mut self,
        constructor: &Type,
        args: &[Type],
        span: Span,
    ) -> Result<Kind> {
        // Infer the kind of the constructor
        let constructor_kind = self.infer_constructor_kind(constructor)?;

        // Verify the constructor is actually a type constructor (not kind *)
        if constructor_kind == Kind::Type && !args.is_empty() {
            let constructor_name = self.type_name(constructor);
            return Err(KindError::HigherKindedAsType {
                name: constructor_name,
                kind: constructor_kind,
                span,
            }
            .to_type_error());
        }

        // Check arity matches
        let expected_arity = constructor_kind.arity();
        let actual_arity = args.len();

        if expected_arity != actual_arity {
            let constructor_name = self.type_name(constructor);
            return Err(KindError::ArityMismatch {
                constructor: constructor_name,
                expected: expected_arity,
                found: actual_arity,
                span,
            }
            .to_type_error());
        }

        // Verify each argument has kind *
        for (i, arg) in args.iter().enumerate() {
            let arg_kind = self.infer_kind(arg)?;
            if arg_kind != Kind::Type {
                return Err(KindError::Mismatch {
                    expected: Kind::Type,
                    found: arg_kind.clone(),
                    span,
                    reason: format!(
                        "Type argument {} to '{}' must have kind *, but has kind {}",
                        i + 1,
                        self.type_name(constructor),
                        arg_kind
                    )
                    .into(),
                }
                .to_type_error());
            }
        }

        // Compute the result kind after applying all arguments
        match constructor_kind.apply_n(actual_arity) {
            Maybe::Some(result_kind) => Ok(result_kind),
            Maybe::None => {
                let constructor_name = self.type_name(constructor);
                Err(KindError::ArityMismatch {
                    constructor: constructor_name,
                    expected: expected_arity,
                    found: actual_arity,
                    span,
                }
                .to_type_error())
            }
        }
    }

    /// Infer the kind of a type constructor.
    ///
    /// For Type::TypeConstructor, returns the declared kind.
    /// For Type::Named/Generic, looks up the kind from known_kinds.
    /// For Type::Var, returns a fresh kind variable.
    fn infer_constructor_kind(&mut self, constructor: &Type) -> Result<Kind> {
        match constructor {
            Type::TypeConstructor { kind, .. } => Ok(kind.clone()),

            Type::Named { path, args: _ } => {
                let name = self.path_to_name(path);
                if let Some(kind) = self.known_kinds.get(&name) {
                    Ok(kind.clone())
                } else {
                    // Unknown constructor - return fresh kind variable
                    Ok(self.fresh_kind_var())
                }
            }

            Type::Generic { name, args: _ } => {
                if let Some(kind) = self.known_kinds.get(name) {
                    Ok(kind.clone())
                } else {
                    Ok(self.fresh_kind_var())
                }
            }

            Type::Var(_) => {
                // Type variable used as constructor - return fresh kind variable
                Ok(self.fresh_kind_var())
            }

            // For type applications, infer the kind of the constructor part
            Type::TypeApp { constructor, args } => {
                let ctor_kind = self.infer_constructor_kind(constructor)?;
                // Apply the arguments and return the resulting kind
                match ctor_kind.apply_n(args.len()) {
                    Maybe::Some(result) => Ok(result),
                    Maybe::None => Ok(Kind::Type), // Fallback
                }
            }

            _ => {
                // Other types have kind *
                Ok(Kind::Type)
            }
        }
    }

    /// Instantiate an HKT parameter with a concrete type constructor.
    ///
    /// Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — HKT parameter instantiation
    ///
    /// When calling `fn foo<F<_>: Functor>(x: F<Int>)` with `foo::<List>(...)`,
    /// this verifies:
    /// 1. `List` has kind `* -> *` (matches F's expected kind)
    /// 2. `List` implements `Functor` (satisfies protocol bound)
    ///
    /// # Arguments
    ///
    /// * `hkt_param_name` - Name of the HKT parameter (e.g., "F")
    /// * `expected_kind` - The expected kind for the parameter (e.g., * -> *)
    /// * `concrete_constructor` - The concrete type constructor being substituted (e.g., List)
    /// * `protocol_bounds` - Protocol bounds that must be satisfied (e.g., Functor)
    /// * `span` - Source location for error reporting
    ///
    /// # Returns
    ///
    /// * `Ok(HKTInstantiationResult)` - Successful instantiation with result info
    /// * `Err(TypeError)` - If kind mismatch or protocol not implemented
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Instantiate F<_> with List where F<_>: Functor
    /// let result = inferer.instantiate_hkt_param(
    ///     "F",
    ///     &Kind::unary_constructor(),
    ///     &Type::TypeConstructor { name: "List".into(), arity: 1, kind: Kind::unary_constructor() },
    ///     &[ProtocolBound::simple("Functor".into())],
    ///     Span::default(),
    ///     |ty, bound| protocol_checker.implements(ty, bound)
    /// )?;
    /// ```
    pub fn instantiate_hkt_param<F>(
        &mut self,
        hkt_param_name: &str,
        expected_kind: &Kind,
        concrete_constructor: &Type,
        protocol_bounds: &[ProtocolBound],
        span: Span,
        check_protocol: F,
    ) -> Result<HKTInstantiationResult>
    where
        F: Fn(&Type, &ProtocolBound) -> bool,
    {
        // Step 1: Infer the kind of the concrete constructor
        let concrete_kind = self.infer_constructor_kind(concrete_constructor)?;

        // Step 2: Unify the concrete kind with the expected kind
        let unifier = self.unify(
            &concrete_kind,
            expected_kind,
            span,
            format!(
                "HKT parameter '{}' expects kind {}, but '{}' has kind {}",
                hkt_param_name,
                expected_kind,
                self.type_name(concrete_constructor),
                concrete_kind
            )
            .into(),
        )?;

        // Apply the substitution
        self.substitution = self.substitution.compose(&unifier);

        // Step 3: Check protocol bounds
        let mut all_bounds_satisfied = true;
        for bound in protocol_bounds {
            if !check_protocol(concrete_constructor, bound) {
                all_bounds_satisfied = false;
                // We don't fail immediately - let the caller handle this
                // This allows collecting all violations before reporting
            }
        }

        Ok(HKTInstantiationResult {
            resulting_kind: expected_kind.apply(&self.substitution),
            protocol_bounds_satisfied: all_bounds_satisfied,
            substitution: self.substitution.clone(),
        })
    }

    /// Check if a type constructor implements a protocol.
    ///
    /// Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — Protocol checking for type constructors
    ///
    /// For HKT bounds like `F<_>: Functor + Monad`, this checks if the type
    /// constructor (e.g., List, Maybe) implements the required protocol.
    ///
    /// Note: This method validates the kind compatibility. Actual protocol
    /// implementation checking is delegated to ProtocolChecker.
    ///
    /// # Arguments
    ///
    /// * `constructor` - The type constructor to check
    /// * `protocol_name` - Name of the protocol
    /// * `expected_hkt_kind` - The expected kind for the protocol's type parameter
    /// * `span` - Source location for error reporting
    ///
    /// # Returns
    ///
    /// * `Ok(())` - If the constructor has compatible kind
    /// * `Err(TypeError)` - If kind is incompatible
    pub fn check_constructor_protocol_compatibility(
        &mut self,
        constructor: &Type,
        protocol_name: &str,
        expected_hkt_kind: &Kind,
        span: Span,
    ) -> Result<()> {
        let constructor_kind = self.infer_constructor_kind(constructor)?;

        // Verify kinds are compatible
        let _ = self.unify(
            &constructor_kind,
            expected_hkt_kind,
            span,
            format!(
                "Type constructor '{}' has kind {} but protocol '{}' requires kind {}",
                self.type_name(constructor),
                constructor_kind,
                protocol_name,
                expected_hkt_kind
            )
            .into(),
        )?;

        Ok(())
    }

    /// Helper to extract a type's name for error messages
    fn type_name(&self, ty: &Type) -> Text {
        match ty {
            Type::TypeConstructor { name, .. } => name.clone(),
            Type::Named { path, .. } => self.path_to_name(path),
            Type::Generic { name, .. } => name.clone(),
            Type::Var(var) => format!("?{}", var.id()).into(),
            Type::TypeApp { constructor, .. } => self.type_name(constructor),
            _ => format!("{}", ty).into(),
        }
    }

    /// Convert a Path to a name string
    fn path_to_name(&self, path: &verum_ast::ty::Path) -> Text {
        path.segments
            .last()
            .map(|seg| match seg {
                verum_ast::ty::PathSegment::Name(ident) => ident.name.clone(),
                verum_ast::ty::PathSegment::SelfValue => "Self".into(),
                verum_ast::ty::PathSegment::Super => "super".into(),
                verum_ast::ty::PathSegment::Cog => "cog".into(),
                verum_ast::ty::PathSegment::Relative => ".".into(),
            })
            .unwrap_or_else(|| "unknown".into())
    }

    /// Get the kind for a named type constructor.
    ///
    /// Returns Some(kind) if the constructor is known, None otherwise.
    pub fn get_constructor_kind(&self, name: &str) -> Maybe<&Kind> {
        match self.known_kinds.get(&Text::from(name)) {
            Some(k) => Maybe::Some(k),
            None => Maybe::None,
        }
    }

    /// Check if a type is a valid type constructor (has kind * -> ... -> *)
    pub fn is_type_constructor(&mut self, ty: &Type) -> bool {
        match self.infer_constructor_kind(ty) {
            Ok(kind) => kind.arity() > 0,
            Err(_) => false,
        }
    }

    /// Get the arity of a type constructor.
    ///
    /// Returns Some(arity) for type constructors, None for concrete types.
    pub fn get_constructor_arity(&mut self, ty: &Type) -> Maybe<usize> {
        match self.infer_constructor_kind(ty) {
            Ok(kind) => {
                let arity = kind.arity();
                if arity > 0 {
                    Maybe::Some(arity)
                } else {
                    Maybe::None
                }
            }
            Err(_) => Maybe::None,
        }
    }
}

// ==================== TypeApp Helper Methods ====================

impl Type {
    /// Create a type application from a constructor and arguments.
    ///
    /// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .3 - Type applications
    ///
    /// # Arguments
    ///
    /// * `constructor` - The type constructor (must have kind * -> ... -> *)
    /// * `args` - Type arguments to apply
    ///
    /// # Example
    ///
    /// ```ignore
    /// let list_int = Type::make_type_app(
    ///     Type::type_constructor("List", 1, Kind::unary_constructor()),
    ///     vec![Type::Int]
    /// );
    /// ```
    pub fn make_type_app(constructor: Type, args: List<Type>) -> Self {
        Type::TypeApp {
            constructor: Box::new(constructor),
            args,
        }
    }

    /// Check if this type is a TypeApp
    pub fn is_type_app(&self) -> bool {
        matches!(self, Type::TypeApp { .. })
    }

    /// Check if this type is a TypeConstructor
    pub fn is_type_constructor_variant(&self) -> bool {
        matches!(self, Type::TypeConstructor { .. })
    }

    /// Get the constructor from a TypeApp, if this is one
    pub fn get_type_app_constructor(&self) -> Maybe<&Type> {
        match self {
            Type::TypeApp { constructor, .. } => Maybe::Some(constructor),
            _ => Maybe::None,
        }
    }

    /// Get the arguments from a TypeApp, if this is one
    pub fn get_type_app_args(&self) -> Maybe<&List<Type>> {
        match self {
            Type::TypeApp { args, .. } => Maybe::Some(args),
            _ => Maybe::None,
        }
    }

    /// Get the name from a TypeConstructor, if this is one
    pub fn get_constructor_name(&self) -> Maybe<&Text> {
        match self {
            Type::TypeConstructor { name, .. } => Maybe::Some(name),
            _ => Maybe::None,
        }
    }

    /// Get the arity from a TypeConstructor, if this is one
    pub fn get_constructor_arity(&self) -> Maybe<usize> {
        match self {
            Type::TypeConstructor { arity, .. } => Maybe::Some(*arity),
            _ => Maybe::None,
        }
    }

    /// Get the kind from a TypeConstructor, if this is one
    pub fn get_constructor_kind(&self) -> Maybe<&crate::advanced_protocols::Kind> {
        match self {
            Type::TypeConstructor { kind, .. } => Maybe::Some(kind),
            _ => Maybe::None,
        }
    }

    /// Decompose a TypeApp into constructor and arguments
    pub fn decompose_type_app(&self) -> Maybe<(&Type, &List<Type>)> {
        match self {
            Type::TypeApp { constructor, args } => Maybe::Some((constructor, args)),
            _ => Maybe::None,
        }
    }

    /// Apply additional type arguments to a type.
    ///
    /// If the type is already a TypeApp, extends the arguments.
    /// If it's a TypeConstructor or other type, creates a new TypeApp.
    pub fn apply_type_args(self, additional_args: List<Type>) -> Self {
        if additional_args.is_empty() {
            return self;
        }

        match self {
            Type::TypeApp {
                constructor,
                mut args,
            } => {
                args.extend(additional_args);
                Type::TypeApp { constructor, args }
            }
            _ => Type::TypeApp {
                constructor: Box::new(self),
                args: additional_args,
            },
        }
    }

    /// Check if this type is fully applied (all type arguments provided).
    ///
    /// A TypeConstructor with arity N is fully applied when it has N arguments.
    /// A concrete type (kind *) is always considered fully applied.
    pub fn is_fully_applied(&self) -> bool {
        match self {
            Type::TypeConstructor { arity, .. } => *arity == 0,
            Type::TypeApp { constructor, args } => {
                if let Type::TypeConstructor { arity, .. } = constructor.as_ref() {
                    args.len() >= *arity
                } else {
                    true // Conservative: assume applied if we can't determine
                }
            }
            _ => true, // Concrete types are fully applied
        }
    }

    /// Get the number of remaining type parameters needed.
    ///
    /// Returns 0 for fully applied types or concrete types.
    pub fn remaining_arity(&self) -> usize {
        match self {
            Type::TypeConstructor { arity, .. } => *arity,
            Type::TypeApp { constructor, args } => {
                if let Type::TypeConstructor { arity, .. } = constructor.as_ref() {
                    arity.saturating_sub(args.len())
                } else {
                    0
                }
            }
            _ => 0,
        }
    }
}

// ==================== Integration with Type System ====================

/// Extension trait for TypeChecker to add kind inference
///
/// This will be integrated into the main TypeChecker in infer.rs
pub trait KindInference {
    /// Get the kind inferer
    fn kind_inferer(&mut self) -> &mut KindInferer;

    /// Check kind of type matches expected kind
    fn check_kind(&mut self, ty: &Type, expected_kind: &Kind) -> Result<()>;

    /// Infer kind of type
    fn infer_kind(&mut self, ty: &Type) -> Result<Kind>;

    /// Check protocol has correct kinds
    fn check_protocol_kinds(&mut self, protocol: &Protocol) -> Result<()>;
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kind_display() {
        assert_eq!(Kind::Type.to_string(), "*");
        assert_eq!(Kind::unary_constructor().to_string(), "* -> *");
        assert_eq!(Kind::binary_constructor().to_string(), "* -> * -> *");
        assert_eq!(Kind::KindVar(0).to_string(), "?k0");
    }

    #[test]
    fn test_kind_arity() {
        assert_eq!(Kind::Type.arity(), 0);
        assert_eq!(Kind::unary_constructor().arity(), 1);
        assert_eq!(Kind::binary_constructor().arity(), 2);
    }

    #[test]
    fn test_kind_apply_n() {
        let kind = Kind::binary_constructor(); // * -> * -> *

        assert_eq!(kind.apply_n(0), Maybe::Some(kind.clone()));
        assert_eq!(kind.apply_n(1), Maybe::Some(Kind::unary_constructor()));
        assert_eq!(kind.apply_n(2), Maybe::Some(Kind::Type));
        assert_eq!(kind.apply_n(3), Maybe::None);
    }

    #[test]
    fn test_kind_substitution() {
        let mut subst = KindSubstitution::new();
        subst.insert(0, Kind::Type);
        subst.insert(1, Kind::unary_constructor());

        let kind = Kind::KindVar(0);
        assert_eq!(kind.apply(&subst), Kind::Type);

        let kind = Kind::KindVar(1);
        assert_eq!(kind.apply(&subst), Kind::unary_constructor());

        let kind = Kind::KindVar(2);
        assert_eq!(kind.apply(&subst), Kind::KindVar(2));
    }

    #[test]
    fn test_kind_unification() {
        let mut inferer = KindInferer::new();

        // Unify * with *
        let result = inferer.unify(&Kind::Type, &Kind::Type, Span::default(), "test".into());
        assert!(result.is_ok());

        // Unify ?k0 with *
        let result = inferer.unify(
            &Kind::KindVar(0),
            &Kind::Type,
            Span::default(),
            "test".into(),
        );
        assert!(result.is_ok());

        // Unify * -> * with * -> *
        let result = inferer.unify(
            &Kind::unary_constructor(),
            &Kind::unary_constructor(),
            Span::default(),
            "test".into(),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_kind_inference_primitives() {
        let mut inferer = KindInferer::new();

        assert_eq!(inferer.infer_kind(&Type::Int).unwrap(), Kind::Type);
        assert_eq!(inferer.infer_kind(&Type::Bool).unwrap(), Kind::Type);
        assert_eq!(inferer.infer_kind(&Type::Text).unwrap(), Kind::Type);
    }

    #[test]
    fn test_occurs_check() {
        let mut inferer = KindInferer::new();

        // Create infinite kind: ?k0 = ?k0 -> *
        let infinite_kind = Kind::Arrow(Box::new(Kind::KindVar(0)), Box::new(Kind::Type));

        let result = inferer.bind_kind_var(0, infinite_kind, Span::default());
        assert!(result.is_err());
    }
}
