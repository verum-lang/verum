//! Error types for LLVM-based VBC lowering.
//!
//! This module defines errors that can occur during the VBC → LLVM IR
//! lowering process.

use thiserror::Error;
use verum_common::Text;

/// Result type for LLVM lowering operations.
pub type Result<T> = std::result::Result<T, LlvmLoweringError>;

/// Error type for VBC → LLVM IR lowering.
#[derive(Debug, Error)]
pub enum LlvmLoweringError {
    /// Unsupported VBC instruction encountered.
    #[error("Unsupported VBC instruction: {0}")]
    UnsupportedInstruction(Text),

    /// Type lowering error.
    #[error("Type lowering error: {0}")]
    TypeLowering(Text),

    /// CBGR lowering error.
    #[error("CBGR lowering error: {0}")]
    CbgrLowering(Text),

    /// Invalid register reference.
    #[error("Invalid register: r{0}")]
    InvalidRegister(u16),

    /// Missing function definition.
    #[error("Missing function: {0}")]
    MissingFunction(Text),

    /// Missing basic block.
    #[error("Missing basic block: {0}")]
    MissingBlock(Text),

    /// Module verification failed.
    #[error("Module verification failed: {0}")]
    VerificationFailed(Text),

    /// LLVM error from underlying library.
    #[error("LLVM error: {0}")]
    LlvmError(Text),

    /// Invalid constant pool reference.
    #[error("Invalid constant pool index: {0}")]
    InvalidConstant(u32),

    /// Type mismatch during lowering.
    #[error("Type mismatch: expected {expected}, got {actual}")]
    TypeMismatch { expected: Text, actual: Text },

    /// Internal compiler error.
    #[error("Internal error: {0}")]
    Internal(Text),

    /// LLVM intrinsic not found.
    #[error("Intrinsic not found: {0}")]
    IntrinsicNotFound(Text),

    /// Invalid type for operation.
    #[error("Invalid type: {0}")]
    InvalidType(Text),

    /// Builder operation failed.
    #[error("Builder error: {0}")]
    BuilderError(Text),
}

/// Severity level for lowering diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoweringSeverity {
    Warning,
    Info,
}

/// A structured diagnostic emitted during LLVM lowering.
///
/// Replaces raw `eprintln!` warnings with a collected, structured format
/// that can be filtered, counted, and displayed consistently.
#[derive(Debug, Clone)]
pub struct LoweringDiagnostic {
    /// Severity level
    pub severity: LoweringSeverity,
    /// Human-readable message
    pub message: Text,
    /// Category of the diagnostic (e.g. "ArithExtended", "MathExtended")
    pub category: Text,
    /// The sub-opcode that triggered the diagnostic, if applicable
    pub sub_opcode: Option<u8>,
    /// The function being lowered when the diagnostic was emitted
    pub function_name: Text,
}

impl LoweringDiagnostic {
    /// Create a warning for an unimplemented sub-opcode.
    pub fn unimplemented_sub_op(category: impl Into<Text>, sub_op: u8, function_name: impl Into<Text>) -> Self {
        let cat: Text = category.into();
        Self {
            severity: LoweringSeverity::Warning,
            message: Text::from(format!("Unimplemented {} sub_op: 0x{:02x}", cat, sub_op)),
            category: cat,
            sub_opcode: Some(sub_op),
            function_name: function_name.into(),
        }
    }

    /// Create a general warning.
    pub fn warning(category: impl Into<Text>, message: impl Into<Text>, function_name: impl Into<Text>) -> Self {
        Self {
            severity: LoweringSeverity::Warning,
            message: message.into(),
            category: category.into(),
            sub_opcode: None,
            function_name: function_name.into(),
        }
    }

    /// Format this diagnostic for display.
    pub fn display(&self) -> String {
        let prefix = match self.severity {
            LoweringSeverity::Warning => "warning",
            LoweringSeverity::Info => "info",
        };
        format!("[AOT {}] in `{}`: {}", prefix, self.function_name, self.message)
    }
}

// =============================================================================
// BuildExt — Zero-cost error propagation for LLVM builder operations
// =============================================================================
//
// LLVM builder methods (build_store, build_gep, etc.) return Result<T, BuilderError>.
// Instead of .unwrap() which panics on any LLVM error, use .or_llvm_err()? to
// propagate errors as LlvmLoweringError::BuilderError with the original message.
//
// Usage:
//   builder.build_store(ptr, val).or_llvm_err()?;
//   let gep = builder.build_gep(ty, ptr, &indices, "name").or_llvm_err()?;

/// Extension trait for converting any `Result<T, E: Display>` into
/// `Result<T, LlvmLoweringError>` via `.or_llvm_err()`.
///
/// This replaces `.unwrap()` calls on LLVM builder operations with proper
/// error propagation through the lowering pipeline.
pub trait BuildExt<T> {
    /// Convert a builder Result into a lowering Result.
    ///
    /// Equivalent to `.map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))`.
    fn or_llvm_err(self) -> Result<T>;
}

impl<T, E: std::fmt::Display> BuildExt<T> for std::result::Result<T, E> {
    #[inline]
    fn or_llvm_err(self) -> Result<T> {
        self.map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))
    }
}

/// Extension trait for converting `Option<T>` into `Result<T, LlvmLoweringError>`.
///
/// Usage:
///   let block = builder.get_insert_block().or_internal("no current basic block")?;
///   let param = func.get_nth_param(0).or_internal("missing param 0")?;
pub trait OptionExt<T> {
    /// Convert None into an internal error with the given message.
    fn or_internal(self, msg: &str) -> Result<T>;

    /// Convert None into a missing function error.
    fn or_missing_fn(self, name: &str) -> Result<T>;

    /// Convert None into a missing block error.
    fn or_missing_block(self, name: &str) -> Result<T>;
}

impl<T> OptionExt<T> for Option<T> {
    #[inline]
    fn or_internal(self, msg: &str) -> Result<T> {
        self.ok_or_else(|| LlvmLoweringError::Internal(msg.into()))
    }

    #[inline]
    fn or_missing_fn(self, name: &str) -> Result<T> {
        self.ok_or_else(|| LlvmLoweringError::MissingFunction(name.into()))
    }

    #[inline]
    fn or_missing_block(self, name: &str) -> Result<T> {
        self.ok_or_else(|| LlvmLoweringError::MissingBlock(name.into()))
    }
}

impl LlvmLoweringError {
    /// Create an unsupported instruction error.
    pub fn unsupported_instruction(name: impl Into<Text>) -> Self {
        LlvmLoweringError::UnsupportedInstruction(name.into())
    }

    /// Create a type lowering error.
    pub fn type_lowering(msg: impl Into<Text>) -> Self {
        LlvmLoweringError::TypeLowering(msg.into())
    }

    /// Create a CBGR lowering error.
    pub fn cbgr_lowering(msg: impl Into<Text>) -> Self {
        LlvmLoweringError::CbgrLowering(msg.into())
    }

    /// Create an LLVM error.
    pub fn llvm_error(msg: impl Into<Text>) -> Self {
        LlvmLoweringError::LlvmError(msg.into())
    }

    /// Create an internal error.
    pub fn internal(msg: impl Into<Text>) -> Self {
        LlvmLoweringError::Internal(msg.into())
    }
}
