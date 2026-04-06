//! Comprehensive Context System dialect operations for Verum.
//!
//! The context system provides dependency injection (DI) capabilities in Verum,
//! allowing functions to declare required contexts and receive them implicitly.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                      Context Stack                               │
//! ├─────────────────────────────────────────────────────────────────┤
//! │  Frame N:   { Database, Logger, Config }                        │
//! │  Frame N-1: { Database, Config }                                │
//! │  Frame 0:   { Config }  ← root contexts                         │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Context Lookup (~5-30ns overhead)
//!
//! 1. Check current frame for context
//! 2. Walk up the stack if not found
//! 3. Monomorphization pass can eliminate lookups
//!
//! # Operations
//!
//! - `context_get`: Retrieve context value
//! - `context_provide`: Provide new context value
//! - `context_scope`: Scoped context provision
//! - `context_require`: Assert context availability
//! - `context_with`: Transform context value

use verum_mlir::{
    Context,
    ir::{
        Attribute, Block, Identifier, Location, Module, Operation, Region, Type, Value,
        attribute::{
            ArrayAttribute, DenseI32ArrayAttribute, DenseI64ArrayAttribute,
            IntegerAttribute, StringAttribute, TypeAttribute,
        },
        operation::{OperationBuilder, OperationLike},
        r#type::IntegerType,
    },
};
use verum_common::Text;
use crate::mlir::error::{MlirError, Result};
use crate::mlir::dialect::{op_names, attr_names};

// ============================================================================
// Context Type System
// ============================================================================

/// Context capability - what can be done with a context value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextCapability {
    /// Context can be read.
    Read,
    /// Context can be modified.
    Modify,
    /// Context can be replaced.
    Replace,
    /// Context can be inherited by child scopes.
    Inherit,
}

/// Context lifetime - how long a context value lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextLifetime {
    /// Scoped - lives until scope exit.
    Scoped,
    /// Static - lives for program duration.
    Static,
    /// Request - lives for a single request/transaction.
    Request,
    /// Custom - user-defined lifetime.
    Custom,
}

impl ContextLifetime {
    pub fn to_attr_value(&self) -> i64 {
        match self {
            Self::Scoped => 0,
            Self::Static => 1,
            Self::Request => 2,
            Self::Custom => 3,
        }
    }

    pub fn from_attr_value(value: i64) -> Self {
        match value {
            0 => Self::Scoped,
            1 => Self::Static,
            2 => Self::Request,
            _ => Self::Custom,
        }
    }
}

/// Context resolution strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextResolution {
    /// Direct lookup - O(1) if monomorphized.
    Direct,
    /// Stack walk - O(n) worst case.
    StackWalk,
    /// Cached - previous lookup result cached.
    Cached,
}

// ============================================================================
// Context Operations - Get
// ============================================================================

/// Context get operation - retrieves a context value.
///
/// ```mlir
/// %db = verum.context_get "Database" {
///     resolution = "direct",
///     cached = true,
///     fallback = none
/// } : !verum.context<Database>
/// ```
pub struct ContextGetOp;

impl ContextGetOp {
    /// Build a full context get operation.
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        context_name: &str,
        result_type: Type<'c>,
        resolution: ContextResolution,
        cached: bool,
        has_fallback: bool,
    ) -> Result<Operation<'c>> {
        let i1_type: Type<'c> = IntegerType::new(context, 1).into();

        let attrs = vec![
            (
                Identifier::new(context, attr_names::CONTEXT_NAME),
                StringAttribute::new(context, context_name).into(),
            ),
            (
                Identifier::new(context, "resolution"),
                StringAttribute::new(
                    context,
                    match resolution {
                        ContextResolution::Direct => "direct",
                        ContextResolution::StackWalk => "stack_walk",
                        ContextResolution::Cached => "cached",
                    },
                )
                .into(),
            ),
            (
                Identifier::new(context, "cached"),
                IntegerAttribute::new(i1_type, if cached { 1 } else { 0 }).into(),
            ),
            (
                Identifier::new(context, "has_fallback"),
                IntegerAttribute::new(i1_type, if has_fallback { 1 } else { 0 }).into(),
            ),
        ];

        OperationBuilder::new(op_names::CONTEXT_GET, location)
            .add_attributes(&attrs)
            .add_results(&[result_type])
            .build()
            .map_err(|e| MlirError::operation(op_names::CONTEXT_GET, format!("{:?}", e)))
    }

    /// Build a simple context get operation.
    pub fn build_simple<'c>(
        context: &'c Context,
        location: Location<'c>,
        context_name: &str,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        Self::build(
            context,
            location,
            context_name,
            result_type,
            ContextResolution::StackWalk,
            false,
            false,
        )
    }
}

/// Context get with fallback operation.
///
/// ```mlir
/// %db = verum.context_get_or "Database", %default : !verum.context<Database>
/// ```
pub struct ContextGetOrOp;

impl ContextGetOrOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        context_name: &str,
        fallback: Value<'c, '_>,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.context_get_or", location)
            .add_operands(&[fallback])
            .add_attributes(&[(
                Identifier::new(context, attr_names::CONTEXT_NAME),
                StringAttribute::new(context, context_name).into(),
            )])
            .add_results(&[result_type])
            .build()
            .map_err(|e| MlirError::operation("verum.context_get_or", format!("{:?}", e)))
    }
}

/// Context try-get operation - returns Maybe<T>.
///
/// ```mlir
/// %maybe_db = verum.context_try_get "Database" : !verum.maybe<!verum.context<Database>>
/// ```
pub struct ContextTryGetOp;

impl ContextTryGetOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        context_name: &str,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.context_try_get", location)
            .add_attributes(&[(
                Identifier::new(context, attr_names::CONTEXT_NAME),
                StringAttribute::new(context, context_name).into(),
            )])
            .add_results(&[result_type])
            .build()
            .map_err(|e| MlirError::operation("verum.context_try_get", format!("{:?}", e)))
    }
}

// ============================================================================
// Context Operations - Provide
// ============================================================================

/// Context provide operation - provides a context value.
///
/// ```mlir
/// verum.context_provide "Database" = %db {
///     lifetime = "scoped",
///     inherit = true
/// } : !verum.context<Database>
/// ```
pub struct ContextProvideOp;

impl ContextProvideOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        context_name: &str,
        value: Value<'c, '_>,
        lifetime: ContextLifetime,
        inherit: bool,
    ) -> Result<Operation<'c>> {
        let i32_type: Type<'c> = IntegerType::new(context, 32).into();
        let i1_type: Type<'c> = IntegerType::new(context, 1).into();

        OperationBuilder::new(op_names::CONTEXT_PROVIDE, location)
            .add_operands(&[value])
            .add_attributes(&[
                (
                    Identifier::new(context, attr_names::CONTEXT_NAME),
                    StringAttribute::new(context, context_name).into(),
                ),
                (
                    Identifier::new(context, "lifetime"),
                    IntegerAttribute::new(i32_type, lifetime.to_attr_value()).into(),
                ),
                (
                    Identifier::new(context, "inherit"),
                    IntegerAttribute::new(i1_type, if inherit { 1 } else { 0 }).into(),
                ),
            ])
            .build()
            .map_err(|e| MlirError::operation(op_names::CONTEXT_PROVIDE, format!("{:?}", e)))
    }

    /// Build a simple context provide operation.
    pub fn build_simple<'c>(
        context: &'c Context,
        location: Location<'c>,
        context_name: &str,
        value: Value<'c, '_>,
    ) -> Result<Operation<'c>> {
        Self::build(context, location, context_name, value, ContextLifetime::Scoped, true)
    }
}

/// Context provide with alias operation.
///
/// ```mlir
/// verum.context_provide_as "Database", "db" = %conn : !verum.context<Database>
/// ```
pub struct ContextProvideAsOp;

impl ContextProvideAsOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        context_name: &str,
        alias: &str,
        value: Value<'c, '_>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.context_provide_as", location)
            .add_operands(&[value])
            .add_attributes(&[
                (
                    Identifier::new(context, attr_names::CONTEXT_NAME),
                    StringAttribute::new(context, context_name).into(),
                ),
                (
                    Identifier::new(context, "alias"),
                    StringAttribute::new(context, alias).into(),
                ),
            ])
            .build()
            .map_err(|e| MlirError::operation("verum.context_provide_as", format!("{:?}", e)))
    }
}

// ============================================================================
// Context Operations - Scoped
// ============================================================================

/// Context scope operation - provides context for a region.
///
/// ```mlir
/// %result = verum.context_scope ["Database" = %db, "Logger" = %log] {
///     // body uses provided contexts
///     verum.context_yield %value : T
/// } : T
/// ```
pub struct ContextScopeOp;

impl ContextScopeOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        context_names: &[&str],
        context_values: &[Value<'c, '_>],
        result_types: &[Type<'c>],
        body: Region<'c>,
    ) -> Result<Operation<'c>> {
        // Create array attribute for context names
        let name_attrs: Vec<Attribute<'c>> = context_names
            .iter()
            .map(|n| StringAttribute::new(context, n).into())
            .collect();

        OperationBuilder::new("verum.context_scope", location)
            .add_operands(context_values)
            .add_attributes(&[(
                Identifier::new(context, "context_names"),
                ArrayAttribute::new(context, &name_attrs).into(),
            )])
            .add_results(result_types)
            .add_regions([body])
            .build()
            .map_err(|e| MlirError::operation("verum.context_scope", format!("{:?}", e)))
    }
}

/// Context yield operation - terminates a context scope.
pub struct ContextYieldOp;

impl ContextYieldOp {
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        values: &[Value<'c, '_>],
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.context_yield", location)
            .add_operands(values)
            .build()
            .map_err(|e| MlirError::operation("verum.context_yield", format!("{:?}", e)))
    }
}

// ============================================================================
// Context Operations - Requirements
// ============================================================================

/// Context require operation - asserts context availability.
///
/// ```mlir
/// verum.context_require ["Database", "Logger"] {
///     message = "Database and Logger contexts required"
/// }
/// ```
pub struct ContextRequireOp;

impl ContextRequireOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        required_contexts: &[&str],
        message: Option<&str>,
    ) -> Result<Operation<'c>> {
        let name_attrs: Vec<Attribute<'c>> = required_contexts
            .iter()
            .map(|n| StringAttribute::new(context, n).into())
            .collect();

        let mut attrs = vec![(
            Identifier::new(context, attr_names::REQUIRED_CONTEXTS),
            ArrayAttribute::new(context, &name_attrs).into(),
        )];

        if let Some(msg) = message {
            attrs.push((
                Identifier::new(context, "message"),
                StringAttribute::new(context, msg).into(),
            ));
        }

        OperationBuilder::new("verum.context_require", location)
            .add_attributes(&attrs)
            .build()
            .map_err(|e| MlirError::operation("verum.context_require", format!("{:?}", e)))
    }
}

/// Context check operation - returns bool for context availability.
///
/// ```mlir
/// %has_db = verum.context_has "Database" : i1
/// ```
pub struct ContextHasOp;

impl ContextHasOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        context_name: &str,
    ) -> Result<Operation<'c>> {
        let i1_type: Type<'c> = IntegerType::new(context, 1).into();

        OperationBuilder::new("verum.context_has", location)
            .add_attributes(&[(
                Identifier::new(context, attr_names::CONTEXT_NAME),
                StringAttribute::new(context, context_name).into(),
            )])
            .add_results(&[i1_type])
            .build()
            .map_err(|e| MlirError::operation("verum.context_has", format!("{:?}", e)))
    }
}

// ============================================================================
// Context Operations - Transformation
// ============================================================================

/// Context with operation - transforms a context value.
///
/// ```mlir
/// %result = verum.context_with "Database" {
///     ^bb0(%db: !verum.context<Database>):
///         // transform db
///         verum.context_with_yield %transformed : T
/// } : T
/// ```
pub struct ContextWithOp;

impl ContextWithOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        context_name: &str,
        result_types: &[Type<'c>],
        body: Region<'c>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.context_with", location)
            .add_attributes(&[(
                Identifier::new(context, attr_names::CONTEXT_NAME),
                StringAttribute::new(context, context_name).into(),
            )])
            .add_results(result_types)
            .add_regions([body])
            .build()
            .map_err(|e| MlirError::operation("verum.context_with", format!("{:?}", e)))
    }
}

/// Context with yield operation.
pub struct ContextWithYieldOp;

impl ContextWithYieldOp {
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        values: &[Value<'c, '_>],
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.context_with_yield", location)
            .add_operands(values)
            .build()
            .map_err(|e| MlirError::operation("verum.context_with_yield", format!("{:?}", e)))
    }
}

// ============================================================================
// Context Operations - Stack Management
// ============================================================================

/// Context push frame operation - for manual stack management.
///
/// ```mlir
/// %frame = verum.context_push_frame : !verum.context_frame
/// ```
pub struct ContextPushFrameOp;

impl ContextPushFrameOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        frame_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.context_push_frame", location)
            .add_results(&[frame_type])
            .build()
            .map_err(|e| MlirError::operation("verum.context_push_frame", format!("{:?}", e)))
    }
}

/// Context pop frame operation.
///
/// ```mlir
/// verum.context_pop_frame %frame : !verum.context_frame
/// ```
pub struct ContextPopFrameOp;

impl ContextPopFrameOp {
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        frame: Value<'c, '_>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.context_pop_frame", location)
            .add_operands(&[frame])
            .build()
            .map_err(|e| MlirError::operation("verum.context_pop_frame", format!("{:?}", e)))
    }
}

// ============================================================================
// Context Operations - Monomorphization Support
// ============================================================================

/// Context monomorphize marker - marks a context access for monomorphization.
///
/// ```mlir
/// %db = verum.context_mono "Database" {
///     concrete_type = !my.database_impl,
///     inline_access = true
/// } : !verum.context<Database>
/// ```
pub struct ContextMonoOp;

impl ContextMonoOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        context_name: &str,
        concrete_type: Type<'c>,
        inline_access: bool,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        let i1_type: Type<'c> = IntegerType::new(context, 1).into();

        OperationBuilder::new("verum.context_mono", location)
            .add_attributes(&[
                (
                    Identifier::new(context, attr_names::CONTEXT_NAME),
                    StringAttribute::new(context, context_name).into(),
                ),
                (
                    Identifier::new(context, "concrete_type"),
                    TypeAttribute::new(concrete_type).into(),
                ),
                (
                    Identifier::new(context, "inline_access"),
                    IntegerAttribute::new(i1_type, if inline_access { 1 } else { 0 }).into(),
                ),
            ])
            .add_results(&[result_type])
            .build()
            .map_err(|e| MlirError::operation("verum.context_mono", format!("{:?}", e)))
    }
}

// ============================================================================
// Context Function Attributes
// ============================================================================

/// Creates the required contexts attribute for a function.
pub fn create_required_contexts_attr<'c>(
    context: &'c Context,
    contexts: &[&str],
) -> Attribute<'c> {
    let attrs: Vec<Attribute<'c>> = contexts
        .iter()
        .map(|c| StringAttribute::new(context, c).into())
        .collect();
    ArrayAttribute::new(context, &attrs).into()
}

/// Creates the provided contexts attribute for a function.
pub fn create_provided_contexts_attr<'c>(
    context: &'c Context,
    contexts: &[&str],
) -> Attribute<'c> {
    let attrs: Vec<Attribute<'c>> = contexts
        .iter()
        .map(|c| StringAttribute::new(context, c).into())
        .collect();
    ArrayAttribute::new(context, &attrs).into()
}

// ============================================================================
// Context Type Builder
// ============================================================================

/// Builder for context-related types.
pub struct ContextTypeBuilder<'c> {
    context: &'c Context,
}

impl<'c> ContextTypeBuilder<'c> {
    /// Create a new context type builder.
    pub fn new(context: &'c Context) -> Self {
        Self { context }
    }

    /// Create a context type for a named context.
    pub fn context_type(&self, _name: &str) -> Result<Type<'c>> {
        // Contexts are represented as opaque pointers
        Type::parse(self.context, "!llvm.ptr")
            .ok_or_else(|| MlirError::type_translation("context", "failed to parse"))
    }

    /// Create a context frame type.
    pub fn context_frame_type(&self) -> Result<Type<'c>> {
        // Frame is a struct with frame pointer and count
        Type::parse(self.context, "!llvm.struct<(ptr, i32)>")
            .ok_or_else(|| MlirError::type_translation("context_frame", "failed to parse"))
    }

    /// Create a context stack type.
    pub fn context_stack_type(&self) -> Result<Type<'c>> {
        // Stack is an opaque pointer to the stack structure
        Type::parse(self.context, "!llvm.ptr")
            .ok_or_else(|| MlirError::type_translation("context_stack", "failed to parse"))
    }
}

// ============================================================================
// Context Analysis Utilities
// ============================================================================

/// Analyze function for required contexts.
#[derive(Debug, Clone, Default)]
pub struct ContextAnalysis {
    /// Required contexts for this function.
    pub required: Vec<Text>,
    /// Contexts provided by this function.
    pub provided: Vec<Text>,
    /// Contexts accessed within the function.
    pub accessed: Vec<Text>,
    /// Whether the function can be monomorphized.
    pub can_monomorphize: bool,
}

impl ContextAnalysis {
    /// Create a new context analysis.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a required context.
    pub fn require(&mut self, name: impl Into<Text>) {
        let name = name.into();
        if !self.required.contains(&name) {
            self.required.push(name);
        }
    }

    /// Add a provided context.
    pub fn provide(&mut self, name: impl Into<Text>) {
        let name = name.into();
        if !self.provided.contains(&name) {
            self.provided.push(name);
        }
    }

    /// Add an accessed context.
    pub fn access(&mut self, name: impl Into<Text>) {
        let name = name.into();
        if !self.accessed.contains(&name) {
            self.accessed.push(name);
        }
    }

    /// Check if a context is required.
    pub fn requires(&self, name: &str) -> bool {
        self.required.iter().any(|n| n.as_str() == name)
    }

    /// Check if a context is provided.
    pub fn provides(&self, name: &str) -> bool {
        self.provided.iter().any(|n| n.as_str() == name)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_lifetime() {
        assert_eq!(ContextLifetime::Scoped.to_attr_value(), 0);
        assert_eq!(ContextLifetime::Static.to_attr_value(), 1);
        assert_eq!(ContextLifetime::from_attr_value(0), ContextLifetime::Scoped);
        assert_eq!(ContextLifetime::from_attr_value(1), ContextLifetime::Static);
    }

    #[test]
    fn test_context_analysis() {
        let mut analysis = ContextAnalysis::new();
        analysis.require("Database");
        analysis.provide("Logger");
        analysis.access("Config");

        assert!(analysis.requires("Database"));
        assert!(!analysis.requires("Config"));
        assert!(analysis.provides("Logger"));
        assert!(!analysis.provides("Database"));
    }

    #[test]
    fn test_context_analysis_dedup() {
        let mut analysis = ContextAnalysis::new();
        analysis.require("Database");
        analysis.require("Database");
        analysis.require("Database");

        assert_eq!(analysis.required.len(), 1);
    }
}
