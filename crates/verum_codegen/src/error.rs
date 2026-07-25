//! Code Generation Errors
//!
//! Error types for MLIR-based code generation.

use std::fmt::Display;
use thiserror::Error;
use verum_common::Text;

/// Result type for codegen operations.
pub type Result<T> = std::result::Result<T, CodegenError>;

/// Alias for Result<T>.
pub type CodegenResult<T> = Result<T>;

/// Errors that can occur during code generation.
#[derive(Debug, Error)]
pub enum CodegenError {
    /// MLIR infrastructure error.
    #[error("MLIR error: {0}")]
    MlirError(Text),

    /// JIT compilation error.
    #[error("JIT error: {0}")]
    JitError(Text),

    /// Type error during codegen.
    #[error("Type error: {0}")]
    TypeError(Text),

    /// Undefined variable.
    #[error("Undefined variable: {0}")]
    UndefinedVariable(Text),

    /// Undefined function.
    #[error("Undefined function: {0}")]
    UndefinedFunction(Text),

    /// Undefined type.
    #[error("Undefined type: {0}")]
    UndefinedType(Text),

    /// Feature not implemented.
    #[error("Not implemented: {0}")]
    NotImplemented(Text),

    /// Internal error.
    #[error("Internal error: {0}")]
    Internal(Text),

    /// I/O error.
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    /// Verification failed.
    #[error("Verification failed: {0}")]
    VerificationFailed(Text),

    /// Invalid target.
    #[error("Invalid target: {0}")]
    InvalidTarget(Text),
}

impl CodegenError {
    /// Create MLIR error.
    pub fn mlir(msg: impl Into<Text>) -> Self {
        Self::MlirError(msg.into())
    }

    /// Create JIT error.
    pub fn jit(msg: impl Into<Text>) -> Self {
        Self::JitError(msg.into())
    }

    /// Create type error.
    pub fn type_error(msg: impl Into<Text>) -> Self {
        Self::TypeError(msg.into())
    }

    /// Create undefined variable error.
    pub fn undefined_var(name: impl Into<Text>) -> Self {
        Self::UndefinedVariable(name.into())
    }

    /// Create undefined function error.
    pub fn undefined_func(name: impl Into<Text>) -> Self {
        Self::UndefinedFunction(name.into())
    }

    /// Create undefined type error.
    pub fn undefined_type(name: impl Into<Text>) -> Self {
        Self::UndefinedType(name.into())
    }

    /// Create not implemented error.
    pub fn not_implemented(feature: impl Into<Text>) -> Self {
        Self::NotImplemented(feature.into())
    }

    /// Create internal error.
    pub fn internal(msg: impl Into<Text>) -> Self {
        Self::Internal(msg.into())
    }

    /// Create verification error.
    pub fn verification(msg: impl Into<Text>) -> Self {
        Self::VerificationFailed(msg.into())
    }
}
