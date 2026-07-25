//! Meta Intermediate Representation
//!
//! This module defines the IR for compile-time meta expressions,
//! statements, and patterns.
//!
//! ## Module Structure
//!
//! - [`expr`] - Meta expression nodes (literals, variables, calls, etc.)
//! - [`stmt`] - Meta statement nodes (let, return, expression)
//! - [`pattern`] - Meta pattern nodes (wildcard, literal, binding, etc.)
//! - [`meta_type`] - Meta type system for parameter typing
//!
//! ## Design
//!
//! The meta IR is a simplified representation optimized for compile-time
//! evaluation. It supports:
//!
//! - Constant evaluation
//! - Pattern matching
//! - Quote/unquote for AST manipulation
//! - List comprehensions
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).

pub mod expr;
pub mod meta_type;
pub mod pattern;
pub mod stmt;

// Re-export main types
pub use expr::{MetaArm, MetaExpr};
pub use meta_type::MetaType;
pub use pattern::MetaPattern;
pub use stmt::MetaStmt;
