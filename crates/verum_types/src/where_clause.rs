//! Where Clause Disambiguation
//!
//! Where clause disambiguation: value-level refinements vs type-level constraints (where type T: Protocol)
//!
//! Starting with Verum v1.2, the `where` keyword supports four distinct uses
//! with explicit prefixes for LL(1) parsing and improved clarity:
//!
//! 1. **Type Constraints**: `where type T: Ord` - Generic protocol bounds
//! 2. **Meta Constraints**: `where meta N > 0` - Compile-time meta constraints
//! 3. **Value Refinements**: `where value it > 0` - Runtime value constraints
//! 4. **Postconditions**: `where ensures result >= 0` - Return value guarantees
//!
//! # Examples
//!
//! ```verum
//! // Type constraint
//! fn sort<T>(list: List<T>) where type T: Ord { ... }
//!
//! // Meta constraint
//! fn zeros<N>() -> [Int; N] where meta N > 0 { ... }
//!
//! // Value refinement
//! type Positive is Int where value it > 0;
//!
//! // Postcondition
//! fn abs(x: Int) -> Int where ensures result >= 0 { ... }
//! ```

use crate::protocol::ProtocolBound;
#[allow(unused_imports)]
use crate::ty::Type;
use verum_ast::expr::Expr;
use verum_ast::span::Span;
use verum_common::{List, Text};

/// Type of where clause
#[derive(Debug, Clone, PartialEq)]
pub enum WhereClauseKind {
    /// Type constraint: `where type T: Ord`
    ///
    /// Used for generic type parameter protocol bounds.
    /// The type parameter must implement the specified protocols.
    TypeConstraint {
        /// Type parameter name (e.g., "T")
        param: Text,
        /// Required protocols (e.g., ["Ord", "Clone"])
        bounds: List<ProtocolBound>,
        span: Span,
    },

    /// Meta constraint: `where meta N > 0`
    ///
    /// Used for compile-time meta parameter constraints.
    /// The meta parameter must satisfy the compile-time predicate.
    MetaConstraint {
        /// Meta parameter name (e.g., "N")
        param: Text,
        /// Compile-time boolean expression
        predicate: Expr,
        span: Span,
    },

    /// Value refinement: `where value it > 0`
    ///
    /// Used for runtime value constraints in refinement types.
    /// The special identifier `it` refers to the refined value.
    ValueRefinement {
        /// Runtime boolean expression
        /// Use `it` to refer to the value being refined
        predicate: Expr,
        span: Span,
    },

    /// Postcondition: `where ensures result >= 0`
    ///
    /// Used for function return value guarantees.
    /// The special identifier `result` refers to the return value.
    Postcondition {
        /// Boolean expression over `result`
        predicate: Expr,
        span: Span,
    },
}

impl WhereClauseKind {
    /// Get a human-readable description of this where clause kind
    pub fn description(&self) -> &'static str {
        match self {
            WhereClauseKind::TypeConstraint { .. } => "type constraint",
            WhereClauseKind::MetaConstraint { .. } => "meta constraint",
            WhereClauseKind::ValueRefinement { .. } => "value refinement",
            WhereClauseKind::Postcondition { .. } => "postcondition",
        }
    }

    /// Get the span of this where clause
    pub fn span(&self) -> Span {
        match self {
            WhereClauseKind::TypeConstraint { span, .. }
            | WhereClauseKind::MetaConstraint { span, .. }
            | WhereClauseKind::ValueRefinement { span, .. }
            | WhereClauseKind::Postcondition { span, .. } => *span,
        }
    }

    /// Check if this is a type constraint
    pub fn is_type_constraint(&self) -> bool {
        matches!(self, WhereClauseKind::TypeConstraint { .. })
    }

    /// Check if this is a meta constraint
    pub fn is_meta_constraint(&self) -> bool {
        matches!(self, WhereClauseKind::MetaConstraint { .. })
    }

    /// Check if this is a value refinement
    pub fn is_value_refinement(&self) -> bool {
        matches!(self, WhereClauseKind::ValueRefinement { .. })
    }

    /// Check if this is a postcondition
    pub fn is_postcondition(&self) -> bool {
        matches!(self, WhereClauseKind::Postcondition { .. })
    }
}

/// Where clause with explicit prefix for disambiguation
#[derive(Debug, Clone, PartialEq)]
pub struct DisambiguatedWhereClause {
    /// The kind of where clause
    pub kind: WhereClauseKind,
    /// Source location
    pub span: Span,
}

impl DisambiguatedWhereClause {
    /// Create a new type constraint where clause
    pub fn type_constraint(
        param: impl Into<Text>,
        bounds: List<ProtocolBound>,
        span: Span,
    ) -> Self {
        Self {
            kind: WhereClauseKind::TypeConstraint {
                param: param.into(),
                bounds,
                span,
            },
            span,
        }
    }

    /// Create a new meta constraint where clause
    pub fn meta_constraint(param: impl Into<Text>, predicate: Expr, span: Span) -> Self {
        Self {
            kind: WhereClauseKind::MetaConstraint {
                param: param.into(),
                predicate,
                span,
            },
            span,
        }
    }

    /// Create a new value refinement where clause
    pub fn value_refinement(predicate: Expr, span: Span) -> Self {
        Self {
            kind: WhereClauseKind::ValueRefinement { predicate, span },
            span,
        }
    }

    /// Create a new postcondition where clause
    pub fn postcondition(predicate: Expr, span: Span) -> Self {
        Self {
            kind: WhereClauseKind::Postcondition { predicate, span },
            span,
        }
    }

    /// Validate that this where clause is used in the correct context
    ///
    /// - Type constraints: Only on generic functions/types
    /// - Meta constraints: Only on meta parameters
    /// - Value refinements: Only on refinement types
    /// - Postconditions: Only on functions
    pub fn validate_context(&self, context: WhereClauseContext) -> Result<(), Text> {
        match (&self.kind, context) {
            (WhereClauseKind::TypeConstraint { .. }, WhereClauseContext::GenericFunction)
            | (WhereClauseKind::TypeConstraint { .. }, WhereClauseContext::GenericType) => Ok(()),

            (WhereClauseKind::MetaConstraint { .. }, WhereClauseContext::GenericFunction)
            | (WhereClauseKind::MetaConstraint { .. }, WhereClauseContext::GenericType) => Ok(()),

            (WhereClauseKind::ValueRefinement { .. }, WhereClauseContext::RefinementType) => Ok(()),

            (WhereClauseKind::Postcondition { .. }, WhereClauseContext::Function) => Ok(()),

            _ => {
                let context_str = match context {
                    WhereClauseContext::RefinementType => "refinement type",
                    WhereClauseContext::Function => "function",
                    WhereClauseContext::GenericFunction => "generic function",
                    WhereClauseContext::GenericType => "generic type",
                };
                Err(format!(
                    "{} cannot be used in {} context",
                    self.kind.description(),
                    context_str
                )
                .into())
            }
        }
    }
}

/// Context where a where clause appears
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhereClauseContext {
    /// Generic function with type parameters
    GenericFunction,
    /// Generic type declaration
    GenericType,
    /// Refinement type definition
    RefinementType,
    /// Regular function
    Function,
}

// Tests moved to tests/where_clause_tests.rs per project testing guidelines.
