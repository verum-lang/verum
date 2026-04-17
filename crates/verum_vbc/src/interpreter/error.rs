//! Interpreter error types.

use std::fmt;

use crate::instruction::{Opcode, Reg};
use crate::module::FunctionId;
use crate::types::TypeId;

// Re-export CbgrViolationKind from verum_common (single source of truth)
// CBGR violation kinds: generation mismatch (use-after-free), epoch violation (dangling ref),
// capability violation (read/write/execute permission), and null dereference.
pub use verum_common::CbgrViolationKind;

/// Result type for interpreter operations.
pub type InterpreterResult<T> = Result<T, InterpreterError>;

/// Interpreter runtime errors.
#[derive(Debug, Clone)]
#[allow(missing_docs)]
pub enum InterpreterError {
    /// Invalid opcode encountered.
    InvalidOpcode {
        /// The invalid opcode byte.
        opcode: u8,
        /// Program counter where error occurred.
        pc: usize,
    },

    /// Invalid sub-opcode encountered in extended instruction.
    InvalidSubOpcode {
        /// The parent opcode.
        opcode: u8,
        /// The invalid sub-opcode byte.
        sub_opcode: u8,
    },

    /// Function not found.
    FunctionNotFound(FunctionId),

    /// Invalid register access.
    InvalidRegister {
        /// The register that was accessed.
        reg: Reg,
        /// Size of the current frame.
        frame_size: u16,
    },

    /// Stack overflow.
    StackOverflow {
        /// Current stack depth.
        depth: usize,
        /// Maximum allowed depth.
        max_depth: usize,
    },

    /// Stack underflow (pop on empty stack).
    StackUnderflow,

    /// Type mismatch in operation.
    TypeMismatch {
        /// Expected type name.
        expected: &'static str,
        /// Actual type name.
        got: &'static str,
        /// Operation being performed.
        operation: &'static str,
    },

    /// Division by zero.
    DivisionByZero,

    /// Invalid character conversion (Int to Char with invalid codepoint).
    InvalidCharConversion {
        /// The integer value that couldn't be converted.
        value: i64,
        /// Reason for the failure.
        reason: String,
    },

    /// Integer overflow.
    IntegerOverflow {
        /// Operation that overflowed.
        operation: &'static str,
    },

    /// Null pointer dereference.
    NullPointer,

    /// Invalid field index.
    InvalidFieldIndex {
        /// Type being accessed.
        type_id: TypeId,
        /// Field index attempted.
        field: u16,
        /// Number of fields in type.
        num_fields: u16,
    },

    /// Array index out of bounds.
    IndexOutOfBounds {
        /// Index attempted.
        index: i64,
        /// Actual array length.
        length: usize,
    },

    /// Invalid constant pool index.
    InvalidConstant(u32),

    /// Invalid type ID.
    InvalidType(TypeId),

    /// CBGR validation failed (use-after-free, etc.).
    CbgrViolation {
        /// Type of violation.
        kind: CbgrViolationKind,
        /// Pointer address.
        ptr: usize,
    },

    /// Assertion failed.
    AssertionFailed {
        /// Assertion message.
        message: String,
        /// Program counter.
        pc: usize,
    },

    /// Explicit panic.
    Panic {
        /// Panic message.
        message: String,
    },

    /// Unreachable code executed.
    Unreachable {
        /// Program counter.
        pc: usize,
    },

    /// Out of memory.
    OutOfMemory {
        /// Bytes requested.
        requested: usize,
        /// Bytes available.
        available: usize,
    },

    /// Execution timeout.
    Timeout {
        /// Milliseconds elapsed.
        elapsed_ms: u64,
        /// Timeout limit in milliseconds.
        limit_ms: u64,
    },

    /// Invalid bytecode (corrupt or truncated).
    InvalidBytecode {
        /// Program counter.
        pc: usize,
        /// Error description.
        message: String,
    },

    /// Feature not implemented.
    NotImplemented {
        /// Feature name.
        feature: &'static str,
        /// Related opcode if any.
        opcode: Option<Opcode>,
    },

    /// Async task failed.
    TaskFailed {
        /// Task ID.
        task_id: super::state::TaskId,
    },

    /// Invalid task ID.
    InvalidTaskId {
        /// Task ID.
        task_id: super::state::TaskId,
    },

    /// Context not available.
    ContextNotProvided {
        /// Context type ID.
        ctx_type: u32,
    },

    /// Invalid generator ID.
    InvalidGeneratorId {
        /// Generator ID.
        generator_id: super::state::GeneratorId,
    },

    /// Generator is not in a resumable state.
    GeneratorNotResumable {
        /// Generator ID.
        generator_id: super::state::GeneratorId,
        /// Current status description.
        status: &'static str,
    },

    /// FFI call failed.
    ///
    /// This error occurs when a call through the RuntimeFfiGate fails.
    /// The underlying FFI error is converted to this format.
    FfiError {
        /// FFI slot ID that failed.
        slot: u32,
        /// Error message from FFI layer.
        message: String,
    },

    /// Capability check failed.
    ///
    /// This error occurs when RequireCapability opcode fails.
    CapabilityDenied {
        /// Required capability bits.
        required: u64,
        /// Actual capability bits.
        actual: u64,
    },

    /// FFI runtime error (for libffi-based FFI calls).
    ///
    /// This error occurs when the new FFI system (using FfiRuntime with libffi)
    /// encounters an error during symbol resolution or function calls.
    /// Distinct from FfiError which is for the RuntimeFfiGate (slot-based FFI).
    FfiRuntimeError(String),

    /// Invalid operand for instruction.
    ///
    /// This error occurs when an instruction receives an operand that is
    /// invalid for the operation (e.g., invalid size for raw pointer deref).
    InvalidOperand {
        /// Error message.
        message: String,
    },

    /// Execution exceeded instruction limit.
    InstructionLimitExceeded {
        /// Number of instructions executed.
        count: u64,
        /// The configured limit.
        limit: u64,
    },

    /// Module is not interpretable.
    ///
    /// This error occurs when attempting to interpret a VBC module that has
    /// the NOT_INTERPRETABLE flag set. Systems profile modules are NOT
    /// interpretable - VBC serves only as intermediate IR for AOT compilation.
    ///
    /// V-LLSI: Systems/embedded profile modules cannot be interpreted (AOT-only).
    ModuleNotInterpretable {
        /// Module name.
        module_name: String,
        /// Reason for non-interpretability.
        reason: &'static str,
    },

    /// Allocation too large.
    ///
    /// This error occurs when a typed array or buffer allocation exceeds
    /// the maximum allowed size.
    AllocationTooLarge {
        /// Bytes requested.
        requested: usize,
        /// Maximum allowed bytes.
        max_allowed: usize,
    },
}

// Note: CbgrViolationKind is re-exported from verum_common above
// Old local definition removed to use single source of truth

impl fmt::Display for InterpreterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOpcode { opcode, pc } => {
                write!(f, "Invalid opcode {:#04x} at pc {}", opcode, pc)
            }
            Self::InvalidSubOpcode { opcode, sub_opcode } => {
                write!(f, "Invalid sub-opcode {:#04x} for opcode {:#04x}", sub_opcode, opcode)
            }
            Self::FunctionNotFound(id) => {
                write!(f, "Function {} not found", id.0)
            }
            Self::InvalidRegister { reg, frame_size } => {
                write!(
                    f,
                    "Invalid register r{} (frame size: {})",
                    reg.0, frame_size
                )
            }
            Self::StackOverflow { depth, max_depth } => {
                write!(
                    f,
                    "Stack overflow: depth {} exceeds maximum {}",
                    depth, max_depth
                )
            }
            Self::StackUnderflow => {
                write!(f, "Stack underflow: cannot pop from empty call stack")
            }
            Self::TypeMismatch {
                expected,
                got,
                operation,
            } => {
                write!(
                    f,
                    "Type mismatch in {}: expected {}, got {}",
                    operation, expected, got
                )
            }
            Self::DivisionByZero => write!(f, "Division by zero"),
            Self::InvalidCharConversion { value, reason } => {
                write!(f, "Invalid character conversion: {} ({})", value, reason)
            }
            Self::IntegerOverflow { operation } => {
                write!(f, "Integer overflow in {}", operation)
            }
            Self::NullPointer => write!(f, "Null pointer dereference"),
            Self::InvalidFieldIndex {
                type_id,
                field,
                num_fields,
            } => {
                write!(
                    f,
                    "Invalid field index {} for type {} (has {} fields)",
                    field, type_id.0, num_fields
                )
            }
            Self::IndexOutOfBounds { index, length } => {
                write!(
                    f,
                    "Index out of bounds: index {} for list of length {}",
                    index, length
                )
            }
            Self::InvalidConstant(id) => {
                write!(f, "Invalid constant pool index {}", id)
            }
            Self::InvalidType(id) => {
                write!(f, "Invalid type ID {}", id.0)
            }
            Self::CbgrViolation { kind, ptr } => {
                write!(f, "CBGR violation: {:?} at {:#x}", kind, ptr)
            }
            Self::AssertionFailed { message, pc } => {
                write!(f, "Assertion failed at pc {}: {}", pc, message)
            }
            Self::Panic { message } => {
                write!(f, "Panic: {}", message)
            }
            Self::Unreachable { pc } => {
                write!(f, "Unreachable code executed at pc {}", pc)
            }
            Self::OutOfMemory {
                requested,
                available,
            } => {
                write!(
                    f,
                    "Out of memory: requested {} bytes, {} available",
                    requested, available
                )
            }
            Self::Timeout { elapsed_ms, limit_ms } => {
                write!(
                    f,
                    "Execution timeout: {}ms elapsed (limit: {}ms)",
                    elapsed_ms, limit_ms
                )
            }
            Self::InvalidBytecode { pc, message } => {
                write!(f, "Invalid bytecode at pc {}: {}", pc, message)
            }
            Self::NotImplemented { feature, opcode } => {
                match opcode {
                    Some(op) => write!(
                        f,
                        "Not implemented: {} (opcode {:?})",
                        feature, op
                    ),
                    None => write!(f, "Not implemented: {}", feature),
                }
            }
            Self::TaskFailed { task_id } => {
                write!(f, "Async task {} failed", task_id.0)
            }
            Self::InvalidTaskId { task_id } => {
                write!(f, "Invalid task ID {}", task_id.0)
            }
            Self::ContextNotProvided { ctx_type } => {
                write!(f, "Context type {} not provided", ctx_type)
            }
            Self::InvalidGeneratorId { generator_id } => {
                write!(f, "Invalid generator ID {}", generator_id.0)
            }
            Self::GeneratorNotResumable { generator_id, status } => {
                write!(
                    f,
                    "Generator {} is not resumable (status: {})",
                    generator_id.0, status
                )
            }
            Self::FfiError { slot, message } => {
                write!(f, "FFI call to slot {:#04x} failed: {}", slot, message)
            }
            Self::CapabilityDenied { required, actual } => {
                write!(
                    f,
                    "Capability denied: required {:#x}, actual {:#x}",
                    required, actual
                )
            }
            Self::FfiRuntimeError(msg) => {
                write!(f, "FFI runtime error: {}", msg)
            }
            Self::InvalidOperand { message } => {
                write!(f, "Invalid operand: {}", message)
            }
            Self::InstructionLimitExceeded { count, limit } => {
                write!(
                    f,
                    "Execution exceeded instruction limit: {} instructions (limit: {})",
                    count, limit
                )
            }
            Self::ModuleNotInterpretable { module_name, reason } => {
                write!(
                    f,
                    "Module '{}' is not interpretable: {}. VBC is intermediate IR only - AOT compilation required.",
                    module_name, reason
                )
            }
            Self::AllocationTooLarge { requested, max_allowed } => {
                write!(
                    f,
                    "Allocation too large: requested {} bytes (max allowed: {} bytes)",
                    requested, max_allowed
                )
            }
        }
    }
}

impl std::error::Error for InterpreterError {}

// Note: Display for CbgrViolationKind is implemented in verum_common

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = InterpreterError::DivisionByZero;
        assert_eq!(err.to_string(), "Division by zero");

        let err = InterpreterError::InvalidOpcode { opcode: 0xFF, pc: 42 };
        assert!(err.to_string().contains("0xff"));
        assert!(err.to_string().contains("42"));
    }

    #[test]
    fn test_cbgr_violation_display() {
        let err = InterpreterError::CbgrViolation {
            kind: CbgrViolationKind::UseAfterFree,
            ptr: 0xDEADBEEF,
        };
        assert!(err.to_string().contains("UseAfterFree"));
    }

    #[test]
    fn test_all_error_variants() {
        // Test that all error variants can be formatted
        let errors: Vec<InterpreterError> = vec![
            InterpreterError::InvalidOpcode { opcode: 0, pc: 0 },
            InterpreterError::FunctionNotFound(FunctionId(0)),
            InterpreterError::InvalidRegister { reg: Reg(0), frame_size: 10 },
            InterpreterError::StackOverflow { depth: 100, max_depth: 50 },
            InterpreterError::StackUnderflow,
            InterpreterError::TypeMismatch { expected: "int", got: "float", operation: "add" },
            InterpreterError::DivisionByZero,
            InterpreterError::IntegerOverflow { operation: "mul" },
            InterpreterError::NullPointer,
            InterpreterError::InvalidFieldIndex { type_id: TypeId::INT, field: 5, num_fields: 3 },
            InterpreterError::IndexOutOfBounds { index: 10, length: 5 },
            InterpreterError::InvalidConstant(42),
            InterpreterError::InvalidType(TypeId(999)),
            InterpreterError::CbgrViolation { kind: CbgrViolationKind::UseAfterFree, ptr: 0 },
            InterpreterError::AssertionFailed { message: "test".to_string(), pc: 0 },
            InterpreterError::Panic { message: "test".to_string() },
            InterpreterError::Unreachable { pc: 0 },
            InterpreterError::OutOfMemory { requested: 1000, available: 100 },
            InterpreterError::Timeout { elapsed_ms: 5000, limit_ms: 1000 },
            InterpreterError::InvalidBytecode { pc: 0, message: "test".to_string() },
            InterpreterError::NotImplemented { feature: "test", opcode: None },
            InterpreterError::NotImplemented { feature: "test", opcode: Some(Opcode::Mov) },
            InterpreterError::ModuleNotInterpretable {
                module_name: "test_module".to_string(),
                reason: "Systems profile code is AOT-only",
            },
        ];

        for err in errors {
            let _ = err.to_string();
            let _ = format!("{:?}", err);
        }
    }

    #[test]
    fn test_cbgr_violation_kinds() {
        // Test all CbgrViolationKind variants from verum_common
        let kinds = [
            CbgrViolationKind::UseAfterFree,
            CbgrViolationKind::DoubleFree,
            CbgrViolationKind::GenerationMismatch,
            CbgrViolationKind::EpochExpired,
            CbgrViolationKind::CapabilityDenied,
            CbgrViolationKind::InvalidReference,
            CbgrViolationKind::NullPointer,
            CbgrViolationKind::OutOfBounds,
        ];

        for kind in kinds {
            let _ = kind.to_string();
            let _ = format!("{:?}", kind);
            // Verify conversion to FFI error code works
            let code = kind.ffi_error_code();
            assert!((0x1001..=0x1008).contains(&code));
        }
    }

    #[test]
    fn test_error_is_error_trait() {
        fn assert_error<E: std::error::Error>(_: &E) {}

        let err = InterpreterError::DivisionByZero;
        assert_error(&err);
    }
}
