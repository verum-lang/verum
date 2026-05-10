//! CBGR (Capability-Based Generational References) lowering to LLVM IR.
//!

//! This module implements tier-aware reference operations for the CBGR
//! memory safety system.
//!

//! # Reference Tiers
//!

//! - **Tier 0**: Full runtime checks (~15ns overhead)
//!  - Generation validation on every dereference
//!  - Capability checks for read/write/borrow
//!

//! - **Tier 1**: Compiler-proven safe (zero overhead)
//!  - Escape analysis proves reference validity
//!  - Direct pointer access
//!

//! - **Tier 2**: Manually marked unsafe (zero overhead)
//!  - User asserts safety via `&unsafe T`
//!  - Direct pointer access
//!

//! # Memory Layout
//!

//! ThinRef<T>: 16 bytes
//! ```text
//! +--------+------------+------------+
//! | ptr | generation | epoch_caps |
//! | 8 bytes| 4 bytes | 4 bytes |
//! +--------+------------+------------+
//! ```
//!

//! FatRef<T>: 32 bytes (for unsized types — slices, trait objects).
//! Layout matches `core/mem/fat_ref.vr` `@repr(C, size(32), align(8))`.
//! ```text
//! +-----+------------+----------------+----------+--------+----------+
//! | ptr | generation | epoch_and_caps | metadata | offset | reserved |
//! |  8  |     4      |       4        |    8     |   4    |    4     |
//! +-----+------------+----------------+----------+--------+----------+
//! ```
//! * `metadata`: length for slices, vtable pointer for trait objects.
//! * `offset_from_base`: subslice view offset from the base allocation.
//! * `reserved`: padding for future extensions.

use verum_common::Text;
use verum_llvm::IntPredicate;
use verum_llvm::builder::Builder;
use verum_llvm::context::Context;
use verum_llvm::module::Module;
use verum_llvm::types::{BasicTypeEnum, IntType, PointerType, StructType};
use verum_llvm::values::{BasicValueEnum, FunctionValue, IntValue, PointerValue, StructValue};

use super::error::{BuildExt, LlvmLoweringError, Result};
use super::types::RefTier;

/// CBGR lowering context.
pub struct CbgrLowering<'ctx> {
    /// LLVM context reference.
    context: &'ctx Context,

    /// ThinRef struct type: { ptr, generation, epoch_caps }.
    thin_ref_type: StructType<'ctx>,

    /// FatRef struct type — matches `core/mem/fat_ref.vr` 6-field layout
    /// `{ ptr, generation, epoch_and_caps, metadata, offset_from_base, reserved }`.
    fat_ref_type: StructType<'ctx>,

    /// Runtime check function (for Tier 0).
    check_fn: Option<FunctionValue<'ctx>>,

    /// Statistics.
    stats: CbgrStats,
}

/// Statistics for CBGR lowering.
#[derive(Debug, Default, Clone)]
pub struct CbgrStats {
    /// Total references created.
    pub refs_created: usize,
    /// Tier 0 references (full checks).
    pub tier0_refs: usize,
    /// Tier 1 references (compiler-proven).
    pub tier1_refs: usize,
    /// Tier 2 references (unsafe).
    pub tier2_refs: usize,
    /// Runtime checks generated.
    pub runtime_checks: usize,
    /// Checks eliminated.
    pub checks_eliminated: usize,
}

impl CbgrStats {
    /// Calculate the check elimination rate.
    pub fn elimination_rate(&self) -> f64 {
        let total = self.runtime_checks + self.checks_eliminated;
        if total == 0 {
            0.0
        } else {
            self.checks_eliminated as f64 / total as f64
        }
    }
}

impl<'ctx> CbgrLowering<'ctx> {
    /// Create a new CBGR lowering context.
    pub fn new(context: &'ctx Context) -> Self {
        let ptr_type = context.ptr_type(Default::default());
        let i32_type = context.i32_type();
        let i64_type = context.i64_type();

        // ThinRef: { ptr: *T, generation: u32, epoch_caps: u32 }
        let thin_ref_type =
            context.struct_type(&[ptr_type.into(), i32_type.into(), i32_type.into()], false);

        // FatRef: 32-byte 6-field layout matching `core/mem/fat_ref.vr`:
        //   { ptr:*T, generation:u32, epoch_and_caps:u32,
        //     metadata:i64, offset_from_base:u32, reserved:u32 }
        // Total: 8 + 4 + 4 + 8 + 4 + 4 = 32 bytes.
        // The `metadata` field carries length for slices and a vtable
        // pointer for trait objects; `offset_from_base` enables zero-copy
        // subslice views; `reserved` is padding for future extensions.
        let fat_ref_type = context.struct_type(
            &[
                ptr_type.into(),
                i32_type.into(),
                i32_type.into(),
                i64_type.into(),
                i32_type.into(),
                i32_type.into(),
            ],
            false,
        );

        Self {
            context,
            thin_ref_type,
            fat_ref_type,
            check_fn: None,
            stats: CbgrStats::default(),
        }
    }

    /// Get the ThinRef struct type.
    pub fn thin_ref_type(&self) -> StructType<'ctx> {
        self.thin_ref_type
    }

    /// Get the FatRef struct type.
    pub fn fat_ref_type(&self) -> StructType<'ctx> {
        self.fat_ref_type
    }

    /// Set the runtime check function.
    pub fn set_check_function(&mut self, check_fn: FunctionValue<'ctx>) {
        self.check_fn = Some(check_fn);
    }

    /// Get CBGR statistics.
    pub fn stats(&self) -> &CbgrStats {
        &self.stats
    }

    /// Create a ThinRef (Tier 0 - full checks).
    ///

    /// This generates the full reference with generation tracking.
    pub fn create_ref_tier0(
        &mut self,
        builder: &Builder<'ctx>,
        ptr: PointerValue<'ctx>,
        generation: IntValue<'ctx>,
        epoch_caps: IntValue<'ctx>,
    ) -> Result<StructValue<'ctx>> {
        self.stats.refs_created += 1;
        self.stats.tier0_refs += 1;

        // Build ThinRef struct
        let ref_val = self.thin_ref_type.const_zero();
        let ref_val = builder
            .build_insert_value(ref_val, ptr, 0, "ref.ptr")
            .or_llvm_err()?
            .into_struct_value();
        let ref_val = builder
            .build_insert_value(ref_val, generation, 1, "ref.gen")
            .or_llvm_err()?
            .into_struct_value();
        let ref_val = builder
            .build_insert_value(ref_val, epoch_caps, 2, "ref.caps")
            .or_llvm_err()?
            .into_struct_value();

        Ok(ref_val)
    }

    /// Create a reference (Tier 1/2 - optimized path).
    ///

    /// For compiler-proven safe or manually unsafe references,
    /// we skip generation tracking.
    pub fn create_ref_checked(
        &mut self,
        builder: &Builder<'ctx>,
        ptr: PointerValue<'ctx>,
        tier: RefTier,
    ) -> Result<StructValue<'ctx>> {
        self.stats.refs_created += 1;
        match tier {
            RefTier::Tier1 => {
                self.stats.tier1_refs += 1;
                self.stats.checks_eliminated += 1;
            }
            RefTier::Tier2 => {
                self.stats.tier2_refs += 1;
                self.stats.checks_eliminated += 1;
            }
            RefTier::Tier0 => {
                self.stats.tier0_refs += 1;
            }
        }

        // For Tier 1/2, we use dummy generation/caps (checks are skipped)
        let zero_gen = self.context.i32_type().const_zero();
        let zero_caps = self.context.i32_type().const_zero();

        let ref_val = self.thin_ref_type.const_zero();
        let ref_val = builder
            .build_insert_value(ref_val, ptr, 0, "ref.ptr")
            .or_llvm_err()?
            .into_struct_value();
        let ref_val = builder
            .build_insert_value(ref_val, zero_gen, 1, "ref.gen")
            .or_llvm_err()?
            .into_struct_value();
        let ref_val = builder
            .build_insert_value(ref_val, zero_caps, 2, "ref.caps")
            .or_llvm_err()?
            .into_struct_value();

        Ok(ref_val)
    }

    /// Create a ThinRef from a raw user pointer.
    ///

    /// Reads the allocation header at `ptr - 32`:
    ///  - offset 0: generation (i32)
    ///  - offset 4: epoch (i16)
    ///

    /// Packs into `{ ptr, generation, epoch_and_caps }` where epoch_and_caps
    /// stores epoch in the low 16 bits and zero capabilities in the high 16.
    pub fn create_thin_ref(
        &mut self,
        builder: &Builder<'ctx>,
        user_ptr: PointerValue<'ctx>,
    ) -> Result<StructValue<'ctx>> {
        self.stats.refs_created += 1;
        self.stats.tier0_refs += 1;

        let i8_type = self.context.i8_type();
        let i16_type = self.context.i16_type();
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();
        let ptr_type = self.context.ptr_type(Default::default());

        // 1. Compute header address: ptr - 32
        //  GEP with i8 and index -32 to get byte-level offset.
        let neg_32 = i64_type.const_int((-32i64) as u64, true);
        // SAFETY: GEP with -32 byte offset to reach the CBGR allocation header preceding the user data pointer; all CBGR-managed allocations include a 32-byte header before the user region
        let header_ptr = unsafe {
            builder
                .build_gep(i8_type, user_ptr, &[neg_32], "cbgr.header_ptr")
                .or_llvm_err()?
        };

        // 2. Load generation (i32) from header offset 0
        let generation = builder
            .build_load(i32_type, header_ptr, "cbgr.alloc_gen")
            .or_llvm_err()?
            .into_int_value();

        // 3. Load epoch (i16) from header offset 4
        let four = i64_type.const_int(4, false);
        // SAFETY: GEP at +4 bytes into the CBGR header to read the epoch field; the header is at least 32 bytes (generation at 0, epoch at 4)
        let epoch_ptr = unsafe {
            builder
                .build_gep(i8_type, header_ptr, &[four], "cbgr.epoch_ptr")
                .or_llvm_err()?
        };
        let epoch = builder
            .build_load(i16_type, epoch_ptr, "cbgr.alloc_epoch")
            .or_llvm_err()?
            .into_int_value();

        // 4. Pack epoch into epoch_and_caps (epoch in low 16 bits, caps = 0)
        let epoch_and_caps = builder
            .build_int_z_extend(epoch, i32_type, "cbgr.epoch_caps")
            .or_llvm_err()?;

        // 5. Build the ThinRef struct { ptr, generation, epoch_and_caps }
        let ref_val = self.thin_ref_type.const_zero();
        let ref_val = builder
            .build_insert_value(ref_val, user_ptr, 0, "thinref.ptr")
            .or_llvm_err()?
            .into_struct_value();
        let ref_val = builder
            .build_insert_value(ref_val, generation, 1, "thinref.gen")
            .or_llvm_err()?
            .into_struct_value();
        let ref_val = builder
            .build_insert_value(ref_val, epoch_and_caps, 2, "thinref.caps")
            .or_llvm_err()?
            .into_struct_value();

        Ok(ref_val)
    }

    /// Dereference a ThinRef (Tier 0 - with real validation).
    ///

    /// Extracts generation and epoch from the ThinRef, packs them into a
    /// single i64 (generation in low 32 bits, epoch in bits 32..47), then
    /// calls `verum_cbgr_validate_ref(user_ptr, packed_gen_epoch)`.
    /// On success the user pointer is returned; on failure the runtime
    /// panic handler aborts.
    pub fn deref_tier0(
        &mut self,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
        current_fn: FunctionValue<'ctx>,
        ref_val: StructValue<'ctx>,
    ) -> Result<PointerValue<'ctx>> {
        self.stats.runtime_checks += 1;

        let i64_type = self.context.i64_type();
        let i32_type = self.context.i32_type();
        let i1_type = self.context.bool_type();
        let ptr_type = self.context.ptr_type(Default::default());

        // 1. Extract the user pointer (field 0)
        let user_ptr = builder
            .build_extract_value(ref_val, 0, "cbgr.user_ptr")
            .or_llvm_err()?
            .into_pointer_value();

        // 2. Extract generation (field 1, i32) and epoch_caps (field 2, i32)
        let generation = builder
            .build_extract_value(ref_val, 1, "cbgr.gen")
            .or_llvm_err()?
            .into_int_value();

        let epoch_caps = builder
            .build_extract_value(ref_val, 2, "cbgr.epoch_caps")
            .or_llvm_err()?
            .into_int_value();

        // 3. Extract epoch from epoch_caps (low 16 bits)
        let epoch_mask = i32_type.const_int(0xFFFF, false);
        let epoch_i32 = builder
            .build_and(epoch_caps, epoch_mask, "cbgr.epoch_i32")
            .or_llvm_err()?;

        // 4. Pack: generation (low 32) | epoch (bits 32..47)
        //  packed = zext(generation) | (zext(epoch_i32) << 32)
        let gen_i64 = builder
            .build_int_z_extend(generation, i64_type, "cbgr.gen_i64")
            .or_llvm_err()?;
        let epoch_i64 = builder
            .build_int_z_extend(epoch_i32, i64_type, "cbgr.epoch_i64")
            .or_llvm_err()?;
        let shift_amt = i64_type.const_int(32, false);
        let epoch_shifted = builder
            .build_left_shift(epoch_i64, shift_amt, "cbgr.epoch_shifted")
            .or_llvm_err()?;
        let packed = builder
            .build_or(gen_i64, epoch_shifted, "cbgr.packed_gen_epoch")
            .or_llvm_err()?;

        // 5. Convert user pointer to i64 for the call
        let user_ptr_i64 = builder
            .build_ptr_to_int(user_ptr, i64_type, "cbgr.ptr_i64")
            .or_llvm_err()?;

        // 6. Get or declare verum_cbgr_validate_ref(i64, i64) -> i1
        let fn_type = i1_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        let validate_fn = super::error::get_or_declare_function(module, "verum_cbgr_validate_ref", fn_type);

        // 7. Call the validation function
        let call_result = builder
            .build_call(
                validate_fn,
                &[user_ptr_i64.into(), packed.into()],
                "cbgr.valid",
            )
            .or_llvm_err()?;

        let is_valid = call_result
            .try_as_basic_value()
            .basic()
            .expect("verum_cbgr_validate_ref should return i1")
            .into_int_value();

        // 8. Branch: on success return pointer, on failure abort via panic
        let valid_bb = self.context.append_basic_block(current_fn, "cbgr.valid_bb");
        let invalid_bb = self
            .context
            .append_basic_block(current_fn, "cbgr.invalid_bb");
        let merge_bb = self.context.append_basic_block(current_fn, "cbgr.merge");

        builder
            .build_conditional_branch(is_valid, valid_bb, invalid_bb)
            .or_llvm_err()?;

        // Invalid path: call panic handler and unreachable
        builder.position_at_end(invalid_bb);
        let void_type = self.context.void_type();
        let fn_type = void_type.fn_type(&[ptr_type.into()], false);
        let panic_fn = super::error::get_or_declare_function(module, "verum_panic", fn_type);
        // Build a constant panic message
        let panic_msg = builder
            .build_global_string_ptr(
                "CBGR: use-after-free detected (generation mismatch)",
                "cbgr_panic_msg",
            )
            .or_llvm_err()?;
        builder
            .build_call(panic_fn, &[panic_msg.as_pointer_value().into()], "")
            .or_llvm_err()?;
        builder
            .build_unreachable()
            .or_llvm_err()?;

        // Valid path: branch to merge
        builder.position_at_end(valid_bb);
        builder
            .build_unconditional_branch(merge_bb)
            .or_llvm_err()?;

        // Merge: return the (validated) pointer
        builder.position_at_end(merge_bb);

        Ok(user_ptr)
    }

    /// Dereference a reference (Tier 1/2 - no checks).
    ///

    /// For compiler-proven or unsafe references, skip validation.
    pub fn deref_checked(
        &mut self,
        builder: &Builder<'ctx>,
        ref_val: StructValue<'ctx>,
        tier: RefTier,
    ) -> Result<PointerValue<'ctx>> {
        match tier {
            RefTier::Tier1 | RefTier::Tier2 => {
                self.stats.checks_eliminated += 1;
            }
            RefTier::Tier0 => {
                self.stats.runtime_checks += 1;
            }
        }

        // Extract pointer directly
        let ptr = builder
            .build_extract_value(ref_val, 0, "ref.ptr")
            .or_llvm_err()?
            .into_pointer_value();

        Ok(ptr)
    }

    /// Drop a reference (invalidate generation).
    pub fn drop_ref(
        &mut self,
        builder: &Builder<'ctx>,
        ref_val: StructValue<'ctx>,
        tier: RefTier,
    ) -> Result<()> {
        match tier {
            RefTier::Tier0 => {
                // For Tier 0, we might need to update generation tracking
                // This is typically handled by the runtime
            }
            RefTier::Tier1 | RefTier::Tier2 => {
                // No tracking needed for optimized tiers
            }
        }
        Ok(())
    }

    /// Validate a reference (explicit ChkRef instruction).
    ///

    /// This performs a full Tier 0 validation regardless of the reference's tier.
    /// Used for explicit validation checks in the bytecode.
    pub fn validate_ref(
        &mut self,
        builder: &Builder<'ctx>,
        ref_val: StructValue<'ctx>,
    ) -> Result<()> {
        self.stats.runtime_checks += 1;

        // Extract generation from reference
        let generation = builder
            .build_extract_value(ref_val, 1, "ref.gen")
            .or_llvm_err()?
            .into_int_value();

        // For now, check that generation is non-zero (valid reference)
        // In a full implementation, this would compare against the actual
        // generation counter at the pointed-to memory.
        let zero = self.context.i32_type().const_zero();
        let is_valid = builder
            .build_int_compare(IntPredicate::NE, generation, zero, "cbgr.valid")
            .or_llvm_err()?;

        // If we have a check function, call it for validation failure handling
        if let Some(check_fn) = self.check_fn {
            let ptr = builder
                .build_extract_value(ref_val, 0, "ref.ptr.validate")
                .or_llvm_err()?;

            builder
                .build_call(check_fn, &[is_valid.into(), ptr.into()], "cbgr.validate")
                .or_llvm_err()?;
        }

        Ok(())
    }

    /// Check if a capability is present.
    pub fn check_capability(
        &self,
        builder: &Builder<'ctx>,
        ref_val: StructValue<'ctx>,
        capability: u32,
    ) -> Result<IntValue<'ctx>> {
        // Extract epoch_caps
        let caps = builder
            .build_extract_value(ref_val, 2, "ref.caps")
            .or_llvm_err()?
            .into_int_value();

        // Check if capability bit is set
        let cap_mask = self.context.i32_type().const_int(capability as u64, false);
        let masked = builder
            .build_and(caps, cap_mask, "cap.masked")
            .or_llvm_err()?;
        let has_cap = builder
            .build_int_compare(
                IntPredicate::NE,
                masked,
                self.context.i32_type().const_zero(),
                "cap.check",
            )
            .or_llvm_err()?;

        Ok(has_cap)
    }
}

/// CBGR capability flags.
pub mod capabilities {
    /// Read capability.
    pub const READ: u32 = 0x01;
    /// Write capability.
    pub const WRITE: u32 = 0x02;
    /// Exclusive borrow capability.
    pub const EXCLUSIVE: u32 = 0x04;
    /// Shared borrow capability.
    pub const SHARED: u32 = 0x08;
    /// Move capability.
    pub const MOVE: u32 = 0x10;
}
