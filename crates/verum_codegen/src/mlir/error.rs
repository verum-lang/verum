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
    /// MLIR context creation or configuration error.
    #[error("MLIR context error: {message}")]
    ContextError { message: Text },

    /// Dialect loading or registration error.
    #[error("Dialect error: {dialect} - {message}")]
    DialectError { dialect: Text, message: Text },

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

    /// Unsupported AST node during lowering.
    #[error("Unsupported expression kind: {kind} at {span:?}")]
    UnsupportedExpression { kind: Text, span: Option<Span> },

    /// Unsupported statement kind during lowering.
    #[error("Unsupported statement kind: {kind} at {span:?}")]
    UnsupportedStatement { kind: Text, span: Option<Span> },

    /// Symbol not found during lowering.
    #[error("Symbol not found: {name} at {span:?}")]
    SymbolNotFoundInScope { name: Text, span: Option<Span> },

    /// Type mismatch error.
    #[error("Type mismatch: expected {expected}, got {actual}")]
    TypeMismatch { expected: Text, actual: Text },

    /// Pass execution error.
    #[error("Pass '{pass_name}' failed: {message}")]
    PassError { pass_name: Text, message: Text },

    /// Pass pipeline configuration error.
    #[error("Pass pipeline error: {message}")]
    PassPipelineError { message: Text },

    /// JIT compilation error.
    #[error("JIT compilation error: {message}")]
    JitError { message: Text },

    /// JIT symbol lookup failed.
    #[error("JIT symbol lookup failed: {symbol}")]
    JitSymbolNotFound { symbol: Text },

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

    /// Object file generation error.
    #[error("Object file error: {message}")]
    ObjectFileError { message: Text },

    /// CBGR operation error.
    #[error("CBGR error: {message}")]
    CbgrError { message: Text },

    /// Context system operation error.
    #[error("Context system error: {message}")]
    ContextSystemError { message: Text },

    /// LLVM lowering error.
    #[error("LLVM lowering error: {message}")]
    LlvmLoweringError { message: Text },

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
    /// Create a context error.
    pub fn context(message: impl Into<Text>) -> Self {
        Self::ContextError {
            message: message.into(),
        }
    }

    /// Create a dialect error.
    pub fn dialect(dialect: impl Into<Text>, message: impl Into<Text>) -> Self {
        Self::DialectError {
            dialect: dialect.into(),
            message: message.into(),
        }
    }

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

    /// Create an unsupported expression error.
    pub fn unsupported_expr(kind: impl Into<Text>, span: Option<Span>) -> Self {
        Self::UnsupportedExpression {
            kind: kind.into(),
            span,
        }
    }

    /// Create an unsupported statement error.
    pub fn unsupported_stmt(kind: impl Into<Text>, span: Option<Span>) -> Self {
        Self::UnsupportedStatement {
            kind: kind.into(),
            span,
        }
    }

    /// Create a symbol not found in scope error (for lowering).
    pub fn symbol_not_found_in_scope(name: impl Into<Text>, span: Option<Span>) -> Self {
        Self::SymbolNotFoundInScope {
            name: name.into(),
            span,
        }
    }

    /// Create a symbol not found error (for JIT resolution).
    pub fn symbol_not_found(name: impl Into<Text>) -> Self {
        Self::SymbolNotFound { name: name.into() }
    }

    /// Create a library not found error.
    pub fn library_not_found(name: impl Into<Text>) -> Self {
        Self::LibraryNotFound { name: name.into() }
    }

    /// Create a library load error.
    pub fn library_load(path: impl Into<Text>, message: impl Into<Text>) -> Self {
        Self::LibraryLoadError {
            path: path.into(),
            message: message.into(),
        }
    }

    /// Create a cache error.
    pub fn cache(message: impl Into<Text>) -> Self {
        Self::CacheError {
            message: message.into(),
        }
    }

    /// Create a REPL error.
    pub fn repl(message: impl Into<Text>) -> Self {
        Self::ReplError {
            message: message.into(),
        }
    }

    /// Create a hot code replacement error.
    pub fn hot_code(message: impl Into<Text>) -> Self {
        Self::HotCodeError {
            message: message.into(),
        }
    }

    /// Create a type mismatch error.
    pub fn type_mismatch(expected: impl Into<Text>, actual: impl Into<Text>) -> Self {
        Self::TypeMismatch {
            expected: expected.into(),
            actual: actual.into(),
        }
    }

    /// Create a pass error.
    pub fn pass(pass_name: impl Into<Text>, message: impl Into<Text>) -> Self {
        Self::PassError {
            pass_name: pass_name.into(),
            message: message.into(),
        }
    }

    /// Create a JIT error.
    pub fn jit(message: impl Into<Text>) -> Self {
        Self::JitError {
            message: message.into(),
        }
    }

    /// Create an AOT error.
    pub fn aot(message: impl Into<Text>) -> Self {
        Self::AotError {
            message: message.into(),
        }
    }

    /// Create a CBGR error.
    pub fn cbgr(message: impl Into<Text>) -> Self {
        Self::CbgrError {
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

/// Extension trait for converting melior errors.
impl From<verum_mlir::Error> for MlirError {
    fn from(err: verum_mlir::Error) -> Self {
        MlirError::InternalError {
            message: Text::from(format!("{:?}", err)),
        }
    }
}
