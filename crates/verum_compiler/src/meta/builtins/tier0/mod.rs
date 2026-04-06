//! Tier 0: Core Primitive Builtins (Always Available)
//!
//! These functions operate only on their input values without accessing
//! any external state. They are always available without requiring
//! a context declaration.
//!
//! ## Modules
//!
//! - [`arithmetic`] - Numeric operations: `abs`, `min`, `max`, `int_to_text`, `text_to_int`
//! - [`collections`] - Collection operations: `list_*`, `text_*`, `map_*`, `set_*`
//! - [`code_gen`] - Code generation: `quote`, `unquote`, `stringify`, `concat_idents`
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).
//! Meta context unification: all compile-time features desugar to meta-system
//! operations, providing one coherent model with convenient syntax sugar.

// Re-export from the main modules (for now, to maintain backward compatibility)
// These can be moved here gradually as the codebase migrates
pub use super::arithmetic;
pub use super::collections;
pub use super::code_gen;

use super::context_requirements::BuiltinRegistry;

/// Register all Tier 0 builtins to the registry
pub fn register_all(registry: &mut BuiltinRegistry) {
    arithmetic::register_builtins(registry);
    collections::register_builtins(registry);
    code_gen::register_tier0_builtins(registry);
}
