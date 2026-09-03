//! Attribute system for Verum type checking and validation.
//!
//! This module provides the infrastructure for validating attributes
//! during type checking, including:
//!
//! - [`AttributeRegistry`]: Central registry of all known attributes
//! - [`registry()`] / [`registry_mut()`]: Access to global registry
//! - [`AttributeError`]: Validation error types with diagnostics
//! - Standard attribute registration
//!
//! # Overview
//!
//! The attribute system works in conjunction with `verum_ast::attr` to provide:
//!
//! 1. **Compile-time validation** - Unknown attributes, invalid targets, bad arguments
//! 2. **IDE support** - Completion, hover, diagnostics
//! 3. **Documentation** - Generated from attribute metadata
//!
//! # Usage
//!
//! ```rust
//! use verum_types::attr::{registry, AttributeRegistry};
//! use verum_ast::attr::{Attribute, AttributeTarget};
//!
//! // Access global registry
//! let reg = registry();
//!
//! // Validate an attribute
//! let attr = Attribute::simple("inline".into(), Default::default());
//! match reg.validate(&attr, AttributeTarget::Function) {
//!  Ok(result) => {
//!  for warning in result.warnings {
//!  println!("Warning: {}", warning.message());
//!  }
//!  }
//!  Err(e) => {
//!  println!("Error: {}", e.message());
//!  }
//! }
//! ```
//!
//! # Architecture
//!
//! ```text
//! verum_ast::attr verum_types::attr
//! ┌────────────────┐ ┌──────────────────┐
//! │ AttributeTarget│◄──────────│ AttributeRegistry│
//! │ ArgSpec │ │ │
//! │ Metadata │◄──────────│ Standard attrs │
//! │ Attribute │◄──────────│ Validation │
//! └────────────────┘ │ Error types │
//!  └──────────────────┘
//! ```
//!
//! # Specification
//!
//! Attribute registry: validation rules for @derive, @verify, @cfg, @repr and other compile-time attributes

mod error;
mod registry;
mod standard;

// Re-exports
pub use error::{AttributeError, errors_to_diagnostics};
pub use registry::{
    AttributeRegistry, REGISTRY, RegistryError, ValidationResult, ValidationWarning, registry,
    registry_mut,
};

// Re-export AST types for convenience
pub use verum_ast::attr::{
    ArgSpec, ArgType, Attribute, AttributeCategory, AttributeMetadata, AttributeTarget,
    FromAttribute, NamedArgSpec, Stability,
};

// `validate_attributes` REMOVED (T1073): superseded by
// `verum_fast_parser::attr_validation`, which is what actually runs —
// measured, an unknown attribute is caught there as W0400 while this
// path had no caller at all.  It fronted
// `AttributeRegistry::validate_collection`, whose only other mention
// in the workspace is a string literal in an LSP quick-fix template,
// so the whole chain was unreachable.

/// Check if an attribute name is known.
///
/// Convenience function that uses the global registry.
#[must_use]
pub fn is_known_attribute(name: &str) -> bool {
    registry().exists(name)
}

/// Get metadata for an attribute.
///
/// Convenience function that uses the global registry.
#[must_use]
pub fn get_attribute_metadata(name: &str) -> Option<AttributeMetadata> {
    registry().get(name).cloned()
}
