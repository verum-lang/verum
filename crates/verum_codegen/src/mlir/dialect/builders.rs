//! Operation builders for the Verum dialect.
//!
//! High-level builders for constructing Verum MLIR operations with
//! convenient APIs and proper error handling.

use verum_mlir::{
    Context,
    ir::{Block, BlockLike, Identifier, Location, Module, Operation, Region, RegionLike, Type, Value},
    ir::operation::OperationBuilder,
    dialect::{arith, cf, func, llvm, memref, scf},
    ir::attribute::{
        DenseI32ArrayAttribute, DenseI64ArrayAttribute, FlatSymbolRefAttribute,
        IntegerAttribute, StringAttribute, TypeAttribute, FloatAttribute,
    },
    ir::r#type::{FunctionType, IntegerType, MemRefType},
};
use verum_common::Text;
use crate::mlir::error::{MlirError, Result};
use crate::mlir::dialect::types::{RefTier, VerumType};
use crate::mlir::dialect::ops::*;

/// Builder for constructing Verum MLIR operations.
///
/// Provides a fluent API for building MLIR operations with proper
/// error handling and type safety.
pub struct VerumOpBuilder<'c> {
    context: &'c Context,
}

impl<'c> VerumOpBuilder<'c> {
    /// Create a new operation builder.
    pub fn new(context: &'c Context) -> Self {
        Self { context }
    }

    /// Get the MLIR context.
    pub fn context(&self) -> &'c Context {
        self.context
    }

    // =========================================================================
    // Type Helpers
    // =========================================================================

    /// Create an i1 (boolean) type.
    pub fn i1_type(&self) -> Type<'c> {
        IntegerType::new(self.context, 1).into()
    }

    /// Create an i32 type.
    pub fn i32_type(&self) -> Type<'c> {
        IntegerType::new(self.context, 32).into()
    }

    /// Create an i64 type.
    pub fn i64_type(&self) -> Type<'c> {
        IntegerType::new(self.context, 64).into()
    }

    /// Create an f32 type.
    pub fn f32_type(&self) -> Type<'c> {
        Type::float32(self.context)
    }

    /// Create an f64 type.
    pub fn f64_type(&self) -> Type<'c> {
        Type::float64(self.context)
    }

    /// Create an index type.
    pub fn index_type(&self) -> Type<'c> {
        Type::index(self.context)
    }

    /// Create a pointer type (LLVM dialect).
    pub fn ptr_type(&self) -> Result<Type<'c>> {
        Type::parse(self.context, "!llvm.ptr")
            .ok_or_else(|| MlirError::type_translation("ptr", "failed to parse pointer type"))
    }

    /// Create a CBGR ThinRef type.
    pub fn thin_ref_type(&self) -> Result<Type<'c>> {
        // ThinRef: { ptr, generation: i32, epoch_caps: i32 }
        Type::parse(self.context, "!llvm.struct<(ptr, i32, i32)>")
            .ok_or_else(|| MlirError::type_translation("ThinRef", "failed to parse ThinRef type"))
    }

    /// Create a Text type (ptr + len).
    pub fn text_type(&self) -> Result<Type<'c>> {
        Type::parse(self.context, "!llvm.struct<(ptr, i64)>")
            .ok_or_else(|| MlirError::type_translation("Text", "failed to parse Text type"))
    }

    // =========================================================================
    // Constant Operations (using arith dialect)
    // =========================================================================

    /// Build an integer constant.
    pub fn constant_int(
        &self,
        location: Location<'c>,
        value: i64,
        bits: u32,
    ) -> Operation<'c> {
        let int_type = IntegerType::new(self.context, bits).into();
        arith::constant(
            self.context,
            IntegerAttribute::new(int_type, value).into(),
            location,
        )
    }

    /// Build an i64 constant.
    pub fn constant_i64(&self, location: Location<'c>, value: i64) -> Operation<'c> {
        self.constant_int(location, value, 64)
    }

    /// Build an i32 constant.
    pub fn constant_i32(&self, location: Location<'c>, value: i32) -> Operation<'c> {
        self.constant_int(location, value as i64, 32)
    }

    /// Build an i1 (boolean) constant.
    pub fn constant_bool(&self, location: Location<'c>, value: bool) -> Operation<'c> {
        self.constant_int(location, value as i64, 1)
    }

    /// Build an index constant.
    pub fn constant_index(&self, location: Location<'c>, value: i64) -> Operation<'c> {
        arith::constant(
            self.context,
            IntegerAttribute::new(Type::index(self.context), value).into(),
            location,
        )
    }

    /// Build a float constant.
    pub fn constant_float(&self, location: Location<'c>, value: f64, bits: u32) -> Operation<'c> {
        let float_type = if bits == 32 {
            Type::float32(self.context)
        } else {
            Type::float64(self.context)
        };

        arith::constant(
            self.context,
            FloatAttribute::new(self.context, float_type, value).into(),
            location,
        )
    }

    // =========================================================================
    // Arithmetic Operations
    // =========================================================================

    /// Build an integer addition.
    pub fn addi(
        &self,
        lhs: Value<'c, '_>,
        rhs: Value<'c, '_>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::addi(lhs, rhs, location)
    }

    /// Build an integer subtraction.
    pub fn subi(
        &self,
        lhs: Value<'c, '_>,
        rhs: Value<'c, '_>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::subi(lhs, rhs, location)
    }

    /// Build an integer multiplication.
    pub fn muli(
        &self,
        lhs: Value<'c, '_>,
        rhs: Value<'c, '_>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::muli(lhs, rhs, location)
    }

    /// Build a signed integer division.
    pub fn divsi(
        &self,
        lhs: Value<'c, '_>,
        rhs: Value<'c, '_>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::divsi(lhs, rhs, location)
    }

    /// Build a signed integer remainder.
    pub fn remsi(
        &self,
        lhs: Value<'c, '_>,
        rhs: Value<'c, '_>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::remsi(lhs, rhs, location)
    }

    /// Build a floating-point addition.
    pub fn addf(
        &self,
        lhs: Value<'c, '_>,
        rhs: Value<'c, '_>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::addf(lhs, rhs, location)
    }

    /// Build a floating-point subtraction.
    pub fn subf(
        &self,
        lhs: Value<'c, '_>,
        rhs: Value<'c, '_>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::subf(lhs, rhs, location)
    }

    /// Build a floating-point multiplication.
    pub fn mulf(
        &self,
        lhs: Value<'c, '_>,
        rhs: Value<'c, '_>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::mulf(lhs, rhs, location)
    }

    /// Build a floating-point division.
    pub fn divf(
        &self,
        lhs: Value<'c, '_>,
        rhs: Value<'c, '_>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::divf(lhs, rhs, location)
    }

    // =========================================================================
    // Comparison Operations
    // =========================================================================

    /// Build an integer comparison.
    pub fn cmpi(
        &self,
        predicate: arith::CmpiPredicate,
        lhs: Value<'c, '_>,
        rhs: Value<'c, '_>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::cmpi(self.context, predicate, lhs, rhs, location)
    }

    /// Build a floating-point comparison.
    pub fn cmpf(
        &self,
        predicate: arith::CmpfPredicate,
        lhs: Value<'c, '_>,
        rhs: Value<'c, '_>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::cmpf(self.context, predicate, lhs, rhs, location)
    }

    // =========================================================================
    // Control Flow Operations (using scf dialect)
    // =========================================================================

    /// Build an if operation.
    pub fn if_op(
        &self,
        condition: Value<'c, '_>,
        result_types: &[Type<'c>],
        then_region: Region<'c>,
        else_region: Region<'c>,
        location: Location<'c>,
    ) -> Operation<'c> {
        scf::r#if(condition, result_types, then_region, else_region, location)
    }

    /// Build a for loop.
    pub fn for_op(
        &self,
        lower_bound: Value<'c, '_>,
        upper_bound: Value<'c, '_>,
        step: Value<'c, '_>,
        body_region: Region<'c>,
        location: Location<'c>,
    ) -> Operation<'c> {
        scf::r#for(lower_bound, upper_bound, step, body_region, location)
    }

    /// Build a while loop.
    pub fn while_op(
        &self,
        initial_values: &[Value<'c, '_>],
        result_types: &[Type<'c>],
        before_region: Region<'c>,
        after_region: Region<'c>,
        location: Location<'c>,
    ) -> Operation<'c> {
        scf::r#while(initial_values, result_types, before_region, after_region, location)
    }

    /// Build a yield operation.
    pub fn yield_op(&self, values: &[Value<'c, '_>], location: Location<'c>) -> Operation<'c> {
        scf::r#yield(values, location)
    }

    // =========================================================================
    // Function Operations (using func dialect)
    // =========================================================================

    /// Build a function definition.
    pub fn func(
        &self,
        name: &str,
        function_type: FunctionType<'c>,
        body: Region<'c>,
        location: Location<'c>,
    ) -> Operation<'c> {
        func::func(
            self.context,
            StringAttribute::new(self.context, name),
            TypeAttribute::new(function_type.into()),
            body,
            &[],
            location,
        )
    }

    /// Build a return operation.
    pub fn return_op(&self, values: &[Value<'c, '_>], location: Location<'c>) -> Operation<'c> {
        func::r#return(values, location)
    }

    /// Build a call operation.
    pub fn call(
        &self,
        callee: &str,
        args: &[Value<'c, '_>],
        result_types: &[Type<'c>],
        location: Location<'c>,
    ) -> Operation<'c> {
        func::call(
            self.context,
            FlatSymbolRefAttribute::new(self.context, callee),
            args,
            result_types,
            location,
        )
    }

    /// Build an indirect call operation (call through function pointer).
    pub fn call_indirect(
        &self,
        callee: Value<'c, '_>,
        args: &[Value<'c, '_>],
        result_types: &[Type<'c>],
        location: Location<'c>,
    ) -> Operation<'c> {
        func::call_indirect(callee, args, result_types, location)
    }

    /// Build a function constant (get address of function).
    pub fn func_constant(
        &self,
        function_name: &str,
        function_type: FunctionType<'c>,
        location: Location<'c>,
    ) -> Operation<'c> {
        func::constant(
            self.context,
            FlatSymbolRefAttribute::new(self.context, function_name),
            function_type,
            location,
        )
    }

    /// Build an external function declaration (no body).
    pub fn func_extern(
        &self,
        name: &str,
        function_type: FunctionType<'c>,
        location: Location<'c>,
    ) -> Operation<'c> {
        // External functions have empty regions
        let empty_region = Region::new();
        func::func(
            self.context,
            StringAttribute::new(self.context, name),
            TypeAttribute::new(function_type.into()),
            empty_region,
            &[(
                Identifier::new(self.context, "sym_visibility"),
                StringAttribute::new(self.context, "private").into(),
            )],
            location,
        )
    }

    /// Build a public function declaration (no body, visible outside module).
    pub fn func_public(
        &self,
        name: &str,
        function_type: FunctionType<'c>,
        body: Region<'c>,
        location: Location<'c>,
    ) -> Operation<'c> {
        func::func(
            self.context,
            StringAttribute::new(self.context, name),
            TypeAttribute::new(function_type.into()),
            body,
            &[(
                Identifier::new(self.context, "sym_visibility"),
                StringAttribute::new(self.context, "public").into(),
            )],
            location,
        )
    }

    // =========================================================================
    // Verum Dialect Operations
    // =========================================================================

    /// Build a CBGR allocation.
    pub fn cbgr_alloc(
        &self,
        location: Location<'c>,
        value: Value<'c, '_>,
        result_type: Type<'c>,
        tier: RefTier,
    ) -> Result<Operation<'c>> {
        CbgrAllocOp::build(self.context, location, value, result_type, tier)
    }

    /// Build a CBGR check.
    pub fn cbgr_check(
        &self,
        location: Location<'c>,
        reference: Value<'c, '_>,
        expected_generation: Value<'c, '_>,
    ) -> Result<Operation<'c>> {
        CbgrCheckOp::build(self.context, location, reference, expected_generation)
    }

    /// Build a CBGR dereference.
    pub fn cbgr_deref(
        &self,
        location: Location<'c>,
        reference: Value<'c, '_>,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        CbgrDerefOp::build(self.context, location, reference, result_type)
    }

    /// Build a CBGR drop.
    pub fn cbgr_drop(
        &self,
        location: Location<'c>,
        reference: Value<'c, '_>,
    ) -> Result<Operation<'c>> {
        CbgrDropOp::build(self.context, location, reference)
    }

    /// Build a context get operation.
    pub fn context_get(
        &self,
        location: Location<'c>,
        context_name: &str,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        ContextGetOp::build(self.context, location, context_name, result_type)
    }

    /// Build a context provide operation.
    pub fn context_provide(
        &self,
        location: Location<'c>,
        context_name: &str,
        value: Value<'c, '_>,
    ) -> Result<Operation<'c>> {
        ContextProvideOp::build(self.context, location, context_name, value)
    }

    /// Build a spawn operation.
    pub fn spawn(
        &self,
        location: Location<'c>,
        closure: Value<'c, '_>,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        SpawnOp::build(self.context, location, closure, result_type)
    }

    /// Build an await operation.
    pub fn await_op(
        &self,
        location: Location<'c>,
        future: Value<'c, '_>,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        AwaitOp::build(self.context, location, future, result_type)
    }

    /// Build a select operation.
    pub fn select(
        &self,
        location: Location<'c>,
        futures: &[Value<'c, '_>],
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        SelectOp::build(self.context, location, futures, result_type)
    }

    /// Build a list new operation.
    pub fn list_new(
        &self,
        location: Location<'c>,
        element_type_name: &str,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        ListNewOp::build(self.context, location, element_type_name, result_type)
    }

    /// Build a list push operation.
    pub fn list_push(
        &self,
        location: Location<'c>,
        list: Value<'c, '_>,
        element: Value<'c, '_>,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        ListPushOp::build(self.context, location, list, element, result_type)
    }

    /// Build a list get operation.
    pub fn list_get(
        &self,
        location: Location<'c>,
        list: Value<'c, '_>,
        index: Value<'c, '_>,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        ListGetOp::build(self.context, location, list, index, result_type)
    }

    /// Build a refinement check operation.
    pub fn refinement_check(
        &self,
        location: Location<'c>,
        value: Value<'c, '_>,
        predicate: &str,
    ) -> Result<Operation<'c>> {
        RefinementCheckOp::build(self.context, location, value, predicate)
    }

    /// Build a stdlib call operation.
    pub fn stdlib_call(
        &self,
        location: Location<'c>,
        function_name: &str,
        args: &[Value<'c, '_>],
        result_types: &[Type<'c>],
    ) -> Result<Operation<'c>> {
        StdlibCallOp::build(self.context, location, function_name, args, result_types)
    }

    /// Build a print operation.
    pub fn print(
        &self,
        location: Location<'c>,
        value: Value<'c, '_>,
    ) -> Result<Operation<'c>> {
        PrintOp::build(self.context, location, value)
    }

    /// Build a panic operation.
    pub fn panic(
        &self,
        location: Location<'c>,
        message: Value<'c, '_>,
    ) -> Result<Operation<'c>> {
        PanicOp::build(self.context, location, message)
    }

    /// Build an assert operation.
    pub fn assert(
        &self,
        location: Location<'c>,
        condition: Value<'c, '_>,
    ) -> Result<Operation<'c>> {
        AssertOp::build(self.context, location, condition)
    }

    // =========================================================================
    // Memory Reference Operations (memref dialect)
    // =========================================================================

    /// Create a memref type.
    pub fn memref_type(&self, element_type: Type<'c>, shape: &[i64]) -> MemRefType<'c> {
        MemRefType::new(element_type, shape, None, None)
    }

    /// Build a memref.alloc operation.
    pub fn memref_alloc(
        &self,
        memref_type: MemRefType<'c>,
        dynamic_sizes: &[Value<'c, '_>],
        alignment: Option<i64>,
        location: Location<'c>,
    ) -> Operation<'c> {
        let align_attr = alignment.map(|a| {
            IntegerAttribute::new(IntegerType::new(self.context, 64).into(), a)
        });
        memref::alloc(self.context, memref_type, dynamic_sizes, &[], align_attr, location)
    }

    /// Build a memref.alloca operation (stack allocation).
    pub fn memref_alloca(
        &self,
        memref_type: MemRefType<'c>,
        dynamic_sizes: &[Value<'c, '_>],
        alignment: Option<i64>,
        location: Location<'c>,
    ) -> Operation<'c> {
        let align_attr = alignment.map(|a| {
            IntegerAttribute::new(IntegerType::new(self.context, 64).into(), a)
        });
        memref::alloca(self.context, memref_type, dynamic_sizes, &[], align_attr, location)
    }

    /// Build a memref.dealloc operation.
    pub fn memref_dealloc(
        &self,
        memref_value: Value<'c, '_>,
        location: Location<'c>,
    ) -> Operation<'c> {
        memref::dealloc(memref_value, location)
    }

    /// Build a memref.load operation.
    pub fn memref_load(
        &self,
        memref_value: Value<'c, '_>,
        indices: &[Value<'c, '_>],
        location: Location<'c>,
    ) -> Operation<'c> {
        memref::load(memref_value, indices, location)
    }

    /// Build a memref.store operation.
    pub fn memref_store(
        &self,
        value: Value<'c, '_>,
        memref_value: Value<'c, '_>,
        indices: &[Value<'c, '_>],
        location: Location<'c>,
    ) -> Operation<'c> {
        memref::store(value, memref_value, indices, location)
    }

    /// Build a memref.dim operation to get dimension size.
    pub fn memref_dim(
        &self,
        memref_value: Value<'c, '_>,
        index: Value<'c, '_>,
        location: Location<'c>,
    ) -> Operation<'c> {
        memref::dim(memref_value, index, location)
    }

    /// Build a memref.cast operation.
    pub fn memref_cast(
        &self,
        value: Value<'c, '_>,
        result_type: MemRefType<'c>,
        location: Location<'c>,
    ) -> Operation<'c> {
        memref::cast(value, result_type, location)
    }

    // =========================================================================
    // LLVM Dialect Operations
    // =========================================================================

    /// Build an llvm.alloca operation.
    pub fn llvm_alloca(
        &self,
        array_size: Value<'c, '_>,
        element_type: Type<'c>,
        ptr_type: Type<'c>,
        alignment: Option<i64>,
        location: Location<'c>,
    ) -> Operation<'c> {
        let options = llvm::AllocaOptions::new()
            .elem_type(Some(TypeAttribute::new(element_type)));
        let options = if let Some(align) = alignment {
            options.align(Some(IntegerAttribute::new(
                IntegerType::new(self.context, 64).into(),
                align,
            )))
        } else {
            options
        };
        llvm::alloca(self.context, array_size, ptr_type, location, options)
    }

    /// Build an llvm.load operation.
    pub fn llvm_load(
        &self,
        ptr: Value<'c, '_>,
        result_type: Type<'c>,
        location: Location<'c>,
    ) -> Operation<'c> {
        let options = llvm::LoadStoreOptions::new();
        llvm::load(self.context, ptr, result_type, location, options)
    }

    /// Build an llvm.store operation.
    pub fn llvm_store(
        &self,
        value: Value<'c, '_>,
        ptr: Value<'c, '_>,
        location: Location<'c>,
    ) -> Operation<'c> {
        let options = llvm::LoadStoreOptions::new();
        llvm::store(self.context, value, ptr, location, options)
    }

    /// Build an llvm.getelementptr operation.
    pub fn llvm_gep(
        &self,
        ptr: Value<'c, '_>,
        indices: &[i32],
        element_type: Type<'c>,
        result_type: Type<'c>,
        location: Location<'c>,
    ) -> Operation<'c> {
        let indices_attr = DenseI32ArrayAttribute::new(self.context, indices);
        llvm::get_element_ptr(self.context, ptr, indices_attr, element_type, result_type, location)
    }

    /// Build an llvm.getelementptr operation with dynamic indices.
    pub fn llvm_gep_dynamic<const N: usize>(
        &self,
        ptr: Value<'c, '_>,
        indices: &[Value<'c, '_>; N],
        element_type: Type<'c>,
        result_type: Type<'c>,
        location: Location<'c>,
    ) -> Operation<'c> {
        llvm::get_element_ptr_dynamic(self.context, ptr, indices, element_type, result_type, location)
    }

    /// Build an llvm.extractvalue operation.
    pub fn llvm_extractvalue(
        &self,
        container: Value<'c, '_>,
        position: &[i64],
        result_type: Type<'c>,
        location: Location<'c>,
    ) -> Operation<'c> {
        let pos_attr = DenseI64ArrayAttribute::new(self.context, position);
        llvm::extract_value(self.context, container, pos_attr, result_type, location)
    }

    /// Build an llvm.insertvalue operation.
    pub fn llvm_insertvalue(
        &self,
        container: Value<'c, '_>,
        value: Value<'c, '_>,
        position: &[i64],
        location: Location<'c>,
    ) -> Operation<'c> {
        let pos_attr = DenseI64ArrayAttribute::new(self.context, position);
        llvm::insert_value(self.context, container, pos_attr, value, location)
    }

    /// Build an llvm.mlir.undef operation.
    pub fn llvm_undef(&self, result_type: Type<'c>, location: Location<'c>) -> Operation<'c> {
        llvm::undef(result_type, location)
    }

    /// Build an llvm.mlir.zero operation.
    pub fn llvm_zero(&self, result_type: Type<'c>, location: Location<'c>) -> Operation<'c> {
        llvm::zero(result_type, location)
    }

    /// Build an llvm.bitcast operation.
    pub fn llvm_bitcast(
        &self,
        value: Value<'c, '_>,
        result_type: Type<'c>,
        location: Location<'c>,
    ) -> Operation<'c> {
        llvm::bitcast(value, result_type, location)
    }

    /// Build an llvm.func operation.
    pub fn llvm_func(
        &self,
        name: &str,
        function_type: Type<'c>,
        body: Region<'c>,
        location: Location<'c>,
    ) -> Operation<'c> {
        llvm::func(
            self.context,
            StringAttribute::new(self.context, name),
            TypeAttribute::new(function_type),
            body,
            &[],
            location,
        )
    }

    /// Build an llvm.return operation.
    pub fn llvm_return(
        &self,
        value: Option<Value<'c, '_>>,
        location: Location<'c>,
    ) -> Operation<'c> {
        llvm::r#return(value, location)
    }

    /// Build an llvm.unreachable operation.
    pub fn llvm_unreachable(&self, location: Location<'c>) -> Operation<'c> {
        llvm::unreachable(location)
    }

    /// Build an llvm.call_intrinsic operation.
    pub fn llvm_call_intrinsic(
        &self,
        intrinsic_name: &str,
        args: &[Value<'c, '_>],
        result_types: &[Type<'c>],
        location: Location<'c>,
    ) -> Operation<'c> {
        llvm::call_intrinsic(
            self.context,
            StringAttribute::new(self.context, intrinsic_name),
            args,
            result_types,
            location,
        )
    }

    // =========================================================================
    // Control Flow Operations (cf dialect)
    // =========================================================================

    /// Build a cf.br operation (unconditional branch).
    pub fn cf_br(
        &self,
        dest: &Block<'c>,
        dest_args: &[Value<'c, '_>],
        location: Location<'c>,
    ) -> Operation<'c> {
        cf::br(dest, dest_args, location)
    }

    /// Build a cf.cond_br operation (conditional branch).
    pub fn cf_cond_br(
        &self,
        condition: Value<'c, '_>,
        true_dest: &Block<'c>,
        true_args: &[Value<'c, '_>],
        false_dest: &Block<'c>,
        false_args: &[Value<'c, '_>],
        location: Location<'c>,
    ) -> Operation<'c> {
        cf::cond_br(
            self.context,
            condition,
            true_dest,
            false_dest,
            true_args,
            false_args,
            location,
        )
    }

    // =========================================================================
    // Additional Arithmetic Operations
    // =========================================================================

    /// Build an arith.select operation (conditional select).
    /// Note: This is different from the async select operation.
    pub fn arith_select(
        &self,
        condition: Value<'c, '_>,
        true_value: Value<'c, '_>,
        false_value: Value<'c, '_>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::select(condition, true_value, false_value, location)
    }

    /// Build an arith.negf operation (floating-point negation).
    pub fn negf(
        &self,
        value: Value<'c, '_>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::negf(value, location)
    }

    /// Build an arith.andi operation (bitwise AND).
    pub fn andi(
        &self,
        lhs: Value<'c, '_>,
        rhs: Value<'c, '_>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::andi(lhs, rhs, location)
    }

    /// Build an arith.ori operation (bitwise OR).
    pub fn ori(
        &self,
        lhs: Value<'c, '_>,
        rhs: Value<'c, '_>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::ori(lhs, rhs, location)
    }

    /// Build an arith.xori operation (bitwise XOR).
    pub fn xori(
        &self,
        lhs: Value<'c, '_>,
        rhs: Value<'c, '_>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::xori(lhs, rhs, location)
    }

    /// Build an arith.shli operation (left shift).
    pub fn shli(
        &self,
        lhs: Value<'c, '_>,
        rhs: Value<'c, '_>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::shli(lhs, rhs, location)
    }

    /// Build an arith.shrsi operation (signed right shift).
    pub fn shrsi(
        &self,
        lhs: Value<'c, '_>,
        rhs: Value<'c, '_>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::shrsi(lhs, rhs, location)
    }

    /// Build an arith.shrui operation (unsigned right shift).
    pub fn shrui(
        &self,
        lhs: Value<'c, '_>,
        rhs: Value<'c, '_>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::shrui(lhs, rhs, location)
    }

    /// Build an arith.extsi operation (sign extend integer).
    pub fn extsi(
        &self,
        value: Value<'c, '_>,
        result_type: Type<'c>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::extsi(value, result_type, location)
    }

    /// Build an arith.extui operation (zero extend integer).
    pub fn extui(
        &self,
        value: Value<'c, '_>,
        result_type: Type<'c>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::extui(value, result_type, location)
    }

    /// Build an arith.trunci operation (truncate integer).
    pub fn trunci(
        &self,
        value: Value<'c, '_>,
        result_type: Type<'c>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::trunci(value, result_type, location)
    }

    /// Build an arith.sitofp operation (signed int to float).
    pub fn sitofp(
        &self,
        value: Value<'c, '_>,
        result_type: Type<'c>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::sitofp(value, result_type, location)
    }

    /// Build an arith.uitofp operation (unsigned int to float).
    pub fn uitofp(
        &self,
        value: Value<'c, '_>,
        result_type: Type<'c>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::uitofp(value, result_type, location)
    }

    /// Build an arith.fptosi operation (float to signed int).
    pub fn fptosi(
        &self,
        value: Value<'c, '_>,
        result_type: Type<'c>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::fptosi(value, result_type, location)
    }

    /// Build an arith.fptoui operation (float to unsigned int).
    pub fn fptoui(
        &self,
        value: Value<'c, '_>,
        result_type: Type<'c>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::fptoui(value, result_type, location)
    }

    /// Build an arith.extf operation (extend float precision).
    pub fn extf(
        &self,
        value: Value<'c, '_>,
        result_type: Type<'c>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::extf(value, result_type, location)
    }

    /// Build an arith.truncf operation (truncate float precision).
    pub fn truncf_op(
        &self,
        value: Value<'c, '_>,
        result_type: Type<'c>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::truncf(value, location)
    }

    /// Build an arith.bitcast operation.
    pub fn arith_bitcast(
        &self,
        value: Value<'c, '_>,
        result_type: Type<'c>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::bitcast(value, result_type, location)
    }

    /// Build an arith.index_cast operation.
    pub fn index_cast(
        &self,
        value: Value<'c, '_>,
        result_type: Type<'c>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::index_cast(value, result_type, location)
    }

    /// Build unsigned integer division.
    pub fn divui(
        &self,
        lhs: Value<'c, '_>,
        rhs: Value<'c, '_>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::divui(lhs, rhs, location)
    }

    /// Build unsigned integer remainder.
    pub fn remui(
        &self,
        lhs: Value<'c, '_>,
        rhs: Value<'c, '_>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::remui(lhs, rhs, location)
    }

    /// Build floating-point remainder.
    pub fn remf(
        &self,
        lhs: Value<'c, '_>,
        rhs: Value<'c, '_>,
        location: Location<'c>,
    ) -> Operation<'c> {
        arith::remf(lhs, rhs, location)
    }

    // =========================================================================
    // SCF Parallel and Reduce Operations
    // =========================================================================

    /// Build a scf.condition operation (for while loop).
    pub fn scf_condition(
        &self,
        condition: Value<'c, '_>,
        args: &[Value<'c, '_>],
        location: Location<'c>,
    ) -> Operation<'c> {
        scf::condition(condition, args, location)
    }

    /// Build an scf.execute_region operation.
    pub fn scf_execute_region(
        &self,
        result_types: &[Type<'c>],
        body: Region<'c>,
        location: Location<'c>,
    ) -> Operation<'c> {
        scf::execute_region(result_types, body, location)
    }

    // =========================================================================
    // Type Parsing Helpers
    // =========================================================================

    /// Parse an MLIR type from string.
    pub fn parse_type(&self, type_str: &str) -> Result<Type<'c>> {
        Type::parse(self.context, type_str)
            .ok_or_else(|| MlirError::type_translation("custom", format!("Failed to parse type: {}", type_str)))
    }

    /// Create an LLVM struct type.
    pub fn llvm_struct_type(&self, field_types: &[Type<'c>]) -> Result<Type<'c>> {
        let type_strs: Vec<String> = field_types.iter().map(|t| format!("{}", t)).collect();
        let struct_str = format!("!llvm.struct<({})>", type_strs.join(", "));
        self.parse_type(&struct_str)
    }

    /// Create an LLVM array type.
    pub fn llvm_array_type(&self, element_type: Type<'c>, size: usize) -> Result<Type<'c>> {
        let array_str = format!("!llvm.array<{} x {}>", size, element_type);
        self.parse_type(&array_str)
    }

    /// Create an LLVM pointer type.
    pub fn llvm_ptr_type(&self) -> Result<Type<'c>> {
        self.parse_type("!llvm.ptr")
    }
}

/// Helper for building blocks with arguments.
pub fn build_block_with_args<'c>(
    arg_types: &[(Type<'c>, Location<'c>)],
) -> Block<'c> {
    Block::new(arg_types)
}

/// Helper for building empty regions.
pub fn build_empty_region<'c>() -> Region<'c> {
    Region::new()
}

/// Helper for building a region with a single block.
pub fn build_region_with_block<'c>(block: Block<'c>) -> Region<'c> {
    let region = Region::new();
    region.append_block(block);
    region
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_mlir::ir::operation::OperationLike;
    use verum_mlir::ir::{Block, BlockLike};

    fn create_test_context() -> Context {
        let ctx = Context::new();
        ctx.load_all_available_dialects();
        ctx
    }

    #[test]
    fn test_type_helpers() {
        let ctx = create_test_context();
        let builder = VerumOpBuilder::new(&ctx);

        let i64_type = builder.i64_type();
        let f64_type = builder.f64_type();
        let i1_type = builder.i1_type();

        // Types should be valid
        assert!(!format!("{}", i64_type).is_empty());
        assert!(!format!("{}", f64_type).is_empty());
        assert!(!format!("{}", i1_type).is_empty());
    }

    #[test]
    fn test_builder_creation() {
        let ctx = create_test_context();
        let _builder = VerumOpBuilder::new(&ctx);
        // Builder should be created successfully
    }

    // Note: Constant operation tests require a complete MLIR module context
    // with proper dialect registration. Full operation tests are done in
    // integration tests that use the MlirContext wrapper.
}
