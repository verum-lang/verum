//! Script mode parsing for REPL and interactive sessions
//!
//! This module provides specialized parsing for script-like environments where:
//! - Expressions can be evaluated standalone
//! - Incremental parsing is essential for performance
//! - Partial input needs graceful handling
//! - Type inference should provide immediate feedback
//!
//! # Architecture
//!
//! The script parser wraps the main parser with additional features:
//! - **Expression-first parsing**: Try expressions before statements
//! - **Completion detection**: Identify incomplete vs. complete input
//! - **Context preservation**: Maintain state across REPL sessions
//! - **Smart recovery**: Handle common REPL errors gracefully
//! - **Incremental caching**: Only re-parse changed lines
//!
//! # Modules
//!
//! - [`parser`]: Core script parser with expression-first parsing
//! - [`context`]: Session context tracking (bindings, delimiters)
//! - [`result`]: Parse result types and modes
//! - [`recovery`]: Error recovery and suggestions
//! - [`incremental`]: Incremental parsing with caching

pub mod context;
pub mod incremental;
pub mod parser;
pub mod recovery;
pub mod result;

// Re-export main types for convenience
pub use context::ScriptContext;
pub use incremental::{
    detect_dependencies, CachedLine, DependencyGraph, IncrementalScriptParser, IncrementalStats,
};
pub use parser::{needs_continuation, suggest_completion, ScriptParser};
pub use recovery::{explain_error, suggest_autocompletion, RecoveryResult, ScriptRecovery};
pub use result::{ParseMode, ScriptParseResult};
