//! Tensor runtime as LLVM IR — replaces verum_tensor.c (3,709 LOC, ~225 functions).
//!
//! Uses LLVM intrinsics for math (llvm.sqrt.f64, llvm.exp.f64, etc.) instead of
//! software approximations, giving native hardware performance.
//!
//! Strategy:
//!   - Core ~30 tensor functions: full LLVM IR bodies
//!   - Math intrinsics: direct LLVM intrinsic calls
//!   - Remaining ~195 functions: extern declarations (linked from C if needed)
//!
//! Tensor Handle System:
//!   A tensor handle (i64) is a pointer to a heap-allocated header:
//!   ```text
//!   VerumTensor {           // sizeof = 152 bytes (matching C layout)
//!     refcount: u32,        // offset  0
//!     dtype:    u8,         // offset  4
//!     ndim:     u8,         // offset  5
//!     _pad:     u16,        // offset  6
//!     shape:    [i64; 8],   // offset  8
//!     strides:  [i64; 8],   // offset 72
//!     numel:    i64,        // offset 136
//!     data:     *f64,       // offset 144
//!   }
//!   ```

use verum_llvm::context::Context;
use verum_llvm::module::Module;
use verum_llvm::types::FunctionType;
use verum_llvm::values::FunctionValue;
use verum_llvm::{AddressSpace, FloatPredicate, IntPredicate};
use super::error::{BuildExt, OptionExt, Result};

// ============================================================================
// Tensor header layout constants (must match VerumTensor in verum_tensor.c)
// ============================================================================

/// Offset of `refcount` (u32) in VerumTensor.
const OFF_REFCOUNT: u64 = 0;
/// Offset of `dtype` (u8) in VerumTensor.
const OFF_DTYPE: u64 = 4;
/// Offset of `ndim` (u8) in VerumTensor.
const OFF_NDIM: u64 = 5;
/// Offset of `shape` ([i64; 8]) in VerumTensor.
const OFF_SHAPE: u64 = 8;
/// Offset of `strides` ([i64; 8]) in VerumTensor.
const OFF_STRIDES: u64 = 72;
/// Offset of `numel` (i64) in VerumTensor.
const OFF_NUMEL: u64 = 136;
/// Offset of `data` (*f64) in VerumTensor.
const OFF_DATA: u64 = 144;
/// Total size of VerumTensor header.
const TENSOR_HEADER_SIZE: u64 = 152;
/// Maximum number of dimensions.
const TENSOR_MAX_DIMS: u64 = 8;

// Operation codes (must match verum_tensor.c defines)
const TENSOR_BINOP_ADD: u64 = 0;
const TENSOR_BINOP_SUB: u64 = 1;
const TENSOR_BINOP_MUL: u64 = 2;
const TENSOR_BINOP_DIV: u64 = 3;
const TENSOR_BINOP_POW: u64 = 4;
const TENSOR_BINOP_MIN: u64 = 6;
const TENSOR_BINOP_MAX: u64 = 7;

const TENSOR_UNOP_NEG: u64 = 0x00;
const TENSOR_UNOP_ABS: u64 = 0x01;
const TENSOR_UNOP_SQRT: u64 = 0x02;
const TENSOR_UNOP_EXP: u64 = 0x03;
const TENSOR_UNOP_LOG: u64 = 0x04;
const TENSOR_UNOP_SIN: u64 = 0x05;
const TENSOR_UNOP_COS: u64 = 0x06;
const TENSOR_UNOP_TANH: u64 = 0x08;
const TENSOR_UNOP_SIGMOID: u64 = 0x09;
const TENSOR_UNOP_RELU: u64 = 0x0A;
const TENSOR_UNOP_FLOOR: u64 = 0x0D;
const TENSOR_UNOP_CEIL: u64 = 0x0E;
const TENSOR_UNOP_ROUND: u64 = 0x0F;

const TENSOR_REDUCE_SUM: u64 = 0;
const TENSOR_REDUCE_MEAN: u64 = 1;
const TENSOR_REDUCE_MAX: u64 = 2;
const TENSOR_REDUCE_MIN: u64 = 3;
const TENSOR_REDUCE_PROD: u64 = 4;

/// Emit tensor runtime functions as LLVM IR.
pub struct TensorIR<'ctx> {
    context: &'ctx Context,
}

impl<'ctx> TensorIR<'ctx> {
    pub fn new(context: &'ctx Context) -> Self {
        Self { context }
    }

    /// Emit all tensor runtime functions into the module.
    pub fn emit_tensor_functions(&self, module: &Module<'ctx>) -> Result<()> {
        // LLVM math intrinsic declarations
        self.emit_math_intrinsics(module);

        // Core tensor lifecycle
        self.emit_tensor_new(module)?;
        self.emit_tensor_fill(module)?;
        self.emit_tensor_from_data(module)?;
        self.emit_tensor_free(module)?;
        self.emit_tensor_clone(module)?;
        self.emit_tensor_get_scalar(module)?;
        self.emit_tensor_set_scalar(module)?;

        // Core tensor operations
        self.emit_tensor_unop(module)?;
        self.emit_tensor_binop(module)?;
        self.emit_tensor_reduce_all(module)?;
        self.emit_tensor_reduce(module)?;
        self.emit_tensor_matmul(module)?;
        self.emit_tensor_reshape(module)?;
        self.emit_tensor_transpose(module)?;
        self.emit_tensor_softmax(module)?;

        // Math wrapper functions (call LLVM intrinsics)
        self.emit_math_wrappers(module)?;

        // Autodiff gradient tape
        self.emit_grad_tape(module);

        // Bridge / utility
        self.emit_tensor_fill_scalar(module)?;
        self.emit_tensor_reshape_flat(module)?;

        // All remaining functions: extern declarations
        self.emit_extern_stubs(module)?;
        Ok(())
    }

    // ========================================================================
    // Helpers
    // ========================================================================

    /// Safely extract i64 from a function parameter that may be IntValue or PointerValue.
    /// VBC lowering may create function declarations with ptr params instead of i64,
    /// so tensor_ir must handle both cases when adding function bodies.
    fn param_as_i64(
        &self,
        builder: &verum_llvm::builder::Builder<'ctx>,
        param: verum_llvm::values::BasicValueEnum<'ctx>,
        name: &str,
    ) -> Result<verum_llvm::values::IntValue<'ctx>> {
        if param.is_int_value() {
            Ok(param.into_int_value())
        } else if param.is_pointer_value() {
            let i64_type = self.context.i64_type();
            Ok(builder.build_ptr_to_int(param.into_pointer_value(), i64_type, name).or_llvm_err()?)
        } else if param.is_float_value() {
            let i64_type = self.context.i64_type();
            Ok(builder.build_bit_cast(param.into_float_value(), i64_type, name).or_llvm_err()?.into_int_value())
        } else {
            Ok(self.context.i64_type().const_zero())
        }
    }

    /// Get or declare a function in the module.
    fn get_or_declare(
        &self, module: &Module<'ctx>, name: &str, fn_type: FunctionType<'ctx>,
    ) -> FunctionValue<'ctx> {
        module.get_function(name).unwrap_or_else(|| module.add_function(name, fn_type, None))
    }

    /// Get or declare a function, returning early if it already has a body.
    /// Returns None if the function already has basic blocks (already defined).
    fn get_or_declare_new(
        &self, module: &Module<'ctx>, name: &str, fn_type: FunctionType<'ctx>,
    ) -> Option<FunctionValue<'ctx>> {
        let func = self.get_or_declare(module, name, fn_type);
        if func.count_basic_blocks() > 0 {
            return None;
        }
        Some(func)
    }

    /// Load a field from the tensor header at the given byte offset.
    /// `base` is a pointer (i8*), result is i64 by default.
    fn load_header_i64(
        &self,
        builder: &verum_llvm::builder::Builder<'ctx>,
        base_ptr: verum_llvm::values::PointerValue<'ctx>,
        byte_offset: u64,
        name: &str,
    ) -> Result<verum_llvm::values::IntValue<'ctx>> {
        let i8_type = self.context.i8_type();
        let i64_type = self.context.i64_type();
        let offset = i64_type.const_int(byte_offset, false);
        let field_ptr = unsafe {
            builder.build_in_bounds_gep(i8_type, base_ptr, &[offset], &format!("{}_ptr", name)).or_llvm_err()?
        };
        Ok(builder.build_load(i64_type, field_ptr, name).or_llvm_err()?.into_int_value())
    }

    /// Load a pointer field from the tensor header at the given byte offset.
    fn load_header_ptr(
        &self,
        builder: &verum_llvm::builder::Builder<'ctx>,
        base_ptr: verum_llvm::values::PointerValue<'ctx>,
        byte_offset: u64,
        name: &str,
    ) -> Result<verum_llvm::values::PointerValue<'ctx>> {
        let i8_type = self.context.i8_type();
        let i64_type = self.context.i64_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let offset = i64_type.const_int(byte_offset, false);
        let field_ptr = unsafe {
            builder.build_in_bounds_gep(i8_type, base_ptr, &[offset], &format!("{}_ptr", name)).or_llvm_err()?
        };
        Ok(builder.build_load(ptr_type, field_ptr, name).or_llvm_err()?.into_pointer_value())
    }

    /// Store an i64 value to a field in the tensor header.
    fn store_header_i64(
        &self,
        builder: &verum_llvm::builder::Builder<'ctx>,
        base_ptr: verum_llvm::values::PointerValue<'ctx>,
        byte_offset: u64,
        value: verum_llvm::values::IntValue<'ctx>,
        name: &str,
    ) -> Result<()> {
        let i8_type = self.context.i8_type();
        let i64_type = self.context.i64_type();
        let offset = i64_type.const_int(byte_offset, false);
        let field_ptr = unsafe {
            builder.build_in_bounds_gep(i8_type, base_ptr, &[offset], &format!("{}_ptr", name)).or_llvm_err()?
        };
        builder.build_store(field_ptr, value).or_llvm_err()?;
        Ok(())
    }

    /// Store a pointer value to a field in the tensor header.
    fn store_header_ptr(
        &self,
        builder: &verum_llvm::builder::Builder<'ctx>,
        base_ptr: verum_llvm::values::PointerValue<'ctx>,
        byte_offset: u64,
        value: verum_llvm::values::PointerValue<'ctx>,
        name: &str,
    ) -> Result<()> {
        let i8_type = self.context.i8_type();
        let i64_type = self.context.i64_type();
        let offset = i64_type.const_int(byte_offset, false);
        let field_ptr = unsafe {
            builder.build_in_bounds_gep(i8_type, base_ptr, &[offset], &format!("{}_ptr", name)).or_llvm_err()?
        };
        builder.build_store(field_ptr, value).or_llvm_err()?;
        Ok(())
    }

    /// Store a u32 value to a field in the tensor header.
    fn store_header_i32(
        &self,
        builder: &verum_llvm::builder::Builder<'ctx>,
        base_ptr: verum_llvm::values::PointerValue<'ctx>,
        byte_offset: u64,
        value: verum_llvm::values::IntValue<'ctx>,
        name: &str,
    ) -> Result<()> {
        let i8_type = self.context.i8_type();
        let i64_type = self.context.i64_type();
        let offset = i64_type.const_int(byte_offset, false);
        let field_ptr = unsafe {
            builder.build_in_bounds_gep(i8_type, base_ptr, &[offset], &format!("{}_ptr", name)).or_llvm_err()?
        };
        builder.build_store(field_ptr, value).or_llvm_err()?;
        Ok(())
    }

    /// Store a u8 value to a field in the tensor header.
    fn store_header_i8(
        &self,
        builder: &verum_llvm::builder::Builder<'ctx>,
        base_ptr: verum_llvm::values::PointerValue<'ctx>,
        byte_offset: u64,
        value: verum_llvm::values::IntValue<'ctx>,
        name: &str,
    ) -> Result<()> {
        let i8_type = self.context.i8_type();
        let i64_type = self.context.i64_type();
        let offset = i64_type.const_int(byte_offset, false);
        let field_ptr = unsafe {
            builder.build_in_bounds_gep(i8_type, base_ptr, &[offset], &format!("{}_ptr", name)).or_llvm_err()?
        };
        builder.build_store(field_ptr, value).or_llvm_err()?;
        Ok(())
    }

    /// Load a u8 value from a field in the tensor header.
    fn load_header_i8(
        &self,
        builder: &verum_llvm::builder::Builder<'ctx>,
        base_ptr: verum_llvm::values::PointerValue<'ctx>,
        byte_offset: u64,
        name: &str,
    ) -> Result<verum_llvm::values::IntValue<'ctx>> {
        let i8_type = self.context.i8_type();
        let i64_type = self.context.i64_type();
        let offset = i64_type.const_int(byte_offset, false);
        let field_ptr = unsafe {
            builder.build_in_bounds_gep(i8_type, base_ptr, &[offset], &format!("{}_ptr", name)).or_llvm_err()?
        };
        Ok(builder.build_load(i8_type, field_ptr, name).or_llvm_err()?.into_int_value())
    }

    /// Load a u32 value from a field in the tensor header.
    fn load_header_i32(
        &self,
        builder: &verum_llvm::builder::Builder<'ctx>,
        base_ptr: verum_llvm::values::PointerValue<'ctx>,
        byte_offset: u64,
        name: &str,
    ) -> Result<verum_llvm::values::IntValue<'ctx>> {
        let i8_type = self.context.i8_type();
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();
        let offset = i64_type.const_int(byte_offset, false);
        let field_ptr = unsafe {
            builder.build_in_bounds_gep(i8_type, base_ptr, &[offset], &format!("{}_ptr", name)).or_llvm_err()?
        };
        Ok(builder.build_load(i32_type, field_ptr, name).or_llvm_err()?.into_int_value())
    }

    /// Convert an i64 handle to a pointer (inttoptr).
    fn handle_to_ptr(
        &self,
        builder: &verum_llvm::builder::Builder<'ctx>,
        handle: verum_llvm::values::IntValue<'ctx>,
        name: &str,
    ) -> Result<verum_llvm::values::PointerValue<'ctx>> {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        Ok(builder.build_int_to_ptr(handle, ptr_type, name).or_llvm_err()?)
    }

    /// Convert a pointer to an i64 handle (ptrtoint).
    fn ptr_to_handle(
        &self,
        builder: &verum_llvm::builder::Builder<'ctx>,
        ptr: verum_llvm::values::PointerValue<'ctx>,
        name: &str,
    ) -> Result<verum_llvm::values::IntValue<'ctx>> {
        let i64_type = self.context.i64_type();
        Ok(builder.build_ptr_to_int(ptr, i64_type, name).or_llvm_err()?)
    }

    // ========================================================================
    // BATCH 1: LLVM math intrinsic declarations
    // ========================================================================

    fn emit_math_intrinsics(&self, module: &Module<'ctx>) {
        let f64_type = self.context.f64_type();

        // f64 -> f64 intrinsics
        let f64_f64 = f64_type.fn_type(&[f64_type.into()], false);
        let intrinsics_1arg = [
            "llvm.sqrt.f64", "llvm.sin.f64", "llvm.cos.f64",
            "llvm.exp.f64", "llvm.log.f64", "llvm.fabs.f64",
            "llvm.floor.f64", "llvm.ceil.f64", "llvm.round.f64",
            "llvm.log2.f64", "llvm.exp2.f64",
        ];
        for name in &intrinsics_1arg {
            self.get_or_declare(module, name, f64_f64);
        }

        // (f64, f64) -> f64 intrinsics
        let f64_f64_f64 = f64_type.fn_type(&[f64_type.into(), f64_type.into()], false);
        let intrinsics_2arg = [
            "llvm.pow.f64", "llvm.copysign.f64", "llvm.minnum.f64", "llvm.maxnum.f64",
        ];
        for name in &intrinsics_2arg {
            self.get_or_declare(module, name, f64_f64_f64);
        }

        // (f64, f64, f64) -> f64 intrinsics
        let f64_f64_f64_f64 = f64_type.fn_type(
            &[f64_type.into(), f64_type.into(), f64_type.into()], false,
        );
        self.get_or_declare(module, "llvm.fma.f64", f64_f64_f64_f64);

        // llvm.memset.p0.i64
        let void_type = self.context.void_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i8_type = self.context.i8_type();
        let i64_type = self.context.i64_type();
        let i1_type = self.context.bool_type();
        let memset_type = void_type.fn_type(
            &[ptr_type.into(), i8_type.into(), i64_type.into(), i1_type.into()], false,
        );
        self.get_or_declare(module, "llvm.memset.p0.i64", memset_type);

        // llvm.memcpy.p0.p0.i64
        let memcpy_type = void_type.fn_type(
            &[ptr_type.into(), ptr_type.into(), i64_type.into(), i1_type.into()], false,
        );
        self.get_or_declare(module, "llvm.memcpy.p0.p0.i64", memcpy_type);
    }

    /// Emit math wrapper functions that call LLVM intrinsics.
    /// These replace the software approximations in verum_tensor.c.
    fn emit_math_wrappers(&self, module: &Module<'ctx>) -> Result<()> {
        let f64_type = self.context.f64_type();
        let f64_f64 = f64_type.fn_type(&[f64_type.into()], false);

        // Simple 1-arg wrappers: verum_X -> @llvm.X.f64
        let wrappers_1arg = [
            ("verum_fabs", "llvm.fabs.f64"),
            ("verum_sqrt_approx", "llvm.sqrt.f64"),
            ("verum_exp_approx", "llvm.exp.f64"),
            ("verum_log_approx", "llvm.log.f64"),
            ("verum_sin_approx", "llvm.sin.f64"),
            ("verum_cos_approx", "llvm.cos.f64"),
            ("verum_floor_approx", "llvm.floor.f64"),
            ("verum_ceil_approx", "llvm.ceil.f64"),
            ("verum_round_approx", "llvm.round.f64"),
            ("verum_log2_approx", "llvm.log2.f64"),
        ];

        let ctx = self.context;
        let builder = ctx.create_builder();

        for (wrapper_name, intrinsic_name) in &wrappers_1arg {
            let func = match self.get_or_declare_new(module, wrapper_name, f64_f64) {
                Some(f) => f,
                None => continue,
            };

            let entry = ctx.append_basic_block(func, "entry");
            builder.position_at_end(entry);
            let x = func.get_first_param().or_internal("missing first param")?.into_float_value();
            let intrinsic = module.get_function(intrinsic_name).or_missing_fn(intrinsic_name)?;
            let result = builder.build_call(intrinsic, &[x.into()], "result").or_llvm_err()?;
            builder.build_return(Some(&result.try_as_basic_value().basic().or_internal("expected basic value")?)).or_llvm_err()?;
        }

        // verum_pow_approx(base, exp) -> @llvm.pow.f64(base, exp)
        {
            let f64_f64_f64 = f64_type.fn_type(&[f64_type.into(), f64_type.into()], false);
            if let Some(func) = self.get_or_declare_new(module, "verum_pow_approx", f64_f64_f64) {
                let entry = ctx.append_basic_block(func, "entry");
                builder.position_at_end(entry);
                let base = func.get_nth_param(0).or_internal("missing param")?.into_float_value();
                let exp = func.get_nth_param(1).or_internal("missing param")?.into_float_value();
                let intrinsic = module.get_function("llvm.pow.f64").or_missing_fn("llvm.pow.f64")?;
                let result = builder.build_call(intrinsic, &[base.into(), exp.into()], "result").or_llvm_err()?;
                builder.build_return(Some(&result.try_as_basic_value().basic().or_internal("expected basic value")?)).or_llvm_err()?;
            }
        }

        // verum_tan_approx(x) = sin(x) / cos(x)
        if let Some(func) = self.get_or_declare_new(module, "verum_tan_approx", f64_f64) {
            let entry = ctx.append_basic_block(func, "entry");
            builder.position_at_end(entry);
            let x = func.get_first_param().or_internal("missing first param")?.into_float_value();
            let sin_fn = module.get_function("llvm.sin.f64").or_missing_fn("llvm.sin.f64")?;
            let cos_fn = module.get_function("llvm.cos.f64").or_missing_fn("llvm.cos.f64")?;
            let s = builder.build_call(sin_fn, &[x.into()], "s").or_llvm_err()?
                .try_as_basic_value().basic().or_internal("expected basic value")?.into_float_value();
            let c = builder.build_call(cos_fn, &[x.into()], "c").or_llvm_err()?
                .try_as_basic_value().basic().or_internal("expected basic value")?.into_float_value();
            let result = builder.build_float_div(s, c, "tan").or_llvm_err()?;
            builder.build_return(Some(&result)).or_llvm_err()?;
        }

        // verum_tanh_approx(x) = (exp(x) - exp(-x)) / (exp(x) + exp(-x))
        if let Some(func) = self.get_or_declare_new(module, "verum_tanh_approx", f64_f64) {
            let entry = ctx.append_basic_block(func, "entry");
            builder.position_at_end(entry);
            let x = func.get_first_param().or_internal("missing first param")?.into_float_value();
            let exp_fn = module.get_function("llvm.exp.f64").or_missing_fn("llvm.exp.f64")?;
            let neg_x = builder.build_float_neg(x, "neg_x").or_llvm_err()?;
            let ep = builder.build_call(exp_fn, &[x.into()], "ep").or_llvm_err()?
                .try_as_basic_value().basic().or_internal("expected basic value")?.into_float_value();
            let em = builder.build_call(exp_fn, &[neg_x.into()], "em").or_llvm_err()?
                .try_as_basic_value().basic().or_internal("expected basic value")?.into_float_value();
            let num = builder.build_float_sub(ep, em, "num").or_llvm_err()?;
            let den = builder.build_float_add(ep, em, "den").or_llvm_err()?;
            let result = builder.build_float_div(num, den, "tanh").or_llvm_err()?;
            builder.build_return(Some(&result)).or_llvm_err()?;
        }

        // verum_sigmoid_approx(x) = 1 / (1 + exp(-x))
        if let Some(func) = self.get_or_declare_new(module, "verum_sigmoid_approx", f64_f64) {
            let entry = ctx.append_basic_block(func, "entry");
            builder.position_at_end(entry);
            let x = func.get_first_param().or_internal("missing first param")?.into_float_value();
            let exp_fn = module.get_function("llvm.exp.f64").or_missing_fn("llvm.exp.f64")?;
            let neg_x = builder.build_float_neg(x, "neg_x").or_llvm_err()?;
            let exp_neg = builder.build_call(exp_fn, &[neg_x.into()], "exp_neg").or_llvm_err()?
                .try_as_basic_value().basic().or_internal("expected basic value")?.into_float_value();
            let one = f64_type.const_float(1.0);
            let denom = builder.build_float_add(one, exp_neg, "denom").or_llvm_err()?;
            let result = builder.build_float_div(one, denom, "sigmoid").or_llvm_err()?;
            builder.build_return(Some(&result)).or_llvm_err()?;
        }

        // verum_gelu_approx(x) = 0.5 * x * (1 + erf(x / sqrt(2)))
        // We approximate erf using tanh: erf(x) ~ tanh(sqrt(2/pi) * (x + 0.044715 * x^3))
        if let Some(func) = self.get_or_declare_new(module, "verum_gelu_approx", f64_f64) {
            let entry = ctx.append_basic_block(func, "entry");
            builder.position_at_end(entry);
            let x = func.get_first_param().or_internal("missing first param")?.into_float_value();
            // GELU approximation: 0.5 * x * (1 + tanh(sqrt(2/pi) * (x + 0.044715 * x^3)))
            let half = f64_type.const_float(0.5);
            let one = f64_type.const_float(1.0);
            let coeff = f64_type.const_float(0.044715);
            let sqrt_2_pi = f64_type.const_float(0.7978845608028654); // sqrt(2/pi)
            let x3 = builder.build_float_mul(x, x, "x2").or_llvm_err()?;
            let x3 = builder.build_float_mul(x3, x, "x3").or_llvm_err()?;
            let cx3 = builder.build_float_mul(coeff, x3, "cx3").or_llvm_err()?;
            let inner = builder.build_float_add(x, cx3, "inner").or_llvm_err()?;
            let scaled = builder.build_float_mul(sqrt_2_pi, inner, "scaled").or_llvm_err()?;
            // Call our tanh wrapper
            let tanh_fn = self.get_or_declare(module, "verum_tanh_approx", f64_f64);
            let th = builder.build_call(tanh_fn, &[scaled.into()], "th").or_llvm_err()?
                .try_as_basic_value().basic().or_internal("expected basic value")?.into_float_value();
            let one_plus = builder.build_float_add(one, th, "one_plus").or_llvm_err()?;
            let half_x = builder.build_float_mul(half, x, "half_x").or_llvm_err()?;
            let result = builder.build_float_mul(half_x, one_plus, "gelu").or_llvm_err()?;
            builder.build_return(Some(&result)).or_llvm_err()?;
        }

        // verum_silu_approx(x) = x * sigmoid(x)
        if let Some(func) = self.get_or_declare_new(module, "verum_silu_approx", f64_f64) {
            let entry = ctx.append_basic_block(func, "entry");
            builder.position_at_end(entry);
            let x = func.get_first_param().or_internal("missing first param")?.into_float_value();
            let sig_fn = self.get_or_declare(module, "verum_sigmoid_approx", f64_f64);
            let sig = builder.build_call(sig_fn, &[x.into()], "sig").or_llvm_err()?
                .try_as_basic_value().basic().or_internal("expected basic value")?.into_float_value();
            let result = builder.build_float_mul(x, sig, "silu").or_llvm_err()?;
            builder.build_return(Some(&result)).or_llvm_err()?;
        }

        // verum_erf_approx(x) — Abramowitz & Stegun 7.1.26
        if let Some(func) = self.get_or_declare_new(module, "verum_erf_approx", f64_f64) {
            let entry = ctx.append_basic_block(func, "entry");
            builder.position_at_end(entry);
            let x = func.get_first_param().or_internal("missing first param")?.into_float_value();
            let fabs_fn = module.get_function("llvm.fabs.f64").or_missing_fn("llvm.fabs.f64")?;
            let exp_fn = module.get_function("llvm.exp.f64").or_missing_fn("llvm.exp.f64")?;

            let ax = builder.build_call(fabs_fn, &[x.into()], "ax").or_llvm_err()?
                .try_as_basic_value().basic().or_internal("expected basic value")?.into_float_value();
            // t = 1 / (1 + 0.3275911 * ax)
            let p = f64_type.const_float(0.3275911);
            let one = f64_type.const_float(1.0);
            let pax = builder.build_float_mul(p, ax, "pax").or_llvm_err()?;
            let denom = builder.build_float_add(one, pax, "denom").or_llvm_err()?;
            let t = builder.build_float_div(one, denom, "t").or_llvm_err()?;
            // Polynomial: a1*t + a2*t^2 + a3*t^3 + a4*t^4 + a5*t^5
            let a1 = f64_type.const_float(0.254829592);
            let a2 = f64_type.const_float(-0.284496736);
            let a3 = f64_type.const_float(1.421413741);
            let a4 = f64_type.const_float(-1.453152027);
            let a5 = f64_type.const_float(1.061405429);
            // Horner form: t*(a1 + t*(a2 + t*(a3 + t*(a4 + t*a5))))
            let r = builder.build_float_mul(t, a5, "r5").or_llvm_err()?;
            let r = builder.build_float_add(r, a4, "r4").or_llvm_err()?;
            let r = builder.build_float_mul(r, t, "r4t").or_llvm_err()?;
            let r = builder.build_float_add(r, a3, "r3").or_llvm_err()?;
            let r = builder.build_float_mul(r, t, "r3t").or_llvm_err()?;
            let r = builder.build_float_add(r, a2, "r2").or_llvm_err()?;
            let r = builder.build_float_mul(r, t, "r2t").or_llvm_err()?;
            let r = builder.build_float_add(r, a1, "r1").or_llvm_err()?;
            let r = builder.build_float_mul(r, t, "poly").or_llvm_err()?;
            // exp(-ax*ax)
            let neg_ax2 = builder.build_float_mul(ax, ax, "ax2").or_llvm_err()?;
            let neg_ax2 = builder.build_float_neg(neg_ax2, "neg_ax2").or_llvm_err()?;
            let exp_val = builder.build_call(exp_fn, &[neg_ax2.into()], "exp_negax2").or_llvm_err()?
                .try_as_basic_value().basic().or_internal("expected basic value")?.into_float_value();
            // result = 1 - poly * exp(-ax^2)
            let poly_exp = builder.build_float_mul(r, exp_val, "poly_exp").or_llvm_err()?;
            let abs_result = builder.build_float_sub(one, poly_exp, "abs_erf").or_llvm_err()?;
            // Restore sign: copysign(abs_result, x)
            let copysign_fn = module.get_function("llvm.copysign.f64").or_missing_fn("llvm.copysign.f64")?;
            let result = builder.build_call(copysign_fn, &[abs_result.into(), x.into()], "erf").or_llvm_err()?;
            builder.build_return(Some(&result.try_as_basic_value().basic().or_internal("expected basic value")?)).or_llvm_err()?;
        }

        // verum_softplus_approx(x) = log(1 + exp(x)), stable for large x
        if let Some(func) = self.get_or_declare_new(module, "verum_softplus_approx", f64_f64) {
            let entry = ctx.append_basic_block(func, "entry");
            let bb_large = ctx.append_basic_block(func, "large");
            let bb_small = ctx.append_basic_block(func, "small");
            let bb_normal = ctx.append_basic_block(func, "normal");
            let bb_ret = ctx.append_basic_block(func, "ret");

            builder.position_at_end(entry);
            let x = func.get_first_param().or_internal("missing first param")?.into_float_value();
            let twenty = f64_type.const_float(20.0);
            let neg_twenty = f64_type.const_float(-20.0);
            let is_large = builder.build_float_compare(FloatPredicate::OGT, x, twenty, "is_large").or_llvm_err()?;
            builder.build_conditional_branch(is_large, bb_large, bb_small).or_llvm_err()?;

            builder.position_at_end(bb_large);
            builder.build_unconditional_branch(bb_ret).or_llvm_err()?;

            builder.position_at_end(bb_small);
            let is_neg = builder.build_float_compare(FloatPredicate::OLT, x, neg_twenty, "is_neg").or_llvm_err()?;
            builder.build_conditional_branch(is_neg, bb_ret, bb_normal).or_llvm_err()?;

            builder.position_at_end(bb_normal);
            let exp_fn = module.get_function("llvm.exp.f64").or_missing_fn("llvm.exp.f64")?;
            let log_fn = module.get_function("llvm.log.f64").or_missing_fn("llvm.log.f64")?;
            let one = f64_type.const_float(1.0);
            let exp_x = builder.build_call(exp_fn, &[x.into()], "exp_x").or_llvm_err()?
                .try_as_basic_value().basic().or_internal("expected basic value")?.into_float_value();
            let one_plus = builder.build_float_add(one, exp_x, "one_plus").or_llvm_err()?;
            let log_val = builder.build_call(log_fn, &[one_plus.into()], "log_val").or_llvm_err()?
                .try_as_basic_value().basic().or_internal("expected basic value")?.into_float_value();
            builder.build_unconditional_branch(bb_ret).or_llvm_err()?;

            builder.position_at_end(bb_ret);
            let phi = builder.build_phi(f64_type, "softplus").or_llvm_err()?;
            let zero = f64_type.const_float(0.0);
            phi.add_incoming(&[(&x, bb_large), (&zero, bb_small), (&log_val, bb_normal)]);
            builder.build_return(Some(&phi.as_basic_value())).or_llvm_err()?;
        }

        // verum_mish_approx(x) = x * tanh(softplus(x))
        if let Some(func) = self.get_or_declare_new(module, "verum_mish_approx", f64_f64) {
            let entry = ctx.append_basic_block(func, "entry");
            builder.position_at_end(entry);
            let x = func.get_first_param().or_internal("missing first param")?.into_float_value();
            let sp_fn = self.get_or_declare(module, "verum_softplus_approx", f64_f64);
            let th_fn = self.get_or_declare(module, "verum_tanh_approx", f64_f64);
            let sp = builder.build_call(sp_fn, &[x.into()], "sp").or_llvm_err()?
                .try_as_basic_value().basic().or_internal("expected basic value")?.into_float_value();
            let th = builder.build_call(th_fn, &[sp.into()], "th").or_llvm_err()?
                .try_as_basic_value().basic().or_internal("expected basic value")?.into_float_value();
            let result = builder.build_float_mul(x, th, "mish").or_llvm_err()?;
            builder.build_return(Some(&result)).or_llvm_err()?;
        }
        Ok(())
    }

    // ========================================================================
    // BATCH 2: Tensor creation/destruction
    // ========================================================================

    /// verum_tensor_new(ndim: i64, shape_ptr: i64, dtype: i64) -> i64
    fn emit_tensor_new(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let i32_type = ctx.i32_type();
        let i8_type = ctx.i8_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());
        let f64_type = ctx.f64_type();
        let fn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into(), i64_type.into()], false);

        let func = match self.get_or_declare_new(module, "verum_tensor_new", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let bb_ok = ctx.append_basic_block(func, "alloc_ok");
        let bb_fail = ctx.append_basic_block(func, "alloc_fail");
        let bb_data_ok = ctx.append_basic_block(func, "data_ok");
        let bb_compute_strides = ctx.append_basic_block(func, "compute_strides");
        let bb_stride_loop = ctx.append_basic_block(func, "stride_loop");
        let bb_stride_done = ctx.append_basic_block(func, "stride_done");
        let bb_shape_loop = ctx.append_basic_block(func, "shape_loop");
        let bb_shape_done = ctx.append_basic_block(func, "shape_done");
        let bb_numel_loop = ctx.append_basic_block(func, "numel_loop");
        let bb_numel_done = ctx.append_basic_block(func, "numel_done");
        let bb_zero = ctx.append_basic_block(func, "zero_data");
        let bb_ret = ctx.append_basic_block(func, "ret");

        builder.position_at_end(entry);
        let ndim_param = self.param_as_i64(&builder, func.get_nth_param(0).or_internal("missing param")?, "p0_i64")?;
        let shape_param = self.param_as_i64(&builder, func.get_nth_param(1).or_internal("missing param")?, "p1_i64")?;
        let dtype_param = self.param_as_i64(&builder, func.get_nth_param(2).or_internal("missing param")?, "p2_i64")?;
        // Allocate header
        let alloc_fn = self.get_or_declare(
            module, "verum_alloc",
            ptr_type.fn_type(&[i64_type.into()], false),
        );
        let header_size = i64_type.const_int(TENSOR_HEADER_SIZE, false);
        let header_ptr_val = builder.build_call(alloc_fn, &[header_size.into()], "hdr").or_llvm_err()?;
        let header_ptr = header_ptr_val.try_as_basic_value().basic().or_internal("expected basic value")?.into_pointer_value();
        // Check null
        let hdr_i64 = self.ptr_to_handle(&builder, header_ptr, "hdr_i64")?;
        let is_null = builder.build_int_compare(IntPredicate::EQ, hdr_i64, i64_type.const_zero(), "is_null").or_llvm_err()?;
        builder.build_conditional_branch(is_null, bb_fail, bb_ok).or_llvm_err()?;

        builder.position_at_end(bb_fail);
        builder.build_return(Some(&i64_type.const_zero())).or_llvm_err()?;

        builder.position_at_end(bb_ok);
        // Zero out the header
        builder.build_memset(header_ptr, 1, i8_type.const_zero(), header_size).or_llvm_err()?;
        // Store refcount = 1
        self.store_header_i32(&builder, header_ptr, OFF_REFCOUNT, i32_type.const_int(1, false), "refcount")?;
        // Store dtype (u8)
        let dtype_u8 = builder.build_int_truncate(dtype_param, i8_type, "dtype_u8").or_llvm_err()?;
        self.store_header_i8(&builder, header_ptr, OFF_DTYPE, dtype_u8, "dtype")?;
        // Clamp ndim to MAX_DIMS
        let max_dims = i64_type.const_int(TENSOR_MAX_DIMS, false);
        let ndim_clamped = builder.build_select(
            builder.build_int_compare(IntPredicate::UGT, ndim_param, max_dims, "ndim_gt").or_llvm_err()?,
            max_dims, ndim_param, "ndim_clamped",
        ).or_llvm_err()?.into_int_value();
        let ndim_u8 = builder.build_int_truncate(ndim_clamped, i8_type, "ndim_u8").or_llvm_err()?;
        self.store_header_i8(&builder, header_ptr, OFF_NDIM, ndim_u8, "ndim")?;

        // Copy shape from input pointer
        let shape_ptr = self.handle_to_ptr(&builder, shape_param, "shape_ptr")?;
        builder.build_unconditional_branch(bb_shape_loop).or_llvm_err()?;

        // Shape copy loop
        builder.position_at_end(bb_shape_loop);
        let phi_i = builder.build_phi(i64_type, "i").or_llvm_err()?;
        phi_i.add_incoming(&[(&i64_type.const_zero(), bb_ok)]);
        let i_val = phi_i.as_basic_value().into_int_value();
        let cmp = builder.build_int_compare(IntPredicate::ULT, i_val, ndim_clamped, "cmp").or_llvm_err()?;
        builder.build_conditional_branch(cmp, bb_shape_done, bb_numel_loop).or_llvm_err()?;

        builder.position_at_end(bb_shape_done);
        // Load shape[i] from input
        let src_elem = unsafe {
            builder.build_in_bounds_gep(i64_type, shape_ptr, &[i_val], "src_elem").or_llvm_err()?
        };
        let shape_val = builder.build_load(i64_type, src_elem, "shape_val").or_llvm_err()?;
        // Store to header shape field
        let shape_base_offset = i64_type.const_int(OFF_SHAPE, false);
        let elem_byte_off = builder.build_int_mul(i_val, i64_type.const_int(8, false), "ebo").or_llvm_err()?;
        let total_off = builder.build_int_add(shape_base_offset, elem_byte_off, "toff").or_llvm_err()?;
        let dst_elem = unsafe {
            builder.build_in_bounds_gep(i8_type, header_ptr, &[total_off], "dst_elem").or_llvm_err()?
        };
        builder.build_store(dst_elem, shape_val).or_llvm_err()?;
        let next_i = builder.build_int_add(i_val, i64_type.const_int(1, false), "next_i").or_llvm_err()?;
        phi_i.add_incoming(&[(&next_i, bb_shape_done)]);
        builder.build_unconditional_branch(bb_shape_loop).or_llvm_err()?;

        // Compute numel = product of shape dims
        builder.position_at_end(bb_numel_loop);
        // Simplified: just call a helper to compute strides and numel
        // For now, compute numel = prod(shape[0..ndim])
        let numel_phi_j = builder.build_phi(i64_type, "j").or_llvm_err()?;
        numel_phi_j.add_incoming(&[(&i64_type.const_zero(), bb_shape_loop)]);
        let numel_phi_prod = builder.build_phi(i64_type, "prod").or_llvm_err()?;
        numel_phi_prod.add_incoming(&[(&i64_type.const_int(1, false), bb_shape_loop)]);
        let j_val = numel_phi_j.as_basic_value().into_int_value();
        let prod_val = numel_phi_prod.as_basic_value().into_int_value();
        let j_cmp = builder.build_int_compare(IntPredicate::ULT, j_val, ndim_clamped, "j_cmp").or_llvm_err()?;
        builder.build_conditional_branch(j_cmp, bb_numel_done, bb_compute_strides).or_llvm_err()?;

        builder.position_at_end(bb_numel_done);
        // Load shape[j] from header
        let shape_off = builder.build_int_add(
            i64_type.const_int(OFF_SHAPE, false),
            builder.build_int_mul(j_val, i64_type.const_int(8, false), "j8").or_llvm_err()?,
            "shape_off",
        ).or_llvm_err()?;
        let shape_elem_ptr = unsafe {
            builder.build_in_bounds_gep(i8_type, header_ptr, &[shape_off], "se_ptr").or_llvm_err()?
        };
        let dim_val = builder.build_load(i64_type, shape_elem_ptr, "dim").or_llvm_err()?.into_int_value();
        let new_prod = builder.build_int_mul(prod_val, dim_val, "new_prod").or_llvm_err()?;
        let next_j = builder.build_int_add(j_val, i64_type.const_int(1, false), "next_j").or_llvm_err()?;
        numel_phi_j.add_incoming(&[(&next_j, bb_numel_done)]);
        numel_phi_prod.add_incoming(&[(&new_prod, bb_numel_done)]);
        builder.build_unconditional_branch(bb_numel_loop).or_llvm_err()?;

        // Compute strides (reverse order: strides[ndim-1] = 1, strides[i] = strides[i+1] * shape[i+1])
        builder.position_at_end(bb_compute_strides);
        let numel = prod_val; // final product
        self.store_header_i64(&builder, header_ptr, OFF_NUMEL, numel, "numel")?;

        // Check ndim > 0 for stride computation
        let bb_stride_init = ctx.append_basic_block(func, "stride_init");
        let ndim_gt0 = builder.build_int_compare(IntPredicate::UGT, ndim_clamped, i64_type.const_zero(), "ndim_gt0").or_llvm_err()?;
        builder.build_conditional_branch(ndim_gt0, bb_stride_init, bb_data_ok).or_llvm_err()?;

        // stride_init: set strides[ndim-1] = 1, then jump to stride_loop
        builder.position_at_end(bb_stride_init);
        let last_idx = builder.build_int_sub(ndim_clamped, i64_type.const_int(1, false), "last_idx").or_llvm_err()?;
        let stride_off_last = builder.build_int_add(
            i64_type.const_int(OFF_STRIDES, false),
            builder.build_int_mul(last_idx, i64_type.const_int(8, false), "li8").or_llvm_err()?,
            "stride_off_last",
        ).or_llvm_err()?;
        let stride_ptr_last = unsafe {
            builder.build_in_bounds_gep(i8_type, header_ptr, &[stride_off_last], "sl_ptr").or_llvm_err()?
        };
        builder.build_store(stride_ptr_last, i64_type.const_int(1, false)).or_llvm_err()?;
        builder.build_unconditional_branch(bb_stride_loop).or_llvm_err()?;

        // Stride loop: PHI nodes at the top of the block
        builder.position_at_end(bb_stride_loop);
        let stride_count = last_idx; // ndim-1 iterations (may be 0)
        let phi_k = builder.build_phi(i64_type, "k_idx").or_llvm_err()?;
        phi_k.add_incoming(&[(&i64_type.const_zero(), bb_stride_init)]);
        let k_idx = phi_k.as_basic_value().into_int_value();
        let k_cmp = builder.build_int_compare(IntPredicate::ULT, k_idx, stride_count, "k_cmp").or_llvm_err()?;
        builder.build_conditional_branch(k_cmp, bb_stride_done, bb_data_ok).or_llvm_err()?;

        builder.position_at_end(bb_stride_done);
        // actual dimension index = ndim - 2 - k_idx
        let dim_idx = builder.build_int_sub(
            builder.build_int_sub(ndim_clamped, i64_type.const_int(2, false), "nm2").or_llvm_err()?,
            k_idx, "dim_idx",
        ).or_llvm_err()?;
        let next_dim_idx = builder.build_int_add(dim_idx, i64_type.const_int(1, false), "ndi").or_llvm_err()?;

        // Load strides[dim_idx + 1]
        let stride_next_off = builder.build_int_add(
            i64_type.const_int(OFF_STRIDES, false),
            builder.build_int_mul(next_dim_idx, i64_type.const_int(8, false), "ndi8").or_llvm_err()?,
            "sno",
        ).or_llvm_err()?;
        let stride_next_ptr = unsafe {
            builder.build_in_bounds_gep(i8_type, header_ptr, &[stride_next_off], "snp").or_llvm_err()?
        };
        let stride_next = builder.build_load(i64_type, stride_next_ptr, "stride_next").or_llvm_err()?.into_int_value();

        // Load shape[dim_idx + 1]
        let shape_next_off = builder.build_int_add(
            i64_type.const_int(OFF_SHAPE, false),
            builder.build_int_mul(next_dim_idx, i64_type.const_int(8, false), "ndi8s").or_llvm_err()?,
            "shno",
        ).or_llvm_err()?;
        let shape_next_ptr = unsafe {
            builder.build_in_bounds_gep(i8_type, header_ptr, &[shape_next_off], "shnp").or_llvm_err()?
        };
        let shape_next = builder.build_load(i64_type, shape_next_ptr, "shape_next").or_llvm_err()?.into_int_value();

        // strides[dim_idx] = stride_next * shape_next
        let new_stride = builder.build_int_mul(stride_next, shape_next, "new_stride").or_llvm_err()?;
        let stride_cur_off = builder.build_int_add(
            i64_type.const_int(OFF_STRIDES, false),
            builder.build_int_mul(dim_idx, i64_type.const_int(8, false), "di8").or_llvm_err()?,
            "sco",
        ).or_llvm_err()?;
        let stride_cur_ptr = unsafe {
            builder.build_in_bounds_gep(i8_type, header_ptr, &[stride_cur_off], "scp").or_llvm_err()?
        };
        builder.build_store(stride_cur_ptr, new_stride).or_llvm_err()?;

        let next_k = builder.build_int_add(k_idx, i64_type.const_int(1, false), "next_k").or_llvm_err()?;
        phi_k.add_incoming(&[(&next_k, bb_stride_done)]);
        builder.build_unconditional_branch(bb_stride_loop).or_llvm_err()?;

        // Allocate data array
        builder.position_at_end(bb_data_ok);
        let data_size = builder.build_int_mul(numel, i64_type.const_int(8, false), "data_size").or_llvm_err()?;
        let data_ptr_val = builder.build_call(alloc_fn, &[data_size.into()], "data").or_llvm_err()?;
        let data_ptr = data_ptr_val.try_as_basic_value().basic().or_internal("expected basic value")?.into_pointer_value();
        self.store_header_ptr(&builder, header_ptr, OFF_DATA, data_ptr, "data")?;

        // Zero data
        builder.build_memset(data_ptr, 1, i8_type.const_zero(), data_size).or_llvm_err()?;

        builder.build_unconditional_branch(bb_ret).or_llvm_err()?;

        // Terminate unused zero_data block (dead code, needed for LLVM verification)
        builder.position_at_end(bb_zero);
        builder.build_unconditional_branch(bb_ret).or_llvm_err()?;

        builder.position_at_end(bb_ret);
        let result = self.ptr_to_handle(&builder, header_ptr, "result")?;
        builder.build_return(Some(&result)).or_llvm_err()?;
        Ok(())
    }

    /// verum_tensor_fill(ndim: i64, shape_ptr: i64, value: f64, dtype: i64) -> i64
    fn emit_tensor_fill(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let f64_type = ctx.f64_type();
        let fn_type = i64_type.fn_type(
            &[i64_type.into(), i64_type.into(), f64_type.into(), i64_type.into()], false,
        );

        let func = match self.get_or_declare_new(module, "verum_tensor_fill", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let bb_loop = ctx.append_basic_block(func, "loop");
        let bb_body = ctx.append_basic_block(func, "body");
        let bb_done = ctx.append_basic_block(func, "done");

        builder.position_at_end(entry);
        let ndim = self.param_as_i64(&builder, func.get_nth_param(0).or_internal("missing param")?, "p0_i64")?;
        let shape = self.param_as_i64(&builder, func.get_nth_param(1).or_internal("missing param")?, "shape_p2i")?;
        let value = func.get_nth_param(2).or_internal("missing param")?.into_float_value();
        let dtype = self.param_as_i64(&builder, func.get_nth_param(3).or_internal("missing param")?, "p3_i64")?;

        // Call tensor_new to allocate
        let new_fn = self.get_or_declare(module, "verum_tensor_new",
            i64_type.fn_type(&[i64_type.into(), i64_type.into(), i64_type.into()], false));
        let handle = builder.build_call(new_fn, &[ndim.into(), shape.into(), dtype.into()], "handle").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_int_value();

        // Get data pointer and numel
        let ptr = self.handle_to_ptr(&builder, handle, "hdr_ptr")?;
        let numel = self.load_header_i64(&builder, ptr, OFF_NUMEL, "numel")?;
        let data_ptr = self.load_header_ptr(&builder, ptr, OFF_DATA, "data_ptr")?;
        builder.build_unconditional_branch(bb_loop).or_llvm_err()?;

        // Fill loop
        builder.position_at_end(bb_loop);
        let phi_i = builder.build_phi(i64_type, "i").or_llvm_err()?;
        phi_i.add_incoming(&[(&i64_type.const_zero(), entry)]);
        let i_val = phi_i.as_basic_value().into_int_value();
        let cmp = builder.build_int_compare(IntPredicate::ULT, i_val, numel, "cmp").or_llvm_err()?;
        builder.build_conditional_branch(cmp, bb_body, bb_done).or_llvm_err()?;

        builder.position_at_end(bb_body);
        let elem_ptr = unsafe {
            builder.build_in_bounds_gep(f64_type, data_ptr, &[i_val], "elem").or_llvm_err()?
        };
        builder.build_store(elem_ptr, value).or_llvm_err()?;
        let next = builder.build_int_add(i_val, i64_type.const_int(1, false), "next").or_llvm_err()?;
        phi_i.add_incoming(&[(&next, bb_body)]);
        builder.build_unconditional_branch(bb_loop).or_llvm_err()?;

        builder.position_at_end(bb_done);
        builder.build_return(Some(&handle)).or_llvm_err()?;
        Ok(())
    }

    /// verum_tensor_from_data(ndim: i64, shape_ptr: i64, data: i64, dtype: i64) -> i64
    fn emit_tensor_from_data(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let i1_type = ctx.bool_type();
        let fn_type = i64_type.fn_type(
            &[i64_type.into(), i64_type.into(), i64_type.into(), i64_type.into()], false,
        );

        let func = match self.get_or_declare_new(module, "verum_tensor_from_data", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");

        builder.position_at_end(entry);
        let ndim = self.param_as_i64(&builder, func.get_nth_param(0).or_internal("missing param")?, "p0_i64")?;
        let shape = self.param_as_i64(&builder, func.get_nth_param(1).or_internal("missing param")?, "shape_p2i")?;
        let data_param = self.param_as_i64(&builder, func.get_nth_param(2).or_internal("missing param")?, "p2_i64")?;
        let dtype = self.param_as_i64(&builder, func.get_nth_param(3).or_internal("missing param")?, "p3_i64")?;

        let new_fn = self.get_or_declare(module, "verum_tensor_new",
            i64_type.fn_type(&[i64_type.into(), i64_type.into(), i64_type.into()], false));
        let handle = builder.build_call(new_fn, &[ndim.into(), shape.into(), dtype.into()], "handle").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_int_value();

        // Copy data: memcpy(tensor->data, data_param, numel * 8)
        let ptr = self.handle_to_ptr(&builder, handle, "hdr_ptr")?;
        let numel = self.load_header_i64(&builder, ptr, OFF_NUMEL, "numel")?;
        let data_ptr = self.load_header_ptr(&builder, ptr, OFF_DATA, "data_ptr")?;
        let src_ptr = self.handle_to_ptr(&builder, data_param, "src_ptr")?;
        let byte_count = builder.build_int_mul(numel, i64_type.const_int(8, false), "bytes").or_llvm_err()?;

        let memcpy_fn = self.get_or_declare(module, "llvm.memcpy.p0.p0.i64",
            ctx.void_type().fn_type(
                &[ctx.ptr_type(AddressSpace::default()).into(),
                  ctx.ptr_type(AddressSpace::default()).into(),
                  i64_type.into(), i1_type.into()], false));
        builder.build_call(memcpy_fn,
            &[data_ptr.into(), src_ptr.into(), byte_count.into(), i1_type.const_zero().into()],
            "").or_llvm_err()?;

        builder.build_return(Some(&handle)).or_llvm_err()?;
        Ok(())
    }

    /// verum_tensor_free(handle: i64) -> void
    fn emit_tensor_free(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let i32_type = ctx.i32_type();
        let void_type = ctx.void_type();
        let fn_type = void_type.fn_type(&[i64_type.into()], false);

        let func = match self.get_or_declare_new(module, "verum_tensor_free", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let bb_not_null = ctx.append_basic_block(func, "not_null");
        let bb_ret = ctx.append_basic_block(func, "ret");

        builder.position_at_end(entry);
        let handle = func.get_first_param().or_internal("missing first param")?.into_int_value();
        let is_null = builder.build_int_compare(IntPredicate::EQ, handle, i64_type.const_zero(), "is_null").or_llvm_err()?;
        builder.build_conditional_branch(is_null, bb_ret, bb_not_null).or_llvm_err()?;

        builder.position_at_end(bb_not_null);
        let ptr = self.handle_to_ptr(&builder, handle, "hdr")?;
        // Decrement refcount
        let rc = self.load_header_i32(&builder, ptr, OFF_REFCOUNT, "rc")?;
        let new_rc = builder.build_int_sub(rc, i32_type.const_int(1, false), "new_rc").or_llvm_err()?;
        self.store_header_i32(&builder, ptr, OFF_REFCOUNT, new_rc, "rc")?;
        // No actual deallocation (arena allocator - freed with arena)
        builder.build_unconditional_branch(bb_ret).or_llvm_err()?;

        builder.position_at_end(bb_ret);
        builder.build_return(None).or_llvm_err()?;
        Ok(())
    }

    /// verum_tensor_clone(handle: i64) -> i64
    fn emit_tensor_clone(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let fn_type = i64_type.fn_type(&[i64_type.into()], false);

        let func = match self.get_or_declare_new(module, "verum_tensor_clone", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let bb_not_null = ctx.append_basic_block(func, "not_null");
        let bb_null = ctx.append_basic_block(func, "null");

        builder.position_at_end(entry);
        let handle = func.get_first_param().or_internal("missing first param")?.into_int_value();
        let is_null = builder.build_int_compare(IntPredicate::EQ, handle, i64_type.const_zero(), "is_null").or_llvm_err()?;
        builder.build_conditional_branch(is_null, bb_null, bb_not_null).or_llvm_err()?;

        builder.position_at_end(bb_null);
        builder.build_return(Some(&i64_type.const_zero())).or_llvm_err()?;

        builder.position_at_end(bb_not_null);
        let ptr = self.handle_to_ptr(&builder, handle, "hdr")?;
        let ndim = self.load_header_i8(&builder, ptr, OFF_NDIM, "ndim")?;
        let ndim_i64 = builder.build_int_z_extend(ndim, i64_type, "ndim_i64").or_llvm_err()?;
        let dtype = self.load_header_i8(&builder, ptr, OFF_DTYPE, "dtype")?;
        let dtype_i64 = builder.build_int_z_extend(dtype, i64_type, "dtype_i64").or_llvm_err()?;
        let data_ptr = self.load_header_ptr(&builder, ptr, OFF_DATA, "data_ptr")?;
        let data_i64 = self.ptr_to_handle(&builder, data_ptr, "data_i64")?;

        // Get pointer to shape array in header
        let shape_off = i64_type.const_int(OFF_SHAPE, false);
        let shape_ptr = unsafe {
            builder.build_in_bounds_gep(ctx.i8_type(), ptr, &[shape_off], "shape_ptr").or_llvm_err()?
        };
        let shape_i64 = self.ptr_to_handle(&builder, shape_ptr, "shape_i64")?;

        let from_data_fn = self.get_or_declare(module, "verum_tensor_from_data",
            i64_type.fn_type(&[i64_type.into(), i64_type.into(), i64_type.into(), i64_type.into()], false));
        let result = builder.build_call(from_data_fn,
            &[ndim_i64.into(), shape_i64.into(), data_i64.into(), dtype_i64.into()], "clone").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?;
        builder.build_return(Some(&result)).or_llvm_err()?;
        Ok(())
    }

    /// verum_tensor_get_scalar(handle: i64, idx: i64) -> f64
    fn emit_tensor_get_scalar(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let f64_type = ctx.f64_type();
        let fn_type = f64_type.fn_type(&[i64_type.into(), i64_type.into()], false);

        let func = match self.get_or_declare_new(module, "verum_tensor_get_scalar", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let bb_ok = ctx.append_basic_block(func, "ok");
        let bb_fail = ctx.append_basic_block(func, "fail");

        builder.position_at_end(entry);
        let handle = self.param_as_i64(&builder, func.get_nth_param(0).or_internal("missing param")?, "p0_i64")?;
        let idx = self.param_as_i64(&builder, func.get_nth_param(1).or_internal("missing param")?, "p1_i64")?;
        let is_null = builder.build_int_compare(IntPredicate::EQ, handle, i64_type.const_zero(), "is_null").or_llvm_err()?;
        builder.build_conditional_branch(is_null, bb_fail, bb_ok).or_llvm_err()?;

        builder.position_at_end(bb_fail);
        builder.build_return(Some(&f64_type.const_float(0.0))).or_llvm_err()?;

        builder.position_at_end(bb_ok);
        let ptr = self.handle_to_ptr(&builder, handle, "hdr")?;
        let data_ptr = self.load_header_ptr(&builder, ptr, OFF_DATA, "data_ptr")?;
        let elem_ptr = unsafe {
            builder.build_in_bounds_gep(f64_type, data_ptr, &[idx], "elem").or_llvm_err()?
        };
        let val = builder.build_load(f64_type, elem_ptr, "val").or_llvm_err()?;
        builder.build_return(Some(&val)).or_llvm_err()?;
        Ok(())
    }

    /// verum_tensor_set_scalar(handle: i64, idx: i64, value: f64) -> void
    fn emit_tensor_set_scalar(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let f64_type = ctx.f64_type();
        let void_type = ctx.void_type();
        let fn_type = void_type.fn_type(&[i64_type.into(), i64_type.into(), f64_type.into()], false);

        let func = match self.get_or_declare_new(module, "verum_tensor_set_scalar", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let bb_ok = ctx.append_basic_block(func, "ok");
        let bb_ret = ctx.append_basic_block(func, "ret");

        builder.position_at_end(entry);
        let handle = self.param_as_i64(&builder, func.get_nth_param(0).or_internal("missing param")?, "p0_i64")?;
        let idx = self.param_as_i64(&builder, func.get_nth_param(1).or_internal("missing param")?, "p1_i64")?;
        let value = func.get_nth_param(2).or_internal("missing param")?.into_float_value();
        let is_null = builder.build_int_compare(IntPredicate::EQ, handle, i64_type.const_zero(), "is_null").or_llvm_err()?;
        builder.build_conditional_branch(is_null, bb_ret, bb_ok).or_llvm_err()?;

        builder.position_at_end(bb_ok);
        let ptr = self.handle_to_ptr(&builder, handle, "hdr")?;
        let data_ptr = self.load_header_ptr(&builder, ptr, OFF_DATA, "data_ptr")?;
        let elem_ptr = unsafe {
            builder.build_in_bounds_gep(f64_type, data_ptr, &[idx], "elem").or_llvm_err()?
        };
        builder.build_store(elem_ptr, value).or_llvm_err()?;
        builder.build_unconditional_branch(bb_ret).or_llvm_err()?;

        builder.position_at_end(bb_ret);
        builder.build_return(None).or_llvm_err()?;
        Ok(())
    }

    // ========================================================================
    // BATCH 3: Tensor operations (core)
    // ========================================================================

    /// verum_tensor_unop(handle: i64, op: i64) -> i64
    /// Element-wise unary operation using LLVM intrinsics.
    fn emit_tensor_unop(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let f64_type = ctx.f64_type();
        let fn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);

        let func = match self.get_or_declare_new(module, "verum_tensor_unop", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let bb_null = ctx.append_basic_block(func, "null");
        let bb_alloc = ctx.append_basic_block(func, "alloc");
        let bb_loop = ctx.append_basic_block(func, "loop");
        let bb_body = ctx.append_basic_block(func, "body");
        let bb_done = ctx.append_basic_block(func, "done");
        // Switch blocks for each op
        let bb_neg = ctx.append_basic_block(func, "neg");
        let bb_abs = ctx.append_basic_block(func, "abs");
        let bb_sqrt = ctx.append_basic_block(func, "sqrt");
        let bb_exp = ctx.append_basic_block(func, "exp");
        let bb_log = ctx.append_basic_block(func, "log");
        let bb_sin = ctx.append_basic_block(func, "sin");
        let bb_cos = ctx.append_basic_block(func, "cos");
        let bb_tanh = ctx.append_basic_block(func, "tanh");
        let bb_sigmoid = ctx.append_basic_block(func, "sigmoid");
        let bb_relu = ctx.append_basic_block(func, "relu");
        let bb_floor = ctx.append_basic_block(func, "floor");
        let bb_ceil = ctx.append_basic_block(func, "ceil");
        let bb_round = ctx.append_basic_block(func, "round");
        let bb_default = ctx.append_basic_block(func, "default");
        let bb_store = ctx.append_basic_block(func, "store");

        builder.position_at_end(entry);
        let handle = self.param_as_i64(&builder, func.get_nth_param(0).or_internal("missing param")?, "p0_i64")?;
        let op = self.param_as_i64(&builder, func.get_nth_param(1).or_internal("missing param")?, "p1_i64")?;
        let is_null = builder.build_int_compare(IntPredicate::EQ, handle, i64_type.const_zero(), "is_null").or_llvm_err()?;
        builder.build_conditional_branch(is_null, bb_null, bb_alloc).or_llvm_err()?;

        builder.position_at_end(bb_null);
        builder.build_return(Some(&i64_type.const_zero())).or_llvm_err()?;

        // Allocate result tensor (same shape)
        builder.position_at_end(bb_alloc);
        let src_ptr = self.handle_to_ptr(&builder, handle, "src")?;
        let ndim = self.load_header_i8(&builder, src_ptr, OFF_NDIM, "ndim")?;
        let ndim_i64 = builder.build_int_z_extend(ndim, i64_type, "ndim64").or_llvm_err()?;
        let dtype = self.load_header_i8(&builder, src_ptr, OFF_DTYPE, "dtype")?;
        let dtype_i64 = builder.build_int_z_extend(dtype, i64_type, "dtype64").or_llvm_err()?;
        let numel = self.load_header_i64(&builder, src_ptr, OFF_NUMEL, "numel")?;
        let src_data = self.load_header_ptr(&builder, src_ptr, OFF_DATA, "src_data")?;

        let shape_off = i64_type.const_int(OFF_SHAPE, false);
        let shape_ptr = unsafe {
            builder.build_in_bounds_gep(ctx.i8_type(), src_ptr, &[shape_off], "shape_ptr").or_llvm_err()?
        };
        let shape_i64 = self.ptr_to_handle(&builder, shape_ptr, "shape_i64")?;

        let new_fn = self.get_or_declare(module, "verum_tensor_new",
            i64_type.fn_type(&[i64_type.into(), i64_type.into(), i64_type.into()], false));
        let result_handle = builder.build_call(new_fn, &[ndim_i64.into(), shape_i64.into(), dtype_i64.into()], "result").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_int_value();
        let result_ptr = self.handle_to_ptr(&builder, result_handle, "res_ptr")?;
        let result_data = self.load_header_ptr(&builder, result_ptr, OFF_DATA, "res_data")?;
        builder.build_unconditional_branch(bb_loop).or_llvm_err()?;

        // Main loop
        builder.position_at_end(bb_loop);
        let phi_i = builder.build_phi(i64_type, "i").or_llvm_err()?;
        phi_i.add_incoming(&[(&i64_type.const_zero(), bb_alloc)]);
        let i_val = phi_i.as_basic_value().into_int_value();
        let cmp = builder.build_int_compare(IntPredicate::ULT, i_val, numel, "cmp").or_llvm_err()?;
        builder.build_conditional_branch(cmp, bb_body, bb_done).or_llvm_err()?;

        // Load element
        builder.position_at_end(bb_body);
        let elem_ptr = unsafe {
            builder.build_in_bounds_gep(f64_type, src_data, &[i_val], "elem").or_llvm_err()?
        };
        let x = builder.build_load(f64_type, elem_ptr, "x").or_llvm_err()?.into_float_value();

        // Switch on op code
        builder.build_switch(op, bb_default, &[
            (i64_type.const_int(TENSOR_UNOP_NEG, false), bb_neg),
            (i64_type.const_int(TENSOR_UNOP_ABS, false), bb_abs),
            (i64_type.const_int(TENSOR_UNOP_SQRT, false), bb_sqrt),
            (i64_type.const_int(TENSOR_UNOP_EXP, false), bb_exp),
            (i64_type.const_int(TENSOR_UNOP_LOG, false), bb_log),
            (i64_type.const_int(TENSOR_UNOP_SIN, false), bb_sin),
            (i64_type.const_int(TENSOR_UNOP_COS, false), bb_cos),
            (i64_type.const_int(TENSOR_UNOP_TANH, false), bb_tanh),
            (i64_type.const_int(TENSOR_UNOP_SIGMOID, false), bb_sigmoid),
            (i64_type.const_int(TENSOR_UNOP_RELU, false), bb_relu),
            (i64_type.const_int(TENSOR_UNOP_FLOOR, false), bb_floor),
            (i64_type.const_int(TENSOR_UNOP_CEIL, false), bb_ceil),
            (i64_type.const_int(TENSOR_UNOP_ROUND, false), bb_round),
        ]).or_llvm_err()?;

        // Helper to emit intrinsic-based unop cases
        let call_intrinsic_1 = |bb: verum_llvm::basic_block::BasicBlock<'ctx>, intrinsic_name: &str| -> Result<verum_llvm::values::FloatValue<'ctx>> {
            builder.position_at_end(bb);
            let intr = module.get_function(intrinsic_name).or_missing_fn(intrinsic_name)?;
            let r = builder.build_call(intr, &[x.into()], "r").or_llvm_err()?
                .try_as_basic_value().basic().or_internal("expected basic value")?.into_float_value();
            builder.build_unconditional_branch(bb_store).or_llvm_err()?;
            Ok(r)
        };

        // NEG
        builder.position_at_end(bb_neg);
        let neg_r = builder.build_float_neg(x, "neg").or_llvm_err()?;
        builder.build_unconditional_branch(bb_store).or_llvm_err()?;

        // ABS, SQRT, EXP, LOG, SIN, COS, FLOOR, CEIL, ROUND
        let abs_r = call_intrinsic_1(bb_abs, "llvm.fabs.f64")?;
        let sqrt_r = call_intrinsic_1(bb_sqrt, "llvm.sqrt.f64")?;
        let exp_r = call_intrinsic_1(bb_exp, "llvm.exp.f64")?;
        let log_r = call_intrinsic_1(bb_log, "llvm.log.f64")?;
        let sin_r = call_intrinsic_1(bb_sin, "llvm.sin.f64")?;
        let cos_r = call_intrinsic_1(bb_cos, "llvm.cos.f64")?;
        let floor_r = call_intrinsic_1(bb_floor, "llvm.floor.f64")?;
        let ceil_r = call_intrinsic_1(bb_ceil, "llvm.ceil.f64")?;
        let round_r = call_intrinsic_1(bb_round, "llvm.round.f64")?;

        // TANH: call our wrapper
        builder.position_at_end(bb_tanh);
        let tanh_fn = self.get_or_declare(module, "verum_tanh_approx",
            f64_type.fn_type(&[f64_type.into()], false));
        let tanh_r = builder.build_call(tanh_fn, &[x.into()], "r").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_float_value();
        builder.build_unconditional_branch(bb_store).or_llvm_err()?;

        // SIGMOID: call our wrapper
        builder.position_at_end(bb_sigmoid);
        let sigmoid_fn = self.get_or_declare(module, "verum_sigmoid_approx",
            f64_type.fn_type(&[f64_type.into()], false));
        let sigmoid_r = builder.build_call(sigmoid_fn, &[x.into()], "r").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_float_value();
        builder.build_unconditional_branch(bb_store).or_llvm_err()?;

        // RELU: max(x, 0)
        builder.position_at_end(bb_relu);
        let zero = f64_type.const_float(0.0);
        let is_pos = builder.build_float_compare(FloatPredicate::OGT, x, zero, "is_pos").or_llvm_err()?;
        let relu_r = builder.build_select(is_pos, x, zero, "relu").or_llvm_err()?.into_float_value();
        builder.build_unconditional_branch(bb_store).or_llvm_err()?;

        // DEFAULT: identity
        builder.position_at_end(bb_default);
        builder.build_unconditional_branch(bb_store).or_llvm_err()?;

        // Store result
        builder.position_at_end(bb_store);
        let phi_result = builder.build_phi(f64_type, "result_val").or_llvm_err()?;
        phi_result.add_incoming(&[
            (&neg_r, bb_neg), (&abs_r, bb_abs), (&sqrt_r, bb_sqrt),
            (&exp_r, bb_exp), (&log_r, bb_log), (&sin_r, bb_sin),
            (&cos_r, bb_cos), (&tanh_r, bb_tanh), (&sigmoid_r, bb_sigmoid),
            (&relu_r, bb_relu), (&floor_r, bb_floor), (&ceil_r, bb_ceil),
            (&round_r, bb_round), (&x, bb_default),
        ]);
        let out_elem = unsafe {
            builder.build_in_bounds_gep(f64_type, result_data, &[i_val], "out_elem").or_llvm_err()?
        };
        builder.build_store(out_elem, phi_result.as_basic_value()).or_llvm_err()?;
        let next = builder.build_int_add(i_val, i64_type.const_int(1, false), "next").or_llvm_err()?;
        phi_i.add_incoming(&[(&next, bb_store)]);
        builder.build_unconditional_branch(bb_loop).or_llvm_err()?;

        builder.position_at_end(bb_done);
        builder.build_return(Some(&result_handle)).or_llvm_err()?;
        Ok(())
    }

    /// verum_tensor_binop(a: i64, b: i64, op: i64) -> i64
    /// Element-wise binary op (fast path: same shape only).
    fn emit_tensor_binop(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let f64_type = ctx.f64_type();
        let fn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into(), i64_type.into()], false);

        let func = match self.get_or_declare_new(module, "verum_tensor_binop", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let bb_null = ctx.append_basic_block(func, "null");
        let bb_ok = ctx.append_basic_block(func, "ok");
        let bb_loop = ctx.append_basic_block(func, "loop");
        let bb_body = ctx.append_basic_block(func, "body");
        let bb_done = ctx.append_basic_block(func, "done");
        // Op dispatch blocks
        let bb_add = ctx.append_basic_block(func, "add");
        let bb_sub = ctx.append_basic_block(func, "sub");
        let bb_mul = ctx.append_basic_block(func, "mul");
        let bb_div = ctx.append_basic_block(func, "div");
        let bb_pow = ctx.append_basic_block(func, "pow");
        let bb_min = ctx.append_basic_block(func, "min");
        let bb_max = ctx.append_basic_block(func, "max");
        let bb_default = ctx.append_basic_block(func, "default_op");
        let bb_store = ctx.append_basic_block(func, "store");

        builder.position_at_end(entry);
        let a_handle = self.param_as_i64(&builder, func.get_nth_param(0).or_internal("missing param")?, "p0_i64")?;
        let b_handle = self.param_as_i64(&builder, func.get_nth_param(1).or_internal("missing param")?, "p1_i64")?;
        let op = self.param_as_i64(&builder, func.get_nth_param(2).or_internal("missing param")?, "p2_i64")?;
        let a_null = builder.build_int_compare(IntPredicate::EQ, a_handle, i64_type.const_zero(), "a_null").or_llvm_err()?;
        let b_null = builder.build_int_compare(IntPredicate::EQ, b_handle, i64_type.const_zero(), "b_null").or_llvm_err()?;
        let either_null = builder.build_or(a_null, b_null, "either_null").or_llvm_err()?;
        builder.build_conditional_branch(either_null, bb_null, bb_ok).or_llvm_err()?;

        builder.position_at_end(bb_null);
        builder.build_return(Some(&i64_type.const_zero())).or_llvm_err()?;

        // Allocate result with a's shape (assumes same shape for fast path)
        builder.position_at_end(bb_ok);
        let a_ptr = self.handle_to_ptr(&builder, a_handle, "a_ptr")?;
        let b_ptr = self.handle_to_ptr(&builder, b_handle, "b_ptr")?;
        let a_numel = self.load_header_i64(&builder, a_ptr, OFF_NUMEL, "a_numel")?;
        let a_data = self.load_header_ptr(&builder, a_ptr, OFF_DATA, "a_data")?;
        let b_data = self.load_header_ptr(&builder, b_ptr, OFF_DATA, "b_data")?;
        let ndim = self.load_header_i8(&builder, a_ptr, OFF_NDIM, "ndim")?;
        let ndim_i64 = builder.build_int_z_extend(ndim, i64_type, "ndim64").or_llvm_err()?;
        let dtype = self.load_header_i8(&builder, a_ptr, OFF_DTYPE, "dtype")?;
        let dtype_i64 = builder.build_int_z_extend(dtype, i64_type, "dtype64").or_llvm_err()?;
        let shape_off = i64_type.const_int(OFF_SHAPE, false);
        let shape_ptr = unsafe {
            builder.build_in_bounds_gep(ctx.i8_type(), a_ptr, &[shape_off], "shape_ptr").or_llvm_err()?
        };
        let shape_i64 = self.ptr_to_handle(&builder, shape_ptr, "shape_i64")?;

        let new_fn = self.get_or_declare(module, "verum_tensor_new",
            i64_type.fn_type(&[i64_type.into(), i64_type.into(), i64_type.into()], false));
        let result_handle = builder.build_call(new_fn, &[ndim_i64.into(), shape_i64.into(), dtype_i64.into()], "result").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_int_value();
        let result_ptr = self.handle_to_ptr(&builder, result_handle, "res_ptr")?;
        let result_data = self.load_header_ptr(&builder, result_ptr, OFF_DATA, "res_data")?;
        builder.build_unconditional_branch(bb_loop).or_llvm_err()?;

        // Loop
        builder.position_at_end(bb_loop);
        let phi_i = builder.build_phi(i64_type, "i").or_llvm_err()?;
        phi_i.add_incoming(&[(&i64_type.const_zero(), bb_ok)]);
        let i_val = phi_i.as_basic_value().into_int_value();
        let cmp = builder.build_int_compare(IntPredicate::ULT, i_val, a_numel, "cmp").or_llvm_err()?;
        builder.build_conditional_branch(cmp, bb_body, bb_done).or_llvm_err()?;

        builder.position_at_end(bb_body);
        let a_elem = unsafe { builder.build_in_bounds_gep(f64_type, a_data, &[i_val], "ae").or_llvm_err()? };
        let b_elem = unsafe { builder.build_in_bounds_gep(f64_type, b_data, &[i_val], "be").or_llvm_err()? };
        let av = builder.build_load(f64_type, a_elem, "av").or_llvm_err()?.into_float_value();
        let bv = builder.build_load(f64_type, b_elem, "bv").or_llvm_err()?.into_float_value();

        builder.build_switch(op, bb_default, &[
            (i64_type.const_int(TENSOR_BINOP_ADD, false), bb_add),
            (i64_type.const_int(TENSOR_BINOP_SUB, false), bb_sub),
            (i64_type.const_int(TENSOR_BINOP_MUL, false), bb_mul),
            (i64_type.const_int(TENSOR_BINOP_DIV, false), bb_div),
            (i64_type.const_int(TENSOR_BINOP_POW, false), bb_pow),
            (i64_type.const_int(TENSOR_BINOP_MIN, false), bb_min),
            (i64_type.const_int(TENSOR_BINOP_MAX, false), bb_max),
        ]).or_llvm_err()?;

        // ADD
        builder.position_at_end(bb_add);
        let add_r = builder.build_float_add(av, bv, "add").or_llvm_err()?;
        builder.build_unconditional_branch(bb_store).or_llvm_err()?;

        // SUB
        builder.position_at_end(bb_sub);
        let sub_r = builder.build_float_sub(av, bv, "sub").or_llvm_err()?;
        builder.build_unconditional_branch(bb_store).or_llvm_err()?;

        // MUL
        builder.position_at_end(bb_mul);
        let mul_r = builder.build_float_mul(av, bv, "mul").or_llvm_err()?;
        builder.build_unconditional_branch(bb_store).or_llvm_err()?;

        // DIV
        builder.position_at_end(bb_div);
        let div_r = builder.build_float_div(av, bv, "div").or_llvm_err()?;
        builder.build_unconditional_branch(bb_store).or_llvm_err()?;

        // POW
        builder.position_at_end(bb_pow);
        let pow_fn = module.get_function("llvm.pow.f64").or_missing_fn("llvm.pow.f64")?;
        let pow_r = builder.build_call(pow_fn, &[av.into(), bv.into()], "pow").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_float_value();
        builder.build_unconditional_branch(bb_store).or_llvm_err()?;

        // MIN
        builder.position_at_end(bb_min);
        let minnum_fn = module.get_function("llvm.minnum.f64").or_missing_fn("llvm.minnum.f64")?;
        let min_r = builder.build_call(minnum_fn, &[av.into(), bv.into()], "min").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_float_value();
        builder.build_unconditional_branch(bb_store).or_llvm_err()?;

        // MAX
        builder.position_at_end(bb_max);
        let maxnum_fn = module.get_function("llvm.maxnum.f64").or_missing_fn("llvm.maxnum.f64")?;
        let max_r = builder.build_call(maxnum_fn, &[av.into(), bv.into()], "max").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_float_value();
        builder.build_unconditional_branch(bb_store).or_llvm_err()?;

        // DEFAULT
        builder.position_at_end(bb_default);
        let zero = f64_type.const_float(0.0);
        builder.build_unconditional_branch(bb_store).or_llvm_err()?;

        // Store
        builder.position_at_end(bb_store);
        let phi_r = builder.build_phi(f64_type, "result_val").or_llvm_err()?;
        phi_r.add_incoming(&[
            (&add_r, bb_add), (&sub_r, bb_sub), (&mul_r, bb_mul),
            (&div_r, bb_div), (&pow_r, bb_pow), (&min_r, bb_min),
            (&max_r, bb_max), (&zero, bb_default),
        ]);
        let out_elem = unsafe {
            builder.build_in_bounds_gep(f64_type, result_data, &[i_val], "out").or_llvm_err()?
        };
        builder.build_store(out_elem, phi_r.as_basic_value()).or_llvm_err()?;
        let next = builder.build_int_add(i_val, i64_type.const_int(1, false), "next").or_llvm_err()?;
        phi_i.add_incoming(&[(&next, bb_store)]);
        builder.build_unconditional_branch(bb_loop).or_llvm_err()?;

        builder.position_at_end(bb_done);
        builder.build_return(Some(&result_handle)).or_llvm_err()?;
        Ok(())
    }

    /// verum_tensor_reduce_all(handle: i64, op: i64) -> f64
    fn emit_tensor_reduce_all(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let f64_type = ctx.f64_type();
        let fn_type = f64_type.fn_type(&[i64_type.into(), i64_type.into()], false);

        let func = match self.get_or_declare_new(module, "verum_tensor_reduce_all", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let bb_null = ctx.append_basic_block(func, "null");
        let bb_init = ctx.append_basic_block(func, "init");
        let bb_loop = ctx.append_basic_block(func, "loop");
        let bb_body = ctx.append_basic_block(func, "body");
        let bb_done = ctx.append_basic_block(func, "done");

        builder.position_at_end(entry);
        let handle = self.param_as_i64(&builder, func.get_nth_param(0).or_internal("missing param")?, "p0_i64")?;
        let op = self.param_as_i64(&builder, func.get_nth_param(1).or_internal("missing param")?, "p1_i64")?;
        let is_null = builder.build_int_compare(IntPredicate::EQ, handle, i64_type.const_zero(), "is_null").or_llvm_err()?;
        builder.build_conditional_branch(is_null, bb_null, bb_init).or_llvm_err()?;

        builder.position_at_end(bb_null);
        builder.build_return(Some(&f64_type.const_float(0.0))).or_llvm_err()?;

        builder.position_at_end(bb_init);
        let ptr = self.handle_to_ptr(&builder, handle, "hdr")?;
        let numel = self.load_header_i64(&builder, ptr, OFF_NUMEL, "numel")?;
        let data_ptr = self.load_header_ptr(&builder, ptr, OFF_DATA, "data_ptr")?;

        // Initialize accumulator based on op
        let is_sum = builder.build_int_compare(IntPredicate::EQ, op, i64_type.const_int(TENSOR_REDUCE_SUM, false), "is_sum").or_llvm_err()?;
        let is_mean = builder.build_int_compare(IntPredicate::EQ, op, i64_type.const_int(TENSOR_REDUCE_MEAN, false), "is_mean").or_llvm_err()?;
        let is_prod = builder.build_int_compare(IntPredicate::EQ, op, i64_type.const_int(TENSOR_REDUCE_PROD, false), "is_prod").or_llvm_err()?;
        let is_sum_or_mean = builder.build_or(is_sum, is_mean, "sum_or_mean").or_llvm_err()?;
        // For sum/mean: init=0, for prod: init=1, for max/min: init=data[0]
        let zero = f64_type.const_float(0.0);
        let one = f64_type.const_float(1.0);
        let first_elem_ptr = unsafe {
            builder.build_in_bounds_gep(f64_type, data_ptr, &[i64_type.const_zero()], "first").or_llvm_err()?
        };
        let first = builder.build_load(f64_type, first_elem_ptr, "first_val").or_llvm_err()?.into_float_value();
        let prod_or_first = builder.build_select(is_prod, one, first, "prod_or_first").or_llvm_err()?.into_float_value();
        let init_val = builder.build_select(is_sum_or_mean, zero, prod_or_first, "init").or_llvm_err()?.into_float_value();
        builder.build_unconditional_branch(bb_loop).or_llvm_err()?;

        // Loop
        builder.position_at_end(bb_loop);
        let phi_i = builder.build_phi(i64_type, "i").or_llvm_err()?;
        phi_i.add_incoming(&[(&i64_type.const_zero(), bb_init)]);
        let phi_acc = builder.build_phi(f64_type, "acc").or_llvm_err()?;
        phi_acc.add_incoming(&[(&init_val, bb_init)]);
        let i_val = phi_i.as_basic_value().into_int_value();
        let acc = phi_acc.as_basic_value().into_float_value();
        let cmp = builder.build_int_compare(IntPredicate::ULT, i_val, numel, "cmp").or_llvm_err()?;
        builder.build_conditional_branch(cmp, bb_body, bb_done).or_llvm_err()?;

        builder.position_at_end(bb_body);
        let elem = unsafe { builder.build_in_bounds_gep(f64_type, data_ptr, &[i_val], "elem").or_llvm_err()? };
        let val = builder.build_load(f64_type, elem, "val").or_llvm_err()?.into_float_value();

        // Dispatch based on op using select chains (cheaper than switch for 5 ops)
        let sum_acc = builder.build_float_add(acc, val, "sum_acc").or_llvm_err()?;
        let prod_acc = builder.build_float_mul(acc, val, "prod_acc").or_llvm_err()?;
        let is_gt = builder.build_float_compare(FloatPredicate::OGT, val, acc, "is_gt").or_llvm_err()?;
        let max_acc = builder.build_select(is_gt, val, acc, "max_acc").or_llvm_err()?.into_float_value();
        let is_lt = builder.build_float_compare(FloatPredicate::OLT, val, acc, "is_lt").or_llvm_err()?;
        let min_acc = builder.build_select(is_lt, val, acc, "min_acc").or_llvm_err()?.into_float_value();

        let is_max = builder.build_int_compare(IntPredicate::EQ, op, i64_type.const_int(TENSOR_REDUCE_MAX, false), "is_max").or_llvm_err()?;
        let is_min = builder.build_int_compare(IntPredicate::EQ, op, i64_type.const_int(TENSOR_REDUCE_MIN, false), "is_min").or_llvm_err()?;

        let mx_mn = builder.build_select(is_max, max_acc, min_acc, "mx_mn").or_llvm_err()?.into_float_value();
        let p_mx = builder.build_select(is_prod, prod_acc, mx_mn, "p_mx").or_llvm_err()?.into_float_value();
        let new_acc = builder.build_select(is_sum_or_mean, sum_acc, p_mx, "new_acc").or_llvm_err()?.into_float_value();

        let next = builder.build_int_add(i_val, i64_type.const_int(1, false), "next").or_llvm_err()?;
        phi_i.add_incoming(&[(&next, bb_body)]);
        phi_acc.add_incoming(&[(&new_acc, bb_body)]);
        builder.build_unconditional_branch(bb_loop).or_llvm_err()?;

        // Done: if mean, divide by numel
        builder.position_at_end(bb_done);
        let numel_f = builder.build_signed_int_to_float(numel, f64_type, "numel_f").or_llvm_err()?;
        let mean_result = builder.build_float_div(acc, numel_f, "mean").or_llvm_err()?;
        let final_result = builder.build_select(is_mean, mean_result, acc, "final").or_llvm_err()?;
        builder.build_return(Some(&final_result)).or_llvm_err()?;
        Ok(())
    }

    /// verum_tensor_reduce(handle: i64, op: i64, axis: i64) -> i64
    /// Axis-specific reduction. For 1D tensors, reduces to a scalar tensor.
    /// Calls reduce_all internally and wraps result in a 1-element tensor.
    fn emit_tensor_reduce(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let f64_type = ctx.f64_type();
        let i8_type = ctx.i8_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());
        let fn_type = i64_type.fn_type(
            &[i64_type.into(), i64_type.into(), i64_type.into()], false);

        let func = match self.get_or_declare_new(module, "verum_tensor_reduce", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let bb_null = ctx.append_basic_block(func, "null");
        let bb_ok = ctx.append_basic_block(func, "ok");
        let bb_ret = ctx.append_basic_block(func, "ret");

        builder.position_at_end(entry);
        let handle = self.param_as_i64(&builder, func.get_nth_param(0).or_internal("missing param")?, "p0_i64")?;
        let op = self.param_as_i64(&builder, func.get_nth_param(1).or_internal("missing param")?, "p1_i64")?;
        let _axis = self.param_as_i64(&builder, func.get_nth_param(2).or_internal("missing param")?, "p2_i64")?;

        let is_null = builder.build_int_compare(IntPredicate::EQ, handle, i64_type.const_zero(), "is_null").or_llvm_err()?;
        builder.build_conditional_branch(is_null, bb_null, bb_ok).or_llvm_err()?;

        builder.position_at_end(bb_null);
        builder.build_return(Some(&i64_type.const_zero())).or_llvm_err()?;

        builder.position_at_end(bb_ok);
        // Call reduce_all to get the scalar result
        let reduce_all_fn = self.get_or_declare(module, "verum_tensor_reduce_all",
            f64_type.fn_type(&[i64_type.into(), i64_type.into()], false));
        let scalar = builder.build_call(reduce_all_fn, &[handle.into(), op.into()], "scalar").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_float_value();

        // Create a 1-element result tensor via tensor_fill
        let fill_fn = self.get_or_declare(module, "verum_tensor_fill",
            i64_type.fn_type(&[i64_type.into(), i64_type.into(), f64_type.into(), i64_type.into()], false));
        let ndim_1 = i64_type.const_int(1, false);
        // Build shape [1] on stack
        let shape_alloca = builder.build_alloca(i64_type, "shape").or_llvm_err()?;
        builder.build_store(shape_alloca, ndim_1).or_llvm_err()?;
        let shape_i64 = builder.build_ptr_to_int(shape_alloca, i64_type, "shape_i64").or_llvm_err()?;
        // Read dtype from source tensor
        let src_ptr = self.handle_to_ptr(&builder, handle, "src_hdr")?;
        let dtype = self.load_header_i8(&builder, src_ptr, OFF_DTYPE, "dtype")?;
        let dtype_i64 = builder.build_int_z_extend(dtype, i64_type, "dtype_i64").or_llvm_err()?;
        let result = builder.build_call(fill_fn,
            &[ndim_1.into(), shape_i64.into(), scalar.into(), dtype_i64.into()], "result_tensor").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_int_value();
        builder.build_unconditional_branch(bb_ret).or_llvm_err()?;

        builder.position_at_end(bb_ret);
        builder.build_return(Some(&result)).or_llvm_err()?;
        Ok(())
    }

    /// verum_tensor_matmul(a: i64, b: i64) -> i64
    /// Matrix multiply (2D, triple loop — LLVM will autovectorize)
    fn emit_tensor_matmul(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let f64_type = ctx.f64_type();
        let fn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);

        let func = match self.get_or_declare_new(module, "verum_tensor_matmul", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let bb_null = ctx.append_basic_block(func, "null");
        let bb_ok = ctx.append_basic_block(func, "ok");
        let bb_i_loop = ctx.append_basic_block(func, "i_loop");
        let bb_j_loop = ctx.append_basic_block(func, "j_loop");
        let bb_k_loop = ctx.append_basic_block(func, "k_loop");
        let bb_k_body = ctx.append_basic_block(func, "k_body");
        let bb_k_done = ctx.append_basic_block(func, "k_done");
        let bb_j_done = ctx.append_basic_block(func, "j_done");
        let bb_i_done = ctx.append_basic_block(func, "i_done");

        builder.position_at_end(entry);
        let a_h = self.param_as_i64(&builder, func.get_nth_param(0).or_internal("missing param")?, "p0_i64")?;
        let b_h = self.param_as_i64(&builder, func.get_nth_param(1).or_internal("missing param")?, "p1_i64")?;
        let a_null = builder.build_int_compare(IntPredicate::EQ, a_h, i64_type.const_zero(), "a_null").or_llvm_err()?;
        let b_null = builder.build_int_compare(IntPredicate::EQ, b_h, i64_type.const_zero(), "b_null").or_llvm_err()?;
        let either = builder.build_or(a_null, b_null, "either").or_llvm_err()?;
        builder.build_conditional_branch(either, bb_null, bb_ok).or_llvm_err()?;

        builder.position_at_end(bb_null);
        builder.build_return(Some(&i64_type.const_zero())).or_llvm_err()?;

        builder.position_at_end(bb_ok);
        let a_ptr = self.handle_to_ptr(&builder, a_h, "a_ptr")?;
        let b_ptr = self.handle_to_ptr(&builder, b_h, "b_ptr")?;
        let a_data = self.load_header_ptr(&builder, a_ptr, OFF_DATA, "a_data")?;
        let b_data = self.load_header_ptr(&builder, b_ptr, OFF_DATA, "b_data")?;

        // shape: a=[m,k], b=[k,n], result=[m,n]
        let i8_type = ctx.i8_type();
        let m = {
            let off = i64_type.const_int(OFF_SHAPE, false);
            let p = unsafe { builder.build_in_bounds_gep(i8_type, a_ptr, &[off], "m_ptr").or_llvm_err()? };
            builder.build_load(i64_type, p, "m").or_llvm_err()?.into_int_value()
        };
        let k_dim = {
            let off = i64_type.const_int(OFF_SHAPE + 8, false);
            let p = unsafe { builder.build_in_bounds_gep(i8_type, a_ptr, &[off], "k_ptr").or_llvm_err()? };
            builder.build_load(i64_type, p, "k").or_llvm_err()?.into_int_value()
        };
        let n = {
            let off = i64_type.const_int(OFF_SHAPE + 8, false);
            let p = unsafe { builder.build_in_bounds_gep(i8_type, b_ptr, &[off], "n_ptr").or_llvm_err()? };
            builder.build_load(i64_type, p, "n").or_llvm_err()?.into_int_value()
        };

        // Create output shape on stack [m, n]
        let out_shape = builder.build_array_alloca(i64_type, i64_type.const_int(2, false), "out_shape").or_llvm_err()?;
        let s0 = unsafe { builder.build_in_bounds_gep(i64_type, out_shape, &[i64_type.const_zero()], "s0").or_llvm_err()? };
        builder.build_store(s0, m).or_llvm_err()?;
        let s1 = unsafe { builder.build_in_bounds_gep(i64_type, out_shape, &[i64_type.const_int(1, false)], "s1").or_llvm_err()? };
        builder.build_store(s1, n).or_llvm_err()?;

        let shape_as_i64 = self.ptr_to_handle(&builder, out_shape, "shape_i64")?;
        let dtype = self.load_header_i8(&builder, a_ptr, OFF_DTYPE, "dtype")?;
        let dtype_i64 = builder.build_int_z_extend(dtype, i64_type, "dtype64").or_llvm_err()?;

        let new_fn = self.get_or_declare(module, "verum_tensor_new",
            i64_type.fn_type(&[i64_type.into(), i64_type.into(), i64_type.into()], false));
        let result_handle = builder.build_call(new_fn,
            &[i64_type.const_int(2, false).into(), shape_as_i64.into(), dtype_i64.into()], "result").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_int_value();
        let result_ptr = self.handle_to_ptr(&builder, result_handle, "res_ptr")?;
        let result_data = self.load_header_ptr(&builder, result_ptr, OFF_DATA, "res_data")?;
        builder.build_unconditional_branch(bb_i_loop).or_llvm_err()?;

        // Triple nested loop: i over m, j over n, k over k_dim
        builder.position_at_end(bb_i_loop);
        let phi_i = builder.build_phi(i64_type, "i").or_llvm_err()?;
        phi_i.add_incoming(&[(&i64_type.const_zero(), bb_ok)]);
        let i_val = phi_i.as_basic_value().into_int_value();
        let i_cmp = builder.build_int_compare(IntPredicate::ULT, i_val, m, "i_cmp").or_llvm_err()?;
        builder.build_conditional_branch(i_cmp, bb_j_loop, bb_i_done).or_llvm_err()?;

        builder.position_at_end(bb_j_loop);
        let phi_j = builder.build_phi(i64_type, "j").or_llvm_err()?;
        phi_j.add_incoming(&[(&i64_type.const_zero(), bb_i_loop)]);
        let j_val = phi_j.as_basic_value().into_int_value();
        let j_cmp = builder.build_int_compare(IntPredicate::ULT, j_val, n, "j_cmp").or_llvm_err()?;
        builder.build_conditional_branch(j_cmp, bb_k_loop, bb_j_done).or_llvm_err()?;

        builder.position_at_end(bb_k_loop);
        let phi_k = builder.build_phi(i64_type, "k").or_llvm_err()?;
        phi_k.add_incoming(&[(&i64_type.const_zero(), bb_j_loop)]);
        let phi_sum = builder.build_phi(f64_type, "sum").or_llvm_err()?;
        phi_sum.add_incoming(&[(&f64_type.const_float(0.0), bb_j_loop)]);
        let k_val = phi_k.as_basic_value().into_int_value();
        let sum_val = phi_sum.as_basic_value().into_float_value();
        let k_cmp = builder.build_int_compare(IntPredicate::ULT, k_val, k_dim, "k_cmp").or_llvm_err()?;
        builder.build_conditional_branch(k_cmp, bb_k_body, bb_k_done).or_llvm_err()?;

        builder.position_at_end(bb_k_body);
        // a[i*k + k_idx]
        let a_idx = builder.build_int_add(
            builder.build_int_mul(i_val, k_dim, "ik").or_llvm_err()?, k_val, "a_idx").or_llvm_err()?;
        let a_elem = unsafe { builder.build_in_bounds_gep(f64_type, a_data, &[a_idx], "a_e").or_llvm_err()? };
        let av = builder.build_load(f64_type, a_elem, "av").or_llvm_err()?.into_float_value();
        // b[k_idx*n + j]
        let b_idx = builder.build_int_add(
            builder.build_int_mul(k_val, n, "kn").or_llvm_err()?, j_val, "b_idx").or_llvm_err()?;
        let b_elem = unsafe { builder.build_in_bounds_gep(f64_type, b_data, &[b_idx], "b_e").or_llvm_err()? };
        let bv = builder.build_load(f64_type, b_elem, "bv").or_llvm_err()?.into_float_value();
        let prod = builder.build_float_mul(av, bv, "prod").or_llvm_err()?;
        let new_sum = builder.build_float_add(sum_val, prod, "new_sum").or_llvm_err()?;
        let next_k = builder.build_int_add(k_val, i64_type.const_int(1, false), "next_k").or_llvm_err()?;
        phi_k.add_incoming(&[(&next_k, bb_k_body)]);
        phi_sum.add_incoming(&[(&new_sum, bb_k_body)]);
        builder.build_unconditional_branch(bb_k_loop).or_llvm_err()?;

        // Store result[i*n + j] = sum
        builder.position_at_end(bb_k_done);
        let r_idx = builder.build_int_add(
            builder.build_int_mul(i_val, n, "in_r").or_llvm_err()?, j_val, "r_idx").or_llvm_err()?;
        let r_elem = unsafe { builder.build_in_bounds_gep(f64_type, result_data, &[r_idx], "r_e").or_llvm_err()? };
        builder.build_store(r_elem, sum_val).or_llvm_err()?;
        let next_j = builder.build_int_add(j_val, i64_type.const_int(1, false), "next_j").or_llvm_err()?;
        phi_j.add_incoming(&[(&next_j, bb_k_done)]);
        builder.build_unconditional_branch(bb_j_loop).or_llvm_err()?;

        builder.position_at_end(bb_j_done);
        let next_i = builder.build_int_add(i_val, i64_type.const_int(1, false), "next_i").or_llvm_err()?;
        phi_i.add_incoming(&[(&next_i, bb_j_done)]);
        builder.build_unconditional_branch(bb_i_loop).or_llvm_err()?;

        builder.position_at_end(bb_i_done);
        builder.build_return(Some(&result_handle)).or_llvm_err()?;
        Ok(())
    }

    /// verum_tensor_reshape(handle: i64, new_ndim: i64, new_shape_ptr: i64) -> i64
    fn emit_tensor_reshape(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let fn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into(), i64_type.into()], false);

        let func = match self.get_or_declare_new(module, "verum_tensor_reshape", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");

        builder.position_at_end(entry);
        let handle = self.param_as_i64(&builder, func.get_nth_param(0).or_internal("missing param")?, "p0_i64")?;
        let new_ndim = self.param_as_i64(&builder, func.get_nth_param(1).or_internal("missing param")?, "p1_i64")?;
        let new_shape = self.param_as_i64(&builder, func.get_nth_param(2).or_internal("missing param")?, "p2_i64")?;

        // Get src data pointer
        let src_ptr = self.handle_to_ptr(&builder, handle, "src")?;
        let data_ptr = self.load_header_ptr(&builder, src_ptr, OFF_DATA, "data")?;
        let data_i64 = self.ptr_to_handle(&builder, data_ptr, "data_i64")?;
        let dtype = self.load_header_i8(&builder, src_ptr, OFF_DTYPE, "dtype")?;
        let dtype_i64 = builder.build_int_z_extend(dtype, i64_type, "dtype64").or_llvm_err()?;

        let from_data_fn = self.get_or_declare(module, "verum_tensor_from_data",
            i64_type.fn_type(&[i64_type.into(), i64_type.into(), i64_type.into(), i64_type.into()], false));
        let result = builder.build_call(from_data_fn,
            &[new_ndim.into(), new_shape.into(), data_i64.into(), dtype_i64.into()], "reshape").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?;
        builder.build_return(Some(&result)).or_llvm_err()?;
        Ok(())
    }

    /// verum_tensor_transpose(handle: i64) -> i64
    fn emit_tensor_transpose(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let i8_type = ctx.i8_type();
        let f64_type = ctx.f64_type();
        let fn_type = i64_type.fn_type(&[i64_type.into()], false);

        let func = match self.get_or_declare_new(module, "verum_tensor_transpose", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let bb_null = ctx.append_basic_block(func, "null");
        let bb_ok = ctx.append_basic_block(func, "ok");
        let bb_i_loop = ctx.append_basic_block(func, "i_loop");
        let bb_j_loop = ctx.append_basic_block(func, "j_loop");
        let bb_j_body = ctx.append_basic_block(func, "j_body");
        let bb_j_done = ctx.append_basic_block(func, "j_done");
        let bb_i_done = ctx.append_basic_block(func, "i_done");

        builder.position_at_end(entry);
        let handle = func.get_first_param().or_internal("missing first param")?.into_int_value();
        let is_null = builder.build_int_compare(IntPredicate::EQ, handle, i64_type.const_zero(), "is_null").or_llvm_err()?;
        builder.build_conditional_branch(is_null, bb_null, bb_ok).or_llvm_err()?;

        builder.position_at_end(bb_null);
        builder.build_return(Some(&i64_type.const_zero())).or_llvm_err()?;

        builder.position_at_end(bb_ok);
        let src_ptr = self.handle_to_ptr(&builder, handle, "src")?;
        let src_data = self.load_header_ptr(&builder, src_ptr, OFF_DATA, "src_data")?;
        let ndim = self.load_header_i8(&builder, src_ptr, OFF_NDIM, "ndim")?;
        let ndim_i64 = builder.build_int_z_extend(ndim, i64_type, "ndim64").or_llvm_err()?;

        // For 1D tensors, transpose is identity — clone the tensor
        let bb_1d = ctx.append_basic_block(func, "is_1d");
        let bb_2d = ctx.append_basic_block(func, "is_2d");
        let is_1d = builder.build_int_compare(IntPredicate::ULT, ndim_i64, i64_type.const_int(2, false), "is_1d").or_llvm_err()?;
        builder.build_conditional_branch(is_1d, bb_1d, bb_2d).or_llvm_err()?;

        builder.position_at_end(bb_1d);
        // Clone: create new tensor with same shape, copy data
        let clone_fn = self.get_or_declare(module, "verum_tensor_clone",
            i64_type.fn_type(&[i64_type.into()], false));
        let cloned = builder.build_call(clone_fn, &[handle.into()], "cloned").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_int_value();
        builder.build_return(Some(&cloned)).or_llvm_err()?;

        // 2D+ transpose
        builder.position_at_end(bb_2d);
        // Load rows = shape[0], cols = shape[1]
        let rows = {
            let off = i64_type.const_int(OFF_SHAPE, false);
            let p = unsafe { builder.build_in_bounds_gep(i8_type, src_ptr, &[off], "r_ptr").or_llvm_err()? };
            builder.build_load(i64_type, p, "rows").or_llvm_err()?.into_int_value()
        };
        let cols = {
            let off = i64_type.const_int(OFF_SHAPE + 8, false);
            let p = unsafe { builder.build_in_bounds_gep(i8_type, src_ptr, &[off], "c_ptr").or_llvm_err()? };
            builder.build_load(i64_type, p, "cols").or_llvm_err()?.into_int_value()
        };

        // Allocate result [cols, rows]
        let out_shape = builder.build_array_alloca(i64_type, i64_type.const_int(2, false), "out_shape").or_llvm_err()?;
        let s0 = unsafe { builder.build_in_bounds_gep(i64_type, out_shape, &[i64_type.const_zero()], "s0").or_llvm_err()? };
        builder.build_store(s0, cols).or_llvm_err()?;
        let s1 = unsafe { builder.build_in_bounds_gep(i64_type, out_shape, &[i64_type.const_int(1, false)], "s1").or_llvm_err()? };
        builder.build_store(s1, rows).or_llvm_err()?;
        let shape_i64 = self.ptr_to_handle(&builder, out_shape, "shape_i64")?;
        let dtype = self.load_header_i8(&builder, src_ptr, OFF_DTYPE, "dtype")?;
        let dtype_i64 = builder.build_int_z_extend(dtype, i64_type, "dtype64").or_llvm_err()?;

        let new_fn = self.get_or_declare(module, "verum_tensor_new",
            i64_type.fn_type(&[i64_type.into(), i64_type.into(), i64_type.into()], false));
        let result_handle = builder.build_call(new_fn,
            &[i64_type.const_int(2, false).into(), shape_i64.into(), dtype_i64.into()], "result").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_int_value();
        let result_ptr = self.handle_to_ptr(&builder, result_handle, "res_ptr")?;
        let result_data = self.load_header_ptr(&builder, result_ptr, OFF_DATA, "res_data")?;
        builder.build_unconditional_branch(bb_i_loop).or_llvm_err()?;

        // dst[j*rows + i] = src[i*cols + j]
        builder.position_at_end(bb_i_loop);
        let phi_i = builder.build_phi(i64_type, "i").or_llvm_err()?;
        phi_i.add_incoming(&[(&i64_type.const_zero(), bb_2d)]);
        let i_val = phi_i.as_basic_value().into_int_value();
        let i_cmp = builder.build_int_compare(IntPredicate::ULT, i_val, rows, "i_cmp").or_llvm_err()?;
        builder.build_conditional_branch(i_cmp, bb_j_loop, bb_i_done).or_llvm_err()?;

        builder.position_at_end(bb_j_loop);
        let phi_j = builder.build_phi(i64_type, "j").or_llvm_err()?;
        phi_j.add_incoming(&[(&i64_type.const_zero(), bb_i_loop)]);
        let j_val = phi_j.as_basic_value().into_int_value();
        let j_cmp = builder.build_int_compare(IntPredicate::ULT, j_val, cols, "j_cmp").or_llvm_err()?;
        builder.build_conditional_branch(j_cmp, bb_j_body, bb_j_done).or_llvm_err()?;

        builder.position_at_end(bb_j_body);
        let src_idx = builder.build_int_add(
            builder.build_int_mul(i_val, cols, "ic").or_llvm_err()?, j_val, "src_idx").or_llvm_err()?;
        let src_elem = unsafe { builder.build_in_bounds_gep(f64_type, src_data, &[src_idx], "se").or_llvm_err()? };
        let val = builder.build_load(f64_type, src_elem, "val").or_llvm_err()?;
        let dst_idx = builder.build_int_add(
            builder.build_int_mul(j_val, rows, "jr").or_llvm_err()?, i_val, "dst_idx").or_llvm_err()?;
        let dst_elem = unsafe { builder.build_in_bounds_gep(f64_type, result_data, &[dst_idx], "de").or_llvm_err()? };
        builder.build_store(dst_elem, val).or_llvm_err()?;
        let next_j = builder.build_int_add(j_val, i64_type.const_int(1, false), "next_j").or_llvm_err()?;
        phi_j.add_incoming(&[(&next_j, bb_j_body)]);
        builder.build_unconditional_branch(bb_j_loop).or_llvm_err()?;

        builder.position_at_end(bb_j_done);
        let next_i = builder.build_int_add(i_val, i64_type.const_int(1, false), "next_i").or_llvm_err()?;
        phi_i.add_incoming(&[(&next_i, bb_j_done)]);
        builder.build_unconditional_branch(bb_i_loop).or_llvm_err()?;

        builder.position_at_end(bb_i_done);
        builder.build_return(Some(&result_handle)).or_llvm_err()?;
        Ok(())
    }

    /// verum_tensor_softmax(handle: i64, axis: i64) -> i64
    /// Simplified: operates element-wise over entire tensor.
    fn emit_tensor_softmax(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let f64_type = ctx.f64_type();
        let fn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);

        let func = match self.get_or_declare_new(module, "verum_tensor_softmax", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let bb_null = ctx.append_basic_block(func, "null");
        let bb_ok = ctx.append_basic_block(func, "ok");
        let bb_max_loop = ctx.append_basic_block(func, "max_loop");
        let bb_max_body = ctx.append_basic_block(func, "max_body");
        let bb_max_done = ctx.append_basic_block(func, "max_done");
        let bb_exp_loop = ctx.append_basic_block(func, "exp_loop");
        let bb_exp_body = ctx.append_basic_block(func, "exp_body");
        let bb_exp_done = ctx.append_basic_block(func, "exp_done");
        let bb_norm_loop = ctx.append_basic_block(func, "norm_loop");
        let bb_norm_body = ctx.append_basic_block(func, "norm_body");
        let bb_norm_done = ctx.append_basic_block(func, "norm_done");

        builder.position_at_end(entry);
        let handle = self.param_as_i64(&builder, func.get_nth_param(0).or_internal("missing param")?, "p0_i64")?;
        let _axis = self.param_as_i64(&builder, func.get_nth_param(1).or_internal("missing param")?, "p1_i64")?;
        let is_null = builder.build_int_compare(IntPredicate::EQ, handle, i64_type.const_zero(), "is_null").or_llvm_err()?;
        builder.build_conditional_branch(is_null, bb_null, bb_ok).or_llvm_err()?;

        builder.position_at_end(bb_null);
        builder.build_return(Some(&i64_type.const_zero())).or_llvm_err()?;

        builder.position_at_end(bb_ok);
        // Clone tensor for result
        let clone_fn = self.get_or_declare(module, "verum_tensor_clone",
            i64_type.fn_type(&[i64_type.into()], false));
        let result_handle = builder.build_call(clone_fn, &[handle.into()], "clone").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_int_value();
        let result_ptr = self.handle_to_ptr(&builder, result_handle, "res_ptr")?;
        let result_data = self.load_header_ptr(&builder, result_ptr, OFF_DATA, "res_data")?;
        let src_ptr = self.handle_to_ptr(&builder, handle, "src")?;
        let src_data = self.load_header_ptr(&builder, src_ptr, OFF_DATA, "src_data")?;
        let numel = self.load_header_i64(&builder, src_ptr, OFF_NUMEL, "numel")?;
        builder.build_unconditional_branch(bb_max_loop).or_llvm_err()?;

        // Pass 1: find max
        builder.position_at_end(bb_max_loop);
        let phi_i = builder.build_phi(i64_type, "i").or_llvm_err()?;
        phi_i.add_incoming(&[(&i64_type.const_zero(), bb_ok)]);
        let phi_max = builder.build_phi(f64_type, "max").or_llvm_err()?;
        phi_max.add_incoming(&[(&f64_type.const_float(-1.0e308), bb_ok)]);
        let i_val = phi_i.as_basic_value().into_int_value();
        let max_val = phi_max.as_basic_value().into_float_value();
        let cmp = builder.build_int_compare(IntPredicate::ULT, i_val, numel, "cmp").or_llvm_err()?;
        builder.build_conditional_branch(cmp, bb_max_body, bb_max_done).or_llvm_err()?;

        builder.position_at_end(bb_max_body);
        let elem = unsafe { builder.build_in_bounds_gep(f64_type, src_data, &[i_val], "e").or_llvm_err()? };
        let val = builder.build_load(f64_type, elem, "v").or_llvm_err()?.into_float_value();
        let is_gt = builder.build_float_compare(FloatPredicate::OGT, val, max_val, "gt").or_llvm_err()?;
        let new_max = builder.build_select(is_gt, val, max_val, "new_max").or_llvm_err()?.into_float_value();
        let next = builder.build_int_add(i_val, i64_type.const_int(1, false), "next").or_llvm_err()?;
        phi_i.add_incoming(&[(&next, bb_max_body)]);
        phi_max.add_incoming(&[(&new_max, bb_max_body)]);
        builder.build_unconditional_branch(bb_max_loop).or_llvm_err()?;

        // Pass 2: exp(x - max) and sum
        builder.position_at_end(bb_max_done);
        builder.build_unconditional_branch(bb_exp_loop).or_llvm_err()?;

        builder.position_at_end(bb_exp_loop);
        let phi_j = builder.build_phi(i64_type, "j").or_llvm_err()?;
        phi_j.add_incoming(&[(&i64_type.const_zero(), bb_max_done)]);
        let phi_sum = builder.build_phi(f64_type, "sum").or_llvm_err()?;
        phi_sum.add_incoming(&[(&f64_type.const_float(0.0), bb_max_done)]);
        let j_val = phi_j.as_basic_value().into_int_value();
        let sum_val = phi_sum.as_basic_value().into_float_value();
        let j_cmp = builder.build_int_compare(IntPredicate::ULT, j_val, numel, "j_cmp").or_llvm_err()?;
        builder.build_conditional_branch(j_cmp, bb_exp_body, bb_exp_done).or_llvm_err()?;

        builder.position_at_end(bb_exp_body);
        let src_elem = unsafe { builder.build_in_bounds_gep(f64_type, src_data, &[j_val], "se").or_llvm_err()? };
        let sv = builder.build_load(f64_type, src_elem, "sv").or_llvm_err()?.into_float_value();
        let shifted = builder.build_float_sub(sv, max_val, "shifted").or_llvm_err()?;
        let exp_fn = module.get_function("llvm.exp.f64").or_missing_fn("llvm.exp.f64")?;
        let exp_val = builder.build_call(exp_fn, &[shifted.into()], "exp").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_float_value();
        let dst_elem = unsafe { builder.build_in_bounds_gep(f64_type, result_data, &[j_val], "de").or_llvm_err()? };
        builder.build_store(dst_elem, exp_val).or_llvm_err()?;
        let new_sum = builder.build_float_add(sum_val, exp_val, "new_sum").or_llvm_err()?;
        let next_j = builder.build_int_add(j_val, i64_type.const_int(1, false), "next_j").or_llvm_err()?;
        phi_j.add_incoming(&[(&next_j, bb_exp_body)]);
        phi_sum.add_incoming(&[(&new_sum, bb_exp_body)]);
        builder.build_unconditional_branch(bb_exp_loop).or_llvm_err()?;

        // Pass 3: normalize
        builder.position_at_end(bb_exp_done);
        builder.build_unconditional_branch(bb_norm_loop).or_llvm_err()?;

        builder.position_at_end(bb_norm_loop);
        let phi_k = builder.build_phi(i64_type, "k").or_llvm_err()?;
        phi_k.add_incoming(&[(&i64_type.const_zero(), bb_exp_done)]);
        let k_val = phi_k.as_basic_value().into_int_value();
        let k_cmp = builder.build_int_compare(IntPredicate::ULT, k_val, numel, "k_cmp").or_llvm_err()?;
        builder.build_conditional_branch(k_cmp, bb_norm_body, bb_norm_done).or_llvm_err()?;

        builder.position_at_end(bb_norm_body);
        let ne = unsafe { builder.build_in_bounds_gep(f64_type, result_data, &[k_val], "ne").or_llvm_err()? };
        let nv = builder.build_load(f64_type, ne, "nv").or_llvm_err()?.into_float_value();
        let normalized = builder.build_float_div(nv, sum_val, "norm").or_llvm_err()?;
        builder.build_store(ne, normalized).or_llvm_err()?;
        let next_k = builder.build_int_add(k_val, i64_type.const_int(1, false), "next_k").or_llvm_err()?;
        phi_k.add_incoming(&[(&next_k, bb_norm_body)]);
        builder.build_unconditional_branch(bb_norm_loop).or_llvm_err()?;

        builder.position_at_end(bb_norm_done);
        builder.build_return(Some(&result_handle)).or_llvm_err()?;
        Ok(())
    }

    // ========================================================================
    // BATCH 5: Autodiff gradient tape (declarations only — complex state)
    // ========================================================================

    fn emit_grad_tape(&self, module: &Module<'ctx>) {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let i32_type = ctx.i32_type();
        let f64_type = ctx.f64_type();
        let void_type = ctx.void_type();

        // verum_grad_begin(mode: i32) -> void
        let fn_begin = void_type.fn_type(&[i32_type.into()], false);
        self.get_or_declare(module, "verum_grad_begin", fn_begin);

        // verum_grad_end() -> void
        let fn_end = void_type.fn_type(&[], false);
        self.get_or_declare(module, "verum_grad_end", fn_end);

        // verum_grad_var(value: f64) -> i32
        let fn_var = i32_type.fn_type(&[f64_type.into()], false);
        self.get_or_declare(module, "verum_grad_var", fn_var);

        // verum_grad_get(id: i32) -> f64
        let fn_get = f64_type.fn_type(&[i32_type.into()], false);
        self.get_or_declare(module, "verum_grad_get", fn_get);

        // verum_grad_set(id: i32, value: f64) -> void
        let fn_set = void_type.fn_type(&[i32_type.into(), f64_type.into()], false);
        self.get_or_declare(module, "verum_grad_set", fn_set);

        // Various grad ops: verum_grad_add/sub/mul/div etc.
        let fn_binop = i32_type.fn_type(&[i32_type.into(), i32_type.into()], false);
        for name in &[
            "verum_grad_add", "verum_grad_sub", "verum_grad_mul", "verum_grad_div", "verum_grad_pow",
        ] {
            self.get_or_declare(module, name, fn_binop);
        }

        let fn_unop = i32_type.fn_type(&[i32_type.into()], false);
        for name in &[
            "verum_grad_neg", "verum_grad_sin", "verum_grad_cos", "verum_grad_exp",
            "verum_grad_log", "verum_grad_sqrt", "verum_grad_tanh", "verum_grad_relu",
            "verum_grad_sigmoid",
        ] {
            self.get_or_declare(module, name, fn_unop);
        }
    }

    // ========================================================================
    // Bridge / utility functions
    // ========================================================================

    /// verum_tensor_fill_scalar(handle: i64, value_bits: i64) -> void
    fn emit_tensor_fill_scalar(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let f64_type = ctx.f64_type();
        let void_type = ctx.void_type();
        let fn_type = void_type.fn_type(&[i64_type.into(), i64_type.into()], false);

        let func = match self.get_or_declare_new(module, "verum_tensor_fill_scalar", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let bb_loop = ctx.append_basic_block(func, "loop");
        let bb_body = ctx.append_basic_block(func, "body");
        let bb_done = ctx.append_basic_block(func, "done");

        builder.position_at_end(entry);
        let handle = self.param_as_i64(&builder, func.get_nth_param(0).or_internal("missing param")?, "p0_i64")?;
        let value_bits = self.param_as_i64(&builder, func.get_nth_param(1).or_internal("missing param")?, "p1_i64")?;
        // Bitcast i64 to f64
        let value = builder.build_bit_cast(value_bits, f64_type, "value").or_llvm_err()?.into_float_value();
        let ptr = self.handle_to_ptr(&builder, handle, "hdr")?;
        let numel = self.load_header_i64(&builder, ptr, OFF_NUMEL, "numel")?;
        let data_ptr = self.load_header_ptr(&builder, ptr, OFF_DATA, "data_ptr")?;
        builder.build_unconditional_branch(bb_loop).or_llvm_err()?;

        builder.position_at_end(bb_loop);
        let phi_i = builder.build_phi(i64_type, "i").or_llvm_err()?;
        phi_i.add_incoming(&[(&i64_type.const_zero(), entry)]);
        let i_val = phi_i.as_basic_value().into_int_value();
        let cmp = builder.build_int_compare(IntPredicate::ULT, i_val, numel, "cmp").or_llvm_err()?;
        builder.build_conditional_branch(cmp, bb_body, bb_done).or_llvm_err()?;

        builder.position_at_end(bb_body);
        let elem = unsafe { builder.build_in_bounds_gep(f64_type, data_ptr, &[i_val], "elem").or_llvm_err()? };
        builder.build_store(elem, value).or_llvm_err()?;
        let next = builder.build_int_add(i_val, i64_type.const_int(1, false), "next").or_llvm_err()?;
        phi_i.add_incoming(&[(&next, bb_body)]);
        builder.build_unconditional_branch(bb_loop).or_llvm_err()?;

        builder.position_at_end(bb_done);
        builder.build_return(None).or_llvm_err()?;
        Ok(())
    }

    /// verum_tensor_reshape_flat(handle: i64, shape_handle: i64) -> i64
    fn emit_tensor_reshape_flat(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let fn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);

        let func = match self.get_or_declare_new(module, "verum_tensor_reshape_flat", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");

        builder.position_at_end(entry);
        let handle = self.param_as_i64(&builder, func.get_nth_param(0).or_internal("missing param")?, "p0_i64")?;
        let shape_handle = self.param_as_i64(&builder, func.get_nth_param(1).or_internal("missing param")?, "p1_i64")?;

        // shape_handle is a List<Int> (tensor of shape). Read its numel as new_ndim.
        let shape_ptr = self.handle_to_ptr(&builder, shape_handle, "shape_tensor")?;
        let new_ndim = self.load_header_i64(&builder, shape_ptr, OFF_NUMEL, "new_ndim")?;
        let shape_data = self.load_header_ptr(&builder, shape_ptr, OFF_DATA, "shape_data")?;
        let shape_data_i64 = self.ptr_to_handle(&builder, shape_data, "shape_data_i64")?;

        let reshape_fn = self.get_or_declare(module, "verum_tensor_reshape",
            i64_type.fn_type(&[i64_type.into(), i64_type.into(), i64_type.into()], false));
        let result = builder.build_call(reshape_fn,
            &[handle.into(), new_ndim.into(), shape_data_i64.into()], "result").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?;
        builder.build_return(Some(&result)).or_llvm_err()?;
        Ok(())
    }

    // ========================================================================
    // Extern stubs: declare all remaining functions
    // ========================================================================

    fn emit_extern_stubs(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let i32_type = ctx.i32_type();
        let f64_type = ctx.f64_type();
        let void_type = ctx.void_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());

        // i64(i64) signatures
        let i64_i64 = i64_type.fn_type(&[i64_type.into()], false);
        let stubs_i64_i64: &[&str] = &[
            "verum_tensor_contiguous",
            "verum_tensor_contiguous_view",
            "verum_tensor_rank",
        ];
        for name in stubs_i64_i64 {
            self.get_or_declare(module, name, i64_i64);
        }

        // i64(i64, i64) signatures
        let i64_i64_i64 = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        let stubs_i64_2: &[&str] = &[
            "verum_tensor_to_device",
            "verum_tensor_cast",
            "verum_tensor_squeeze",
            "verum_tensor_unsqueeze",
            "verum_tensor_repeat",
            "verum_tensor_flip",
            "verum_tensor_cmp",
            "verum_tensor_diag",
            "verum_tensor_triu",
            "verum_tensor_tril",
            "verum_tensor_outer",
            "verum_tensor_kron",
            "verum_tensor_cross",
            "verum_tensor_index",
            "verum_tensor_nonzero",
            "verum_tensor_batch_matmul",
            "verum_tensor_inverse",
            "verum_tensor_solve",
            "verum_tensor_lstsq",
            "verum_tensor_cholesky",
            "verum_tensor_qr",
            "verum_tensor_svd",
            "verum_tensor_lu",
            "verum_tensor_eig",
            "verum_tensor_matrix_power",
            "verum_tensor_expm",
            "verum_tensor_logm",
            "verum_tensor_complex_mul",
            "verum_tensor_complex_pow",
            "verum_tensor_rfft",
            "verum_tensor_irfft",
            "verum_tensor_argmax",
            "verum_tensor_argmin",
            "verum_tensor_masked_select",
            "verum_tensor_einsum",
            "verum_tensor_tokenizer_encode",
            "verum_tensor_tokenizer_load",
            "verum_tensor_tokenizer_decode",
        ];
        for name in stubs_i64_2 {
            self.get_or_declare(module, name, i64_i64_i64);
        }

        // i64(i64, i64, i64)
        let i64_3 = i64_type.fn_type(&[i64_type.into(), i64_type.into(), i64_type.into()], false);
        let stubs_i64_3: &[&str] = &[
            "verum_tensor_concat",
            "verum_tensor_stack",
            "verum_tensor_split",
            "verum_tensor_split_at",
            "verum_tensor_roll",
            "verum_tensor_where",
            "verum_tensor_clamp_i",
            "verum_tensor_lerp_i",
            "verum_tensor_leaky_relu_i",
            "verum_tensor_gather",
            "verum_tensor_topk",
            "verum_tensor_cumulative",
            "verum_tensor_masked_fill_i",
            "verum_tensor_arange_i",
            "verum_tensor_linspace_i",
        ];
        for name in stubs_i64_3 {
            self.get_or_declare(module, name, i64_3);
        }

        // i64(i64, i64, i64, i64)
        let i64_4 = i64_type.fn_type(
            &[i64_type.into(), i64_type.into(), i64_type.into(), i64_type.into()], false);
        let stubs_i64_4: &[&str] = &[
            "verum_tensor_layer_norm",
            "verum_tensor_pool2d",
            "verum_tensor_conv2d",
        ];
        for name in stubs_i64_4 {
            self.get_or_declare(module, name, i64_4);
        }

        // f64(i64) signatures
        let f64_i64 = f64_type.fn_type(&[i64_type.into()], false);
        let stubs_f64_1: &[&str] = &[
            "verum_tensor_trace",
            "verum_tensor_det",
            "verum_tensor_frobenius_norm",
        ];
        for name in stubs_f64_1 {
            self.get_or_declare(module, name, f64_i64);
        }

        // f64(i64, i64) signatures
        let f64_i64_2 = f64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        let stubs_f64_2: &[&str] = &[
            "verum_tensor_dot",
            "verum_tensor_norm",
            "verum_tensor_nansum",
            "verum_tensor_nanmean",
            "verum_tensor_cond",
        ];
        for name in stubs_f64_2 {
            self.get_or_declare(module, name, f64_i64_2);
        }

        // Special signatures
        let fn_identity = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        self.get_or_declare(module, "verum_tensor_identity", fn_identity);

        let fn_rand = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        self.get_or_declare(module, "verum_tensor_rand", fn_rand);

        let fn_one_hot = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        self.get_or_declare(module, "verum_tensor_one_hot", fn_one_hot);

        let fn_from_array = i64_type.fn_type(&[i64_type.into(), ptr_type.into()], false);
        self.get_or_declare(module, "verum_tensor_from_array", fn_from_array);

        // Batch norm: i64(i64, i64, i64, i64, i64, f64, f64, i32)
        let fn_bn = i64_type.fn_type(
            &[i64_type.into(), i64_type.into(), i64_type.into(),
              i64_type.into(), i64_type.into(), f64_type.into(),
              f64_type.into(), i32_type.into()], false);
        self.get_or_declare(module, "verum_tensor_batch_norm", fn_bn);

        // RMS norm
        let fn_rms = i64_type.fn_type(&[i64_type.into(), i64_type.into(), f64_type.into()], false);
        self.get_or_declare(module, "verum_tensor_rms_norm", fn_rms);

        // Flash attention
        let fn_fa = i64_type.fn_type(
            &[i64_type.into(), i64_type.into(), i64_type.into(),
              i64_type.into(), f64_type.into(), i64_type.into()], false);
        self.get_or_declare(module, "verum_tensor_flash_attention", fn_fa);

        // Scatter
        let fn_scatter = i64_type.fn_type(
            &[i64_type.into(), i64_type.into(), i64_type.into(), i64_type.into()], false);
        self.get_or_declare(module, "verum_tensor_scatter", fn_scatter);

        // ML stubs
        let fn_ml_grad = i64_type.fn_type(&[i64_type.into()], false);
        self.get_or_declare(module, "verum_ml_get_grad", fn_ml_grad);

        let fn_ml_backward = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        self.get_or_declare(module, "verum_ml_module_backward", fn_ml_backward);

        // Regex engine
        let fn_regex_2 = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        for name in &[
            "verum_regex_is_match",
            "verum_regex_find_all",
            "verum_regex_split",
        ] {
            self.get_or_declare(module, name, fn_regex_2);
        }
        let fn_regex_3 = i64_type.fn_type(&[i64_type.into(), i64_type.into(), i64_type.into()], false);
        self.get_or_declare(module, "verum_regex_replace_all", fn_regex_3);

        // GPU runtime stubs
        self.emit_gpu_stubs(module)?;
        Ok(())
    }

    /// Emit a GPU stub with a body that returns the given default value.
    /// When no GPU runtime is available, these stubs prevent crashes by returning 0.
    fn emit_gpu_stub_body(&self, module: &Module<'ctx>, name: &str, fn_type: FunctionType<'ctx>, returns_void: bool) -> Result<()> {
        let func = self.get_or_declare(module, name, fn_type);
        if func.count_basic_blocks() > 0 { return Ok(()); }
        let builder = self.context.create_builder();
        let entry = self.context.append_basic_block(func, "entry");
        builder.position_at_end(entry);
        if returns_void {
            builder.build_return(None).or_llvm_err()?;
        } else {
            let ret_type = fn_type.get_return_type();
            if let Some(basic_ty) = ret_type {
                if basic_ty.is_int_type() {
                    builder.build_return(Some(&self.context.i64_type().const_zero())).or_llvm_err()?;
                } else if basic_ty.is_float_type() {
                    builder.build_return(Some(&self.context.f64_type().const_float(0.0))).or_llvm_err()?;
                } else {
                    builder.build_return(Some(&self.context.i64_type().const_zero())).or_llvm_err()?;
                }
            } else {
                builder.build_return(None).or_llvm_err()?;
            }
        }
        Ok(())
    }

    fn emit_gpu_stubs(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let f64_type = ctx.f64_type();
        let void_type = ctx.void_type();

        // i64() -> i64
        let i64_void = i64_type.fn_type(&[], false);
        for name in &[
            "verum_gpu_thread_id_x", "verum_gpu_thread_id_y", "verum_gpu_thread_id_z",
            "verum_gpu_block_id_x", "verum_gpu_block_id_y", "verum_gpu_block_id_z",
            "verum_gpu_block_dim_x", "verum_gpu_block_dim_y", "verum_gpu_block_dim_z",
            "verum_gpu_grid_dim_x", "verum_gpu_grid_dim_y", "verum_gpu_grid_dim_z",
            "verum_gpu_warp_size", "verum_gpu_linear_thread_id",
            "verum_gpu_get_device", "verum_gpu_get_device_count",
            "verum_gpu_stream_create", "verum_gpu_stream_create_nonblocking",
            "verum_gpu_event_create", "verum_gpu_graph_create",
        ] {
            self.emit_gpu_stub_body(module, name, i64_void, false)?;
        }

        // void()
        let void_void = void_type.fn_type(&[], false);
        for name in &[
            "verum_gpu_sync_threads", "verum_gpu_sync_device",
            "verum_gpu_device_reset", "verum_gpu_profile_marker_pop",
        ] {
            self.emit_gpu_stub_body(module, name, void_void, true)?;
        }

        // void(i64)
        let void_i64 = void_type.fn_type(&[i64_type.into()], false);
        for name in &[
            "verum_gpu_sync_warp", "verum_gpu_free", "verum_gpu_stream_destroy",
            "verum_gpu_event_destroy", "verum_gpu_event_sync",
            "verum_gpu_set_device", "verum_gpu_set_device_flags",
            "verum_gpu_enable_peer_access", "verum_gpu_disable_peer_access",
            "verum_gpu_unpin_memory", "verum_gpu_graph_destroy",
            "verum_gpu_graph_exec_destroy", "verum_gpu_profile_range_end",
            "verum_gpu_profile_marker_push",
        ] {
            self.emit_gpu_stub_body(module, name, void_i64, true)?;
        }

        // i64(i64)
        let i64_1 = i64_type.fn_type(&[i64_type.into()], false);
        for name in &[
            "verum_gpu_shared_mem_alloc", "verum_gpu_shared_mem_load_i64",
            "verum_gpu_shared_mem_load_u32", "verum_gpu_stream_query",
            "verum_gpu_event_query", "verum_gpu_stream_get_priority",
            "verum_gpu_stream_create_prio", "verum_gpu_profile_range_start",
            "verum_gpu_graph_instantiate", "verum_gpu_event_create_with_flags",
            "verum_gpu_malloc_managed",
        ] {
            self.emit_gpu_stub_body(module, name, i64_1, false)?;
        }

        // i64(i64, i64)
        let i64_2 = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        for name in &[
            "verum_gpu_shared_mem_atomic_add_i64", "verum_gpu_shared_mem_atomic_max_i64",
            "verum_gpu_shared_mem_atomic_min_i64", "verum_gpu_malloc",
            "verum_gpu_can_access_peer", "verum_gpu_get_device_property",
            "verum_gpu_get_memory_info", "verum_gpu_graph_exec_update",
        ] {
            self.emit_gpu_stub_body(module, name, i64_2, false)?;
        }

        // i64(i64, i64, i64)
        let i64_3 = i64_type.fn_type(&[i64_type.into(), i64_type.into(), i64_type.into()], false);
        self.emit_gpu_stub_body(module, "verum_gpu_shared_mem_atomic_cas_i64", i64_3, false)?;

        // void(i64, i64)
        let void_2 = void_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        for name in &[
            "verum_gpu_shared_mem_store_i64", "verum_gpu_shared_mem_store_u32",
            "verum_gpu_event_record", "verum_gpu_stream_wait_event",
            "verum_gpu_pin_memory", "verum_gpu_stream_add_callback",
            "verum_gpu_graph_begin_capture", "verum_gpu_graph_end_capture",
            "verum_gpu_graph_launch",
        ] {
            self.emit_gpu_stub_body(module, name, void_2, true)?;
        }

        // void(i64, i64, i64)
        let void_3 = void_type.fn_type(&[i64_type.into(), i64_type.into(), i64_type.into()], false);
        for name in &[
            "verum_gpu_memcpy", "verum_gpu_memset", "verum_gpu_prefetch",
            "verum_gpu_memcpy_h2d", "verum_gpu_memcpy_d2h", "verum_gpu_memcpy_d2d",
            "verum_gpu_event_record_with_flags",
        ] {
            self.emit_gpu_stub_body(module, name, void_3, true)?;
        }

        // void(i64, i64, i64, i64)
        let void_4 = void_type.fn_type(
            &[i64_type.into(), i64_type.into(), i64_type.into(), i64_type.into()], false);
        for name in &[
            "verum_gpu_memcpy_async", "verum_gpu_memcpy_async_h2d",
            "verum_gpu_memcpy_async_d2h", "verum_gpu_memset_async",
            "verum_gpu_mem_advise", "verum_gpu_prefetch_async",
        ] {
            self.emit_gpu_stub_body(module, name, void_4, true)?;
        }

        // void(i64, i64, i64, i64, i64)
        let void_5 = void_type.fn_type(
            &[i64_type.into(), i64_type.into(), i64_type.into(),
              i64_type.into(), i64_type.into()], false);
        for name in &[
            "verum_gpu_launch_cooperative", "verum_gpu_launch_multi_device",
        ] {
            self.emit_gpu_stub_body(module, name, void_5, true)?;
        }

        // void(i64, i64, i64, i64, i64, i64)
        let void_6 = void_type.fn_type(
            &[i64_type.into(), i64_type.into(), i64_type.into(),
              i64_type.into(), i64_type.into(), i64_type.into()], false);
        for name in &[
            "verum_gpu_memcpy_2d", "verum_gpu_launch",
        ] {
            self.emit_gpu_stub_body(module, name, void_6, true)?;
        }

        // void(i64, i64, i64, i64, i64, i64, i64)
        let void_7 = void_type.fn_type(
            &[i64_type.into(), i64_type.into(), i64_type.into(),
              i64_type.into(), i64_type.into(), i64_type.into(),
              i64_type.into()], false);
        self.emit_gpu_stub_body(module, "verum_gpu_memcpy_2d_async", void_7, true)?;

        // f64(i64)
        let f64_1 = f64_type.fn_type(&[i64_type.into()], false);
        self.emit_gpu_stub_body(module, "verum_gpu_shared_mem_load_f64", f64_1, false)?;

        // void(i64, f64)
        let void_i64_f64 = void_type.fn_type(&[i64_type.into(), f64_type.into()], false);
        self.emit_gpu_stub_body(module, "verum_gpu_shared_mem_store_f64", void_i64_f64, true)?;

        // f64(i64, f64)
        let f64_i64_f64 = f64_type.fn_type(&[i64_type.into(), f64_type.into()], false);
        self.emit_gpu_stub_body(module, "verum_gpu_shared_mem_atomic_add_f64", f64_i64_f64, false)?;

        // f64(i64, i64) — event elapsed
        let f64_2 = f64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        self.emit_gpu_stub_body(module, "verum_gpu_event_elapsed", f64_2, false)?;

        // i64(i64) — mem attribute
        self.emit_gpu_stub_body(module, "verum_gpu_mem_get_attribute", i64_2, false)?;

        // GPU enumerate stubs
        let i64_void2 = i64_type.fn_type(&[], false);
        for name in &[
            "verum_gpu_enumerate_cuda", "verum_gpu_enumerate_metal",
            "verum_gpu_enumerate_rocm", "verum_gpu_enumerate_vulkan",
        ] {
            self.emit_gpu_stub_body(module, name, i64_void2, false)?;
        }

        // verum_gpu_launch_with_fn(fn_ptr, gx, gy, gz, bx, by, bz, shmem, stream)
        let ptr_type = ctx.ptr_type(AddressSpace::default());
        let fn_launch = void_type.fn_type(
            &[ptr_type.into(), i64_type.into(), i64_type.into(), i64_type.into(),
              i64_type.into(), i64_type.into(), i64_type.into(),
              i64_type.into(), i64_type.into()], false);
        self.emit_gpu_stub_body(module, "verum_gpu_launch_with_fn", fn_launch, true)?;

        // verum_sort_f64 (used internally)
        let fn_sort = void_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
        self.emit_gpu_stub_body(module, "verum_sort_f64", fn_sort, true)?;
        Ok(())
    }
}
