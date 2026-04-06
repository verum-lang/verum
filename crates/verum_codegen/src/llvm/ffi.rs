//! FFI lowering for VBC → LLVM IR.
//!
//! This module provides zero-cost FFI lowering by translating VBC FfiExtended
//! instructions directly to LLVM IR. Unlike the interpreter (which uses libffi
//! at ~150ns/call), AOT-compiled code achieves ~5ns/call through direct
//! native function calls.
//!
//! # Architecture
//!
//! ```text
//! VBC FfiExtended Instruction
//!     │
//!     ├── Symbol Resolution ──────► External function declaration
//!     │
//!     ├── FFI Calls ──────────────► LLVM call instruction with
//!     │                             appropriate calling convention
//!     │
//!     ├── Memory Ops ─────────────► LLVM intrinsics (memcpy, memset)
//!     │                             or libc calls (malloc, free)
//!     │
//!     └── Raw Pointer Ops ────────► LLVM load/store/GEP
//! ```
//!
//! # Calling Conventions
//!
//! | VBC Sub-opcode | LLVM Calling Convention |
//! |----------------|-------------------------|
//! | CallFfiC       | C (0)                   |
//! | CallFfiStdcall | X86_StdCall (64)        |
//! | CallFfiSysV64  | X86_64_SysV (78)        |
//! | CallFfiFastcall| X86_FastCall (65)       |
//! | CallFfiVariadic| C with variadic marker  |
//!
//! # Performance
//!
//! - Direct FFI call: ~5ns (vs 150ns interpreter)
//! - memcpy intrinsic: optimal SIMD on supported platforms
//! - Pointer arithmetic: single instruction

use verum_common::Text;
use verum_llvm::builder::Builder;
use verum_llvm::context::Context;
use verum_llvm::module::Module;
use verum_llvm::types::{BasicTypeEnum, FunctionType};
use verum_llvm::values::{BasicMetadataValueEnum, BasicValue, BasicValueEnum, FunctionValue, IntValue, PointerValue};
use verum_llvm::{AddressSpace, IntPredicate};
use verum_vbc::instruction::FfiSubOpcode;

use super::context::FunctionContext;
use super::error::{LlvmLoweringError, Result};
use super::types::TypeLowering;

/// LLVM calling convention constants.
/// Reference: llvm-c/Core.h LLVMCallConv enum
mod calling_conventions {
    pub const C: u32 = 0;
    pub const X86_STDCALL: u32 = 64;
    pub const X86_FASTCALL: u32 = 65;
    pub const X86_64_SYSV: u32 = 78;
    pub const WIN64: u32 = 79;
    pub const AARCH64_AAPCS: u32 = 81; // ARM64 AAPCS (standard for ARM64)
    pub const AARCH64_AAPCS_VFP: u32 = 82; // ARM64 with VFP (FP/SIMD args in registers)
    pub const AARCH64_AAPCS_SVE: u32 = 83; // ARM64 SVE (Scalable Vector Extension)
}

/// Statistics for FFI lowering operations.
#[derive(Debug, Default, Clone)]
pub struct FfiLoweringStats {
    /// Number of external function declarations.
    pub external_decls: usize,
    /// Number of direct FFI calls.
    pub direct_calls: usize,
    /// Number of indirect FFI calls.
    pub indirect_calls: usize,
    /// Number of memory intrinsic uses.
    pub memory_intrinsics: usize,
    /// Number of raw pointer operations.
    pub raw_ptr_ops: usize,
}

/// FFI lowering helper for a single function context.
pub struct FfiLowering<'ctx> {
    /// LLVM context.
    context: &'ctx Context,
    /// Statistics.
    stats: FfiLoweringStats,
}

impl<'ctx> FfiLowering<'ctx> {
    /// Create a new FFI lowering helper.
    pub fn new(context: &'ctx Context) -> Self {
        Self {
            context,
            stats: FfiLoweringStats::default(),
        }
    }

    /// Get lowering statistics.
    pub fn stats(&self) -> &FfiLoweringStats {
        &self.stats
    }

    // ========================================================================
    // Memory Operations (LLVM Intrinsics)
    // ========================================================================

    /// Lower memcpy to LLVM intrinsic.
    ///
    /// Generates: `llvm.memcpy.p0.p0.i64(dst, src, size, isvolatile=false)`
    pub fn lower_memcpy(
        &mut self,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
        dst: PointerValue<'ctx>,
        src: PointerValue<'ctx>,
        size: IntValue<'ctx>,
    ) -> Result<()> {
        self.stats.memory_intrinsics += 1;

        // Get or declare the memcpy intrinsic
        let memcpy_fn = self.get_or_declare_memcpy(module)?;

        // Build the call
        let is_volatile = self.context.bool_type().const_int(0, false);
        builder
            .build_call(
                memcpy_fn,
                &[dst.into(), src.into(), size.into(), is_volatile.into()],
                "memcpy",
            )
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

        Ok(())
    }

    /// Lower memmove to LLVM intrinsic.
    ///
    /// Generates: `llvm.memmove.p0.p0.i64(dst, src, size, isvolatile=false)`
    pub fn lower_memmove(
        &mut self,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
        dst: PointerValue<'ctx>,
        src: PointerValue<'ctx>,
        size: IntValue<'ctx>,
    ) -> Result<()> {
        self.stats.memory_intrinsics += 1;

        let memmove_fn = self.get_or_declare_memmove(module)?;

        let is_volatile = self.context.bool_type().const_int(0, false);
        builder
            .build_call(
                memmove_fn,
                &[dst.into(), src.into(), size.into(), is_volatile.into()],
                "memmove",
            )
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

        Ok(())
    }

    /// Lower memset to LLVM intrinsic.
    ///
    /// Generates: `llvm.memset.p0.i64(dst, val, size, isvolatile=false)`
    pub fn lower_memset(
        &mut self,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
        dst: PointerValue<'ctx>,
        value: IntValue<'ctx>,
        size: IntValue<'ctx>,
    ) -> Result<()> {
        self.stats.memory_intrinsics += 1;

        let memset_fn = self.get_or_declare_memset(module)?;

        // Truncate value to i8 if needed
        let i8_type = self.context.i8_type();
        let value_i8 = if value.get_type().get_bit_width() > 8 {
            builder
                .build_int_truncate(value, i8_type, "memset_val")
                .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?
        } else {
            value
        };

        let is_volatile = self.context.bool_type().const_int(0, false);
        builder
            .build_call(
                memset_fn,
                &[dst.into(), value_i8.into(), size.into(), is_volatile.into()],
                "memset",
            )
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

        Ok(())
    }

    /// Lower memcmp to libc call.
    ///
    /// Returns: negative if ptr1 < ptr2, 0 if equal, positive otherwise
    pub fn lower_memcmp(
        &mut self,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
        ptr1: PointerValue<'ctx>,
        ptr2: PointerValue<'ctx>,
        size: IntValue<'ctx>,
    ) -> Result<IntValue<'ctx>> {
        self.stats.direct_calls += 1;

        let memcmp_fn = self.get_or_declare_memcmp(module)?;

        let args: [BasicMetadataValueEnum<'ctx>; 3] = [ptr1.into(), ptr2.into(), size.into()];
        let result = builder
            .build_call(memcmp_fn, &args, "memcmp")
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| LlvmLoweringError::internal("memcmp should return i32"))?
            .into_int_value();

        Ok(result)
    }

    // ========================================================================
    // C Memory Allocation
    // ========================================================================

    /// Lower malloc call.
    pub fn lower_malloc(
        &mut self,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
        size: IntValue<'ctx>,
    ) -> Result<PointerValue<'ctx>> {
        self.stats.direct_calls += 1;

        let malloc_fn = self.get_or_declare_malloc(module)?;

        let args: [BasicMetadataValueEnum<'ctx>; 1] = [size.into()];
        let raw_ptr = builder
            .build_call(malloc_fn, &args, "malloc")
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| LlvmLoweringError::internal("malloc should return ptr"))?
            .into_pointer_value();

        // Null check: OOM → _exit(1)
        let current_bb = builder
            .get_insert_block()
            .ok_or_else(|| LlvmLoweringError::internal("lower_malloc: no insert block"))?;
        let func = current_bb
            .get_parent()
            .ok_or_else(|| LlvmLoweringError::internal("lower_malloc: no parent function"))?;
        let oom_bb = self.context.append_basic_block(func, "malloc_oom");
        let ok_bb = self.context.append_basic_block(func, "malloc_ok");
        let is_null = builder
            .build_is_null(raw_ptr, "malloc_null")
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
        builder
            .build_conditional_branch(is_null, oom_bb, ok_bb)
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
        builder.position_at_end(oom_bb);
        let exit_fn = if let Some(f) = module.get_function("_exit") {
            f
        } else {
            let i64_type = self.context.i64_type();
            let fn_type = self.context.void_type().fn_type(&[i64_type.into()], false);
            let f = module.add_function("_exit", fn_type, None);
            f.add_attribute(
                verum_llvm::attributes::AttributeLoc::Function,
                self.context.create_string_attribute("noreturn", ""),
            );
            f
        };
        let i64_type = self.context.i64_type();
        builder
            .build_call(exit_fn, &[i64_type.const_int(1, false).into()], "")
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
        builder
            .build_unreachable()
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
        builder.position_at_end(ok_bb);

        Ok(raw_ptr)
    }

    /// Lower free call.
    pub fn lower_free(
        &mut self,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
        ptr: PointerValue<'ctx>,
    ) -> Result<()> {
        self.stats.direct_calls += 1;

        let free_fn = self.get_or_declare_free(module)?;

        builder
            .build_call(free_fn, &[ptr.into()], "")
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

        Ok(())
    }

    /// Lower realloc call.
    pub fn lower_realloc(
        &mut self,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
        ptr: PointerValue<'ctx>,
        size: IntValue<'ctx>,
    ) -> Result<PointerValue<'ctx>> {
        self.stats.direct_calls += 1;

        let realloc_fn = self.get_or_declare_realloc(module)?;

        let args: [BasicMetadataValueEnum<'ctx>; 2] = [ptr.into(), size.into()];
        let result = builder
            .build_call(realloc_fn, &args, "realloc")
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| LlvmLoweringError::internal("realloc should return ptr"))?
            .into_pointer_value();

        Ok(result)
    }

    // ========================================================================
    // Raw Pointer Operations
    // ========================================================================

    /// Lower raw pointer dereference (load).
    ///
    /// Generates LLVM load instruction bypassing CBGR checks.
    pub fn lower_deref_raw(
        &mut self,
        builder: &Builder<'ctx>,
        ptr: PointerValue<'ctx>,
        size_bytes: u8,
    ) -> Result<IntValue<'ctx>> {
        self.stats.raw_ptr_ops += 1;

        let int_type = match size_bytes {
            1 => self.context.i8_type(),
            2 => self.context.i16_type(),
            4 => self.context.i32_type(),
            8 => self.context.i64_type(),
            _ => {
                return Err(LlvmLoweringError::internal(format!(
                    "Invalid deref size: {}",
                    size_bytes
                )))
            }
        };

        let result = builder
            .build_load(int_type, ptr, "deref_raw")
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?
            .into_int_value();

        Ok(result)
    }

    /// Lower raw pointer write (store).
    pub fn lower_deref_mut_raw(
        &mut self,
        builder: &Builder<'ctx>,
        ptr: PointerValue<'ctx>,
        value: IntValue<'ctx>,
        size_bytes: u8,
    ) -> Result<()> {
        self.stats.raw_ptr_ops += 1;

        // Truncate or extend value to match target size
        let int_type = match size_bytes {
            1 => self.context.i8_type(),
            2 => self.context.i16_type(),
            4 => self.context.i32_type(),
            8 => self.context.i64_type(),
            _ => {
                return Err(LlvmLoweringError::internal(format!(
                    "Invalid store size: {}",
                    size_bytes
                )))
            }
        };

        let val_bits = value.get_type().get_bit_width();
        let target_bits = int_type.get_bit_width();

        let adjusted_value = if val_bits > target_bits {
            builder
                .build_int_truncate(value, int_type, "trunc")
                .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?
        } else if val_bits < target_bits {
            builder
                .build_int_s_extend(value, int_type, "sext")
                .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?
        } else {
            value
        };

        builder
            .build_store(ptr, adjusted_value)
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

        Ok(())
    }

    /// Lower pointer-to-pointer dereference.
    pub fn lower_deref_raw_ptr(
        &mut self,
        builder: &Builder<'ctx>,
        ptr: PointerValue<'ctx>,
    ) -> Result<PointerValue<'ctx>> {
        self.stats.raw_ptr_ops += 1;

        let ptr_type = self.context.ptr_type(AddressSpace::default());

        let result = builder
            .build_load(ptr_type, ptr, "deref_ptr")
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?
            .into_pointer_value();

        Ok(result)
    }

    /// Lower pointer addition.
    pub fn lower_ptr_add(
        &mut self,
        builder: &Builder<'ctx>,
        ptr: PointerValue<'ctx>,
        offset: IntValue<'ctx>,
    ) -> Result<PointerValue<'ctx>> {
        self.stats.raw_ptr_ops += 1;

        let i8_type = self.context.i8_type();

        // Use GEP with i8 element type for byte-level pointer arithmetic
        let result = unsafe {
            builder
                .build_gep(i8_type, ptr, &[offset], "ptr_add")
                .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?
        };

        Ok(result)
    }

    /// Lower pointer subtraction.
    pub fn lower_ptr_sub(
        &mut self,
        builder: &Builder<'ctx>,
        ptr: PointerValue<'ctx>,
        offset: IntValue<'ctx>,
    ) -> Result<PointerValue<'ctx>> {
        self.stats.raw_ptr_ops += 1;

        // Negate the offset
        let neg_offset = builder
            .build_int_neg(offset, "neg_offset")
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

        let i8_type = self.context.i8_type();

        let result = unsafe {
            builder
                .build_gep(i8_type, ptr, &[neg_offset], "ptr_sub")
                .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?
        };

        Ok(result)
    }

    /// Lower pointer difference.
    pub fn lower_ptr_diff(
        &mut self,
        builder: &Builder<'ctx>,
        ptr1: PointerValue<'ctx>,
        ptr2: PointerValue<'ctx>,
    ) -> Result<IntValue<'ctx>> {
        self.stats.raw_ptr_ops += 1;

        let i64_type = self.context.i64_type();

        // Convert pointers to integers and subtract
        let int1 = builder
            .build_ptr_to_int(ptr1, i64_type, "ptr1_int")
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
        let int2 = builder
            .build_ptr_to_int(ptr2, i64_type, "ptr2_int")
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

        let diff = builder
            .build_int_sub(int1, int2, "ptr_diff")
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

        Ok(diff)
    }

    /// Lower pointer null check.
    pub fn lower_ptr_is_null(
        &mut self,
        builder: &Builder<'ctx>,
        ptr: PointerValue<'ctx>,
    ) -> Result<IntValue<'ctx>> {
        self.stats.raw_ptr_ops += 1;

        let null_ptr = self
            .context
            .ptr_type(AddressSpace::default())
            .const_null();

        let result = builder
            .build_int_compare(IntPredicate::EQ, ptr, null_ptr, "is_null")
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

        Ok(result)
    }

    // ========================================================================
    // FFI Calls
    // ========================================================================

    /// Lower a direct FFI call with the specified calling convention.
    ///
    /// # Arguments
    /// * `builder` - LLVM builder
    /// * `module` - LLVM module (for declaring external functions)
    /// * `func` - The function to call (previously resolved via LoadSymbol)
    /// * `args` - Arguments to pass
    /// * `calling_convention` - The calling convention to use
    ///
    /// # Returns
    /// The return value (or unit if void)
    pub fn lower_ffi_call(
        &mut self,
        builder: &Builder<'ctx>,
        func: FunctionValue<'ctx>,
        args: &[BasicValueEnum<'ctx>],
        calling_convention: u32,
    ) -> Result<Option<BasicValueEnum<'ctx>>> {
        self.stats.direct_calls += 1;

        // Set calling convention
        func.set_call_conventions(calling_convention);

        // Convert args to BasicMetadataValueEnum
        let meta_args: Vec<BasicMetadataValueEnum<'ctx>> = args.iter().map(|a| (*a).into()).collect();

        // Build the call
        let call_site = builder
            .build_call(func, &meta_args, "ffi_call")
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

        // Set the calling convention on the call site
        call_site.set_call_convention(calling_convention);

        // Return the result if not void
        let result = call_site.try_as_basic_value().basic();
        Ok(result)
    }

    /// Lower an indirect FFI call through a function pointer.
    pub fn lower_indirect_call(
        &mut self,
        builder: &Builder<'ctx>,
        fn_ptr: PointerValue<'ctx>,
        fn_type: FunctionType<'ctx>,
        args: &[BasicValueEnum<'ctx>],
        calling_convention: u32,
    ) -> Result<Option<BasicValueEnum<'ctx>>> {
        self.stats.indirect_calls += 1;

        // Convert args to BasicMetadataValueEnum
        let meta_args: Vec<BasicMetadataValueEnum<'ctx>> = args.iter().map(|a| (*a).into()).collect();

        let call_site = builder
            .build_indirect_call(fn_type, fn_ptr, &meta_args, "indirect_call")
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

        call_site.set_call_convention(calling_convention);

        let result = call_site.try_as_basic_value().basic();
        Ok(result)
    }

    /// Declare an external FFI function in the module.
    ///
    /// This is used to resolve symbols at link time rather than runtime.
    pub fn declare_external_function(
        &mut self,
        module: &Module<'ctx>,
        name: &str,
        fn_type: FunctionType<'ctx>,
        calling_convention: u32,
    ) -> FunctionValue<'ctx> {
        self.stats.external_decls += 1;

        // Check if already declared
        if let Some(existing) = module.get_function(name) {
            return existing;
        }

        // Declare the function
        let func = module.add_function(name, fn_type, None);
        func.set_call_conventions(calling_convention);

        func
    }

    // ========================================================================
    // Error Handling (errno)
    // ========================================================================

    /// Lower errno get operation.
    ///
    /// On most platforms, this accesses `__errno_location()` or `_errno()`.
    pub fn lower_get_errno(
        &mut self,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
    ) -> Result<IntValue<'ctx>> {
        self.stats.direct_calls += 1;

        let errno_fn = self.get_or_declare_errno_location(module)?;

        // Call errno location function
        let empty_args: [BasicMetadataValueEnum<'ctx>; 0] = [];
        let errno_ptr = builder
            .build_call(errno_fn, &empty_args, "errno_loc")
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| LlvmLoweringError::internal("__errno_location should return ptr"))?
            .into_pointer_value();

        // Load errno value
        let i32_type = self.context.i32_type();
        let errno_val = builder
            .build_load(i32_type, errno_ptr, "errno")
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?
            .into_int_value();

        Ok(errno_val)
    }

    /// Lower errno set operation.
    pub fn lower_set_errno(
        &mut self,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
        value: IntValue<'ctx>,
    ) -> Result<()> {
        self.stats.direct_calls += 1;

        let errno_fn = self.get_or_declare_errno_location(module)?;

        let empty_args: [BasicMetadataValueEnum<'ctx>; 0] = [];
        let errno_ptr = builder
            .build_call(errno_fn, &empty_args, "errno_loc")
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| LlvmLoweringError::internal("__errno_location should return ptr"))?
            .into_pointer_value();

        // Store the new value
        builder
            .build_store(errno_ptr, value)
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

        Ok(())
    }

    // ========================================================================
    // Internal Helpers - Function Declarations
    // ========================================================================

    fn get_or_declare_memcpy(&self, module: &Module<'ctx>) -> Result<FunctionValue<'ctx>> {
        let name = "llvm.memcpy.p0.p0.i64";
        if let Some(func) = module.get_function(name) {
            return Ok(func);
        }

        let void_type = self.context.void_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        let bool_type = self.context.bool_type();

        let fn_type =
            void_type.fn_type(&[ptr_type.into(), ptr_type.into(), i64_type.into(), bool_type.into()], false);

        Ok(module.add_function(name, fn_type, None))
    }

    fn get_or_declare_memmove(&self, module: &Module<'ctx>) -> Result<FunctionValue<'ctx>> {
        let name = "llvm.memmove.p0.p0.i64";
        if let Some(func) = module.get_function(name) {
            return Ok(func);
        }

        let void_type = self.context.void_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        let bool_type = self.context.bool_type();

        let fn_type =
            void_type.fn_type(&[ptr_type.into(), ptr_type.into(), i64_type.into(), bool_type.into()], false);

        Ok(module.add_function(name, fn_type, None))
    }

    fn get_or_declare_memset(&self, module: &Module<'ctx>) -> Result<FunctionValue<'ctx>> {
        let name = "llvm.memset.p0.i64";
        if let Some(func) = module.get_function(name) {
            return Ok(func);
        }

        let void_type = self.context.void_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i8_type = self.context.i8_type();
        let i64_type = self.context.i64_type();
        let bool_type = self.context.bool_type();

        let fn_type =
            void_type.fn_type(&[ptr_type.into(), i8_type.into(), i64_type.into(), bool_type.into()], false);

        Ok(module.add_function(name, fn_type, None))
    }

    fn get_or_declare_memcmp(&self, module: &Module<'ctx>) -> Result<FunctionValue<'ctx>> {
        let name = "memcmp";
        if let Some(func) = module.get_function(name) {
            return Ok(func);
        }

        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        let i32_type = self.context.i32_type();

        let fn_type = i32_type.fn_type(&[ptr_type.into(), ptr_type.into(), i64_type.into()], false);

        Ok(module.add_function(name, fn_type, None))
    }

    fn get_or_declare_malloc(&self, module: &Module<'ctx>) -> Result<FunctionValue<'ctx>> {
        let name = "malloc";
        if let Some(func) = module.get_function(name) {
            return Ok(func);
        }

        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();

        let fn_type = ptr_type.fn_type(&[i64_type.into()], false);

        Ok(module.add_function(name, fn_type, None))
    }

    fn get_or_declare_free(&self, module: &Module<'ctx>) -> Result<FunctionValue<'ctx>> {
        let name = "free";
        if let Some(func) = module.get_function(name) {
            return Ok(func);
        }

        let void_type = self.context.void_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        let fn_type = void_type.fn_type(&[ptr_type.into()], false);

        Ok(module.add_function(name, fn_type, None))
    }

    fn get_or_declare_realloc(&self, module: &Module<'ctx>) -> Result<FunctionValue<'ctx>> {
        let name = "realloc";
        if let Some(func) = module.get_function(name) {
            return Ok(func);
        }

        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();

        let fn_type = ptr_type.fn_type(&[ptr_type.into(), i64_type.into()], false);

        Ok(module.add_function(name, fn_type, None))
    }

    fn get_or_declare_errno_location(&self, module: &Module<'ctx>) -> Result<FunctionValue<'ctx>> {
        self.get_or_declare_errno_location_for_target(module, TargetPlatform::current())
    }

    /// Get or declare errno location function for a specific target platform.
    fn get_or_declare_errno_location_for_target(
        &self,
        module: &Module<'ctx>,
        target: TargetPlatform,
    ) -> Result<FunctionValue<'ctx>> {
        // Platform-specific errno functions:
        // - Linux: __errno_location() -> int*
        // - macOS: __error() -> int*
        // - Windows: _errno() -> int*
        // - FreeBSD/OpenBSD: __error() -> int*
        let name = match target {
            TargetPlatform::Windows => "_errno",
            TargetPlatform::MacOS | TargetPlatform::FreeBSD | TargetPlatform::OpenBSD => "__error",
            TargetPlatform::Linux | TargetPlatform::Unknown => "__errno_location",
        };

        if let Some(func) = module.get_function(name) {
            return Ok(func);
        }

        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let fn_type = ptr_type.fn_type(&[], false);

        Ok(module.add_function(name, fn_type, None))
    }
}

/// Target platform for platform-specific code generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetPlatform {
    Linux,
    MacOS,
    Windows,
    FreeBSD,
    OpenBSD,
    Unknown,
}

impl TargetPlatform {
    /// Detect current platform at compile time.
    #[cfg(target_os = "linux")]
    pub fn current() -> Self {
        TargetPlatform::Linux
    }

    #[cfg(target_os = "macos")]
    pub fn current() -> Self {
        TargetPlatform::MacOS
    }

    #[cfg(target_os = "windows")]
    pub fn current() -> Self {
        TargetPlatform::Windows
    }

    #[cfg(target_os = "freebsd")]
    pub fn current() -> Self {
        TargetPlatform::FreeBSD
    }

    #[cfg(target_os = "openbsd")]
    pub fn current() -> Self {
        TargetPlatform::OpenBSD
    }

    #[cfg(not(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "freebsd",
        target_os = "openbsd"
    )))]
    pub fn current() -> Self {
        TargetPlatform::Unknown
    }
}

/// Map VBC FFI sub-opcode to LLVM calling convention.
pub fn ffi_subop_to_calling_convention(sub_op: FfiSubOpcode) -> u32 {
    match sub_op {
        FfiSubOpcode::CallFfiC | FfiSubOpcode::CallFfiVariadic => calling_conventions::C,
        FfiSubOpcode::CallFfiStdcall => calling_conventions::X86_STDCALL,
        FfiSubOpcode::CallFfiSysV64 => calling_conventions::X86_64_SYSV,
        FfiSubOpcode::CallFfiFastcall => calling_conventions::X86_FASTCALL,
        FfiSubOpcode::CallFfiAarch64 => calling_conventions::AARCH64_AAPCS,
        FfiSubOpcode::CallFfiWin64Arm64 => calling_conventions::WIN64, // Windows ARM64 uses WIN64 variant
        FfiSubOpcode::CallFfiIndirect => calling_conventions::C,       // Default for indirect
        _ => calling_conventions::C,                                   // Default
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calling_convention_mapping() {
        assert_eq!(
            ffi_subop_to_calling_convention(FfiSubOpcode::CallFfiC),
            calling_conventions::C
        );
        assert_eq!(
            ffi_subop_to_calling_convention(FfiSubOpcode::CallFfiStdcall),
            calling_conventions::X86_STDCALL
        );
        assert_eq!(
            ffi_subop_to_calling_convention(FfiSubOpcode::CallFfiSysV64),
            calling_conventions::X86_64_SYSV
        );
        assert_eq!(
            ffi_subop_to_calling_convention(FfiSubOpcode::CallFfiFastcall),
            calling_conventions::X86_FASTCALL
        );
    }
}
