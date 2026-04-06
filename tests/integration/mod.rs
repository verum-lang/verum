//! Comprehensive Integration Test Suite for Verum Language Platform
//!
//! This module provides complete integration testing infrastructure that validates
//! all modules working together in production-ready scenarios.
//!
//! ## Test Categories
//!
//! 1. **Compilation Pipeline** - Lexer → Parser → Type Checker → Codegen
//! 2. **LSP Integration** - Real-time IDE features with document lifecycle
//! 3. **Standard Library** - Cross-module stdlib workflows
//! 4. **CBGR Memory Safety** - 3-tier reference system under load
//! 5. **Context System** - Dependency injection across boundaries
//! 6. **Error Handling** - 5-level defense across modules
//! 7. **Runtime Integration** - 4-tier execution model
//! 8. **Verification** - Gradual verification integration
//! 9. **FFI Integration** - Foreign function interface safety
//! 10. **Real-World Workflows** - Complete applications

// Re-export test utilities
pub mod test_utils;
pub mod fixtures;

// Test category modules
pub mod compilation_pipeline;
pub mod lsp_integration;
pub mod stdlib_integration;
pub mod cbgr_integration;
pub mod context_integration;
pub mod error_handling;
pub mod runtime_integration;
pub mod verification_integration;
pub mod ffi_integration;
pub mod real_world_workflows;

// Common re-exports for tests
pub use verum_ast;
pub use verum_cbgr;
pub use verum_codegen;
pub use verum_context;
pub use verum_diagnostics;
pub use verum_error;
pub use verum_interpreter;
pub use verum_lexer;
pub use verum_lsp;
pub use verum_parser;
pub use verum_resolve;
pub use verum_runtime;
pub use verum_smt;
pub use verum_std;
pub use verum_types;
pub use verum_verification;
