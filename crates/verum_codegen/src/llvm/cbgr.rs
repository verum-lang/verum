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

use verum_llvm::builder::Builder;
use verum_llvm::context::Context;
use verum_llvm::module::Module;
use verum_llvm::types::StructType;
use verum_llvm::values::{BasicValue, FunctionValue, IntValue, PointerValue, StructValue};
use verum_llvm::{AtomicOrdering, IntPredicate};

use super::error::{BuildExt, CallSiteExt, LlvmLoweringError, OptionExt, Result};
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
    /// Reads the allocation header at `ptr - 32` through the canonical
    /// layout (`verum_common::layout::ALLOCATION_HEADER_*`):
    ///  - generation @ 8 (i32)
    ///  - epoch      @ 12 (i16)
    ///
    /// This constructor used to read `gen@0 / epoch@4` — a third,
    /// private arrangement that agreed with `validate_ref` and with
    /// NOTHING else: the allocator put `size` at 4, so the "epoch" a
    /// ThinRef carried was the low half of the allocation size, and the
    /// downstream epoch comparison validated `size == size` (T0846
    /// layer 2 — the Tier-1 epoch check was a tautology).
    ///
    /// Packs into `{ ptr, generation, epoch_and_caps }` where epoch_and_caps
    /// stores epoch in the upper half and allocation capabilities in the lower half.
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

        // 1. Compute header address: ptr - 32
        //  GEP with i8 and index -32 to get byte-level offset.
        let neg_32 = i64_type.const_int(
            (-(verum_common::layout::ALLOCATION_HEADER_SIZE as i64)) as u64,
            true,
        );
        // SAFETY: GEP with -32 byte offset to reach the CBGR allocation header preceding the user data pointer; all CBGR-managed allocations include a 32-byte header before the user region
        let header_ptr = unsafe {
            builder
                .build_gep(i8_type, user_ptr, &[neg_32], "cbgr.header_ptr")
                .or_llvm_err()?
        };

        // 2. Load generation (i32) from the canonical gen@8 slot
        // SAFETY: fixed in-header offset within the 32-byte header.
        let gen_ptr = unsafe {
            builder
                .build_gep(
                    i8_type,
                    header_ptr,
                    &[i64_type.const_int(
                        verum_common::layout::ALLOCATION_HEADER_GENERATION_OFFSET,
                        false,
                    )],
                    "cbgr.gen_ptr",
                )
                .or_llvm_err()?
        };
        let generation = builder
            .build_load(i32_type, gen_ptr, "cbgr.alloc_gen")
            .or_llvm_err()?;
        generation
            .as_instruction_value()
            .or_internal("generation load")?
            .set_atomic_ordering(AtomicOrdering::Acquire)
            .map_err(|_| LlvmLoweringError::internal("generation load ordering"))?;
        let generation = generation.into_int_value();

        // 3. Load epoch (i16) from the canonical epoch@12 slot
        // SAFETY: fixed in-header offset within the 32-byte header.
        let epoch_ptr = unsafe {
            builder
                .build_gep(
                    i8_type,
                    header_ptr,
                    &[i64_type
                        .const_int(verum_common::layout::ALLOCATION_HEADER_EPOCH_OFFSET, false)],
                    "cbgr.epoch_ptr",
                )
                .or_llvm_err()?
        };
        let epoch = builder
            .build_load(i16_type, epoch_ptr, "cbgr.alloc_epoch")
            .or_llvm_err()?;
        epoch
            .as_instruction_value()
            .or_internal("epoch load")?
            .set_atomic_ordering(AtomicOrdering::Monotonic)
            .map_err(|_| LlvmLoweringError::internal("epoch load ordering"))?;
        // SAFETY: the caller supplies a live CBGR allocation, whose canonical
        // header includes the complete atomic capability field.
        let caps_ptr = unsafe {
            builder
                .build_gep(
                    i8_type,
                    header_ptr,
                    &[i64_type.const_int(
                        verum_common::layout::ALLOCATION_HEADER_CAPABILITIES_OFFSET,
                        false,
                    )],
                    "cbgr.caps_ptr",
                )
                .or_llvm_err()?
        };
        let caps = builder
            .build_load(i16_type, caps_ptr, "cbgr.alloc_caps")
            .or_llvm_err()?;
        caps.as_instruction_value()
            .or_internal("capability load")?
            .set_atomic_ordering(AtomicOrdering::Acquire)
            .map_err(|_| LlvmLoweringError::internal("capability load ordering"))?;
        let epoch = builder
            .build_int_z_extend(epoch.into_int_value(), i32_type, "cbgr.epoch32")
            .or_llvm_err()?;
        let caps = builder
            .build_int_z_extend(caps.into_int_value(), i32_type, "cbgr.caps32")
            .or_llvm_err()?;
        let shifted = builder
            .build_left_shift(
                epoch,
                i32_type.const_int(u64::from(verum_common::layout::CAPS_BITS), false),
                "cbgr.epoch_shifted",
            )
            .or_llvm_err()?;
        let epoch_and_caps = builder
            .build_or(shifted, caps, "cbgr.epoch_caps")
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

    /// Validate a real reference snapshot against its still-addressable CBGR
    /// allocation header and the intersection of snapshot/current capabilities.
    /// This does not establish lifetime for an already released address: callers
    /// must keep the allocation metadata addressable during the check.
    pub fn reference_is_valid(
        &mut self,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
        reference: StructValue<'ctx>,
        required: u32,
    ) -> Result<IntValue<'ctx>> {
        self.stats.runtime_checks += 1;
        let c = self.context;
        let pointer = builder
            .build_extract_value(reference, 0, "cbgr.pointer")
            .or_llvm_err()?
            .into_pointer_value();
        let generation = builder
            .build_extract_value(reference, 1, "cbgr.generation")
            .or_llvm_err()?
            .into_int_value();
        let epoch_caps = builder
            .build_extract_value(reference, 2, "cbgr.epoch_caps")
            .or_llvm_err()?
            .into_int_value();
        let caps_ok = self.check_capability(builder, reference, required)?;
        let present = builder
            .build_is_not_null(pointer, "cbgr.present")
            .or_llvm_err()?;
        let live_generation = builder
            .build_int_compare(
                IntPredicate::NE,
                generation,
                c.i32_type().const_zero(),
                "cbgr.nonzero_generation",
            )
            .or_llvm_err()?;
        let candidate = builder
            .build_and(present, live_generation, "cbgr.candidate")
            .or_llvm_err()?;
        let candidate = builder
            .build_and(candidate, caps_ok, "cbgr.authorized_candidate")
            .or_llvm_err()?;
        let from = builder
            .get_insert_block()
            .or_internal("reference validation block")?;
        let function = from
            .get_parent()
            .or_internal("reference validation function")?;
        let check = c.append_basic_block(function, "cbgr.check_allocation");
        let done = c.append_basic_block(function, "cbgr.validation_done");
        builder
            .build_conditional_branch(candidate, check, done)
            .or_llvm_err()?;
        builder.position_at_end(check);
        let epoch = builder
            .build_right_shift(
                epoch_caps,
                c.i32_type()
                    .const_int(u64::from(verum_common::layout::CAPS_BITS), false),
                false,
                "cbgr.epoch",
            )
            .or_llvm_err()?;
        let epoch = builder
            .build_int_z_extend(epoch, c.i64_type(), "cbgr.epoch64")
            .or_llvm_err()?;
        let epoch = builder
            .build_left_shift(
                epoch,
                c.i64_type().const_int(32, false),
                "cbgr.packed_epoch",
            )
            .or_llvm_err()?;
        let generation = builder
            .build_int_z_extend(generation, c.i64_type(), "cbgr.generation64")
            .or_llvm_err()?;
        let packed = builder
            .build_or(epoch, generation, "cbgr.generation_epoch")
            .or_llvm_err()?;
        let address = builder
            .build_ptr_to_int(pointer, c.i64_type(), "cbgr.address")
            .or_llvm_err()?;
        let validate = super::error::get_or_declare_function(
            module,
            "verum_cbgr_validate_ref",
            c.bool_type()
                .fn_type(&[c.i64_type().into(), c.i64_type().into()], false),
        );
        let live = builder
            .build_call(validate, &[address.into(), packed.into()], "cbgr.live")
            .or_llvm_err()?
            .basic_value_or("CBGR validity")?
            .into_int_value();
        // SAFETY: this branch has a nonnull reference into an allocation whose
        // metadata remains addressable by the caller's lifetime contract.
        let caps_pointer = unsafe {
            builder
                .build_gep(
                    c.i8_type(),
                    pointer,
                    &[c.i64_type().const_int(
                        (verum_common::layout::ALLOCATION_HEADER_CAPABILITIES_OFFSET as i64
                            - verum_common::layout::ALLOCATION_HEADER_SIZE as i64)
                            as u64,
                        true,
                    )],
                    "cbgr.current_caps_pointer",
                )
                .or_llvm_err()?
        };
        let current_caps = builder
            .build_load(c.i16_type(), caps_pointer, "cbgr.current_caps")
            .or_llvm_err()?;
        current_caps
            .as_instruction_value()
            .or_internal("current capability load")?
            .set_atomic_ordering(AtomicOrdering::Acquire)
            .map_err(|_| LlvmLoweringError::internal("current capability ordering"))?;
        let current_caps = builder
            .build_int_z_extend(
                current_caps.into_int_value(),
                c.i32_type(),
                "cbgr.current_caps32",
            )
            .or_llvm_err()?;
        let mask = c.i32_type().const_int(u64::from(required), false);
        let current = builder
            .build_and(current_caps, mask, "cbgr.current_required")
            .or_llvm_err()?;
        let current_ok = builder
            .build_int_compare(IntPredicate::EQ, current, mask, "cbgr.current_authority")
            .or_llvm_err()?;
        let valid = builder
            .build_and(live, current_ok, "cbgr.valid")
            .or_llvm_err()?;
        builder.build_unconditional_branch(done).or_llvm_err()?;
        builder.position_at_end(done);
        let result = builder
            .build_phi(c.bool_type(), "cbgr.result")
            .or_llvm_err()?;
        result.add_incoming(&[(&c.bool_type().const_zero(), from), (&valid, check)]);
        Ok(result.as_basic_value().into_int_value())
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
        let is_valid = self.reference_is_valid(builder, module, ref_val, capabilities::READ)?;
        let user_ptr = builder
            .build_extract_value(ref_val, 0, "cbgr.user_ptr")
            .or_llvm_err()?
            .into_pointer_value();
        let ptr_type = self.context.ptr_type(Default::default());

        // 8. Branch: on success return pointer, on failure abort via panic
        let valid_bb = self.context.append_basic_block(current_fn, "cbgr.valid_bb");
        let invalid_bb = self
            .context
            .append_basic_block(current_fn, "cbgr.invalid_bb");
        let merge_bb = self.context.append_basic_block(current_fn, "cbgr.merge");

        builder
            .build_conditional_branch(is_valid, valid_bb, invalid_bb)
            .or_llvm_err()?;

        // Invalid path: call the panic handler and unreachable.
        //
        // verum_panic's CANONICAL ABI is 4-param `void(ptr msg, i64 len,
        // ptr file, i32 line)` — the body is defined with exactly that shape
        // in `emit_panic_ir`. A 1-param `void(ptr)` declaration here was a
        // same-name IMPOSTOR (task #22 class): whichever site declares first
        // wins, and a 1-param winner both (a) makes `emit_panic_ir` skip the
        // body definition (`count_params() < 2`) and (b) leaves every 4-param
        // call ABI-mismatched (the AOT panic-crash surfaced in #21/#26 as
        // `verum_panic void(ptr)` vs `void(ptr,i64,ptr,i32)`). Declare and
        // call the canonical 4-param shape so no impostor can win.
        builder.position_at_end(invalid_bb);
        let void_type = self.context.void_type();
        let i64_type = self.context.i64_type();
        let i32_type = self.context.i32_type();
        let fn_type = void_type.fn_type(
            &[
                ptr_type.into(),
                i64_type.into(),
                ptr_type.into(),
                i32_type.into(),
            ],
            false,
        );
        let panic_fn = super::error::get_or_declare_function(module, "verum_panic", fn_type);
        // Constant panic message; `len` is its byte length so the body can
        // write it without a strlen.
        const CBGR_PANIC_MSG: &str = "CBGR: invalid generation, epoch or read capability";
        let panic_msg = builder
            .build_global_string_ptr(CBGR_PANIC_MSG, "cbgr_panic_msg")
            .or_llvm_err()?;
        builder
            .build_call(
                panic_fn,
                &[
                    panic_msg.as_pointer_value().into(),
                    i64_type
                        .const_int(CBGR_PANIC_MSG.len() as u64, false)
                        .into(),
                    ptr_type.const_null().into(),
                    i32_type.const_int(0, false).into(),
                ],
                "",
            )
            .or_llvm_err()?;
        builder.build_unreachable().or_llvm_err()?;

        // Valid path: branch to merge
        builder.position_at_end(valid_bb);
        builder.build_unconditional_branch(merge_bb).or_llvm_err()?;

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
                return Err(LlvmLoweringError::internal(
                    "Tier 0 dereference requires allocation validation",
                ));
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
        _builder: &Builder<'ctx>,
        _ref_val: StructValue<'ctx>,
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
        module: &Module<'ctx>,
        current_fn: FunctionValue<'ctx>,
        ref_val: StructValue<'ctx>,
    ) -> Result<()> {
        self.deref_tier0(builder, module, current_fn, ref_val)
            .map(|_| ())
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

        if capability & !verum_common::layout::CAPS_MASK_U32 != 0 {
            return Ok(self.context.bool_type().const_zero());
        }
        // Every requested capability must be present; one bit cannot grant a set.
        let cap_mask = self.context.i32_type().const_int(capability as u64, false);
        let masked = builder
            .build_and(caps, cap_mask, "cap.masked")
            .or_llvm_err()?;
        let has_cap = builder
            .build_int_compare(IntPredicate::EQ, masked, cap_mask, "cap.check")
            .or_llvm_err()?;

        Ok(has_cap)
    }
}

/// Canonical CBGR capability meanings; native lowering has no private roster.
pub use verum_common::cbgr::caps as capabilities;
