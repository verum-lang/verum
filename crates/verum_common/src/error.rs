//! Unified Error Hierarchy
//!
//! Verum uses a unified error hierarchy with a single VerumError type that
//! categorizes errors into 5 levels: L0 (type prevention), L1 (static verification),
//! L2 (explicit handling - IO/parse/memory), L3 (fault tolerance - circuit breaker/retry),
//! L4 (security containment). Each error carries message, kind, location, context, and
//! optional backtrace. CBGR violations are a domain-specific subcategory (ErrorKind::Cbgr).
//!
//! This module provides a **single unified error type** ([`VerumError`]) that consolidates
//! all error types from across the Verum platform. This eliminates error type proliferation
//! while maintaining rich error context and diagnostic information.
//!
//! # Core Concept
//!
//! Instead of many different error types:
//! ```text
//! IoError, ParseError, NetworkError, DatabaseError, ...
//! ```
//!
//! Verum uses a single `VerumError` with categorized error kinds:
//! ```text
//! VerumError {
//!     message: "...",
//!     kind: ErrorKind::Parse,
//!     context: Some("..."),
//!     location: Some("src/main.rs:42:15"),
//! }
//! ```
//!
//! # Error Categories
//!
//! [`VerumError`] categorizes errors by kind:
//! - **Parse** - Parsing or lexing failed
//! - **Type** - Type checking failed
//! - **Verification** - SMT verification failed
//! - **Memory** - Memory-related error
//! - **Io** - File or network I/O
//! - **Network** - Network communication
//! - **Timeout** - Operation exceeded time limit
//! - **Invalid** - Invalid argument or state
//! - **NotFound** - Resource not found
//! - And more...
//!
//! # Rich Context
//!
//! Errors capture:
//! - **Message** - Human-readable description
//! - **Kind** - Error category for filtering/handling
//! - **Location** - Source code location (file:line:col)
//! - **Backtrace** - Full stack trace (when enabled)
//! - **Context chain** - Multi-level error context
//!
//! # Example
//!
//! ```rust,ignore
//! use verum_common::error::{VerumError, ErrorKind};
//!
//! // Create an error
//! let err = VerumError::new("Invalid input", ErrorKind::Invalid);
//!
//! // Add context
//! let err = err.with_context("Processing user input");
//!
//! // Add location
//! let err = err.with_location("src/main.rs", 42, 15);
//!
//! // Display with full information
//! eprintln!("{}", err);  // Shows message, kind, location, context
//! ```
//!
//! # Backtrace Support
//!
//! Backtraces are captured when `VERUM_BACKTRACE=1`:
//! ```text
//! error[E0001]: Invalid input (Processing user input)
//!    at src/main.rs:42:15
//!
//! Backtrace:
//!    0: parse_input
//!    1: main
//!    ...
//! ```

use crate::Text;
use core::fmt;

#[cfg(feature = "backtrace")]
use backtrace::Backtrace;

use serde::{Deserialize, Serialize};

/// Result type for Verum operations
pub type Result<T, E = VerumError> = core::result::Result<T, E>;

/// Unified error type for the Verum platform
///
/// This type consolidates all error categories from across the platform,
/// providing a single error type with rich context and diagnostic information.
///
/// # Examples
///
/// ```rust
/// use verum_common::error::{VerumError, ErrorKind};
///
/// let err = VerumError::new("out of bounds", ErrorKind::Memory);
/// assert_eq!(err.kind(), ErrorKind::Memory);
/// ```
#[derive(Debug, Serialize, Deserialize)]
pub struct VerumError {
    /// Error message
    message: Text,

    /// Error category
    kind: ErrorKind,

    /// Error location (file:line:col)
    #[serde(skip)]
    location: Option<ErrorLocation>,

    /// Additional context information
    #[serde(skip)]
    context: Option<Text>,

    /// Backtrace (only captured when VERUM_BACKTRACE=1)
    /// Note: Backtrace is not cloned when VerumError is cloned
    #[cfg(feature = "backtrace")]
    #[serde(skip)]
    backtrace: Option<Backtrace>,
}

// Manual Clone implementation to skip backtrace
impl Clone for VerumError {
    fn clone(&self) -> Self {
        Self {
            message: self.message.clone(),
            kind: self.kind,
            location: self.location.clone(),
            context: self.context.clone(),
            #[cfg(feature = "backtrace")]
            backtrace: None, // Don't clone backtrace
        }
    }
}

/// Error location information
#[derive(Debug, Clone)]
pub struct ErrorLocation {
    /// File path
    pub file: Text,
    /// Line number
    pub line: u32,
    /// Column number
    pub column: u32,
}

impl ErrorLocation {
    /// Create a new error location
    pub fn new(file: impl Into<Text>, line: u32, column: u32) -> Self {
        Self {
            file: file.into(),
            line,
            column,
        }
    }
}

impl fmt::Display for ErrorLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.file, self.line, self.column)
    }
}

/// Error categories aligned with 5-level architecture
///
/// Each kind maps to a specific level of the error handling system:
/// - Level 0: Type/Refinement/Context errors (prevented at compile-time)
/// - Level 1: Verification errors (SMT solver failures)
/// - Level 2: Runtime errors (I/O, network, parsing)
/// - Level 3: Fault tolerance errors (circuit breaker, supervision)
/// - Level 4: Security errors (isolation boundary violations)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ErrorKind {
    // Level 0: Type Prevention
    /// Type system error
    Type,
    /// Refinement constraint violation
    Refinement,
    /// Context requirement not satisfied
    Context,
    /// Affine type violation (use-after-move, double-free)
    Affine,

    // Level 1: Static Verification
    /// SMT verification failure
    Verification,
    /// Proof obligation not satisfied
    Proof,

    // Level 2: Explicit Handling
    /// I/O error
    IO,
    /// Parse error
    Parse,
    /// Lexical analysis error
    Lex,
    /// Memory error (allocation, bounds)
    Memory,
    /// Concurrency error (deadlock, race condition)
    Concurrency,
    /// Network error
    Network,
    /// Database error
    Database,

    // Level 3: Fault Tolerance
    /// Circuit breaker open
    CircuitOpen,
    /// Retry limit exceeded
    RetryExhausted,
    /// Supervision tree failure
    Supervision,
    /// Timeout error
    Timeout,

    // Level 4: Security Containment
    /// Security/authorization error
    Security,
    /// Capability violation
    Capability,
    /// Sandbox escape attempt
    Sandbox,

    // Domain-specific errors
    /// Module system error
    Module,
    /// Code generation error
    Codegen,
    /// CBGR error (use-after-free, generation mismatch)
    Cbgr,
    /// CLI error
    Cli,

    // Cross-cutting
    /// Configuration error
    Config,
    /// Invalid state
    InvalidState,
    /// Not found error
    NotFound,
    /// Not implemented
    NotImplemented,
    /// Other/unknown error
    Other,
}

impl ErrorKind {
    /// Get the level this error kind belongs to
    pub fn level(&self) -> u8 {
        match self {
            ErrorKind::Type | ErrorKind::Refinement | ErrorKind::Context | ErrorKind::Affine => 0,
            ErrorKind::Verification | ErrorKind::Proof => 1,
            ErrorKind::IO
            | ErrorKind::Parse
            | ErrorKind::Lex
            | ErrorKind::Memory
            | ErrorKind::Concurrency
            | ErrorKind::Network
            | ErrorKind::Database => 2,
            ErrorKind::CircuitOpen
            | ErrorKind::RetryExhausted
            | ErrorKind::Supervision
            | ErrorKind::Timeout => 3,
            ErrorKind::Security | ErrorKind::Capability | ErrorKind::Sandbox => 4,
            _ => 2, // Default to Level 2 (runtime)
        }
    }

    /// Check if this error is recoverable
    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            ErrorKind::IO
                | ErrorKind::Network
                | ErrorKind::Database
                | ErrorKind::Timeout
                | ErrorKind::CircuitOpen
                | ErrorKind::RetryExhausted
        )
    }

    /// Check if this error indicates a fatal condition
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            ErrorKind::Memory | ErrorKind::Affine | ErrorKind::Sandbox
        )
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorKind::Type => write!(f, "Type"),
            ErrorKind::Refinement => write!(f, "Refinement"),
            ErrorKind::Context => write!(f, "Context"),
            ErrorKind::Affine => write!(f, "Affine"),
            ErrorKind::Verification => write!(f, "Verification"),
            ErrorKind::Proof => write!(f, "Proof"),
            ErrorKind::IO => write!(f, "I/O"),
            ErrorKind::Parse => write!(f, "Parse"),
            ErrorKind::Lex => write!(f, "Lex"),
            ErrorKind::Memory => write!(f, "Memory"),
            ErrorKind::Concurrency => write!(f, "Concurrency"),
            ErrorKind::Network => write!(f, "Network"),
            ErrorKind::Database => write!(f, "Database"),
            ErrorKind::CircuitOpen => write!(f, "CircuitOpen"),
            ErrorKind::RetryExhausted => write!(f, "RetryExhausted"),
            ErrorKind::Supervision => write!(f, "Supervision"),
            ErrorKind::Timeout => write!(f, "Timeout"),
            ErrorKind::Security => write!(f, "Security"),
            ErrorKind::Capability => write!(f, "Capability"),
            ErrorKind::Sandbox => write!(f, "Sandbox"),
            ErrorKind::Module => write!(f, "Module"),
            ErrorKind::Codegen => write!(f, "Codegen"),
            ErrorKind::Cbgr => write!(f, "CBGR"),
            ErrorKind::Cli => write!(f, "CLI"),
            ErrorKind::Config => write!(f, "Config"),
            ErrorKind::InvalidState => write!(f, "InvalidState"),
            ErrorKind::NotFound => write!(f, "NotFound"),
            ErrorKind::NotImplemented => write!(f, "NotImplemented"),
            ErrorKind::Other => write!(f, "Other"),
        }
    }
}

impl VerumError {
    /// Create a new error with message and kind
    pub fn new(message: impl Into<Text>, kind: ErrorKind) -> Self {
        Self {
            message: message.into(),
            kind,
            location: None,
            context: None,
            #[cfg(feature = "backtrace")]
            backtrace: if core::option_env!("VERUM_BACKTRACE") == Some("1") {
                Some(Backtrace::new())
            } else {
                None
            },
        }
    }

    /// Create error with location information
    pub fn with_location(
        message: impl Into<Text>,
        kind: ErrorKind,
        location: ErrorLocation,
    ) -> Self {
        let mut err = Self::new(message, kind);
        err.location = Some(location);
        err
    }

    /// Add location to existing error
    pub fn at_location(mut self, file: impl Into<Text>, line: u32, column: u32) -> Self {
        self.location = Some(ErrorLocation::new(file, line, column));
        self
    }

    /// Add context to existing error
    pub fn with_context(mut self, context: impl Into<Text>) -> Self {
        self.context = Some(context.into());
        self
    }

    /// Get the error kind
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Get the error message
    pub fn message(&self) -> &Text {
        &self.message
    }

    /// Get the error location
    pub fn location(&self) -> Option<&ErrorLocation> {
        self.location.as_ref()
    }

    /// Get the error context
    pub fn get_context(&self) -> Option<&Text> {
        self.context.as_ref()
    }

    /// Get the backtrace if available
    #[cfg(feature = "backtrace")]
    pub fn backtrace(&self) -> Option<&Backtrace> {
        self.backtrace.as_ref()
    }

    // Convenience constructors for common error kinds

    /// Create a type error
    pub fn type_error(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::Type)
    }

    /// Create a refinement error
    pub fn refinement(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::Refinement)
    }

    /// Create a context error
    pub fn context(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::Context)
    }

    /// Create an affine error
    pub fn affine(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::Affine)
    }

    /// Create a verification error
    pub fn verification(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::Verification)
    }

    /// Create a proof error
    pub fn proof(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::Proof)
    }

    /// Create an I/O error
    pub fn io(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::IO)
    }

    /// Create a parse error
    pub fn parse(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::Parse)
    }

    /// Create a lex error
    pub fn lex(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::Lex)
    }

    /// Create a memory error
    pub fn memory(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::Memory)
    }

    /// Create a concurrency error
    pub fn concurrency(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::Concurrency)
    }

    /// Create a network error
    pub fn network(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::Network)
    }

    /// Create a database error
    pub fn database(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::Database)
    }

    /// Create a circuit breaker error
    pub fn circuit_open(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::CircuitOpen)
    }

    /// Create a retry exhausted error
    pub fn retry_exhausted(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::RetryExhausted)
    }

    /// Create a supervision error
    pub fn supervision(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::Supervision)
    }

    /// Create a timeout error
    pub fn timeout(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::Timeout)
    }

    /// Create a security error
    pub fn security(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::Security)
    }

    /// Create a capability error
    pub fn capability(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::Capability)
    }

    /// Create a sandbox error
    pub fn sandbox(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::Sandbox)
    }

    /// Create a module error
    pub fn module(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::Module)
    }

    /// Create a codegen error
    pub fn codegen(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::Codegen)
    }

    /// Create a CBGR error
    pub fn cbgr(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::Cbgr)
    }

    /// Create a CLI error
    pub fn cli(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::Cli)
    }

    /// Create a config error
    pub fn config(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::Config)
    }

    /// Create an invalid state error
    pub fn invalid_state(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::InvalidState)
    }

    /// Create a not found error
    pub fn not_found(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::NotFound)
    }

    /// Create a not implemented error
    pub fn not_implemented(feature: impl Into<Text>) -> Self {
        let feature_text: Text = feature.into();
        let message: Text = Text::from(format!("Not implemented: {}", feature_text.as_str()));
        Self::new(message, ErrorKind::NotImplemented)
    }

    /// Create an other error
    pub fn other(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::Other)
    }

    // CBGR-specific constructors

    /// Create a use-after-free error (for CBGR)
    pub fn use_after_free(
        expected_gen: u32,
        actual_gen: u32,
        expected_epoch: u16,
        actual_epoch: u16,
        type_name: &str,
        gen_unallocated: u32,
    ) -> Self {
        if expected_gen == gen_unallocated {
            Self::cbgr(format!("null pointer dereference: type={}", type_name))
        } else if expected_epoch != actual_epoch {
            Self::cbgr(format!(
                "epoch mismatch: expected epoch={}, actual epoch={}, gen={}, type={}",
                expected_epoch, actual_epoch, expected_gen, type_name
            ))
        } else {
            Self::cbgr(format!(
                "use-after-free: expected gen={}, actual gen={}, epoch={}, type={}",
                expected_gen, actual_gen, expected_epoch, type_name
            ))
        }
    }

    /// Create a generation mismatch error (for CBGR)
    pub fn generation_mismatch(expected: u32, actual: u32, type_name: &str) -> Self {
        Self::cbgr(format!(
            "generation mismatch: expected={}, actual={}, type={}",
            expected, actual, type_name
        ))
    }

    /// Create an epoch mismatch error (for CBGR)
    pub fn epoch_mismatch(expected: u16, actual: u16, type_name: &str) -> Self {
        Self::cbgr(format!(
            "epoch mismatch: expected={}, actual={}, type={}",
            expected, actual, type_name
        ))
    }
}

impl fmt::Display for VerumError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.kind, self.message)?;

        if let Some(context) = &self.context {
            write!(f, " ({})", context)?;
        }

        if let Some(location) = &self.location {
            write!(f, " at {}", location)?;
        }

        Ok(())
    }
}

#[cfg(feature = "std")]
impl std::error::Error for VerumError {}

// Conversions from standard library errors
#[cfg(feature = "std")]
impl From<std::io::Error> for VerumError {
    fn from(err: std::io::Error) -> Self {
        Self::io(err.to_string())
    }
}

impl From<fmt::Error> for VerumError {
    fn from(err: fmt::Error) -> Self {
        Self::new(err.to_string(), ErrorKind::Other)
    }
}

#[cfg(feature = "anyhow-compat")]
impl From<anyhow::Error> for VerumError {
    fn from(err: anyhow::Error) -> Self {
        Self::new(err.to_string(), ErrorKind::Other)
    }
}

// Helper functions for formatting lists and cycles
// Re-export from formatting module for backward compatibility
pub use crate::formatting::{format_cycle_str as format_cycle, format_list_str as format_list};

// =============================================================================
// CBGR Violation Kinds - Single Source of Truth
// =============================================================================
//
// This enum is the ONLY definition of CBGR violation kinds in the codebase.
// All crates must use this definition via `verum_common::CbgrViolationKind`.
//
// CBGR Violation Kinds — single source of truth for all memory safety violations
// detected by the CBGR (Capability-Based Generation References) system.
// All crates must use this definition via `verum_common::CbgrViolationKind`.
// FFI error codes are in 0x1000-0x10FF range. Tier behavior:
//   Tier 0 (Interpreter): Runtime validation ~100ns
//   Tier 1-2 (JIT): Inline checks with escape analysis ~5-15ns
//   Tier 3 (AOT): Static elimination where provable, 0ns

/// CBGR (Capability-Based Generation References) violation kinds.
///
/// This is the single source of truth for all memory safety violations
/// detected by the CBGR system across all execution tiers.
///
/// # Tier Behavior
///
/// | Tier | Check Method | Overhead |
/// |------|--------------|----------|
/// | 0 (Interpreter) | Runtime validation | ~100ns |
/// | 1-2 (JIT) | Inline checks with escape analysis | ~5-15ns |
/// | 3 (AOT) | Static elimination where provable | 0ns |
///
/// # Error Codes
///
/// Each variant maps to a unique FFI error code in the 0x1000-0x10FF range:
/// - `UseAfterFree` → 0x1001
/// - `DoubleFree` → 0x1002
/// - `GenerationMismatch` → 0x1003
/// - `EpochExpired` → 0x1004
/// - `CapabilityDenied` → 0x1005
/// - `InvalidReference` → 0x1006
/// - `NullPointer` → 0x1007
/// - `OutOfBounds` → 0x1008
///
/// # Examples
///
/// ```rust
/// use verum_common::CbgrViolationKind;
///
/// let violation = CbgrViolationKind::UseAfterFree;
/// assert!(violation.is_fatal());
/// assert_eq!(violation.ffi_error_code(), 0x1001);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum CbgrViolationKind {
    /// Reference used after the object was deallocated.
    ///
    /// This is the most common memory safety error, caught when:
    /// - The generation counter doesn't match the expected value
    /// - The object has been explicitly freed
    UseAfterFree = 0,

    /// Attempt to free an object that was already freed.
    ///
    /// Detected when:
    /// - Deallocation is called with an already-invalidated generation
    /// - The same pointer is freed multiple times
    DoubleFree = 1,

    /// Reference generation doesn't match current object generation.
    ///
    /// This indicates the reference is stale - the object was reallocated
    /// and a new generation was assigned.
    GenerationMismatch = 2,

    /// Reference epoch is older than current runtime epoch.
    ///
    /// Epochs provide coarse-grained temporal safety. When the runtime
    /// advances its epoch (e.g., during GC), all references from
    /// previous epochs become invalid.
    EpochExpired = 3,

    /// Operation requires a capability the reference doesn't have.
    ///
    /// CBGR references carry capability bits (read, write, execute, etc.).
    /// This violation occurs when code attempts an operation not permitted
    /// by the reference's capabilities.
    CapabilityDenied = 4,

    /// Reference is structurally invalid (corrupted or uninitialized).
    ///
    /// This catches:
    /// - Uninitialized references
    /// - References with impossible combinations of fields
    /// - References to deallocated stack frames
    InvalidReference = 5,

    /// Attempt to dereference a null pointer.
    ///
    /// While Verum's type system prevents most null pointers,
    /// FFI boundaries and unsafe code can still produce them.
    NullPointer = 6,

    /// Access beyond the bounds of an allocation.
    ///
    /// For fat references (slices, arrays), this catches
    /// out-of-bounds indexing that would access unallocated memory.
    OutOfBounds = 7,
}

impl CbgrViolationKind {
    /// Check if this violation represents a fatal memory safety error.
    ///
    /// Fatal violations should never be ignored or recovered from,
    /// as they indicate fundamental memory corruption.
    #[inline]
    pub const fn is_fatal(&self) -> bool {
        matches!(
            self,
            Self::UseAfterFree
                | Self::DoubleFree
                | Self::NullPointer
                | Self::InvalidReference
        )
    }

    /// Check if this violation is recoverable.
    ///
    /// Some violations (like capability denial) indicate policy violations
    /// rather than memory corruption, and may be recoverable.
    #[inline]
    pub const fn is_recoverable(&self) -> bool {
        matches!(self, Self::CapabilityDenied | Self::OutOfBounds)
    }

    /// Get the FFI error code for this violation kind.
    ///
    /// Returns a code in the 0x1000-0x10FF range for CBGR errors.
    #[inline]
    pub const fn ffi_error_code(&self) -> u32 {
        0x1000 + (*self as u32) + 1
    }

    /// Create from an FFI error code.
    ///
    /// Returns `None` if the code is not a valid CBGR error code.
    #[inline]
    pub const fn from_ffi_error_code(code: u32) -> Option<Self> {
        if code < 0x1001 || code > 0x1008 {
            return None;
        }
        Some(match code - 0x1001 {
            0 => Self::UseAfterFree,
            1 => Self::DoubleFree,
            2 => Self::GenerationMismatch,
            3 => Self::EpochExpired,
            4 => Self::CapabilityDenied,
            5 => Self::InvalidReference,
            6 => Self::NullPointer,
            7 => Self::OutOfBounds,
            _ => return None,
        })
    }

    /// Get a human-readable description of this violation.
    #[inline]
    pub const fn description(&self) -> &'static str {
        match self {
            Self::UseAfterFree => "use-after-free: reference used after deallocation",
            Self::DoubleFree => "double-free: object freed multiple times",
            Self::GenerationMismatch => "generation mismatch: stale reference to reallocated object",
            Self::EpochExpired => "epoch expired: reference from previous runtime epoch",
            Self::CapabilityDenied => "capability denied: operation not permitted by reference",
            Self::InvalidReference => "invalid reference: corrupted or uninitialized reference",
            Self::NullPointer => "null pointer: attempt to dereference null",
            Self::OutOfBounds => "out of bounds: access beyond allocation bounds",
        }
    }

    /// Get the short name for this violation (for error messages).
    #[inline]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::UseAfterFree => "UseAfterFree",
            Self::DoubleFree => "DoubleFree",
            Self::GenerationMismatch => "GenerationMismatch",
            Self::EpochExpired => "EpochExpired",
            Self::CapabilityDenied => "CapabilityDenied",
            Self::InvalidReference => "InvalidReference",
            Self::NullPointer => "NullPointer",
            Self::OutOfBounds => "OutOfBounds",
        }
    }
}

impl fmt::Display for CbgrViolationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.description())
    }
}

/// Detailed CBGR violation with context information.
///
/// This struct wraps `CbgrViolationKind` with additional diagnostic
/// information useful for debugging and error reporting.
///
/// # Examples
///
/// ```rust
/// use verum_common::{CbgrViolation, CbgrViolationKind};
///
/// let violation = CbgrViolation::new(
///     CbgrViolationKind::UseAfterFree,
///     0xDEADBEEF,
/// ).with_generation(42, 100)
///  .with_epoch(1, 2);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CbgrViolation {
    /// The kind of violation
    pub kind: CbgrViolationKind,
    /// The pointer address involved (if available)
    pub pointer: usize,
    /// Expected generation counter (if applicable)
    pub expected_generation: Option<u32>,
    /// Actual generation counter found (if applicable)
    pub actual_generation: Option<u32>,
    /// Expected epoch (if applicable)
    pub expected_epoch: Option<u16>,
    /// Actual epoch found (if applicable)
    pub actual_epoch: Option<u16>,
    /// Type name of the referenced object (if known)
    pub type_name: Option<Text>,
}

impl CbgrViolation {
    /// Create a new CBGR violation.
    #[inline]
    pub fn new(kind: CbgrViolationKind, pointer: usize) -> Self {
        Self {
            kind,
            pointer,
            expected_generation: None,
            actual_generation: None,
            expected_epoch: None,
            actual_epoch: None,
            type_name: None,
        }
    }

    /// Add generation information.
    #[inline]
    pub fn with_generation(mut self, expected: u32, actual: u32) -> Self {
        self.expected_generation = Some(expected);
        self.actual_generation = Some(actual);
        self
    }

    /// Add epoch information.
    #[inline]
    pub fn with_epoch(mut self, expected: u16, actual: u16) -> Self {
        self.expected_epoch = Some(expected);
        self.actual_epoch = Some(actual);
        self
    }

    /// Add type name information.
    #[inline]
    pub fn with_type_name(mut self, name: impl Into<Text>) -> Self {
        self.type_name = Some(name.into());
        self
    }

    /// Convert to a VerumError.
    #[inline]
    pub fn to_error(&self) -> VerumError {
        VerumError::new(self.to_string(), ErrorKind::Cbgr)
    }
}

impl fmt::Display for CbgrViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CBGR violation: {}", self.kind.name())?;
        write!(f, " at 0x{:016x}", self.pointer)?;

        if let (Some(expected), Some(actual)) = (self.expected_generation, self.actual_generation) {
            write!(f, " (gen: expected={}, actual={})", expected, actual)?;
        }

        if let (Some(expected), Some(actual)) = (self.expected_epoch, self.actual_epoch) {
            write!(f, " (epoch: expected={}, actual={})", expected, actual)?;
        }

        if let Some(ref type_name) = self.type_name {
            write!(f, " [type: {}]", type_name)?;
        }

        Ok(())
    }
}

impl From<CbgrViolation> for VerumError {
    fn from(violation: CbgrViolation) -> Self {
        violation.to_error()
    }
}

impl From<CbgrViolationKind> for VerumError {
    fn from(kind: CbgrViolationKind) -> Self {
        VerumError::new(kind.description(), ErrorKind::Cbgr)
    }
}
