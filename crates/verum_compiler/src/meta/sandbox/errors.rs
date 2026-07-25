//! Sandbox Error Types
//!
//! Defines errors and operations for the meta sandbox system.
//!
//! ## Error Codes
//!
//! Sandbox errors use the M3XX range:
//! - M310-M319: I/O violations (file, network, process)
//! - M320-M329: Resource limits (memory, time, iterations)
//! - M330-M339: Forbidden operations (FFI, unsafe, env)
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).
//! Meta context unification: all compile-time features desugar to meta-system
//! operations, providing one coherent model with convenient syntax sugar.

use verum_common::{List, Text};

/// Operations that can be performed in meta functions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operation {
    /// Basic arithmetic operations (+, -, *, /, %)
    Arithmetic,

    /// Comparison operations (==, !=, <, >, <=, >=)
    Comparison,

    /// Logical operations (&&, ||, !)
    Logic,

    /// Array operations (indexing, length, iteration)
    ArrayOps,

    /// Text operations (concatenation, substring, length)
    StringOps,

    /// Control flow (if, while, for)
    ControlFlow,

    /// Function calls (to other meta functions)
    FunctionCall,

    /// Type operations (typeof, is)
    TypeOps,

    /// AST manipulation (quote, unquote)
    ASTOps,

    /// Pattern matching
    PatternMatch,
}

/// Errors that can occur during sandbox enforcement
#[derive(Debug, Clone)]
pub enum SandboxError {
    /// An I/O operation was attempted (file system)
    FileSystemViolation {
        operation: Text,
        function: Text,
        context: Text,
    },

    /// A network operation was attempted
    NetworkViolation {
        operation: Text,
        function: Text,
        context: Text,
    },

    /// A process operation was attempted
    ProcessViolation { operation: Text, function: Text },

    /// A non-deterministic time operation was attempted
    TimeViolation { operation: Text, function: Text },

    /// Environment variable access was attempted
    EnvViolation { operation: Text, function: Text },

    /// Non-deterministic random operation attempted
    RandomViolation { operation: Text, function: Text },

    /// Unsafe memory operation attempted
    UnsafeViolation { operation: Text, function: Text },

    /// FFI call attempted
    FFIViolation { operation: Text, function: Text },

    /// Asset loading without `using BuildAssets` context
    AssetLoadingNotAllowed { function: Text, message: Text },

    /// Invalid asset path (must be relative to project root)
    InvalidAssetPath { path: Text, message: Text },

    /// Generic I/O violation (catch-all)
    IoViolation { operation: Text, reason: Text },

    /// An unsafe operation was attempted
    UnsafeOperation { operation: Text, reason: Text },

    /// I/O in async meta function (async allowed for parallelism only)
    IoInAsyncMeta {
        function: Text,
        io_operations: List<Text>,
        message: Text,
    },

    /// Execution timeout
    Timeout { elapsed_ms: u64, limit_ms: u64 },

    /// Iteration limit exceeded (infinite loop protection)
    IterationLimitExceeded { iterations: usize, limit: usize },

    /// Stack overflow (too deep recursion)
    StackOverflow { depth: usize, limit: usize },

    /// Memory limit exceeded
    MemoryLimitExceeded { allocated: usize, limit: usize },

    /// Function not allowed in meta context
    ForbiddenFunction { function: Text, reason: Text },

    /// Module not allowed in meta context
    ForbiddenModule { module: Text, reason: Text },
}

impl std::fmt::Display for SandboxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SandboxError::FileSystemViolation {
                operation,
                function,
                context,
            } => {
                write!(
                    f,
                    "File system operation '{}' forbidden in {} context\n\
                     Function: {}\n\
                     Note: Meta functions cannot perform file I/O operations\n\
                     Help: Use `using BuildAssets` context for compile-time asset loading",
                    operation.as_str(),
                    context.as_str(),
                    function.as_str()
                )
            }
            SandboxError::NetworkViolation {
                operation,
                function,
                context,
            } => {
                write!(
                    f,
                    "Network operation '{}' forbidden in {} context\n\
                     Function: {}\n\
                     Note: Meta functions execute in a sandboxed environment\n\
                     Help: Provide data at compile-time through parameters",
                    operation.as_str(),
                    context.as_str(),
                    function.as_str()
                )
            }
            SandboxError::ProcessViolation {
                operation,
                function,
            } => {
                write!(
                    f,
                    "Process operation '{}' forbidden in meta context\n\
                     Function: {}\n\
                     Note: Process spawning is a security risk\n\
                     Help: Meta functions must be pure and deterministic",
                    operation.as_str(),
                    function.as_str()
                )
            }
            SandboxError::TimeViolation {
                operation,
                function,
            } => {
                write!(
                    f,
                    "Non-deterministic time operation '{}' forbidden in meta context\n\
                     Function: {}\n\
                     Note: Meta functions must be deterministic for reproducible builds\n\
                     Help: Use compile_time() intrinsic for build timestamp",
                    operation.as_str(),
                    function.as_str()
                )
            }
            SandboxError::EnvViolation {
                operation,
                function,
            } => {
                write!(
                    f,
                    "Environment variable access '{}' forbidden in meta context\n\
                     Function: {}\n\
                     Note: Runtime environment is non-deterministic\n\
                     Help: Use cfg!() for compile-time configuration flags",
                    operation.as_str(),
                    function.as_str()
                )
            }
            SandboxError::RandomViolation {
                operation,
                function,
            } => {
                write!(
                    f,
                    "Non-deterministic random operation '{}' forbidden in meta context\n\
                     Function: {}\n\
                     Note: Meta functions must be deterministic for reproducible builds\n\
                     Help: Use Random.from_seed() with a fixed seed instead",
                    operation.as_str(),
                    function.as_str()
                )
            }
            SandboxError::UnsafeViolation {
                operation,
                function,
            } => {
                write!(
                    f,
                    "Unsafe memory operation '{}' forbidden in meta context\n\
                     Function: {}\n\
                     Note: Unsafe operations are not allowed in meta functions\n\
                     Help: Meta code must be safe and verifiable",
                    operation.as_str(),
                    function.as_str()
                )
            }
            SandboxError::FFIViolation {
                operation,
                function,
            } => {
                write!(
                    f,
                    "FFI call '{}' forbidden in meta context\n\
                     Function: {}\n\
                     Note: FFI calls are a security risk and non-deterministic\n\
                     Help: Meta functions can only use pure Verum code",
                    operation.as_str(),
                    function.as_str()
                )
            }
            SandboxError::AssetLoadingNotAllowed { function, message } => {
                write!(
                    f,
                    "Asset loading requires `using BuildAssets` context\n\
                     Function: {}\n\
                     {}\n\
                     Help: Add `using BuildAssets` to function signature",
                    function.as_str(),
                    message.as_str()
                )
            }
            SandboxError::InvalidAssetPath { path, message } => {
                write!(
                    f,
                    "Invalid asset path: {}\n\
                     {}\n\
                     Help: Asset paths must be relative to project root (no '..' or absolute paths)",
                    path.as_str(),
                    message.as_str()
                )
            }
            SandboxError::IoViolation { operation, reason } => {
                write!(
                    f,
                    "I/O operation '{}' not allowed in meta context: {}",
                    operation.as_str(),
                    reason.as_str()
                )
            }
            SandboxError::UnsafeOperation { operation, reason } => {
                write!(
                    f,
                    "Unsafe operation '{}' not allowed in meta context: {}",
                    operation.as_str(),
                    reason.as_str()
                )
            }
            SandboxError::IoInAsyncMeta {
                function,
                io_operations,
                message,
            } => {
                write!(
                    f,
                    "I/O operations forbidden in async meta function '{}'\n\
                     {}\n\
                     I/O operations found: {:?}\n\
                     Note: meta async fn allows parallelism, not I/O",
                    function.as_str(),
                    message.as_str(),
                    io_operations
                )
            }
            SandboxError::Timeout {
                elapsed_ms,
                limit_ms,
            } => {
                write!(
                    f,
                    "Meta function execution timeout: {}ms > {}ms limit",
                    elapsed_ms, limit_ms
                )
            }
            SandboxError::IterationLimitExceeded { iterations, limit } => {
                write!(
                    f,
                    "Iteration limit exceeded in meta function: {} > {} limit\n\
                     Note: This may indicate an infinite loop\n\
                     Help: Review loop termination conditions",
                    iterations, limit
                )
            }
            SandboxError::StackOverflow { depth, limit } => {
                write!(
                    f,
                    "Stack overflow in meta function: depth {} exceeds limit {}\n\
                     Note: This may indicate infinite recursion\n\
                     Help: Review recursive function base cases",
                    depth, limit
                )
            }
            SandboxError::MemoryLimitExceeded { allocated, limit } => {
                write!(
                    f,
                    "Memory limit exceeded in meta function: {} bytes > {} bytes limit",
                    allocated, limit
                )
            }
            SandboxError::ForbiddenFunction { function, reason } => {
                write!(
                    f,
                    "Function '{}' forbidden in meta context\n\
                     {}",
                    function.as_str(),
                    reason.as_str()
                )
            }
            SandboxError::ForbiddenModule { module, reason } => {
                write!(
                    f,
                    "Module '{}' forbidden in meta context\n\
                     {}",
                    module.as_str(),
                    reason.as_str()
                )
            }
        }
    }
}

impl std::error::Error for SandboxError {}

impl SandboxError {
    /// Returns the error code for this sandbox error
    ///
    /// Error codes in the M3XX range:
    /// - M310-M319: I/O violations
    /// - M320-M329: Resource limits
    /// - M330-M339: Forbidden operations
    pub fn error_code(&self) -> &'static str {
        match self {
            // M310-M319: I/O violations
            SandboxError::FileSystemViolation { .. } => "M310",
            SandboxError::NetworkViolation { .. } => "M311",
            SandboxError::ProcessViolation { .. } => "M312",
            SandboxError::TimeViolation { .. } => "M313",
            SandboxError::EnvViolation { .. } => "M314",
            SandboxError::RandomViolation { .. } => "M315",
            SandboxError::AssetLoadingNotAllowed { .. } => "M316",
            SandboxError::InvalidAssetPath { .. } => "M317",
            SandboxError::IoViolation { .. } => "M318",
            SandboxError::IoInAsyncMeta { .. } => "M319",

            // M320-M329: Resource limits
            SandboxError::Timeout { .. } => "M320",
            SandboxError::IterationLimitExceeded { .. } => "M321",
            SandboxError::StackOverflow { .. } => "M322",
            SandboxError::MemoryLimitExceeded { .. } => "M323",

            // M330-M339: Forbidden operations
            SandboxError::UnsafeViolation { .. } => "M330",
            SandboxError::FFIViolation { .. } => "M331",
            SandboxError::UnsafeOperation { .. } => "M332",
            SandboxError::ForbiddenFunction { .. } => "M333",
            SandboxError::ForbiddenModule { .. } => "M334",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_operation_equality() {
        assert_eq!(Operation::Arithmetic, Operation::Arithmetic);
        assert_ne!(Operation::Arithmetic, Operation::Comparison);
    }

    #[test]
    fn test_sandbox_error_display() {
        let err = SandboxError::FileSystemViolation {
            operation: Text::from("read"),
            function: Text::from("test_fn"),
            context: Text::from("meta"),
        };
        let msg = format!("{}", err);
        assert!(msg.contains("File system operation"));
        assert!(msg.contains("read"));
    }
}
