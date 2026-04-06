//! Level 0: Type Prevention (Compile-Time Safety)
//!
//! The best errors are the ones that cannot be written in the first place.
//! Level 0 uses the type system to make entire classes of errors unrepresentable.
//! Three mechanisms provide compile-time prevention:
//!
//! - **Refinement types** (`T{pred}`, `T where pred`, `var: T where pred`): extend
//!   base types with logical predicates verified by the SMT solver at compile time,
//!   eliminating runtime checks entirely.
//! - **Affine types**: linear tracking ensures values are used at most once,
//!   preventing use-after-free, double-free, and resource leaks. Enforced by the
//!   CBGR (Compile-time Bounded Generational References) system.
//! - **Context tracking**: capability-based dependency injection at the type level
//!   ensures context requirements (e.g., `using [FileIO]`) are explicitly declared
//!   and statically verified.
//!
//! Errors at this level are **prevented by the type system** and should never
//! occur in well-typed Verum programs. This level includes:
//!
//! - **Refinement types** (e.g., `x: Int{> 0}`) - prove constraints at compile time
//! - **Affine types** - move semantics prevent use-after-free
//! - **Context requirements** - require specific contexts to be available
//! - **Type constraints** - use the protocol system to enforce type safety
//!
//! # When These Errors Occur
//!
//! Level 0 errors indicate a violation of type-system invariants that the
//! compiler should have caught. If you're seeing these at runtime, it means:
//!
//! - The code bypassed type checking (e.g., via unsafe code or FFI)
//! - The compiler had a bug
//! - Someone used `unwrap()` on a checked result
//!
//! # Prevention Strategies
//!
//! - Use refinement types to prove constraints
//! - Use affine types to prevent moves
//! - Use the context system to track required state
//! - Enable SMT verification for critical code
//! - Avoid unsafe blocks unless absolutely necessary
//!
//! This module provides error types for type-level violations that **should
//! never occur in well-typed programs** but may occur when safety is bypassed.

use crate::{ErrorKind, VerumError};
use verum_common::Text;

/// Refinement constraint violation
///
/// Indicates a refinement type predicate was not satisfied.
/// This should be caught at compile-time by SMT verification.
#[derive(Debug, Clone, thiserror::Error)]
#[error("Refinement constraint violated: {predicate}")]
pub struct RefinementError {
    /// The violated predicate
    pub predicate: Text,
    /// The value that violated it
    pub value: Option<Text>,
    /// Expected constraint
    pub expected: Option<Text>,
}

impl RefinementError {
    /// Create a new refinement error
    pub fn new(predicate: impl Into<Text>) -> Self {
        Self {
            predicate: predicate.into(),
            value: None,
            expected: None,
        }
    }

    /// Add the violating value
    pub fn with_value(mut self, value: impl Into<Text>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// Add the expected constraint
    pub fn with_expected(mut self, expected: impl Into<Text>) -> Self {
        self.expected = Some(expected.into());
        self
    }
}

impl From<RefinementError> for VerumError {
    fn from(err: RefinementError) -> Self {
        let mut message = format!("Refinement constraint violated: {}", err.predicate);
        if let Some(value) = err.value {
            message.push_str(&format!(" (value: {})", value));
        }
        if let Some(expected) = err.expected {
            message.push_str(&format!(" (expected: {})", expected));
        }
        VerumError::new(message, ErrorKind::Refinement)
    }
}

/// Affine type violation
///
/// Indicates a value was used after being moved or used more than once.
#[derive(Debug, Clone, thiserror::Error)]
#[error("Affine type violation: {message}")]
pub struct AffineError {
    /// Error message
    pub message: Text,
    /// Variable name (if known)
    pub variable: Option<Text>,
}

impl AffineError {
    /// Create a use-after-move error
    pub fn use_after_move(variable: impl Into<Text>) -> Self {
        let var = variable.into();
        Self {
            message: format!("Value used after move: {}", var).into(),
            variable: Some(var),
        }
    }

    /// Create a double-free error
    pub fn double_free(variable: impl Into<Text>) -> Self {
        let var = variable.into();
        Self {
            message: format!("Value freed twice: {}", var).into(),
            variable: Some(var),
        }
    }
}

impl From<AffineError> for VerumError {
    fn from(err: AffineError) -> Self {
        VerumError::new(err.message, ErrorKind::Affine)
    }
}

/// Context requirement violation
///
/// Indicates a function requiring a context was called without it.
#[derive(Debug, Clone, thiserror::Error)]
#[error("Context requirement not satisfied: {context}")]
pub struct ContextRequirementError {
    /// Required context name
    pub context: Text,
    /// Function that requires it
    pub function: Option<Text>,
}

impl ContextRequirementError {
    /// Create a new context requirement error
    pub fn new(context: impl Into<Text>) -> Self {
        Self {
            context: context.into(),
            function: None,
        }
    }

    /// Add the function name
    pub fn with_function(mut self, function: impl Into<Text>) -> Self {
        self.function = Some(function.into());
        self
    }
}

impl From<ContextRequirementError> for VerumError {
    fn from(err: ContextRequirementError) -> Self {
        let mut message = format!("Context requirement not satisfied: {}", err.context);
        if let Some(function) = err.function {
            message.push_str(&format!(" (in function: {})", function));
        }
        VerumError::new(message, ErrorKind::Context)
    }
}
