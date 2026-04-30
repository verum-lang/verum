//! Meta Context Submodules
//!
//! This module decomposes the monolithic MetaContext into focused sub-contexts
//! following the Single Responsibility Principle.
//!
//! ## Sub-Contexts
//!
//! - [`execution_state`] - Variable bindings, call stack, recursion tracking
//! - [`type_introspection`] - Type registry, definitions, protocols
//! - [`diagnostics`] - Errors, warnings, source mapping
//! - [`build_config`] - RuntimeInfo, BuildAssets, ProjectInfo
//! - [`security`] - EnabledContexts, resource limits, sandboxing
//!
//! ## Architecture
//!
//! The coordinator facade (`MetaContext`) aggregates these sub-contexts while
//! maintaining backward compatibility with existing code.
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).
//! Meta audit: validation of meta function safety, resource limits,
//! and sandbox compliance before execution.

pub mod build_config;
pub mod diagnostics;
pub mod execution_state;
pub mod security;
pub mod type_introspection;

// Re-export all sub-context types
pub use build_config::BuildConfiguration;
pub use diagnostics::DiagnosticsCollector;
pub use execution_state::ExecutionState;
pub use security::{ResourceLimits, SecurityContext};
pub use type_introspection::TypeIntrospection;
