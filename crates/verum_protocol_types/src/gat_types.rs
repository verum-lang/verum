//! Generic Associated Types (GATs) Type Definitions
//!
//! Generic Associated Types (GATs) extend associated types with their own type
//! parameters, enabling higher-kinded patterns like Monad, Functor, and lending
//! iterators. GATs use CBGR generation tracking instead of lifetime annotations.
//! This module contains type definitions for GATs (planned for future release).
//!
//! # Features
//!
//! - Type parameters on associated types
//! - Where clauses specific to GATs
//! - Kind tracking (regular, generic, higher-kinded)
//! - Variance annotations

use verum_ast::span::Span;
use verum_common::{List, Maybe, Text};

use crate::protocol_base::{ProtocolBound, Type};

// ==================== GAT Core Types ====================

/// Type parameter for a Generic Associated Type
///
/// Type parameter for a GAT, e.g., `T` in `type Wrapped<T>` within a Monad protocol.
/// Each parameter has optional bounds (e.g., `T: Clone + Debug`), optional default
/// type, and variance annotation (covariant, contravariant, or invariant).
///
/// Example:
/// ```verum
/// protocol Monad {
///     type Wrapped<T>  // GAT with one type parameter
///     fn pure<T>(value: T) -> Self.Wrapped<T>
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct GATTypeParam {
    /// Parameter name (e.g., "T" in `type Item<T>`)
    pub name: Text,

    /// Bounds on this parameter (e.g., `T: Clone + Debug`)
    pub bounds: List<ProtocolBound>,

    /// Default type for this parameter (if any)
    pub default: Maybe<Type>,

    /// Variance of this parameter (covariant, contravariant, invariant)
    pub variance: Variance,
}

/// Type variance for GAT parameters
///
/// Determines how type parameters can be substituted while maintaining subtyping.
///
/// Determines how type parameters can be substituted while maintaining subtyping.
/// Used for GAT constraints on associated types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variance {
    /// Covariant: If A <: B, then F<A> <: F<B>
    ///
    /// Example: Return types, immutable references
    Covariant,

    /// Contravariant: If A <: B, then F<B> <: F<A>
    ///
    /// Example: Function parameters
    Contravariant,

    /// Invariant: No subtyping allowed
    ///
    /// Example: Mutable references
    Invariant,
}

/// Where clause specific to a GAT (not the protocol itself)
///
/// Where clause that constrains a GAT's own type parameters (separate from
/// the protocol-level where clause). E.g., `type Item<T> where T: Clone + Debug`.
///
/// Example:
/// ```verum
/// protocol Container {
///     type Item<T> where T: Clone + Debug
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct GATWhereClause {
    /// Type parameter being constrained
    pub param: Text,

    /// Protocol bounds that must be satisfied
    pub constraints: List<ProtocolBound>,

    /// Source location
    pub span: Span,
}

/// Kind of associated type
///
/// Classification of associated types by their kind (type-level arity).
/// Regular: plain `type Item`, Generic: `type Item<T>` (GAT with parameters),
/// HigherKinded: `type F<_>` (type constructor, enables Functor/Monad patterns).
#[derive(Debug, Clone, PartialEq)]
pub enum AssociatedTypeKind {
    /// Regular associated type: `type Item`
    Regular,

    /// Generic associated type: `type Item<T>`
    ///
    /// Contains the number of type parameters
    Generic {
        /// Number of type parameters
        arity: usize,
    },

    /// Higher-kinded type: `type F<_>`
    ///
    /// Arity indicates number of type constructor parameters
    HigherKinded {
        /// Number of type constructor parameters
        arity: usize,
    },
}

/// Kind for higher-kinded types
///
/// Kind system for higher-kinded types. Star (*) is a concrete type,
/// Arrow (* -> *) is a type constructor (e.g., List, Maybe).
/// Kind::constructor(n) creates an n-arity type constructor kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// Regular type: `*`
    Star,

    /// Type constructor: `* -> *` (e.g., List, Maybe)
    Arrow {
        /// Input kind
        from: Box<Kind>,
        /// Output kind
        to: Box<Kind>,
    },
}

impl Kind {
    /// Create a type constructor kind with given arity
    ///
    /// # Example
    ///
    /// ```ignore
    /// // List<_> has kind * -> *
    /// let list_kind = Kind::constructor(1);
    ///
    /// // Map<_, _> has kind * -> * -> *
    /// let map_kind = Kind::constructor(2);
    /// ```
    pub fn constructor(arity: usize) -> Self {
        if arity == 0 {
            Kind::Star
        } else {
            let mut kind = Kind::Star;
            for _ in 0..arity {
                kind = Kind::Arrow {
                    from: Box::new(Kind::Star),
                    to: Box::new(kind),
                };
            }
            kind
        }
    }

    /// Get the arity of this kind (number of arguments)
    pub fn arity(&self) -> usize {
        match self {
            Kind::Star => 0,
            Kind::Arrow { to, .. } => 1 + to.arity(),
        }
    }
}

/// Extended AssociatedType with GAT support
///
/// This extends the basic AssociatedType with:
/// - Type parameters for GATs
/// - Per-GAT where clauses
/// - Kind tracking (regular, generic, higher-kinded)
///
/// Extended associated type with full GAT support: type parameters, per-GAT
/// where clauses, and kind tracking. Enables patterns like lending iterators,
/// Monad/Functor abstractions, and zero-copy streaming iteration.
#[derive(Debug, Clone)]
pub struct AssociatedTypeGAT {
    /// Type name
    pub name: Text,

    /// Type parameters (empty for non-GATs)
    pub type_params: List<GATTypeParam>,

    /// Protocol bounds on the associated type itself
    pub bounds: List<ProtocolBound>,

    /// Where clauses specific to this GAT
    pub where_clauses: List<GATWhereClause>,

    /// Default type (if any)
    pub default: Maybe<Type>,

    /// Kind of associated type
    pub kind: AssociatedTypeKind,

    /// Documentation
    pub doc: Maybe<Text>,

    /// Source location
    pub span: Span,
}

impl AssociatedTypeGAT {
    /// Create a simple (non-GAT) associated type
    pub fn simple(name: Text, bounds: List<ProtocolBound>) -> Self {
        Self {
            name,
            type_params: List::new(),
            bounds,
            where_clauses: List::new(),
            default: None,
            kind: AssociatedTypeKind::Regular,
            doc: None,
            span: Span::default(),
        }
    }

    /// Create a GAT with type parameters
    pub fn generic(
        name: Text,
        type_params: List<GATTypeParam>,
        bounds: List<ProtocolBound>,
        where_clauses: List<GATWhereClause>,
    ) -> Self {
        let arity = type_params.len();
        Self {
            name,
            type_params,
            bounds,
            where_clauses,
            default: None,
            kind: AssociatedTypeKind::Generic { arity },
            doc: None,
            span: Span::default(),
        }
    }

    /// Check if this is a GAT (has type parameters)
    pub fn is_gat(&self) -> bool {
        !self.type_params.is_empty()
    }

    /// Get the arity (number of type parameters)
    pub fn arity(&self) -> usize {
        self.type_params.len()
    }
}

// ==================== GAT Verification Types ====================

/// Error in GAT verification
///
/// These errors are detected during GAT well-formedness checking.
#[derive(Debug, Clone)]
pub enum GATError {
    /// Type parameter constraint not satisfied
    ConstraintViolation {
        /// Parameter name
        param: Text,
        /// Constraint that was violated
        constraint: Text,
        /// Counterexample (if available)
        counterexample: Maybe<Text>,
    },

    /// Circular dependency detected
    CircularDependency {
        /// Cycle of dependent types
        cycle: List<Text>,
    },

    /// Where clause not satisfiable
    UnsatisfiableWhereClause {
        /// Parameter name
        param: Text,
        /// Unsatisfiable clause
        clause: Text,
    },

    /// Variance violation
    VarianceViolation {
        /// Parameter name
        param: Text,
        /// Expected variance
        expected: Variance,
        /// Found variance
        found: Variance,
    },

    /// Arity mismatch
    ArityMismatch {
        /// GAT name
        gat_name: Text,
        /// Expected arity
        expected: usize,
        /// Found arity
        found: usize,
    },
}
