//! Error types for verum_mlir.
//!

//! Comprehensive error handling for MLIR-based code generation,
//! including dialect operations, lowering, optimization, and JIT compilation.

use thiserror::Error;
use verum_ast::span::Span;
use verum_common::Text;

/// Result type alias for verum_mlir operations.
pub type Result<T> = std::result::Result<T, MlirError>;

/// Comprehensive error type for MLIR operations.
#[derive(Debug, Error)]
pub enum MlirError {
    /// Type translation error (Verum type → MLIR type).
    #[error("Type translation error: cannot translate '{verum_type}' to MLIR: {reason}")]
    TypeTranslation { verum_type: Text, reason: Text },

    /// Operation building error.
    #[error("Operation error: {op_name} - {message}")]
    OperationError { op_name: Text, message: Text },

    /// MLIR module verification failed.
    #[error("Verification failed: {message}")]
    VerificationError { message: Text },

    /// Lowering error during AST → MLIR transformation.
    #[error("Lowering error at {span:?}: {message}")]
    LoweringError { span: Option<Span>, message: Text },

    /// Pass pipeline configuration error.
    #[error("Pass pipeline error: {message}")]
    PassPipelineError { message: Text },

    /// Dynamic library not found.
    #[error("Library not found: {name}")]
    LibraryNotFound { name: Text },

    /// Dynamic library loading error.
    #[error("Failed to load library '{path}': {message}")]
    LibraryLoadError { path: Text, message: Text },

    /// Symbol not found (without span, for JIT resolution).
    #[error("Symbol not found: {name}")]
    SymbolNotFound { name: Text },

    /// JIT callback error.
    #[error("JIT callback error: {message}")]
    JitCallbackError { message: Text },

    /// Incremental compilation cache error.
    #[error("Cache error: {message}")]
    CacheError { message: Text },

    /// REPL session error.
    #[error("REPL error: {message}")]
    ReplError { message: Text },

    /// Hot code replacement error.
    #[error("Hot code replacement error: {message}")]
    HotCodeError { message: Text },

    /// JIT invocation error.
    #[error("JIT invocation error for '{function}': {message}")]
    JitInvocationError { function: Text, message: Text },

    /// AOT compilation error.
    #[error("AOT compilation error: {message}")]
    AotError { message: Text },

    /// Internal error (should not happen in normal operation).
    #[error("Internal error: {message}")]
    InternalError { message: Text },

    /// IO error.
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// Feature not yet implemented.
    #[error("Not implemented: {feature}")]
    NotImplemented { feature: Text },
}

impl MlirError {
    /// Create a type translation error.
    pub fn type_translation(verum_type: impl Into<Text>, reason: impl Into<Text>) -> Self {
        Self::TypeTranslation {
            verum_type: verum_type.into(),
            reason: reason.into(),
        }
    }

    /// Create an operation error.
    pub fn operation(op_name: impl Into<Text>, message: impl Into<Text>) -> Self {
        Self::OperationError {
            op_name: op_name.into(),
            message: message.into(),
        }
    }

    /// Create a verification error.
    pub fn verification(message: impl Into<Text>) -> Self {
        Self::VerificationError {
            message: message.into(),
        }
    }

    /// Create a lowering error.
    pub fn lowering(span: Option<Span>, message: impl Into<Text>) -> Self {
        Self::LoweringError {
            span,
            message: message.into(),
        }
    }

    /// Create an AOT error.
    pub fn aot(message: impl Into<Text>) -> Self {
        Self::AotError {
            message: message.into(),
        }
    }

    /// Create an internal error.
    pub fn internal(message: impl Into<Text>) -> Self {
        Self::InternalError {
            message: message.into(),
        }
    }

    /// Create a not implemented error.
    pub fn not_implemented(feature: impl Into<Text>) -> Self {
        Self::NotImplemented {
            feature: feature.into(),
        }
    }
}

/// Extension trait for converting `Option<T>` into `Result<T, MlirError>`
/// via [`OptionExt::or_internal`].
///
/// Mirrors the same pattern as `verum_codegen::llvm::error::OptionExt`
/// and `verum_vbc::codegen::error::CodegenOptionExt` — the MLIR
/// lowering path has its own `MlirError` enum, so it gets its own
/// helper trait.  Replaces the verbose
///
///     .ok_or_else(|| MlirError::internal("<msg>"))
///
/// pattern repeated 32+ times across `crates/verum_codegen/src/mlir/`
/// with the much shorter
///
///     .or_internal("<msg>")?
pub trait OptionExt<T> {
    /// Convert `None` into `MlirError::InternalError { message: <msg> }`.
    fn or_internal(self, msg: &str) -> Result<T>;
}

impl<T> OptionExt<T> for Option<T> {
    #[inline]
    fn or_internal(self, msg: &str) -> Result<T> {
        self.ok_or_else(|| MlirError::internal(msg))
    }
}

/// Extension trait for converting melior errors.
impl From<verum_mlir::Error> for MlirError {
    fn from(err: verum_mlir::Error) -> Self {
        MlirError::InternalError {
            message: Text::from(format!("{:?}", err)),
        }
    }
}
