#![allow(dead_code, unused_imports, unused_variables, unused_mut, unused_must_use, unused_unsafe, deprecated, unexpected_cfgs, unused_comparisons, forgetting_copy_types, useless_ptr_null_checks, unused_assignments)]
//! Integration Tests Module
//!
//! This module contains comprehensive end-to-end integration tests for
//! the complete Verum compilation pipeline.
//!
//! Test Organization:
//! - e2e_pipeline: Basic pipeline tests (parse → typecheck → codegen → execute)
//! - cbgr_integration: CBGR memory safety and performance tests
//! - refinements: Refinement type system integration tests
//! - references: Three-tier reference system tests
//! - performance: Execution tier comparison (interpreter vs JIT vs AOT)
//! - module_system: Module loading and multi-file project tests
//!
//! Pipeline: Source -> Lex/Parse -> Meta -> Macros -> Contracts -> Semantic Analysis ->
//! VBC Codegen -> Optimization -> Execution (Interpreter or AOT/LLVM).
//! Module system: hierarchical namespaces, file-system mapped, `mount` for imports,
//! visibility control (public, public(crate), public(super), private default).
//! All integration tests in tests/ directory per project conventions.

pub mod e2e_pipeline;
pub mod cbgr_integration;
pub mod refinements;
pub mod references;
pub mod performance;
pub mod module_system;
pub mod module_integration_p0_tests;
