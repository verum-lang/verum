//! Level 2: Explicit Handling (Runtime Recovery)
//!
//! The primary runtime error handling mechanism in Verum. Errors are first-class
//! values returned from functions via `Result<T, E>`. Error handling must be
//! explicit -- silent failures are prevented by the type system. The `?` operator
//! propagates errors up the call stack with automatic `From` conversion.
//! `ContextError<E>` wraps errors with contextual breadcrumbs at each call level.
//! `with_context(|| ...)` is zero-cost on the success path (closure only executes
//! on error). Additional features include `errdefer` for error-path-only cleanup,
//! `try/recover/finally` blocks for structured recovery with pattern matching,
//! typed `throws` clauses for error boundary declarations, and the `Validated<T, E>`
//! type for accumulating multiple errors instead of short-circuiting.
//!
//! Provides **Result types, error combinators, and recovery operators** for
//! explicit runtime error handling. This is the primary error handling mechanism.
//!
//! # Core Concepts
//!
//! - **Result<T, E>**: Either success (Ok) or failure (Err)
//! - **?-operator**: Propagate errors up the call stack
//! - **Error context**: Add information as errors propagate
//! - **Error combinators**: Transform, map, or recover from errors
//! - **Zero-cost on success**: No overhead in happy path
//!
//! # Usage Patterns
//!
//! ## Basic Error Propagation
//!
//! ```rust,ignore
//! fn process() -> Result<Text> {
//!     let file = read_file("data.txt")?;  // ? propagates error
//!     let data = parse(&file)?;
//!     Ok(format!("Parsed: {}", data))
//! }
//! ```
//!
//! ## Error Mapping
//!
//! ```rust,ignore
//! fn convert_error() -> Result<i32> {
//!     let text = read_text()?;
//!     text.parse::<i32>()  // Converts parse error to our Result
//!         .map_err(|e| VerumError::new(
//!             format!("Parse failed: {}", e),
//!             ErrorKind::Parse
//!         ))
//! }
//! ```
//!
//! ## Error Recovery
//!
//! ```rust,ignore
//! fn with_fallback() -> Result<Text> {
//!     read_config("primary.toml")
//!         .or_else(|_| read_config("backup.toml"))  // Try backup
//!         .or_else(|_| Ok(default_config()))         // Use default
//! }
//! ```
//!
//! ## Error Context
//!
//! ```rust,ignore
//! fn with_context() -> Result<Data> {
//!     let file = read_file("data.json")
//!         .context("Failed to read data file")?;
//!
//!     let data = parse_json(&file)
//!         .context("Invalid JSON in data file")?;
//!
//!     Ok(data)
//! }
//! ```
//!
//! # ResultExt Trait
//!
//! The [`ResultExt`] trait provides additional combinators:
//! - [`map_err_into`] - Convert error type
//! - [`expect_with`] - Custom panic message
//! - [`ok_or_none`] - Convert to Option
//! - [`into_verum_error`] - Convert any error to VerumError
//!
//! # Best Practices
//!
//! 1. **Propagate errors** - Use `?` instead of unwrap
//! 2. **Add context** - Explain what was happening when error occurred
//! 3. **Don't swallow errors** - Don't ignore errors with `let _ =`
//! 4. **Provide defaults** - Use `unwrap_or` for optional recovery
//! 5. **Log before panicking** - If you must panic, log the error first
//!
//! # Error Context Chain
//!
//! Errors maintain context as they propagate:
//!
//! ```text
//! original_error: "connection refused"
//! ↓ context("connecting to database")
//! ↓ context("fetching user record")
//! final_error: "failed to fetch user (connecting to database: connection refused)"
//! ```
//!
//! Provides Result types, try blocks, and error combinators for
//! explicit runtime error handling.

use crate::{Result, VerumError};
use verum_common::Maybe;
use verum_common::Text;

/// Extension trait for Result types
///
/// Provides additional combinators for error handling.
pub trait ResultExt<T, E> {
    /// Convert error using a closure
    fn map_err_into<F, E2>(self, f: F) -> Result<T, E2>
    where
        F: FnOnce(E) -> E2;

    /// Unwrap with a custom panic message
    fn expect_with<F>(self, f: F) -> T
    where
        F: FnOnce(&E) -> Text;

    /// Convert to Maybe, discarding error
    fn ok_or_none(self) -> Maybe<T>;

    /// Convert error to VerumError
    fn into_verum_error(self) -> Result<T, VerumError>
    where
        E: Into<VerumError>;
}

impl<T, E> ResultExt<T, E> for Result<T, E> {
    fn map_err_into<F, E2>(self, f: F) -> Result<T, E2>
    where
        F: FnOnce(E) -> E2,
    {
        self.map_err(f)
    }

    fn expect_with<F>(self, f: F) -> T
    where
        F: FnOnce(&E) -> Text,
    {
        match self {
            Ok(v) => v,
            Err(e) => panic!("{}", f(&e)),
        }
    }

    fn ok_or_none(self) -> Maybe<T> {
        self.ok()
    }

    fn into_verum_error(self) -> Result<T, VerumError>
    where
        E: Into<VerumError>,
    {
        self.map_err(|e| e.into())
    }
}

/// @must_handle annotation support (Phase 3)
///
/// Placeholder for the `@must_handle` annotation on error types. When an error
/// type is annotated with `@must_handle`, any `Result<T, ThatType>` must be
/// explicitly handled before being dropped. The compiler tracks all such Result
/// values through control flow analysis and rejects:
///
/// - Wildcard binding: `let _ = fallible_call();`
/// - Unexamined binding: `let result = fallible_call();` (bound but never checked)
/// - Explicit drop without check: `drop(fallible_call());`
///
/// Allowed operations: `?` propagation, `unwrap()`/`expect()`, pattern matching
/// (`match`/`if let`), and error inspection before drop (`result.is_err()` + handle).
///
/// The annotation applies to the error **type**, not individual functions. All
/// functions returning `Result<T, MarkedType>` automatically inherit enforcement.
/// Diagnostic error code: E0317.
#[derive(Debug, Clone, Copy)]
pub struct MustHandle;

/// Marker trait for error types that must be handled
///
/// Error types implementing this trait will trigger compile-time errors if their
/// Results are dropped without explicit handling. The `@must_handle` annotation
/// on the type declaration causes the compiler to track all `Result` values with
/// this error type through control flow, rejecting wildcard bindings, unexamined
/// bindings, and explicit drops without inspection. Propagation via `?`, `unwrap()`,
/// `expect()`, `match`, and `if let` are all permitted.
///
/// # Phase 3 Feature
///
/// Specified for Phase 3 (v1.2) implementation.
pub trait MustHandleError: std::error::Error {
    /// Error code for diagnostics (E0317: must_handle Result dropped without handling)
    fn error_code() -> &'static str {
        "E0317"
    }
}
