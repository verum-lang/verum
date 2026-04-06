/// Errors for operations involving alignment.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AlignmentError {
    #[error("{0} is not a power of two and cannot be used for alignment")]
    NonPowerOfTwo(u32),
    #[error("The src_align_bytes argument was not a power of two.")]
    SrcNonPowerOfTwo(u32),
    #[error("The dest_align_bytes argument was not a power of two.")]
    DestNonPowerOfTwo(u32),
    #[error(
        "Type is unsized and cannot be aligned. \
    Suggestion: Align memory manually."
    )]
    Unsized,
    #[error("Value is not an alloca, load, or store instruction.")]
    UnalignedInstruction,
}

/// The top-level Error type for the inkwell crate.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("Builder Error: {0}")]
    BuilderError(#[from] crate::builder::BuilderError),
    #[error("InstructionValue Error: {0}")]
    InstructionValueError(#[from] crate::values::InstructionValueError),
    #[error("Basic types must have names.")]
    EmptyNameError,
    #[error("Metadata is expected to be a node.")]
    GlobalMetadataError,
}

/// LLVM error types for code generation and lowering operations.
#[derive(Debug, thiserror::Error)]
pub enum LlvmError {
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
    #[error("Invalid function reference")]
    InvalidFunction,
    #[error("Invalid bitcode format")]
    InvalidBitcode,
    #[error("Target machine creation failed: {0}")]
    TargetMachine(String),
    #[error("Code generation failed: {0}")]
    CodeGen(String),
    #[error("LLVM error: {0}")]
    Llvm(String),
    #[error("Builder error: {0}")]
    Builder(#[from] crate::builder::BuilderError),
    #[error("LTO error: {0}")]
    LtoError(String),
}

/// Result type alias for LLVM operations.
pub type LlvmResult<T> = Result<T, LlvmError>;
