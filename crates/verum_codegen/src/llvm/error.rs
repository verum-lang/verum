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

    /// Internal compiler error.
    #[error("Internal error: {0}")]
    Internal(Text),

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

/// Per-variant projection for [`LoweringSeverity`]. `name` matches
/// the standard diagnostic-severity wire form (`"warning"` /
/// `"info"`); `is_warning` flags the higher-severity variant. The
/// partition is binary by design — `LoweringSeverity` does not
/// carry `Error` (which is a typed `LlvmLoweringError` instead).
#[derive(Debug, Clone, Copy)]
pub struct LoweringSeverityMeta {
    pub name: &'static str,
    pub is_warning: bool,
}

impl LoweringSeverity {
    pub const ALL: &'static [Self] = &[Self::Warning, Self::Info];

    pub const fn meta(self) -> LoweringSeverityMeta {
        match self {
            Self::Warning => LoweringSeverityMeta {
                name: "warning",
                is_warning: true,
            },
            Self::Info => LoweringSeverityMeta {
                name: "info",
                is_warning: false,
            },
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        for v in Self::ALL {
            if v.meta().name == s {
                return Some(*v);
            }
        }
        None
    }

    #[inline]
    pub const fn as_str(&self) -> &'static str {
        self.meta().name
    }

    #[inline]
    pub const fn is_warning(&self) -> bool {
        self.meta().is_warning
    }
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
    pub fn unimplemented_sub_op(
        category: impl Into<Text>,
        sub_op: u8,
        function_name: impl Into<Text>,
    ) -> Self {
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
    pub fn warning(
        category: impl Into<Text>,
        message: impl Into<Text>,
        function_name: impl Into<Text>,
    ) -> Self {
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
        format!(
            "[AOT {}] in `{}`: {}",
            prefix, self.function_name, self.message
        )
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
//  builder.build_store(ptr, val).or_llvm_err()?;
//  let gep = builder.build_gep(ty, ptr, &indices, "name").or_llvm_err()?;

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
///  let block = builder.get_insert_block().or_internal("no current basic block")?;
///  let param = func.get_nth_param(0).or_internal("missing param 0")?;
pub trait OptionExt<T> {
    /// Convert None into an internal error with the given message.
    fn or_internal(self, msg: &str) -> Result<T>;

    /// Convert None into a missing function error.
    fn or_missing_fn(self, name: &str) -> Result<T>;
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
}

impl LlvmLoweringError {

    /// Create a type lowering error.
    pub fn type_lowering(msg: impl Into<Text>) -> Self {
        LlvmLoweringError::TypeLowering(msg.into())
    }

    /// Create an internal error.
    pub fn internal(msg: impl Into<Text>) -> Self {
        LlvmLoweringError::Internal(msg.into())
    }
}

#[cfg(test)]
mod meta_consolidation_pins {
    use super::LoweringSeverity;

    #[test]
    fn lowering_severity_round_trip_unique_and_partition() {
        assert_eq!(LoweringSeverity::ALL.len(), 2);
        for v in LoweringSeverity::ALL {
            let s = v.as_str();
            assert_eq!(LoweringSeverity::from_str(s), Some(*v));
        }
        // Wire form: lowercase (matches the standard
        // diagnostic-severity convention used elsewhere in the
        // codegen layer).
        assert_eq!(LoweringSeverity::Warning.as_str(), "warning");
        assert_eq!(LoweringSeverity::Info.as_str(), "info");
        // is_warning is the binary partition: Warning true, Info false.
        assert!(LoweringSeverity::Warning.is_warning());
        assert!(!LoweringSeverity::Info.is_warning());
        // Pin: enum does NOT carry an Error variant — diagnostics
        // at error-severity flow through the typed
        // `LlvmLoweringError` instead.
        assert!(LoweringSeverity::from_str("error").is_none());
    }
}
