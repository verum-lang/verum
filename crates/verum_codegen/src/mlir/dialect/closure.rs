//! Comprehensive closure dialect operations for Verum.
//!
//! This module implements industrial-grade closure support, including:
//! - Closure creation with capture analysis
//! - Three capture modes: by value, by reference, by move
//! - Closure type representation
//! - Indirect function calls
//! - Closure optimization passes
//!
//! # Closure Layout
//!
//! ```text
//! Closure<F> = {
//!     fn_ptr: *const (),      // Pointer to closure function
//!     env_ptr: *mut Env<F>,   // Pointer to captured environment
//!     drop_fn: *const (),     // Optional destructor for env
//! }
//!
//! Env<F> = {
//!     capture_0: T0,          // First captured variable
//!     capture_1: T1,          // Second captured variable
//!     ...
//! }
//! ```
//!
//! # Capture Modes
//!
//! | Mode     | Symbol | Behavior |
//! |----------|--------|----------|
//! | ByValue  | `=`    | Copy/clone value into closure |
//! | ByRef    | `&`    | Capture reference to value |
//! | ByMove   | `move` | Move value into closure |

use verum_mlir::{
    Context,
    ir::{
        Attribute, Block, Identifier, Location, Operation, Region, Type, Value,
        attribute::{
            ArrayAttribute, DenseI32ArrayAttribute, IntegerAttribute,
            StringAttribute, TypeAttribute,
        },
        operation::{OperationBuilder, OperationLike},
        r#type::{FunctionType, IntegerType},
    },
};
use verum_common::{List, Text};
use crate::mlir::error::{MlirError, Result};
use crate::mlir::dialect::types::VerumType;

// ============================================================================
// Capture Analysis
// ============================================================================

/// Capture mode for closure variables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureMode {
    /// Capture by value (copy/clone).
    ByValue,
    /// Capture by immutable reference.
    ByRef,
    /// Capture by mutable reference.
    ByRefMut,
    /// Capture by move (ownership transfer).
    ByMove,
}

impl CaptureMode {
    /// Get attribute value for this mode.
    pub fn to_attr_value(&self) -> i64 {
        match self {
            Self::ByValue => 0,
            Self::ByRef => 1,
            Self::ByRefMut => 2,
            Self::ByMove => 3,
        }
    }

    /// Create from attribute value.
    pub fn from_attr_value(value: i64) -> Self {
        match value {
            0 => Self::ByValue,
            1 => Self::ByRef,
            2 => Self::ByRefMut,
            3 => Self::ByMove,
            _ => Self::ByValue,
        }
    }

    /// Check if this mode requires cleanup on closure drop.
    pub fn requires_cleanup(&self) -> bool {
        matches!(self, Self::ByMove)
    }

    /// Get the MLIR attribute name for this mode.
    pub fn attr_name(&self) -> &'static str {
        match self {
            Self::ByValue => "by_value",
            Self::ByRef => "by_ref",
            Self::ByRefMut => "by_ref_mut",
            Self::ByMove => "by_move",
        }
    }
}

/// A captured variable in a closure.
#[derive(Debug, Clone)]
pub struct CapturedVar {
    /// Variable name.
    pub name: Text,
    /// Capture mode.
    pub mode: CaptureMode,
    /// Type of the captured variable.
    pub ty: VerumType,
    /// Index in the environment struct.
    pub env_index: usize,
}

/// Closure environment analysis result.
#[derive(Debug, Clone)]
pub struct ClosureEnv {
    /// Captured variables.
    pub captures: Vec<CapturedVar>,
    /// Whether the closure needs a destructor.
    pub needs_drop: bool,
    /// Size of the environment in bytes.
    pub env_size: usize,
    /// Whether the environment can be inlined.
    pub can_inline: bool,
}

impl ClosureEnv {
    /// Create a new empty environment.
    pub fn new() -> Self {
        Self {
            captures: Vec::new(),
            needs_drop: false,
            env_size: 0,
            can_inline: true,
        }
    }

    /// Add a captured variable.
    pub fn add_capture(&mut self, name: Text, mode: CaptureMode, ty: VerumType) {
        let env_index = self.captures.len();
        self.captures.push(CapturedVar {
            name,
            mode,
            ty,
            env_index,
        });

        // Update needs_drop if this capture requires cleanup
        if mode.requires_cleanup() {
            self.needs_drop = true;
        }
    }

    /// Check if the closure captures any variables.
    pub fn is_empty(&self) -> bool {
        self.captures.is_empty()
    }

    /// Get a captured variable by name.
    pub fn get(&self, name: &str) -> Option<&CapturedVar> {
        self.captures.iter().find(|c| c.name.as_str() == name)
    }
}

impl Default for ClosureEnv {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Closure Operations
// ============================================================================

/// Closure create operation.
///
/// Creates a closure from a function and captured environment.
///
/// ```mlir
/// %closure = verum.closure_create @fn_name, [%cap0, %cap1] {
///     capture_modes = ["by_value", "by_ref"],
///     fn_type = (i64) -> i64
/// } : !verum.closure<(i64) -> i64>
/// ```
pub struct ClosureCreateOp;

impl ClosureCreateOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        fn_name: &str,
        captures: &[Value<'c, '_>],
        capture_modes: &[CaptureMode],
        fn_type: Type<'c>,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        let mode_attrs: Vec<Attribute<'c>> = capture_modes
            .iter()
            .map(|m| StringAttribute::new(context, m.attr_name()).into())
            .collect();

        OperationBuilder::new("verum.closure_create", location)
            .add_operands(captures)
            .add_attributes(&[
                (
                    Identifier::new(context, "fn_name"),
                    StringAttribute::new(context, fn_name).into(),
                ),
                (
                    Identifier::new(context, "capture_modes"),
                    ArrayAttribute::new(context, &mode_attrs).into(),
                ),
                (
                    Identifier::new(context, "fn_type"),
                    TypeAttribute::new(fn_type).into(),
                ),
            ])
            .add_results(&[result_type])
            .build()
            .map_err(|e| MlirError::operation("verum.closure_create", format!("{:?}", e)))
    }

    /// Build a simple closure with all captures by value.
    pub fn build_simple<'c>(
        context: &'c Context,
        location: Location<'c>,
        fn_name: &str,
        captures: &[Value<'c, '_>],
        fn_type: Type<'c>,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        let modes: Vec<CaptureMode> = captures.iter().map(|_| CaptureMode::ByValue).collect();
        Self::build(context, location, fn_name, captures, &modes, fn_type, result_type)
    }
}

/// Closure call operation.
///
/// Calls a closure with the given arguments.
///
/// ```mlir
/// %result = verum.closure_call %closure(%arg0, %arg1) : (i64, i64) -> i64
/// ```
pub struct ClosureCallOp;

impl ClosureCallOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        closure: Value<'c, '_>,
        args: &[Value<'c, '_>],
        result_types: &[Type<'c>],
    ) -> Result<Operation<'c>> {
        let mut operands = vec![closure];
        operands.extend(args.iter().copied());

        OperationBuilder::new("verum.closure_call", location)
            .add_operands(&operands)
            .add_results(result_types)
            .build()
            .map_err(|e| MlirError::operation("verum.closure_call", format!("{:?}", e)))
    }
}

/// Closure environment load operation.
///
/// Loads a captured value from the closure environment.
///
/// ```mlir
/// %value = verum.closure_env_load %env, 0 : i64
/// ```
pub struct ClosureEnvLoadOp;

impl ClosureEnvLoadOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        env: Value<'c, '_>,
        index: usize,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        let index_type = IntegerType::new(context, 64).into();

        OperationBuilder::new("verum.closure_env_load", location)
            .add_operands(&[env])
            .add_attributes(&[(
                Identifier::new(context, "index"),
                IntegerAttribute::new(index_type, index as i64).into(),
            )])
            .add_results(&[result_type])
            .build()
            .map_err(|e| MlirError::operation("verum.closure_env_load", format!("{:?}", e)))
    }
}

/// Closure environment store operation.
///
/// Stores a value into the closure environment.
///
/// ```mlir
/// verum.closure_env_store %env, 0, %value : i64
/// ```
pub struct ClosureEnvStoreOp;

impl ClosureEnvStoreOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        env: Value<'c, '_>,
        index: usize,
        value: Value<'c, '_>,
    ) -> Result<Operation<'c>> {
        let index_type = IntegerType::new(context, 64).into();

        OperationBuilder::new("verum.closure_env_store", location)
            .add_operands(&[env, value])
            .add_attributes(&[(
                Identifier::new(context, "index"),
                IntegerAttribute::new(index_type, index as i64).into(),
            )])
            .build()
            .map_err(|e| MlirError::operation("verum.closure_env_store", format!("{:?}", e)))
    }
}

/// Closure environment allocate operation.
///
/// Allocates a new closure environment.
///
/// ```mlir
/// %env = verum.closure_env_alloc {
///     size = 32 : i64,
///     alignment = 8 : i64
/// } : !llvm.ptr
/// ```
pub struct ClosureEnvAllocOp;

impl ClosureEnvAllocOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        size: usize,
        alignment: usize,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        let i64_type = IntegerType::new(context, 64).into();

        OperationBuilder::new("verum.closure_env_alloc", location)
            .add_attributes(&[
                (
                    Identifier::new(context, "size"),
                    IntegerAttribute::new(i64_type, size as i64).into(),
                ),
                (
                    Identifier::new(context, "alignment"),
                    IntegerAttribute::new(i64_type, alignment as i64).into(),
                ),
            ])
            .add_results(&[result_type])
            .build()
            .map_err(|e| MlirError::operation("verum.closure_env_alloc", format!("{:?}", e)))
    }
}

/// Closure environment free operation.
///
/// Frees a closure environment.
///
/// ```mlir
/// verum.closure_env_free %env
/// ```
pub struct ClosureEnvFreeOp;

impl ClosureEnvFreeOp {
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        env: Value<'c, '_>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.closure_env_free", location)
            .add_operands(&[env])
            .build()
            .map_err(|e| MlirError::operation("verum.closure_env_free", format!("{:?}", e)))
    }
}

/// Closure drop operation.
///
/// Drops a closure, cleaning up its environment.
///
/// ```mlir
/// verum.closure_drop %closure
/// ```
pub struct ClosureDropOp;

impl ClosureDropOp {
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        closure: Value<'c, '_>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.closure_drop", location)
            .add_operands(&[closure])
            .build()
            .map_err(|e| MlirError::operation("verum.closure_drop", format!("{:?}", e)))
    }
}

// ============================================================================
// Function Pointer Operations
// ============================================================================

/// Function pointer create operation.
///
/// Creates a function pointer from a function name.
///
/// ```mlir
/// %fn_ptr = verum.fn_ptr @my_function : !verum.fn_ptr<(i64) -> i64>
/// ```
pub struct FnPtrCreateOp;

impl FnPtrCreateOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        fn_name: &str,
        fn_type: Type<'c>,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.fn_ptr", location)
            .add_attributes(&[
                (
                    Identifier::new(context, "fn_name"),
                    StringAttribute::new(context, fn_name).into(),
                ),
                (
                    Identifier::new(context, "fn_type"),
                    TypeAttribute::new(fn_type).into(),
                ),
            ])
            .add_results(&[result_type])
            .build()
            .map_err(|e| MlirError::operation("verum.fn_ptr", format!("{:?}", e)))
    }
}

/// Indirect call operation.
///
/// Calls a function through a function pointer.
///
/// ```mlir
/// %result = verum.indirect_call %fn_ptr(%arg0, %arg1) : (i64, i64) -> i64
/// ```
pub struct IndirectCallOp;

impl IndirectCallOp {
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        fn_ptr: Value<'c, '_>,
        args: &[Value<'c, '_>],
        result_types: &[Type<'c>],
    ) -> Result<Operation<'c>> {
        let mut operands = vec![fn_ptr];
        operands.extend(args.iter().copied());

        OperationBuilder::new("verum.indirect_call", location)
            .add_operands(&operands)
            .add_results(result_types)
            .build()
            .map_err(|e| MlirError::operation("verum.indirect_call", format!("{:?}", e)))
    }
}

// ============================================================================
// Method Call Operations
// ============================================================================

/// Method call operation.
///
/// Calls a method on an object (for trait objects/protocols).
///
/// ```mlir
/// %result = verum.method_call %obj, "method_name"(%arg0) {
///     vtable_index = 3 : i32
/// } : (i64) -> i64
/// ```
pub struct MethodCallOp;

impl MethodCallOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        obj: Value<'c, '_>,
        method_name: &str,
        args: &[Value<'c, '_>],
        vtable_index: Option<i32>,
        result_types: &[Type<'c>],
    ) -> Result<Operation<'c>> {
        let mut operands = vec![obj];
        operands.extend(args.iter().copied());

        let mut attrs = vec![(
            Identifier::new(context, "method_name"),
            StringAttribute::new(context, method_name).into(),
        )];

        if let Some(idx) = vtable_index {
            let i32_type = IntegerType::new(context, 32).into();
            attrs.push((
                Identifier::new(context, "vtable_index"),
                IntegerAttribute::new(i32_type, idx as i64).into(),
            ));
        }

        OperationBuilder::new("verum.method_call", location)
            .add_operands(&operands)
            .add_attributes(&attrs)
            .add_results(result_types)
            .build()
            .map_err(|e| MlirError::operation("verum.method_call", format!("{:?}", e)))
    }
}

/// VTable lookup operation.
///
/// Looks up a method in a VTable.
///
/// ```mlir
/// %fn_ptr = verum.vtable_lookup %vtable, 3 : !verum.fn_ptr<(i64) -> i64>
/// ```
pub struct VTableLookupOp;

impl VTableLookupOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        vtable: Value<'c, '_>,
        index: i32,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        let i32_type = IntegerType::new(context, 32).into();

        OperationBuilder::new("verum.vtable_lookup", location)
            .add_operands(&[vtable])
            .add_attributes(&[(
                Identifier::new(context, "index"),
                IntegerAttribute::new(i32_type, index as i64).into(),
            )])
            .add_results(&[result_type])
            .build()
            .map_err(|e| MlirError::operation("verum.vtable_lookup", format!("{:?}", e)))
    }
}

// ============================================================================
// Closure Type Builder
// ============================================================================

/// Builder for closure-related types.
pub struct ClosureTypeBuilder<'c> {
    context: &'c Context,
}

impl<'c> ClosureTypeBuilder<'c> {
    /// Create a new closure type builder.
    pub fn new(context: &'c Context) -> Self {
        Self { context }
    }

    /// Create a closure type.
    ///
    /// Closure = { fn_ptr, env_ptr, drop_fn }
    pub fn closure_type(&self) -> Result<Type<'c>> {
        Type::parse(self.context, "!llvm.struct<(ptr, ptr, ptr)>")
            .ok_or_else(|| MlirError::type_translation("closure", "failed to parse"))
    }

    /// Create a function pointer type.
    pub fn fn_ptr_type(&self) -> Result<Type<'c>> {
        Type::parse(self.context, "!llvm.ptr")
            .ok_or_else(|| MlirError::type_translation("fn_ptr", "failed to parse"))
    }

    /// Create a vtable type.
    pub fn vtable_type(&self, method_count: usize) -> Result<Type<'c>> {
        // VTable is an array of function pointers
        let type_str = format!("!llvm.array<{} x ptr>", method_count);
        Type::parse(self.context, &type_str)
            .ok_or_else(|| MlirError::type_translation("vtable", "failed to parse"))
    }

    /// Create an environment type for given captures.
    pub fn env_type(&self, capture_count: usize) -> Result<Type<'c>> {
        if capture_count == 0 {
            // Empty environment - just use a pointer
            return Type::parse(self.context, "!llvm.ptr")
                .ok_or_else(|| MlirError::type_translation("env", "failed to parse"));
        }

        // Environment is a struct of captured values
        // For simplicity, use i64 for all captures
        let fields = vec!["i64"; capture_count];
        let type_str = format!("!llvm.struct<({})>", fields.join(", "));
        Type::parse(self.context, &type_str)
            .ok_or_else(|| MlirError::type_translation("env", "failed to parse"))
    }
}

// ============================================================================
// Closure Lowering Helper
// ============================================================================

/// Helper for lowering closures.
pub struct ClosureLowering<'c> {
    /// Type builder.
    types: ClosureTypeBuilder<'c>,
    /// Counter for generating unique names.
    counter: usize,
}

impl<'c> ClosureLowering<'c> {
    /// Create a new closure lowering helper.
    pub fn new(context: &'c Context) -> Self {
        Self {
            types: ClosureTypeBuilder::new(context),
            counter: 0,
        }
    }

    /// Generate a unique closure function name.
    pub fn fresh_closure_name(&mut self, prefix: &str) -> Text {
        self.counter += 1;
        Text::from(format!("{}_closure_{}", prefix, self.counter))
    }

    /// Generate a unique environment struct name.
    pub fn fresh_env_name(&mut self, prefix: &str) -> Text {
        self.counter += 1;
        Text::from(format!("{}_env_{}", prefix, self.counter))
    }

    /// Get the closure type.
    pub fn closure_type(&self) -> Result<Type<'c>> {
        self.types.closure_type()
    }

    /// Get the environment type for given capture count.
    pub fn env_type(&self, capture_count: usize) -> Result<Type<'c>> {
        self.types.env_type(capture_count)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capture_mode() {
        assert_eq!(CaptureMode::ByValue.to_attr_value(), 0);
        assert_eq!(CaptureMode::ByRef.to_attr_value(), 1);
        assert_eq!(CaptureMode::ByMove.to_attr_value(), 3);

        assert_eq!(CaptureMode::from_attr_value(0), CaptureMode::ByValue);
        assert_eq!(CaptureMode::from_attr_value(1), CaptureMode::ByRef);

        assert!(!CaptureMode::ByValue.requires_cleanup());
        assert!(CaptureMode::ByMove.requires_cleanup());
    }

    #[test]
    fn test_closure_env() {
        let mut env = ClosureEnv::new();
        assert!(env.is_empty());

        env.add_capture(Text::from("x"), CaptureMode::ByValue, VerumType::i64());
        assert!(!env.is_empty());
        assert_eq!(env.captures.len(), 1);
        assert!(!env.needs_drop);

        env.add_capture(Text::from("y"), CaptureMode::ByMove, VerumType::i64());
        assert!(env.needs_drop);
    }

    #[test]
    fn test_closure_env_get() {
        let mut env = ClosureEnv::new();
        env.add_capture(Text::from("x"), CaptureMode::ByValue, VerumType::i64());
        env.add_capture(Text::from("y"), CaptureMode::ByRef, VerumType::i64());

        assert!(env.get("x").is_some());
        assert!(env.get("y").is_some());
        assert!(env.get("z").is_none());

        let x = env.get("x").unwrap();
        assert_eq!(x.env_index, 0);
        assert_eq!(x.mode, CaptureMode::ByValue);
    }
}
