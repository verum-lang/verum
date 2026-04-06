//! Tier 1: Capability-Gated Builtins (Require Context)
//!
//! These functions access external state and require explicit context
//! declaration via `using [...]` clause.
//!
//! ## Modules
//!
//! - [`type_introspection`] - Type name, ID, kind checks (MetaTypes)
//! - [`structural_reflection`] - Fields, variants, methods (MetaTypes)
//! - [`constraint_reflection`] - Bounds, lifetimes, where clauses (MetaTypes)
//! - [`diagnostics`] - compile_error, compile_warning (CompileDiag)
//!
//! ## Context Requirements
//!
//! | Context | Module | Purpose |
//! |---------|--------|---------|
//! | `MetaTypes` | type_introspection, structural_reflection, constraint_reflection | Type introspection |
//! | `MetaRuntime` | runtime (in parent) | Build/platform information |
//! | `CompileDiag` | diagnostics | Compiler diagnostics |
//! | `BuildAssets` | build_assets (in parent) | File system access |
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).
//! Meta context unification: all compile-time features desugar to meta-system
//! operations, providing one coherent model with convenient syntax sugar.

pub mod constraint_reflection;
pub mod diagnostics;
pub mod structural_reflection;
pub mod type_introspection;

use super::context_requirements::BuiltinRegistry;

/// Register all Tier 1 builtins to the registry
pub fn register_all(registry: &mut BuiltinRegistry) {
    type_introspection::register_builtins(registry);
    structural_reflection::register_builtins(registry);
    constraint_reflection::register_builtins(registry);
    diagnostics::register_builtins(registry);
}
