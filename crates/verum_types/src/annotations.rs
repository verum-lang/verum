//! Type-Level Annotations for Error Handling and Optimization
//!
//! Advanced type features: existential types and type-level computation
//!
//! This module implements two key annotations:
//! - @must_handle: Critical errors that must be explicitly handled
//! - @cold: Optimization hint for rarely-executed code paths
//!
//! # @must_handle Annotation
//!
//! Some error types represent critical failures that should never be silently ignored.
//! The `@must_handle` annotation ensures that any Result<T, E> where E is marked
//! with `@must_handle` must be explicitly handled before being dropped.
//!
//! ## Example
//!
//! ```verum
//! @must_handle
//! type CriticalError is
//!     | DatabaseConnectionLost
//!     | SecurityViolation
//!     | DataCorruption
//!     | OutOfMemory;
//!
//! fn connect_database() -> Result<Connection, CriticalError> { ... }
//!
//! fn caller() {
//!     let conn = connect_database(); // ERROR: must handle CriticalError
//!     // Must use: match, ?, or explicit pattern matching
//! }
//!
//! fn caller_correct() {
//!     match connect_database() {
//!         Ok(conn) => { /* use conn */ },
//!         Err(e) => { /* handle error */ },
//!     }
//! }
//! ```
//!
//! # @cold Annotation
//!
//! Marks functions or code paths that are rarely executed, allowing the optimizer
//! to deprioritize them in favor of hot paths.
//!
//! ## Example
//!
//! ```verum
//! @cold
//! fn handle_parse_error(contents: &str) -> Error {
//!     log_error("Invalid config format");
//!     send_telemetry("config_parse_error");
//!     Error.InvalidFormat
//! }
//!
//! fn parse_config(path: Path) -> Result<Config, Error> {
//!     let contents = read_file(path)?;
//!     if is_valid_format(contents) {
//!         return Ok(parse_contents(contents));  // Hot path optimized
//!     }
//!     return Err(handle_parse_error(contents));  // Cold path deprioritized
//! }
//! ```

use crate::TypeError;
use verum_ast::span::Span;
use verum_common::{Set, Text};

/// Tracks which types are marked with @must_handle
#[derive(Debug, Clone, Default)]
pub struct MustHandleRegistry {
    /// Set of type names marked with @must_handle
    must_handle_types: Set<Text>,
}

impl MustHandleRegistry {
    /// Create a new registry
    pub fn new() -> Self {
        Self {
            must_handle_types: Set::new(),
        }
    }

    /// Register a type as @must_handle
    pub fn register(&mut self, type_name: impl Into<Text>) {
        self.must_handle_types.insert(type_name.into());
    }

    /// Check if a type is marked with @must_handle
    pub fn is_must_handle(&self, type_name: &str) -> bool {
        self.must_handle_types.contains(&Text::from(type_name))
    }

    /// Check if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.must_handle_types.is_empty()
    }

    /// Iterate over registered must-handle types
    pub fn iter(&self) -> impl Iterator<Item = &Text> {
        self.must_handle_types.iter()
    }

    /// Check if a Result<T, E> has a @must_handle error type
    pub fn check_result_handled(
        &self,
        error_type_name: &str,
        _span: Span,
    ) -> Result<(), TypeError> {
        if self.is_must_handle(error_type_name) {
            // This would be caught at the point where the Result is dropped without handling
            // For now, we just track it. The actual enforcement happens in the type checker
            // when we see a let binding or expression statement with a Result type.
            Ok(())
        } else {
            Ok(())
        }
    }
}

/// Tracks which functions are marked with @cold
#[derive(Debug, Clone, Default)]
pub struct ColdFunctionRegistry {
    /// Set of function names marked with @cold
    cold_functions: Set<Text>,
}

impl ColdFunctionRegistry {
    /// Create a new registry
    pub fn new() -> Self {
        Self {
            cold_functions: Set::new(),
        }
    }

    /// Register a function as @cold
    pub fn register(&mut self, function_name: impl Into<Text>) {
        self.cold_functions.insert(function_name.into());
    }

    /// Check if a function is marked with @cold
    pub fn is_cold(&self, function_name: &str) -> bool {
        self.cold_functions.contains(&Text::from(function_name))
    }

    /// Get optimization hint for code generation
    ///
    /// Returns true if the function should be deprioritized in optimization.
    pub fn get_optimization_hint(&self, function_name: &str) -> OptimizationHint {
        if self.is_cold(function_name) {
            OptimizationHint::Cold
        } else {
            OptimizationHint::Default
        }
    }
}

/// Optimization hints for code generation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptimizationHint {
    /// Default optimization level
    Default,
    /// Cold path - deprioritize in optimization
    Cold,
    /// Hot path - prioritize in optimization (future)
    Hot,
}

/// Combined annotation registry for type checking
#[derive(Debug, Clone, Default)]
pub struct AnnotationRegistry {
    /// @must_handle annotation tracking
    pub must_handle: MustHandleRegistry,
    /// @cold annotation tracking
    pub cold_functions: ColdFunctionRegistry,
}

impl AnnotationRegistry {
    /// Create a new annotation registry
    pub fn new() -> Self {
        Self {
            must_handle: MustHandleRegistry::new(),
            cold_functions: ColdFunctionRegistry::new(),
        }
    }

    /// Register a @must_handle type
    pub fn register_must_handle(&mut self, type_name: impl Into<Text>) {
        self.must_handle.register(type_name);
    }

    /// Register a @cold function
    pub fn register_cold_function(&mut self, function_name: impl Into<Text>) {
        self.cold_functions.register(function_name);
    }

    /// Check if we're about to drop a Result with @must_handle error type
    ///
    /// This should be called when:
    /// - A let binding doesn't use the Result
    /// - An expression statement evaluates to a Result
    /// - A function returns without checking a Result
    pub fn check_must_handle_result(
        &self,
        result_error_type: &str,
        usage: ResultUsage,
        span: Span,
    ) -> Result<(), TypeError> {
        if self.must_handle.is_must_handle(result_error_type) {
            match usage {
                ResultUsage::Handled => Ok(()),
                ResultUsage::Propagated => Ok(()),
                ResultUsage::Ignored => Err(TypeError::Other(format!(
                    "Result with @must_handle error type '{}' must be explicitly handled\n  \
                     at: {}\n  \
                     help: use `match`, `?`, or explicit pattern matching to handle the error\n  \
                     help: @must_handle errors represent critical failures that should never be silently ignored",
                    result_error_type,
                    span.start
                ).into())),
            }
        } else {
            Ok(())
        }
    }
}

/// How a Result value is being used
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultUsage {
    /// Result is explicitly handled (match, if-let, etc.)
    Handled,
    /// Result is propagated with ?
    Propagated,
    /// Result is ignored (dropped without handling)
    Ignored,
}

// Tests moved to tests/annotations_tests.rs per project testing guidelines.
