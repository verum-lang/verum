//! Integration tests for the Verum language platform
//!
//! This crate contains comprehensive integration tests that verify
//! the entire Verum compilation and execution pipeline.

// Re-export all dependencies for test modules
pub use verum_ast;
pub use verum_cbgr;
pub use verum_codegen;
pub use verum_diagnostics;
pub use verum_lexer;
pub use verum_parser;
pub use verum_common;
pub use verum_types;

pub use serde;
pub use serde_json;
pub use tempfile;
