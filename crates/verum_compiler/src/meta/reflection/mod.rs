//! Reflection API for compile-time introspection
//!
//! This module provides type introspection capabilities for meta functions,
//! enabling compile-time inspection of types, fields, variants, and methods.
//!
//! ## Module Structure
//!
//! - [`type_kind`] - Type classification (Struct, Enum, Protocol, etc.)
//! - [`field_info`] - Struct field metadata
//! - [`variant_info`] - Enum variant metadata
//! - [`generic_param`] - Generic parameter information
//! - [`param_info`] - Function parameter metadata
//! - [`function_info`] - Function signature information
//! - [`protocol_info`] - Protocol/trait metadata
//! - [`trait_bound`] - Trait bound information
//! - [`ownership_info`] - Ownership and thread safety metadata
//! - [`method_resolution`] - Method resolution results
//! - [`primitive_type`] - Primitive type information
//! - [`type_info`] - Complete type information
//!
//! ## Usage
//!
//! These types are used by the meta system to provide compile-time reflection:
//!
//! ```ignore
//! // Get fields of a struct at compile time
//! let fields = fields_of(Point);
//! for field in fields {
//!     emit_field_accessor(field.name, field.type_name);
//! }
//! ```
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).

pub mod field_info;
pub mod function_info;
pub mod generic_param;
pub mod method_resolution;
pub mod ownership_info;
pub mod param_info;
pub mod primitive_type;
pub mod protocol_info;
pub mod trait_bound;
pub mod type_info;
pub mod type_kind;
pub mod variant_info;

// Re-export all public types for convenient access
pub use field_info::{FieldInfo, FieldOffset};
pub use function_info::FunctionInfo;
pub use generic_param::{GenericParam, GenericParamKind, LifetimeParam};
pub use method_resolution::{MethodResolution, MethodSource};
pub use ownership_info::OwnershipInfo;
pub use param_info::{ParamInfo, SelfKind};
pub use primitive_type::PrimitiveType;
pub use protocol_info::{AssociatedTypeInfo, ProtocolInfo};
pub use trait_bound::TraitBound;
pub use type_info::TypeInfo;
pub use type_kind::{TypeKind, VariantKind, Visibility};
pub use variant_info::VariantInfo;
