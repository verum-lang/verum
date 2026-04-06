#![allow(unexpected_cfgs)]
//! # Verum Error Handling System
//!
//! Centralized **runtime error handling** system implementing the 5-Level Error Defense Architecture.
//!
//! Verum uses a defense-in-depth approach where five complementary layers each provide
//! progressively stronger safety guarantees, from compile-time prevention through
//! runtime recovery to security containment. The system is built on `Result<T, E>`
//! with explicit error propagation via the `?` operator, zero-cost error context
//! (closures only execute on error path), and rich diagnostics including SMT traces
//! and fix suggestions. Panics are reserved for programmer errors; `Result` is used
//! for all expected failures.
//!
//! The five levels are:
//!
//! - **Level 0: Type Prevention** - Compile-time safety (refinement types, affine types)
//! - **Level 1: Static Verification** - Proof-based safety (SMT integration, @verify modes)
//! - **Level 2: Explicit Handling** - Runtime recovery (Result types, error contexts)
//! - **Level 3: Fault Tolerance** - Resilience patterns (supervision, circuit breakers)
//! - **Level 4: Security Containment** - Isolation boundaries (sandboxing, capability control)
//!
//! ## Separation of Concerns
//!
//! **verum_error** provides **runtime error handling** for Verum programs - the Result types,
//! error contexts, and recovery strategies that Verum code uses at runtime.
//!
//! For **compiler diagnostics** (beautiful error messages, source spans, SMT traces), see the
//! **verum_diagnostics** module which handles compilation errors and warnings.
//!
//! ## Key Features
//!
//! - **Unified error hierarchy** - Single source of truth for all error types
//! - **Zero-cost error contexts** - Context closures only execute on error path
//! - **Automatic context preservation** - Error chains maintained through `?` operator
//! - **Level 3 fault tolerance** - Production-ready supervision trees and circuit breakers
//! - **Integration with ExecutionEnv** - Seamless error recovery in async runtime
//!
//! ## Usage
//!
//! ```rust
//! use verum_error::{Result, VerumError, ErrorKind};
//!
//! fn example() -> Result<i32> {
//!     let value = 42;
//!     if value > 0 {
//!         Ok(value)
//!     } else {
//!         Err(VerumError::new("Invalid value", ErrorKind::InvalidState))
//!     }
//! }
//! ```
//!
//! ## Unified Error Type (New)
//!
//! The `unified` module provides a single error type that can represent errors from all
//! Verum modules, enabling seamless error propagation across module boundaries:
//!
//! ```rust,ignore
//! use verum_error::unified::{VerumError as UnifiedError, Result};
//!
//! fn cross_module_operation() -> Result<()> {
//!     // Each ? automatically converts the specific error type
//!     perform_cbgr_check()?;     // From verum_cbgr::Error
//!     verify_types()?;            // From TypeError
//!     run_smt_solver()?;          // From verum_smt::Error
//!     Ok(())
//! }
//! ```

#![deny(missing_docs)]
#![deny(unsafe_code)]
#![warn(clippy::all)]
#![allow(unused_variables)]
#![allow(unused_imports)]
#![allow(dead_code)]
// Suppress informational clippy lints
#![allow(clippy::result_large_err)]
#![allow(clippy::large_enum_variant)]
#![allow(clippy::type_complexity)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::collapsible_match)]
#![allow(clippy::should_implement_trait)]

// Re-export common types
pub use verum_common::{List, Map, Maybe, Text};

// Core error types
pub mod error;
pub use error::{ErrorKind, Result, VerumError};

// Unified error type system (NEW)
pub mod conversions;
pub mod unified;

// Level-specific modules
pub mod levels;

// Error context chain management
pub mod context;

// Structured error contexts (key-value pairs)
pub mod structured_context;

// Error output formatters
pub mod formatters;

// Recovery strategies (Level 3)
pub mod recovery;

// Panic handler integration (Level 3)
pub mod panic_handler;

// Result extension trait for structured contexts
pub mod result_ext;
pub use result_ext::{
    ContextErrorStructuredExt, ResultStructuredContext, ResultStructuredContextFn,
};

// Prelude for common imports
pub mod prelude {
    //! Common imports for error handling

    pub use crate::context::{ContextError, ErrorContext};
    pub use crate::error::{ErrorKind, Result, VerumError};
    pub use crate::formatters::{FormatError, OutputFormat};
    pub use crate::levels::level2::ResultExt;
    pub use crate::panic_handler::{
        PanicData, PanicLogger, PanicStatistics, catch_task_panics, catch_task_panics_async,
        clear_panic_statistics, get_panic_history, get_panic_statistics, panic_logger,
        setup_panic_hook, setup_panic_hook_with_logger,
    };
    pub use crate::recovery::{
        BackoffStrategy, CircuitBreaker, CircuitBreakerConfig, RecoveryStrategy, RestartStrategy,
    };
    pub use crate::result_ext::{
        ContextErrorStructuredExt, ResultStructuredContext, ResultStructuredContextFn,
    };
    pub use crate::structured_context::{ContextValue, ToContextValue};
}

// Unified error prelude (NEW)
pub mod unified_prelude {
    //! Prelude for unified error handling across all modules
    //!
    //! This prelude is useful when working with multiple Verum modules
    //! and you want seamless error conversion.

    pub use crate::unified::{Result, VerumError};
}
