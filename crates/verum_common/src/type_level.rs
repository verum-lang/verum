//! Unified Type-Level Computation Traits
//!
//! Provides common traits and types for type-level computation across
//! different backends (AST-based and SMT-backed).
//!
//! This module enables code sharing between:
//! - `verum_types/src/type_level_computation.rs` (AST-based)
//! - `verum_smt/src/type_level_computation.rs` (Z3-backed)
//!
//! ## Design
//!
//! The type-level computation system is split into two tiers:
//!
//! 1. **TypeLevelComputation**: Basic evaluation and simplification (AST-only)
//! 2. **SmtCapableComputation**: Full SMT-backed constraint verification
//!
//! This allows choosing the appropriate backend based on requirements:
//! - Use AST backend for fast evaluation of simple expressions
//! - Use SMT backend for complex constraint verification
//!
//! ## Usage
//!
//! ```ignore
//! use verum_common::type_level::{TypeLevelComputation, BackendCapabilities};
//!
//! fn evaluate<C: TypeLevelComputation>(ctx: &mut C, expr: &C::Expr) {
//!     if ctx.capabilities().supports_const_eval {
//!         let result = ctx.eval_to_const(expr)?;
//!         // ...
//!     }
//! }
//! ```
//!
//! Supports dependent type computation: type-level functions, refinement predicates,
//! and SMT-backed constraint verification for compile-time type evaluation.

use std::fmt;

use crate::{Maybe, Text};

/// Unified error type for type-level computation
///
/// Combines error variants from both AST and SMT backends.
#[derive(Debug, Clone)]
pub enum TypeLevelError {
    /// Type error during computation
    TypeError {
        expected: Text,
        actual: Text,
    },

    /// Unbound type variable
    UnboundVariable {
        name: Text,
    },

    /// Function application error
    ApplicationError {
        message: Text,
    },

    /// Generic computation failure
    ComputationFailed {
        message: Text,
    },

    /// Meta parameter error
    MetaParameterError {
        message: Text,
    },

    /// Pattern match error
    MatchError {
        message: Text,
    },

    /// Universe level error
    UniverseError {
        message: Text,
    },

    /// Maximum evaluation depth exceeded
    MaxDepthExceeded(usize),

    /// Arity mismatch in function application
    ArityMismatch {
        expected: usize,
        got: usize,
    },

    /// Invalid type function
    InvalidTypeFunction(Text),

    /// Non-constant argument where constant required
    NonConstantArgument(Text),

    /// Not a type expression
    NotAType,

    /// SMT solver timeout
    SmtTimeout {
        timeout_ms: u64,
    },

    /// SMT solver error
    SmtError {
        message: Text,
    },

    /// Backend not supported
    UnsupportedOperation {
        operation: Text,
    },

    /// Generic other error
    Other(Text),
}

impl fmt::Display for TypeLevelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TypeError { expected, actual } => {
                write!(f, "type error: expected {}, found {}", expected, actual)
            }
            Self::UnboundVariable { name } => {
                write!(f, "unbound type variable: {}", name)
            }
            Self::ApplicationError { message } => {
                write!(f, "function application error: {}", message)
            }
            Self::ComputationFailed { message } => {
                write!(f, "type-level computation failed: {}", message)
            }
            Self::MetaParameterError { message } => {
                write!(f, "meta parameter error: {}", message)
            }
            Self::MatchError { message } => {
                write!(f, "type-level match error: {}", message)
            }
            Self::UniverseError { message } => {
                write!(f, "universe level error: {}", message)
            }
            Self::MaxDepthExceeded(depth) => {
                write!(f, "maximum evaluation depth exceeded: {}", depth)
            }
            Self::ArityMismatch { expected, got } => {
                write!(f, "arity mismatch: expected {} arguments, got {}", expected, got)
            }
            Self::InvalidTypeFunction(name) => {
                write!(f, "invalid type function: {}", name)
            }
            Self::NonConstantArgument(msg) => {
                write!(f, "non-constant argument: {}", msg)
            }
            Self::NotAType => {
                write!(f, "cannot evaluate non-type expression as type")
            }
            Self::SmtTimeout { timeout_ms } => {
                write!(f, "SMT solver timeout after {}ms", timeout_ms)
            }
            Self::SmtError { message } => {
                write!(f, "SMT solver error: {}", message)
            }
            Self::UnsupportedOperation { operation } => {
                write!(f, "operation not supported by backend: {}", operation)
            }
            Self::Other(msg) => {
                write!(f, "{}", msg)
            }
        }
    }
}

impl std::error::Error for TypeLevelError {}

impl TypeLevelError {
    /// Create a type error
    pub fn type_error(expected: impl Into<Text>, actual: impl Into<Text>) -> Self {
        Self::TypeError {
            expected: expected.into(),
            actual: actual.into(),
        }
    }

    /// Create an unbound variable error
    pub fn unbound_variable(name: impl Into<Text>) -> Self {
        Self::UnboundVariable { name: name.into() }
    }

    /// Create a computation failed error
    pub fn computation_failed(message: impl Into<Text>) -> Self {
        Self::ComputationFailed {
            message: message.into(),
        }
    }

    /// Create an arity mismatch error
    pub fn arity_mismatch(expected: usize, got: usize) -> Self {
        Self::ArityMismatch { expected, got }
    }

    /// Create an unsupported operation error
    pub fn unsupported(operation: impl Into<Text>) -> Self {
        Self::UnsupportedOperation {
            operation: operation.into(),
        }
    }
}

/// Result type for type-level computation
pub type TypeLevelResult<T> = std::result::Result<T, TypeLevelError>;

/// Capabilities of a type-level computation backend
///
/// Allows code to query what features are available before attempting
/// operations that may not be supported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BackendCapabilities {
    /// Can evaluate expressions to constant values
    pub supports_const_eval: bool,
    /// Can simplify expressions
    pub supports_simplification: bool,
    /// Can normalize types
    pub supports_type_normalization: bool,
    /// Can verify refinement predicates
    pub supports_refinement_verification: bool,
    /// Has SMT solver integration
    pub supports_smt: bool,
    /// Can handle dependent types
    pub supports_dependent_types: bool,
    /// Can handle higher-kinded types
    pub supports_higher_kinded_types: bool,
}

impl BackendCapabilities {
    /// Create capabilities for AST-only backend
    pub fn ast_only() -> Self {
        Self {
            supports_const_eval: true,
            supports_simplification: true,
            supports_type_normalization: true,
            supports_refinement_verification: false,
            supports_smt: false,
            supports_dependent_types: true,
            supports_higher_kinded_types: false,
        }
    }

    /// Create capabilities for SMT-backed backend
    pub fn smt_backed() -> Self {
        Self {
            supports_const_eval: true,
            supports_simplification: true,
            supports_type_normalization: true,
            supports_refinement_verification: true,
            supports_smt: true,
            supports_dependent_types: true,
            supports_higher_kinded_types: true,
        }
    }

    /// Check if SMT verification is available
    pub fn can_verify(&self) -> bool {
        self.supports_smt && self.supports_refinement_verification
    }
}

/// Result of SMT constraint verification
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationResult {
    /// Constraint is valid (always true)
    Valid,
    /// Constraint is invalid (always false)
    Invalid {
        /// Counter-example if available
        counterexample: Maybe<Text>,
    },
    /// Constraint validity is unknown (timeout or undecidable)
    Unknown {
        reason: Text,
    },
    /// Constraint is satisfiable (can be true)
    Satisfiable {
        /// Witness/model if available
        witness: Maybe<Text>,
    },
    /// Constraint is unsatisfiable (cannot be true)
    Unsatisfiable,
}

impl VerificationResult {
    /// Check if verification succeeded (valid or satisfiable)
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Valid | Self::Satisfiable { .. })
    }

    /// Check if verification failed (invalid or unsatisfiable)
    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Invalid { .. } | Self::Unsatisfiable)
    }

    /// Check if result is definitive (not unknown)
    pub fn is_definitive(&self) -> bool {
        !matches!(self, Self::Unknown { .. })
    }
}

/// Core trait for type-level computation
///
/// Provides basic evaluation and simplification capabilities.
/// Implemented by both AST and SMT backends.
pub trait TypeLevelComputation {
    /// The type representation (e.g., verum_types::Type)
    type Type: Clone;
    /// The expression representation (e.g., verum_ast::Expr)
    type Expr: Clone;
    /// The constant value representation (e.g., ConstValue)
    type Value: Clone;

    /// Get backend capabilities
    fn capabilities(&self) -> BackendCapabilities;

    /// Evaluate an expression to a constant value
    ///
    /// Returns `None` if the expression cannot be evaluated at compile-time.
    fn eval_to_const(&self, expr: &Self::Expr) -> TypeLevelResult<Maybe<Self::Value>>;

    /// Evaluate an expression as a type
    ///
    /// Used for type-level functions that return types.
    fn eval_as_type(&mut self, expr: &Self::Expr) -> TypeLevelResult<Self::Type>;

    /// Simplify an expression
    ///
    /// Performs algebraic simplifications and constant folding.
    fn simplify_expr(&self, expr: &Self::Expr) -> TypeLevelResult<Self::Expr>;

    /// Normalize a type
    ///
    /// Reduces type-level computations to normal form.
    fn normalize_type(&mut self, ty: &Self::Type) -> TypeLevelResult<Self::Type>;

    /// Check if two expressions are equivalent
    fn expr_equal(&self, lhs: &Self::Expr, rhs: &Self::Expr) -> TypeLevelResult<bool>;

    /// Check if two types are equivalent
    fn type_equal(&self, lhs: &Self::Type, rhs: &Self::Type) -> TypeLevelResult<bool>;
}

/// Extended trait for SMT-capable computation backends
///
/// Provides constraint verification and satisfiability checking
/// using an SMT solver.
pub trait SmtCapableComputation: TypeLevelComputation {
    /// Verify that a constraint is valid (always true)
    ///
    /// Uses SMT solver to check if the negation is unsatisfiable.
    fn verify_constraint(
        &self,
        constraint: &Self::Expr,
        timeout_ms: u64,
    ) -> TypeLevelResult<VerificationResult>;

    /// Check if a constraint is satisfiable (can be true)
    fn check_satisfiability(
        &self,
        constraint: &Self::Expr,
        timeout_ms: u64,
    ) -> TypeLevelResult<VerificationResult>;

    /// Verify a refinement predicate
    ///
    /// Checks that a refined type's predicate is satisfiable.
    fn verify_refinement(
        &mut self,
        base_type: &Self::Type,
        predicate: &Self::Expr,
        timeout_ms: u64,
    ) -> TypeLevelResult<VerificationResult>;

    /// Check subtype relationship with refinements
    ///
    /// Verifies that `sub` is a subtype of `sup`, including
    /// checking refinement predicates.
    fn check_subtype(
        &mut self,
        sub: &Self::Type,
        sup: &Self::Type,
        timeout_ms: u64,
    ) -> TypeLevelResult<bool>;
}

/// Reduction strategy for type-level computation
///
/// Controls how type-level expressions are evaluated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReductionStrategy {
    /// Call-by-value: evaluate arguments before substitution
    #[default]
    CallByValue,
    /// Call-by-name: substitute arguments before evaluation (lazy)
    CallByName,
    /// Normal form: reduce under binders (most complete)
    NormalForm,
    /// Weak head normal form: reduce only to outermost constructor
    WeakHeadNormalForm,
}

/// Configuration for type-level evaluator
#[derive(Debug, Clone)]
pub struct TypeLevelConfig {
    /// Maximum evaluation depth (prevents infinite recursion)
    pub max_depth: usize,
    /// Reduction strategy to use
    pub reduction_strategy: ReductionStrategy,
    /// Enable caching of computed types
    pub enable_cache: bool,
    /// SMT solver timeout in milliseconds (0 = no timeout)
    pub smt_timeout_ms: u64,
}

impl Default for TypeLevelConfig {
    fn default() -> Self {
        Self {
            max_depth: 100,
            reduction_strategy: ReductionStrategy::default(),
            enable_cache: true,
            smt_timeout_ms: 5000,
        }
    }
}

impl TypeLevelConfig {
    /// Create config for strict evaluation (call-by-value)
    pub fn strict() -> Self {
        Self {
            reduction_strategy: ReductionStrategy::CallByValue,
            ..Default::default()
        }
    }

    /// Create config for lazy evaluation (call-by-name)
    pub fn lazy() -> Self {
        Self {
            reduction_strategy: ReductionStrategy::CallByName,
            ..Default::default()
        }
    }

    /// Create config with custom max depth
    pub fn with_max_depth(max_depth: usize) -> Self {
        Self {
            max_depth,
            ..Default::default()
        }
    }

    /// Create config with custom SMT timeout
    pub fn with_smt_timeout(timeout_ms: u64) -> Self {
        Self {
            smt_timeout_ms: timeout_ms,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backend_capabilities() {
        let ast = BackendCapabilities::ast_only();
        assert!(ast.supports_const_eval);
        assert!(!ast.supports_smt);
        assert!(!ast.can_verify());

        let smt = BackendCapabilities::smt_backed();
        assert!(smt.supports_const_eval);
        assert!(smt.supports_smt);
        assert!(smt.can_verify());
    }

    #[test]
    fn test_verification_result() {
        assert!(VerificationResult::Valid.is_success());
        assert!(VerificationResult::Invalid { counterexample: None }.is_failure());
        assert!(!VerificationResult::Unknown { reason: "timeout".into() }.is_definitive());
    }

    #[test]
    fn test_type_level_error() {
        let err = TypeLevelError::type_error("Int", "String");
        assert!(err.to_string().contains("Int"));
        assert!(err.to_string().contains("String"));

        let err = TypeLevelError::arity_mismatch(2, 3);
        assert!(err.to_string().contains("2"));
        assert!(err.to_string().contains("3"));
    }

    #[test]
    fn test_type_level_config() {
        let default = TypeLevelConfig::default();
        assert_eq!(default.max_depth, 100);
        assert_eq!(default.reduction_strategy, ReductionStrategy::CallByValue);

        let lazy = TypeLevelConfig::lazy();
        assert_eq!(lazy.reduction_strategy, ReductionStrategy::CallByName);
    }
}
