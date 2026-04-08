//! Protocol for Cross-Implementation Communication
//!
//! This module defines the protocol used for communication between
//! different Verum language implementations during cross-implementation
//! testing. It supports:
//!
//! - Execute program requests
//! - Capability queries
//! - Result reporting
//! - Version negotiation

use std::collections::HashMap;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

/// Protocol version
pub const PROTOCOL_VERSION: &str = "1.0.0";

/// A protocol message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Protocol version
    pub version: String,
    /// Message ID (for request-response correlation)
    pub id: u64,
    /// Message kind
    pub kind: MessageKind,
}

impl Message {
    /// Create a new request message
    pub fn request(id: u64, request: Request) -> Self {
        Self {
            version: PROTOCOL_VERSION.to_string(),
            id,
            kind: MessageKind::Request(request),
        }
    }

    /// Create a new response message
    pub fn response(id: u64, response: Response) -> Self {
        Self {
            version: PROTOCOL_VERSION.to_string(),
            id,
            kind: MessageKind::Response(response),
        }
    }
}

/// Message kind
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageKind {
    /// A request
    Request(Request),
    /// A response
    Response(Response),
    /// An event/notification
    Event(Event),
}

/// A request
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Request {
    /// Execute a program
    Execute(ExecuteRequest),
    /// Query capabilities
    Capability(CapabilityRequest),
    /// Query version
    Version(VersionRequest),
    /// Shutdown
    Shutdown,
}

/// A response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Response {
    /// Execute response
    Execute(ExecuteResponse),
    /// Capability response
    Capability(CapabilityResponse),
    /// Version response
    Version(VersionResponse),
    /// Error response
    Error(ErrorResponse),
    /// Acknowledgment (for shutdown, etc.)
    Ack,
}

/// An event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Event {
    /// Progress update
    Progress(ProgressEvent),
    /// Log message
    Log(LogEvent),
    /// Execution output
    Output(OutputEvent),
}

// =============================================================================
// Execute
// =============================================================================

/// Request to execute a program
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteRequest {
    /// Source code to execute
    pub source: String,
    /// Optional file path (for error messages)
    pub file_path: Option<PathBuf>,
    /// Input to provide to stdin
    pub stdin: Option<String>,
    /// Environment variables
    pub env: HashMap<String, String>,
    /// Arguments to pass to the program
    pub args: Vec<String>,
    /// Timeout in milliseconds
    pub timeout_ms: Option<u64>,
    /// Execution options
    pub options: ExecuteOptions,
}

/// Execution options
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecuteOptions {
    /// Whether to capture stdout
    pub capture_stdout: bool,
    /// Whether to capture stderr
    pub capture_stderr: bool,
    /// Whether to collect timing information
    pub collect_timing: bool,
    /// Whether to collect memory information
    pub collect_memory: bool,
    /// Whether to run in debug mode
    pub debug_mode: bool,
    /// Optimization level (0-3)
    pub optimization_level: u8,
}

/// Response to an execute request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteResponse {
    /// Whether execution succeeded
    pub success: bool,
    /// Exit code
    pub exit_code: Option<i32>,
    /// Standard output
    pub stdout: String,
    /// Standard error
    pub stderr: String,
    /// Execution time in milliseconds
    pub duration_ms: u64,
    /// Whether execution timed out
    pub timed_out: bool,
    /// Whether execution crashed
    pub crashed: bool,
    /// Signal that terminated the process
    pub signal: Option<i32>,
    /// Peak memory usage in bytes
    pub peak_memory: Option<u64>,
    /// Additional metadata
    pub metadata: HashMap<String, String>,
}

// =============================================================================
// Capability
// =============================================================================

/// Request for capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityRequest {
    /// Specific capabilities to query (empty = all)
    pub capabilities: Vec<String>,
}

/// Response with capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityResponse {
    /// Implementation name
    pub implementation: String,
    /// Implementation version
    pub version: String,
    /// Supported capabilities
    pub capabilities: Vec<Capability>,
    /// Supported language version
    pub language_version: String,
    /// Supported features
    pub features: Vec<String>,
    /// Limitations
    pub limitations: Vec<String>,
}

/// A capability
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capability {
    /// Capability name
    pub name: String,
    /// Whether it's supported
    pub supported: bool,
    /// Version (if applicable)
    pub version: Option<String>,
    /// Description
    pub description: Option<String>,
}

impl Capability {
    /// Create a supported capability
    pub fn supported(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            supported: true,
            version: None,
            description: None,
        }
    }

    /// Create an unsupported capability
    pub fn unsupported(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            supported: false,
            version: None,
            description: None,
        }
    }

    /// Add version
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Add description
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }
}

// =============================================================================
// Version
// =============================================================================

/// Version request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionRequest {
    /// Whether to include detailed version info
    pub detailed: bool,
}

/// Version response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionResponse {
    /// Implementation name
    pub implementation: String,
    /// Version string
    pub version: String,
    /// Git commit (if available)
    pub git_commit: Option<String>,
    /// Build date
    pub build_date: Option<String>,
    /// Rust version used to build
    pub rust_version: Option<String>,
    /// LLVM version (for AOT/JIT)
    pub llvm_version: Option<String>,
    /// Protocol version supported
    pub protocol_version: String,
}

// =============================================================================
// Error
// =============================================================================

/// Error response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    /// Error code
    pub code: ErrorCode,
    /// Error message
    pub message: String,
    /// Detailed error information
    pub details: Option<String>,
    /// Stack trace (if available)
    pub stack_trace: Option<String>,
}

/// Error codes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    /// Unknown error
    Unknown = 0,
    /// Invalid request
    InvalidRequest = 1,
    /// Parse error
    ParseError = 2,
    /// Type error
    TypeError = 3,
    /// Runtime error
    RuntimeError = 4,
    /// Timeout
    Timeout = 5,
    /// Out of memory
    OutOfMemory = 6,
    /// Unsupported feature
    UnsupportedFeature = 7,
    /// Internal error
    InternalError = 8,
    /// Protocol error
    ProtocolError = 9,
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErrorCode::Unknown => write!(f, "Unknown"),
            ErrorCode::InvalidRequest => write!(f, "InvalidRequest"),
            ErrorCode::ParseError => write!(f, "ParseError"),
            ErrorCode::TypeError => write!(f, "TypeError"),
            ErrorCode::RuntimeError => write!(f, "RuntimeError"),
            ErrorCode::Timeout => write!(f, "Timeout"),
            ErrorCode::OutOfMemory => write!(f, "OutOfMemory"),
            ErrorCode::UnsupportedFeature => write!(f, "UnsupportedFeature"),
            ErrorCode::InternalError => write!(f, "InternalError"),
            ErrorCode::ProtocolError => write!(f, "ProtocolError"),
        }
    }
}

// =============================================================================
// Events
// =============================================================================

/// Progress event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressEvent {
    /// Current progress (0-100)
    pub progress: u8,
    /// Description of current phase
    pub phase: String,
    /// Estimated time remaining in ms
    pub estimated_remaining_ms: Option<u64>,
}

/// Log event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEvent {
    /// Log level
    pub level: LogLevel,
    /// Message
    pub message: String,
    /// Timestamp
    pub timestamp: String,
    /// Source location
    pub location: Option<String>,
}

/// Log level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

/// Output event (for streaming output)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputEvent {
    /// Output stream
    pub stream: OutputStream,
    /// Output data
    pub data: String,
}

/// Output stream
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

// =============================================================================
// Protocol Trait
// =============================================================================

/// Protocol interface for implementation communication
pub trait Protocol: Send + Sync {
    /// Send a request and get a response
    fn request(&mut self, request: Request) -> Result<Response, ProtocolError>;

    /// Send an event (no response expected)
    fn send_event(&mut self, event: Event) -> Result<(), ProtocolError>;

    /// Check if the connection is alive
    fn is_alive(&self) -> bool;

    /// Close the connection
    fn close(&mut self) -> Result<(), ProtocolError>;
}

/// Protocol error
#[derive(Debug, Clone)]
pub struct ProtocolError {
    pub kind: ProtocolErrorKind,
    pub message: String,
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

impl std::error::Error for ProtocolError {}

/// Protocol error kinds
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolErrorKind {
    /// Connection failed
    ConnectionFailed,
    /// Connection closed
    ConnectionClosed,
    /// Send failed
    SendFailed,
    /// Receive failed
    ReceiveFailed,
    /// Invalid message
    InvalidMessage,
    /// Timeout
    Timeout,
    /// Version mismatch
    VersionMismatch,
}

impl std::fmt::Display for ProtocolErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtocolErrorKind::ConnectionFailed => write!(f, "ConnectionFailed"),
            ProtocolErrorKind::ConnectionClosed => write!(f, "ConnectionClosed"),
            ProtocolErrorKind::SendFailed => write!(f, "SendFailed"),
            ProtocolErrorKind::ReceiveFailed => write!(f, "ReceiveFailed"),
            ProtocolErrorKind::InvalidMessage => write!(f, "InvalidMessage"),
            ProtocolErrorKind::Timeout => write!(f, "Timeout"),
            ProtocolErrorKind::VersionMismatch => write!(f, "VersionMismatch"),
        }
    }
}

// =============================================================================
// Standard Capabilities
// =============================================================================

/// Standard capability names
pub mod capabilities {
    pub const ASYNC: &str = "async";
    pub const GENERICS: &str = "generics";
    pub const CLOSURES: &str = "closures";
    pub const PATTERN_MATCHING: &str = "pattern_matching";
    pub const REFINEMENT_TYPES: &str = "refinement_types";
    pub const CBGR: &str = "cbgr";
    pub const GRADUAL_VERIFICATION: &str = "gradual_verification";
    pub const CONTEXT_SYSTEM: &str = "context_system";
    pub const FFI: &str = "ffi";
    pub const MACROS: &str = "macros";
    pub const DEBUG_INFO: &str = "debug_info";
    pub const OPTIMIZATION: &str = "optimization";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_serialization() {
        let msg = Message::request(1, Request::Capability(CapabilityRequest {
            capabilities: vec!["async".to_string()],
        }));

        let json = serde_json::to_string(&msg).unwrap();
        let parsed: Message = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.id, 1);
        assert_eq!(parsed.version, PROTOCOL_VERSION);
    }

    #[test]
    fn test_capability_builder() {
        let cap = Capability::supported("async")
            .with_version("1.0")
            .with_description("Async/await support");

        assert!(cap.supported);
        assert_eq!(cap.version, Some("1.0".to_string()));
    }

    #[test]
    fn test_error_code_display() {
        assert_eq!(format!("{}", ErrorCode::ParseError), "ParseError");
        assert_eq!(format!("{}", ErrorCode::RuntimeError), "RuntimeError");
    }

    #[test]
    fn test_execute_options_default() {
        let opts = ExecuteOptions::default();
        assert!(!opts.debug_mode);
        assert_eq!(opts.optimization_level, 0);
    }
}
