//! Unified Error Type System
//!
//! This module provides a single unified error type that can represent errors from all Verum modules,
//! eliminating the proliferation of incompatible Result type aliases while maintaining rich error context.
//!
//! # Design Philosophy
//!
//! Instead of having 6+ different Result types across crates:
//! - `verum_cbgr::Result<T>` = `Result<T, Error>`
//! - `verum_runtime::Result<T>` = `Result<T, RuntimeError>`
//! - `verum_smt::Result<T>` = `Result<T, Error>`
//! - `verum_types::Result<T>` = `Result<T, TypeError>`
//! - `verum_verification::Result<T>` = `Result<T, VerificationError>`
//! - `verum_parser::ParseResult<T>` = `Result<List<ParseError>, T>`
//!
//! We provide a single unified `VerumError` that can represent any of these errors with automatic
//! conversion via `From` trait implementations.
//!
//! # Usage
//!
//! ```rust,ignore
//! use verum_error::unified::{VerumError, Result};
//!
//! fn example() -> Result<i32> {
//!     // Automatically converts from any crate-specific error
//!     let value = some_cbgr_operation()?;  // Converts from verum_cbgr::Error
//!     let result = some_type_check()?;      // Converts from TypeError
//!     Ok(value + result)
//! }
//! ```
//!
//! # Migration Strategy
//!
//! This is an **additive change** - existing crate-specific error types are preserved for backward
//! compatibility. Code can gradually migrate to using the unified error type at boundaries where
//! multiple error types need to be handled.

use thiserror::Error;
use verum_common::{List, Text};

/// Unified result type for Verum operations
///
/// This type can be used across all Verum crates, automatically converting from
/// crate-specific error types via the `From` trait.
pub type Result<T> = std::result::Result<T, VerumError>;

/// Unified error type for the Verum platform
///
/// This enum consolidates all error variants from across the Verum ecosystem into a single
/// type hierarchy. This eliminates the need for multiple incompatible Result type aliases
/// and enables seamless error propagation across crate boundaries.
///
/// # Error Categories
///
/// Errors are organized by source crate and functionality:
/// - **CBGR Memory Safety** - UseAfterFree, DoubleFree, NullPointer
/// - **Runtime** - ContextNotFound, TaskPanicked, ExecutionError
/// - **Type System** - TypeMismatch, CannotInferLambda, UnboundVariable
/// - **SMT/Verification** - VerificationFailed, VerificationTimeout
/// - **Parsing** - ParseErrors (list of parse errors)
/// - **General** - IO, Network, Config, Other
///
/// # Examples
///
/// ```rust,ignore
/// use verum_error::unified::{VerumError, Result};
///
/// fn cross_crate_operation() -> Result<()> {
///     // Each ? automatically converts the specific error type
///     perform_cbgr_check()?;     // From verum_cbgr::Error
///     verify_types()?;            // From TypeError
///     run_smt_solver()?;          // From verum_smt::Error
///     Ok(())
/// }
/// ```
#[derive(Debug, Error, Clone)]
pub enum VerumError {
    // ========================================================================
    // CBGR Memory Safety Errors
    // ========================================================================
    /// Use-after-free detected: memory generation mismatch
    ///
    /// This occurs when code attempts to dereference a reference whose generation
    /// counter doesn't match the current generation in the allocator, indicating
    /// the memory has been freed and potentially reallocated.
    #[error("use after free: expected generation {expected}, found {actual}")]
    UseAfterFree {
        /// Expected generation from the reference
        expected: u32,
        /// Actual generation in the allocator
        actual: u32,
    },

    /// Double-free detected
    ///
    /// Attempting to free memory that has already been freed. This is caught
    /// by CBGR's generation tracking.
    #[error("double free: expected generation {expected}, found {actual}")]
    DoubleFree {
        /// Expected generation
        expected: u32,
        /// Actual generation in allocator
        actual: u32,
    },

    /// Null pointer dereference
    ///
    /// Attempting to dereference a null pointer (generation = GEN_UNALLOCATED).
    #[error("null pointer dereference")]
    NullPointer,

    /// Out of bounds access
    ///
    /// Array or slice access outside valid bounds.
    #[error("out of bounds: index {index}, length {length}")]
    OutOfBounds {
        /// Attempted index
        index: usize,
        /// Actual length
        length: usize,
    },

    /// Capability violation
    ///
    /// Attempted operation not permitted by the reference's capabilities.
    #[error("capability violation: {message}")]
    CapabilityViolation {
        /// Description of the violation
        message: Text,
    },

    // ========================================================================
    // Runtime Errors
    // ========================================================================
    /// Context not found
    ///
    /// A required execution context (async, IO, etc.) was not available.
    #[error("context not found: {context_name}")]
    ContextNotFound {
        /// Name of the missing context
        context_name: Text,
    },

    /// Task panicked
    ///
    /// An async task or thread panicked during execution.
    #[error("task panicked: {message}")]
    TaskPanicked {
        /// Panic message
        message: Text,
    },

    /// Execution error
    ///
    /// General execution error from the runtime.
    #[error("execution error: {message}")]
    ExecutionError {
        /// Error description
        message: Text,
    },

    /// Stack overflow
    ///
    /// Call stack exceeded maximum depth.
    #[error("stack overflow: depth {depth}")]
    StackOverflow {
        /// Current stack depth
        depth: usize,
    },

    // ========================================================================
    // Type System Errors
    // ========================================================================
    /// Type mismatch
    ///
    /// Expected one type but found another during type checking.
    #[error("type mismatch: expected {expected}, found {actual}")]
    TypeMismatch {
        /// Expected type
        expected: Text,
        /// Actual type found
        actual: Text,
    },

    /// Cannot infer lambda type
    ///
    /// Lambda expression requires explicit type annotation.
    #[error("cannot infer lambda type without annotation")]
    CannotInferLambda,

    /// Unbound variable
    ///
    /// Variable referenced before being defined.
    #[error("unbound variable: {name}")]
    UnboundVariable {
        /// Variable name
        name: Text,
    },

    /// Not a function type
    ///
    /// Attempted to call a non-function value.
    #[error("not a function: {ty}")]
    NotAFunction {
        /// The actual type
        ty: Text,
    },

    /// Infinite type
    ///
    /// Type inference produced an infinite recursive type.
    #[error("infinite type: {var} = {ty}")]
    InfiniteType {
        /// Type variable
        var: Text,
        /// Type expression
        ty: Text,
    },

    /// Protocol not satisfied
    ///
    /// Type does not implement required protocol (trait).
    #[error("protocol not satisfied: {ty} does not implement {protocol}")]
    ProtocolNotSatisfied {
        /// Type being checked
        ty: Text,
        /// Required protocol
        protocol: Text,
    },

    /// Refinement constraint failed
    ///
    /// Value doesn't satisfy refinement type predicate.
    #[error("refinement constraint not satisfied: {predicate}")]
    RefinementFailed {
        /// The failed predicate
        predicate: Text,
    },

    /// Affine type violation
    ///
    /// Affine-typed value used more than once.
    #[error("affine type violation: {ty} used more than once")]
    AffineViolation {
        /// Type being violated
        ty: Text,
    },

    /// Missing context requirement
    ///
    /// Function requires a context that isn't available.
    #[error("missing context: {context}")]
    MissingContext {
        /// Required context
        context: Text,
    },

    // ========================================================================
    // SMT/Verification Errors
    // ========================================================================
    /// Verification timeout
    ///
    /// SMT solver exceeded time limit.
    #[error("verification timeout after {timeout_ms}ms")]
    VerificationTimeout {
        /// Timeout in milliseconds
        timeout_ms: u64,
    },

    /// Verification failed
    ///
    /// SMT solver found a counterexample or could not prove property.
    #[error("verification failed: {reason}")]
    VerificationFailed {
        /// Reason for failure
        reason: Text,
        /// Optional counterexample
        counterexample: Option<Text>,
    },

    /// Unsupported SMT feature
    ///
    /// Attempted to use an unsupported SMT solver feature.
    #[error("unsupported SMT feature: {feature}")]
    UnsupportedSMT {
        /// Feature description
        feature: Text,
    },

    // ========================================================================
    // Parse Errors
    // ========================================================================
    /// Parse errors
    ///
    /// One or more parsing errors occurred. This variant holds a list of
    /// all parse errors to enable batch error reporting.
    #[error("parse errors: {}", format_parse_errors(.0))]
    ParseErrors(List<Text>),

    /// Lexical analysis error
    ///
    /// Error during tokenization/lexing.
    #[error("lex error: {message}")]
    LexError {
        /// Error message
        message: Text,
    },

    // ========================================================================
    // I/O and System Errors
    // ========================================================================
    /// I/O error
    ///
    /// File system or I/O operation failed.
    #[error("I/O error: {message}")]
    IoError {
        /// Error description
        message: Text,
    },

    /// Network error
    ///
    /// Network operation failed.
    #[error("network error: {message}")]
    NetworkError {
        /// Error description
        message: Text,
    },

    /// Configuration error
    ///
    /// Invalid or missing configuration.
    #[error("configuration error: {message}")]
    ConfigError {
        /// Error description
        message: Text,
    },

    /// Timeout error
    ///
    /// Operation exceeded time limit.
    #[error("timeout: operation exceeded {timeout_ms}ms")]
    Timeout {
        /// Timeout in milliseconds
        timeout_ms: u64,
    },

    // ========================================================================
    // Generic/Other
    // ========================================================================
    /// Other error
    ///
    /// Catch-all for errors that don't fit other categories.
    #[error("{message}")]
    Other {
        /// Error message
        message: Text,
    },

    /// Not implemented
    ///
    /// Feature or functionality not yet implemented.
    #[error("not implemented: {feature}")]
    NotImplemented {
        /// Feature description
        feature: Text,
    },
}

// Helper function to format parse errors
fn format_parse_errors(errors: &List<Text>) -> Text {
    if errors.is_empty() {
        return Text::from("(no details)");
    }

    let mut result = Text::new();
    result.push('\n');
    for (i, err) in errors.iter().enumerate() {
        result.push_str(&format!("  {}. {}\n", i + 1, err.as_str()));
    }
    result
}

impl VerumError {
    /// Create a generic error with a message
    pub fn message(msg: impl Into<Text>) -> Self {
        VerumError::Other {
            message: msg.into(),
        }
    }

    /// Check if this error is recoverable
    ///
    /// Recoverable errors can potentially be retried or handled gracefully.
    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            VerumError::IoError { .. }
                | VerumError::NetworkError { .. }
                | VerumError::Timeout { .. }
                | VerumError::VerificationTimeout { .. }
        )
    }

    /// Check if this error is fatal
    ///
    /// Fatal errors indicate memory corruption or security violations that
    /// should terminate the program.
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            VerumError::UseAfterFree { .. }
                | VerumError::DoubleFree { .. }
                | VerumError::StackOverflow { .. }
        )
    }

    /// Check if this error is a type error
    pub fn is_type_error(&self) -> bool {
        matches!(
            self,
            VerumError::TypeMismatch { .. }
                | VerumError::CannotInferLambda
                | VerumError::UnboundVariable { .. }
                | VerumError::NotAFunction { .. }
                | VerumError::InfiniteType { .. }
                | VerumError::ProtocolNotSatisfied { .. }
        )
    }

    /// Check if this error is a verification error
    pub fn is_verification_error(&self) -> bool {
        matches!(
            self,
            VerumError::VerificationTimeout { .. }
                | VerumError::VerificationFailed { .. }
                | VerumError::UnsupportedSMT { .. }
                | VerumError::RefinementFailed { .. }
        )
    }

    /// Check if this error is a memory error
    pub fn is_memory_error(&self) -> bool {
        matches!(
            self,
            VerumError::UseAfterFree { .. }
                | VerumError::DoubleFree { .. }
                | VerumError::NullPointer
                | VerumError::OutOfBounds { .. }
        )
    }

    /// Get the error category for diagnostic purposes
    pub fn category(&self) -> &'static str {
        match self {
            VerumError::UseAfterFree { .. }
            | VerumError::DoubleFree { .. }
            | VerumError::NullPointer
            | VerumError::OutOfBounds { .. }
            | VerumError::CapabilityViolation { .. } => "memory",

            VerumError::ContextNotFound { .. }
            | VerumError::TaskPanicked { .. }
            | VerumError::ExecutionError { .. }
            | VerumError::StackOverflow { .. } => "runtime",

            VerumError::TypeMismatch { .. }
            | VerumError::CannotInferLambda
            | VerumError::UnboundVariable { .. }
            | VerumError::NotAFunction { .. }
            | VerumError::InfiniteType { .. }
            | VerumError::ProtocolNotSatisfied { .. }
            | VerumError::AffineViolation { .. }
            | VerumError::MissingContext { .. } => "type",

            VerumError::VerificationTimeout { .. }
            | VerumError::VerificationFailed { .. }
            | VerumError::UnsupportedSMT { .. }
            | VerumError::RefinementFailed { .. } => "verification",

            VerumError::ParseErrors(_) | VerumError::LexError { .. } => "parse",

            VerumError::IoError { .. }
            | VerumError::NetworkError { .. }
            | VerumError::ConfigError { .. }
            | VerumError::Timeout { .. } => "io",

            VerumError::Other { .. } | VerumError::NotImplemented { .. } => "other",
        }
    }

    /// Get the error code prefix for diagnostics
    pub fn code_prefix(&self) -> &'static str {
        match self.category() {
            "memory" => "E01",
            "runtime" => "E04",
            "type" => "E02",
            "verification" => "E03",
            "parse" => "E00",
            "io" => "E04",
            "other" => "E09",
            _ => "E09",
        }
    }

    // Convenience constructors

    /// Create a type mismatch error
    pub fn type_mismatch(expected: impl Into<Text>, actual: impl Into<Text>) -> Self {
        VerumError::TypeMismatch {
            expected: expected.into(),
            actual: actual.into(),
        }
    }

    /// Create an unbound variable error
    pub fn unbound_variable(name: impl Into<Text>) -> Self {
        VerumError::UnboundVariable { name: name.into() }
    }

    /// Create an I/O error
    pub fn io(msg: impl Into<Text>) -> Self {
        VerumError::IoError {
            message: msg.into(),
        }
    }

    /// Create a network error
    pub fn network(msg: impl Into<Text>) -> Self {
        VerumError::NetworkError {
            message: msg.into(),
        }
    }

    /// Create a verification failed error
    pub fn verification_failed(reason: impl Into<Text>) -> Self {
        VerumError::VerificationFailed {
            reason: reason.into(),
            counterexample: None,
        }
    }

    /// Create a verification failed error with counterexample
    pub fn verification_failed_with_counterexample(
        reason: impl Into<Text>,
        counterexample: impl Into<Text>,
    ) -> Self {
        VerumError::VerificationFailed {
            reason: reason.into(),
            counterexample: Some(counterexample.into()),
        }
    }

    /// Create a not implemented error
    pub fn not_implemented(feature: impl Into<Text>) -> Self {
        VerumError::NotImplemented {
            feature: feature.into(),
        }
    }

    /// Create a context not found error
    pub fn context_not_found(name: impl Into<Text>) -> Self {
        VerumError::ContextNotFound {
            context_name: name.into(),
        }
    }

    /// Create an execution error
    pub fn execution(msg: impl Into<Text>) -> Self {
        VerumError::ExecutionError {
            message: msg.into(),
        }
    }

    /// Create a parse error from a list of messages
    pub fn parse_errors(errors: impl IntoIterator<Item = impl Into<Text>>) -> Self {
        let errs: List<Text> = errors.into_iter().map(|e| e.into()).collect();
        VerumError::ParseErrors(errs)
    }
}

// ============================================================================
// Conversion Implementations
// ============================================================================

// Note: These conversions require the specific error types from other crates.
// They are implemented conditionally when those crates are available.

// From std::io::Error
impl From<std::io::Error> for VerumError {
    fn from(err: std::io::Error) -> Self {
        VerumError::IoError {
            message: err.to_string().into(),
        }
    }
}

// From std::fmt::Error
impl From<std::fmt::Error> for VerumError {
    fn from(err: std::fmt::Error) -> Self {
        VerumError::Other {
            message: err.to_string().into(),
        }
    }
}

// From Text
impl From<Text> for VerumError {
    fn from(msg: Text) -> Self {
        VerumError::Other { message: msg }
    }
}

// From &str
impl From<&str> for VerumError {
    fn from(msg: &str) -> Self {
        VerumError::Other {
            message: msg.into(),
        }
    }
}
