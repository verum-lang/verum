//! Integration tests for the Verum language platform
//!
//! This module provides integration testing infrastructure for
//! testing the entire Verum compilation and execution pipeline.
//!
//! ## Comprehensive Integration Test Suite v1.0
//!
//! This suite validates all modules working together in production-ready scenarios.
//!
//! ### Test Categories:
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
//!
//! ### Running Tests:
//!
//! ```bash
//! # Run all integration tests
//! cargo test --test integration
//!
//! # Run specific category
//! cargo test --test integration compilation_pipeline
//!
//! # Run with output
//! cargo test --test integration -- --nocapture
//!
//! # Run stress tests
//! cargo test --test integration stress -- --ignored
//! ```
//!
//! ### Test Infrastructure:
//!
//! - `test_utils.rs` - Common utilities, assertions, performance tracking
//! - `fixtures.rs` - Sample Verum programs and test data
//! - Each category has dedicated test file with 10+ tests

// Re-export common dependencies for tests
pub use verum_ast;
pub use verum_cbgr;
pub use verum_context;
pub use verum_diagnostics;
pub use verum_interpreter;
pub use verum_lexer;
pub use verum_parser;
pub use verum_resolve;
pub use verum_runtime;
pub use verum_std;
pub use verum_types;

// Integration test modules
pub mod integration;

// Cross-platform test modules
pub mod cross_platform;
