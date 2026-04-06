//! VBC error types.
//!
//! This module defines all error types used throughout the VBC crate,
//! covering format parsing, validation, serialization, and interpretation.

use thiserror::Error;

use crate::{ConstId, FunctionId, StringId, TypeId};

/// Result type alias for VBC operations.
pub type VbcResult<T> = Result<T, VbcError>;

/// Errors that can occur during VBC operations.
#[derive(Debug, Error)]
pub enum VbcError {
    // === Format Errors ===
    /// Invalid magic number in VBC file.
    #[error("invalid magic number: expected 'VBC1', got {0:?}")]
    InvalidMagic([u8; 4]),

    /// Unsupported VBC version.
    #[error("unsupported VBC version: {major}.{minor} (supported: {supported_major}.{supported_minor})")]
    UnsupportedVersion {
        /// File major version.
        major: u16,
        /// File minor version.
        minor: u16,
        /// Supported major version.
        supported_major: u16,
        /// Supported minor version.
        supported_minor: u16,
    },

    /// Invalid header field.
    #[error("invalid header: {field} at offset {offset:#x}")]
    InvalidHeader {
        /// Field name that is invalid.
        field: &'static str,
        /// Byte offset in header.
        offset: usize,
    },

    /// Section offset out of bounds.
    #[error("section '{section}' offset {offset:#x} exceeds file size {file_size:#x}")]
    SectionOutOfBounds {
        /// Section name.
        section: &'static str,
        /// Section offset.
        offset: u32,
        /// File size.
        file_size: usize,
    },

    /// Section size overflow.
    #[error("section '{section}' at {offset:#x} with size {size:#x} overflows file")]
    SectionOverflow {
        /// Section name.
        section: &'static str,
        /// Section offset.
        offset: u32,
        /// Section size.
        size: u32,
    },

    // === Type Errors ===
    /// Invalid type ID reference.
    #[error("invalid type reference: TypeId({0})")]
    InvalidTypeId(u32),

    /// Invalid type param ID reference.
    #[error("invalid type parameter reference: TypeParamId({0})")]
    InvalidTypeParamId(u32),

    /// Invalid type kind tag.
    #[error("invalid type kind tag: {0:#x}")]
    InvalidTypeKind(u8),

    /// Invalid type ref tag.
    #[error("invalid TypeRef tag: {0:#x}")]
    InvalidTypeRefTag(u8),

    /// Invalid TypeRef discriminant during decoding.
    #[error("invalid TypeRef discriminant {discriminant:#x} at offset {offset:#x}")]
    InvalidTypeRef {
        /// Byte offset where error occurred.
        offset: u32,
        /// Invalid discriminant byte.
        discriminant: u8,
    },

    /// Circular type definition detected.
    #[error("circular type definition: {0:?}")]
    CircularType(TypeId),

    // === Function Errors ===
    /// Invalid function ID reference.
    #[error("invalid function reference: FunctionId({0})")]
    InvalidFunctionId(u32),

    /// Invalid bytecode offset for function.
    #[error("function {func:?} bytecode offset {offset:#x} exceeds bytecode section size {size:#x}")]
    InvalidBytecodeOffset {
        /// Function ID.
        func: FunctionId,
        /// Claimed offset.
        offset: u32,
        /// Bytecode section size.
        size: u32,
    },

    /// Insufficient registers for function.
    #[error("function {func:?} needs {needed} registers but declared {declared}")]
    InsufficientRegisters {
        /// Function ID.
        func: FunctionId,
        /// Registers needed by bytecode.
        needed: u16,
        /// Registers declared in descriptor.
        declared: u16,
    },

    // === Instruction Errors ===
    /// Invalid opcode.
    #[error("invalid opcode: {0:#x}")]
    InvalidOpcode(u8),

    /// Register out of bounds.
    #[error("register r{reg} out of bounds (max: {max}) in {context}")]
    RegisterOutOfBounds {
        /// Register index.
        reg: u16,
        /// Maximum allowed.
        max: u16,
        /// Context (function name, instruction).
        context: String,
    },

    /// Invalid instruction encoding.
    #[error("invalid instruction encoding at offset {offset:#x}: {reason}")]
    InvalidInstructionEncoding {
        /// Bytecode offset.
        offset: usize,
        /// Reason for failure.
        reason: String,
    },

    /// Jump target out of bounds.
    #[error("jump target {target:#x} out of bounds (max: {max:#x}) at offset {offset:#x}")]
    JumpOutOfBounds {
        /// Jump target offset.
        target: i32,
        /// Maximum valid offset.
        max: u32,
        /// Source instruction offset.
        offset: u32,
    },

    // === Constant Pool Errors ===
    /// Invalid constant ID reference.
    #[error("invalid constant reference: ConstId({0})")]
    InvalidConstId(u32),

    /// Invalid constant tag.
    #[error("invalid constant tag: {0:#x}")]
    InvalidConstantTag(u8),

    // === String Table Errors ===
    /// Invalid string ID reference.
    #[error("invalid string reference: StringId({0})")]
    InvalidStringId(u32),

    /// Invalid UTF-8 in string table.
    #[error("invalid UTF-8 in string at offset {offset:#x}: {error}")]
    InvalidUtf8 {
        /// Offset in string table.
        offset: u32,
        /// UTF-8 error.
        error: std::string::FromUtf8Error,
    },

    // === Encoding Errors ===
    /// Unexpected end of data during decoding.
    #[error("unexpected end of data at offset {offset:#x}, expected {expected} more bytes")]
    UnexpectedEof {
        /// Current offset.
        offset: usize,
        /// Bytes expected.
        expected: usize,
    },

    /// VarInt overflow (value too large).
    #[error("VarInt overflow at offset {offset:#x}: value exceeds 64 bits")]
    VarIntOverflow {
        /// Offset where overflow occurred.
        offset: usize,
    },

    // === Validation Errors ===
    /// Content hash mismatch.
    #[error("content hash mismatch: expected {expected:#x}, computed {computed:#x}")]
    ContentHashMismatch {
        /// Expected hash from header.
        expected: u64,
        /// Computed hash.
        computed: u64,
    },

    /// Dependency hash mismatch.
    #[error("dependency hash mismatch: expected {expected:#x}, computed {computed:#x}")]
    DependencyHashMismatch {
        /// Expected hash from header.
        expected: u64,
        /// Computed hash.
        computed: u64,
    },

    /// Multiple validation errors.
    #[error("validation failed with {} errors", .0.len())]
    MultipleErrors(Vec<VbcError>),

    // === I/O Errors ===
    /// I/O error during read/write.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    // === Compression Errors ===
    /// Compression failed.
    #[error("compression failed: {0}")]
    Compression(String),

    /// Decompression failed.
    #[error("decompression failed: {0}")]
    Decompression(String),

    /// Unknown compression algorithm.
    #[error("unknown compression algorithm: {0}")]
    UnknownCompression(u8),

    // === Specialization Errors ===
    /// Invalid specialization entry.
    #[error("invalid specialization entry: {reason}")]
    InvalidSpecialization {
        /// Reason for invalidity.
        reason: String,
    },

    // === Protocol Errors ===
    /// Invalid protocol ID reference.
    #[error("invalid protocol reference: ProtocolId({0})")]
    InvalidProtocolId(u32),

    // === Context Errors ===
    /// Invalid context reference.
    #[error("invalid context reference: ContextRef({0})")]
    InvalidContextRef(u32),

    // === Archive Errors ===
    /// Archive validation error.
    #[error("archive error: {0}")]
    ArchiveError(String),

    // === Serialization/Deserialization Errors ===
    /// Serialization error (e.g., bincode).
    #[error("serialization error: {0}")]
    Serialization(String),

    /// Deserialization error (e.g., bincode).
    #[error("deserialization error: {0}")]
    Deserialization(String),
}

impl VbcError {
    /// Creates an invalid type ID error.
    pub fn invalid_type(id: TypeId) -> Self {
        VbcError::InvalidTypeId(id.0)
    }

    /// Creates an invalid function ID error.
    pub fn invalid_function(id: FunctionId) -> Self {
        VbcError::InvalidFunctionId(id.0)
    }

    /// Creates an invalid constant ID error.
    pub fn invalid_const(id: ConstId) -> Self {
        VbcError::InvalidConstId(id.0)
    }

    /// Creates an invalid string ID error.
    pub fn invalid_string(id: StringId) -> Self {
        VbcError::InvalidStringId(id.0)
    }

    /// Creates an unexpected EOF error.
    pub fn eof(offset: usize, expected: usize) -> Self {
        VbcError::UnexpectedEof { offset, expected }
    }

    /// Checks if this is a validation error.
    pub fn is_validation_error(&self) -> bool {
        matches!(
            self,
            VbcError::InvalidTypeId(_)
                | VbcError::InvalidFunctionId(_)
                | VbcError::InvalidConstId(_)
                | VbcError::InvalidStringId(_)
                | VbcError::CircularType(_)
                | VbcError::ContentHashMismatch { .. }
                | VbcError::DependencyHashMismatch { .. }
                | VbcError::MultipleErrors(_)
        )
    }

    /// Checks if this is a format error (parse-time).
    pub fn is_format_error(&self) -> bool {
        matches!(
            self,
            VbcError::InvalidMagic(_)
                | VbcError::UnsupportedVersion { .. }
                | VbcError::InvalidHeader { .. }
                | VbcError::SectionOutOfBounds { .. }
                | VbcError::SectionOverflow { .. }
        )
    }
}
