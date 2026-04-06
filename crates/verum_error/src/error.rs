//! Unified Error Hierarchy
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
//! use verum_error::{VerumError, ErrorKind};
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
//!
//! This module provides a unified error type that consolidates all error types
//! from across the Verum platform into a single, composable hierarchy.

use std::fmt;
use verum_common::Text;

#[cfg(feature = "backtrace")]
use backtrace::Backtrace;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Result type for Verum operations
pub type Result<T, E = VerumError> = std::result::Result<T, E>;

/// Unified error type for the Verum platform
///
/// This type consolidates all error categories from across the platform,
/// providing a single error type with rich context and diagnostic information.
///
/// # Examples
///
/// ```rust
/// use verum_error::{VerumError, ErrorKind};
///
/// let err = VerumError::new("out of bounds", ErrorKind::Memory);
/// assert_eq!(err.kind(), ErrorKind::Memory);
/// ```
#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct VerumError {
    /// Error message
    message: Text,

    /// Error category
    kind: ErrorKind,

    /// Error location (file:line:col)
    #[cfg_attr(feature = "serde", serde(skip))]
    location: Option<ErrorLocation>,

    /// Backtrace (only captured when VERUM_BACKTRACE=1)
    /// Note: Backtrace is not cloned when VerumError is cloned
    #[cfg(feature = "backtrace")]
    #[cfg_attr(feature = "serde", serde(skip))]
    backtrace: Option<Backtrace>,
}

// Manual Clone implementation to skip backtrace
impl Clone for VerumError {
    fn clone(&self) -> Self {
        Self {
            message: self.message.clone(),
            kind: self.kind,
            location: self.location.clone(),
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
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

    // Cross-cutting
    /// Code generation error
    Codegen,
    /// Configuration error
    Config,
    /// Invalid state
    InvalidState,
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
            ErrorKind::Codegen => write!(f, "Codegen"),
            ErrorKind::Config => write!(f, "Config"),
            ErrorKind::InvalidState => write!(f, "InvalidState"),
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
            #[cfg(feature = "backtrace")]
            backtrace: if std::env::var("VERUM_BACKTRACE").as_deref() == Ok("1") {
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

    /// Create a verification error
    pub fn verification(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::Verification)
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

    /// Create a codegen error
    pub fn codegen(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::Codegen)
    }

    /// Create a config error
    pub fn config(message: impl Into<Text>) -> Self {
        Self::new(message, ErrorKind::Config)
    }

    /// Create a not implemented error
    pub fn not_implemented(feature: impl Into<Text>) -> Self {
        let feature_text: Text = feature.into();
        let message: Text = format!("Not implemented: {}", feature_text.as_str()).into();
        Self::new(message, ErrorKind::NotImplemented)
    }
}

impl fmt::Display for VerumError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.kind, self.message)?;

        if let Some(location) = &self.location {
            write!(f, " at {}", location)?;
        }

        Ok(())
    }
}

impl std::error::Error for VerumError {}

// Conversions from standard library errors
impl From<std::io::Error> for VerumError {
    fn from(err: std::io::Error) -> Self {
        Self::io(err.to_string())
    }
}

impl From<std::fmt::Error> for VerumError {
    fn from(err: std::fmt::Error) -> Self {
        Self::new(err.to_string(), ErrorKind::Other)
    }
}

#[cfg(feature = "anyhow-compat")]
impl From<anyhow::Error> for VerumError {
    fn from(err: anyhow::Error) -> Self {
        Self::new(err.to_string(), ErrorKind::Other)
    }
}
