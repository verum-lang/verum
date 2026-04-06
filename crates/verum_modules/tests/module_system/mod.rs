#![allow(dead_code, unused_imports, unused_variables, unused_mut, unused_must_use, unused_unsafe, deprecated, unexpected_cfgs, unused_comparisons, forgetting_copy_types, useless_ptr_null_checks, unused_assignments)]
//! Multi-file module system integration tests.
//!
//! This module contains comprehensive end-to-end tests for the Verum module system.
//! Each submodule tests a specific aspect of module loading, resolution, and organization.
//!
//! # Test Organization
//!
//! - `simple`: Basic two-file imports and module loading
//! - `visibility`: Visibility modifier enforcement across modules
//! - `directory`: Directory-based module structures with mod.vr
//! - `reexport`: Re-exporting types through module boundaries
//! - `imports`: Various import patterns (glob, nested, aliasing)
//! - `circular`: Circular dependency detection and handling
//!
//! # Running Tests
//!
//! ```bash
//! # Run all integration tests
//! cargo test --test integration
//!
//! # Run specific category
//! cargo test --test integration simple::
//! cargo test --test integration visibility::
//!
//! # Run specific test
//! cargo test --test integration test_basic_two_file_import
//! ```
//!
//! # Specification
//!
//! All tests reference specific sections of the module system specification:
//! the Verum module system specification (namespace management, visibility
//! control, dependency resolution, file system mapping, and import/export).
//!
//! # Test Count
//!
//! - simple.rs: 7 tests
//! - visibility.rs: 8 tests
//! - directory.rs: 10 tests
//! - reexport.rs: 8 tests
//! - imports.rs: 13 tests
//! - circular.rs: 13 tests
//!
//! **Total: 59 integration tests**

pub mod circular;
pub mod directory;
pub mod imports;
pub mod reexport;
pub mod simple;
pub mod visibility;
