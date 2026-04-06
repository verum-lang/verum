//! Verum dialect operations.
//!
//! Custom MLIR operations for Verum language constructs. These operations
//! are built using MLIR's generic OperationBuilder with custom attributes.
//!
//! # CBGR Operations
//!
//! - `CbgrAllocOp`: Allocate CBGR-tracked memory
//! - `CbgrCheckOp`: Validate CBGR reference (generation check)
//! - `CbgrDerefOp`: Dereference with CBGR validation
//! - `CbgrDropOp`: Drop CBGR-tracked allocation
//!
//! # Context Operations
//!
//! - `ContextGetOp`: Get context value from environment
//! - `ContextProvideOp`: Provide context value
//!
//! # Async Operations
//!
//! - `SpawnOp`: Spawn async task
//! - `AwaitOp`: Await future completion
//! - `SelectOp`: Select on multiple futures

use verum_mlir::{
    Context,
    ir::{
        Block, Location, Module, Operation, OperationRef, Region, Type, Value,
        attribute::{IntegerAttribute, StringAttribute, TypeAttribute, Attribute},
        operation::OperationBuilder,
    },
};
use verum_common::Text;
use crate::mlir::error::{MlirError, Result};
use crate::mlir::dialect::{op_names, attr_names, types::RefTier};

/// CBGR allocation operation.
///
/// Allocates memory with CBGR tracking (generation + epoch).
///
/// ```mlir
/// %ref = verum.cbgr_alloc %value : T -> !verum.ref<T, managed>
/// ```
pub struct CbgrAllocOp;

impl CbgrAllocOp {
    /// Build a CBGR allocation operation.
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        value: Value<'c, '_>,
        result_type: Type<'c>,
        tier: RefTier,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new(op_names::CBGR_ALLOC, location)
            .add_operands(&[value])
            .add_results(&[result_type])
            .add_attributes(&[(
                verum_mlir::ir::Identifier::new(context, attr_names::CBGR_TIER),
                IntegerAttribute::new(Type::index(context), tier as i64).into(),
            )])
            .build()
            .map_err(|e| MlirError::operation(op_names::CBGR_ALLOC, format!("{:?}", e)))
    }
}

/// CBGR check operation.
///
/// Validates a CBGR reference (checks generation matches).
///
/// ```mlir
/// %valid = verum.cbgr_check %ref, %expected_gen : i1
/// ```
pub struct CbgrCheckOp;

impl CbgrCheckOp {
    /// Build a CBGR check operation.
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        reference: Value<'c, '_>,
        expected_generation: Value<'c, '_>,
    ) -> Result<Operation<'c>> {
        let i1_type = verum_mlir::ir::r#type::IntegerType::new(context, 1).into();

        OperationBuilder::new(op_names::CBGR_CHECK, location)
            .add_operands(&[reference, expected_generation])
            .add_results(&[i1_type])
            .build()
            .map_err(|e| MlirError::operation(op_names::CBGR_CHECK, format!("{:?}", e)))
    }
}

/// CBGR dereference operation.
///
/// Dereferences a CBGR reference with validation.
///
/// ```mlir
/// %value = verum.cbgr_deref %ref : !verum.ref<T, managed> -> T
/// ```
pub struct CbgrDerefOp;

impl CbgrDerefOp {
    /// Build a CBGR dereference operation.
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        reference: Value<'c, '_>,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new(op_names::CBGR_DEREF, location)
            .add_operands(&[reference])
            .add_results(&[result_type])
            .build()
            .map_err(|e| MlirError::operation(op_names::CBGR_DEREF, format!("{:?}", e)))
    }
}

/// CBGR drop operation.
///
/// Drops a CBGR-tracked allocation.
///
/// ```mlir
/// verum.cbgr_drop %ref : !verum.ref<T, managed>
/// ```
pub struct CbgrDropOp;

impl CbgrDropOp {
    /// Build a CBGR drop operation.
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        reference: Value<'c, '_>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new(op_names::CBGR_DROP, location)
            .add_operands(&[reference])
            .build()
            .map_err(|e| MlirError::operation(op_names::CBGR_DROP, format!("{:?}", e)))
    }
}

/// Context get operation.
///
/// Gets a context value from the current environment.
///
/// ```mlir
/// %ctx = verum.context_get "Database" : !verum.context<Database>
/// ```
pub struct ContextGetOp;

impl ContextGetOp {
    /// Build a context get operation.
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        context_name: &str,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new(op_names::CONTEXT_GET, location)
            .add_attributes(&[(
                verum_mlir::ir::Identifier::new(context, attr_names::CONTEXT_NAME),
                StringAttribute::new(context, context_name).into(),
            )])
            .add_results(&[result_type])
            .build()
            .map_err(|e| MlirError::operation(op_names::CONTEXT_GET, format!("{:?}", e)))
    }
}

/// Context provide operation.
///
/// Provides a context value for nested operations.
///
/// ```mlir
/// verum.context_provide "Database" = %db : !verum.context<Database>
/// ```
pub struct ContextProvideOp;

impl ContextProvideOp {
    /// Build a context provide operation.
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        context_name: &str,
        value: Value<'c, '_>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new(op_names::CONTEXT_PROVIDE, location)
            .add_operands(&[value])
            .add_attributes(&[(
                verum_mlir::ir::Identifier::new(context, attr_names::CONTEXT_NAME),
                StringAttribute::new(context, context_name).into(),
            )])
            .build()
            .map_err(|e| MlirError::operation(op_names::CONTEXT_PROVIDE, format!("{:?}", e)))
    }
}

/// Spawn operation.
///
/// Spawns an async task.
///
/// ```mlir
/// %handle = verum.spawn %closure : () -> !verum.future<T>
/// ```
pub struct SpawnOp;

impl SpawnOp {
    /// Build a spawn operation.
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        closure: Value<'c, '_>,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new(op_names::SPAWN, location)
            .add_operands(&[closure])
            .add_results(&[result_type])
            .build()
            .map_err(|e| MlirError::operation(op_names::SPAWN, format!("{:?}", e)))
    }
}

/// Await operation.
///
/// Awaits a future's completion.
///
/// ```mlir
/// %result = verum.await %future : !verum.future<T> -> T
/// ```
pub struct AwaitOp;

impl AwaitOp {
    /// Build an await operation.
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        future: Value<'c, '_>,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new(op_names::AWAIT, location)
            .add_operands(&[future])
            .add_results(&[result_type])
            .build()
            .map_err(|e| MlirError::operation(op_names::AWAIT, format!("{:?}", e)))
    }
}

/// Select operation.
///
/// Selects on multiple futures (first to complete wins).
///
/// ```mlir
/// %result = verum.select %future1, %future2, ... : T
/// ```
pub struct SelectOp;

impl SelectOp {
    /// Build a select operation.
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        futures: &[Value<'c, '_>],
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new(op_names::SELECT, location)
            .add_operands(futures)
            .add_results(&[result_type])
            .build()
            .map_err(|e| MlirError::operation(op_names::SELECT, format!("{:?}", e)))
    }
}

/// List new operation.
///
/// Creates a new empty list.
///
/// ```mlir
/// %list = verum.list_new : !verum.list<T>
/// ```
pub struct ListNewOp;

impl ListNewOp {
    /// Build a list new operation.
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        element_type_name: &str,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new(op_names::LIST_NEW, location)
            .add_attributes(&[(
                verum_mlir::ir::Identifier::new(context, attr_names::ELEMENT_TYPE),
                StringAttribute::new(context, element_type_name).into(),
            )])
            .add_results(&[result_type])
            .build()
            .map_err(|e| MlirError::operation(op_names::LIST_NEW, format!("{:?}", e)))
    }
}

/// List push operation.
///
/// Pushes an element to a list.
///
/// ```mlir
/// %new_list = verum.list_push %list, %element : !verum.list<T>
/// ```
pub struct ListPushOp;

impl ListPushOp {
    /// Build a list push operation.
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        list: Value<'c, '_>,
        element: Value<'c, '_>,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new(op_names::LIST_PUSH, location)
            .add_operands(&[list, element])
            .add_results(&[result_type])
            .build()
            .map_err(|e| MlirError::operation(op_names::LIST_PUSH, format!("{:?}", e)))
    }
}

/// List get operation.
///
/// Gets an element from a list by index.
///
/// ```mlir
/// %element = verum.list_get %list, %index : T
/// ```
pub struct ListGetOp;

impl ListGetOp {
    /// Build a list get operation.
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        list: Value<'c, '_>,
        index: Value<'c, '_>,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new(op_names::LIST_GET, location)
            .add_operands(&[list, index])
            .add_results(&[result_type])
            .build()
            .map_err(|e| MlirError::operation(op_names::LIST_GET, format!("{:?}", e)))
    }
}

/// Refinement check operation.
///
/// Performs runtime refinement type check.
///
/// ```mlir
/// %valid = verum.refinement_check %value, "x > 0" : i1
/// ```
pub struct RefinementCheckOp;

impl RefinementCheckOp {
    /// Build a refinement check operation.
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        value: Value<'c, '_>,
        predicate: &str,
    ) -> Result<Operation<'c>> {
        let i1_type = verum_mlir::ir::r#type::IntegerType::new(context, 1).into();

        OperationBuilder::new(op_names::REFINEMENT_CHECK, location)
            .add_operands(&[value])
            .add_attributes(&[(
                verum_mlir::ir::Identifier::new(context, attr_names::REFINEMENT_PREDICATE),
                StringAttribute::new(context, predicate).into(),
            )])
            .add_results(&[i1_type])
            .build()
            .map_err(|e| MlirError::operation(op_names::REFINEMENT_CHECK, format!("{:?}", e)))
    }
}

/// Stdlib call operation.
///
/// Calls a standard library function via FFI.
///
/// ```mlir
/// %result = verum.stdlib_call "verum_std_list_i64_push" (%list, %value) : (ptr, i64) -> i32
/// ```
pub struct StdlibCallOp;

impl StdlibCallOp {
    /// Build a stdlib call operation.
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        function_name: &str,
        args: &[Value<'c, '_>],
        result_types: &[Type<'c>],
    ) -> Result<Operation<'c>> {
        let mut builder = OperationBuilder::new(op_names::STDLIB_CALL, location)
            .add_operands(args)
            .add_attributes(&[(
                verum_mlir::ir::Identifier::new(context, "callee"),
                StringAttribute::new(context, function_name).into(),
            )]);

        if !result_types.is_empty() {
            builder = builder.add_results(result_types);
        }

        builder
            .build()
            .map_err(|e| MlirError::operation(op_names::STDLIB_CALL, format!("{:?}", e)))
    }
}

/// Print operation.
///
/// Prints a value to stdout.
///
/// ```mlir
/// verum.print %text : !verum.text
/// ```
pub struct PrintOp;

impl PrintOp {
    /// Build a print operation.
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        value: Value<'c, '_>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new(op_names::PRINT, location)
            .add_operands(&[value])
            .build()
            .map_err(|e| MlirError::operation(op_names::PRINT, format!("{:?}", e)))
    }
}

/// Panic operation.
///
/// Panics with a message.
///
/// ```mlir
/// verum.panic %message : !verum.text
/// ```
pub struct PanicOp;

impl PanicOp {
    /// Build a panic operation.
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        message: Value<'c, '_>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new(op_names::PANIC, location)
            .add_operands(&[message])
            .build()
            .map_err(|e| MlirError::operation(op_names::PANIC, format!("{:?}", e)))
    }
}

/// Assert operation.
///
/// Asserts a condition.
///
/// ```mlir
/// verum.assert %condition : i1
/// ```
pub struct AssertOp;

impl AssertOp {
    /// Build an assert operation.
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        condition: Value<'c, '_>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new(op_names::ASSERT, location)
            .add_operands(&[condition])
            .build()
            .map_err(|e| MlirError::operation(op_names::ASSERT, format!("{:?}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_mlir::ir::r#type::IntegerType;

    fn create_test_context() -> Context {
        let ctx = Context::new();
        ctx.load_all_available_dialects();
        ctx
    }

    #[test]
    fn test_cbgr_alloc_op_name() {
        assert_eq!(op_names::CBGR_ALLOC, "verum.cbgr_alloc");
    }

    #[test]
    fn test_context_get_op_name() {
        assert_eq!(op_names::CONTEXT_GET, "verum.context_get");
    }

    #[test]
    fn test_spawn_op_name() {
        assert_eq!(op_names::SPAWN, "verum.spawn");
    }
}
