//! Atomic retain is the final fallible transition: allocation precedes it.
use super::*;

fn guard(ctx: &FunctionContext<'_, '_>, condition: IntValue<'_>, message: &str) -> Result<()> {
    let context = ctx.llvm_context();
    let passed = context.append_basic_block(ctx.function(), "retain_valid");
    let failed = context.append_basic_block(ctx.function(), "retain_invalid");
    ctx.builder()
        .build_conditional_branch(condition, passed, failed)
        .or_llvm_err()?;
    ctx.builder().position_at_end(failed);
    emit_runtime_abort(ctx, message, "retain_failure")?;
    ctx.builder().build_unreachable().or_llvm_err()?;
    ctx.builder().position_at_end(passed);
    Ok(())
}

/// Actual reference carriers are authoritative. A ThinRef is validated with the
/// canonical packed epoch/capabilities layout; plain addresses come from checked
/// native slot producers or the caller's explicit unsafe-reference contract.
fn validate_reference<'ctx>(
    ctx: &mut FunctionContext<'_, 'ctx>,
    value: BasicValueEnum<'ctx>,
    write: bool,
) -> Result<()> {
    let BasicValueEnum::StructValue(reference) = value else {
        return Ok(());
    };
    let required = if write {
        verum_common::cbgr::caps::WRITE
    } else {
        verum_common::cbgr::caps::READ
    };
    let module = ctx.get_module();
    let (builder, cbgr) = ctx.builder_and_cbgr();
    let valid = cbgr.reference_is_valid(builder, module, reference, required)?;
    guard(
        ctx,
        valid,
        "retain_copy reference is stale or lacks its required capability",
    )
}

pub(super) fn lower<'ctx>(
    ctx: &mut FunctionContext<'_, 'ctx>,
    dst: Reg,
    src: Reg,
    counter: Reg,
    ty: &TypeRef,
) -> Result<()> {
    let module = ctx
        .vbc_module()
        .or_internal("retain_copy requires declaration metadata")?;
    let layout = match verum_vbc::retain_copy::layout(module, ty) {
        Ok(layout) => layout,
        Err(reason) if !ty.is_generic() => return Err(LlvmLoweringError::Internal(reason)),
        Err(reason) => {
            emit_runtime_abort(ctx, &reason, "retain_unresolved")?;
            ctx.set_register(dst.0, ctx.types().i64_type().const_zero().into());
            return Ok(());
        }
    };
    let source = ctx.get_register(src.0)?;
    validate_reference(ctx, source, false)?;
    let source = ctx.reference_value_view(src.0, source)?;
    let source = as_ptr(ctx, source, "retain_handle")?;
    let counter_value = ctx.get_register(counter.0)?;
    validate_reference(ctx, counter_value, true)?;
    let counter_value =
        field_address_or_value(ctx, counter.0, counter_value, "retain_counter_slot")?;
    let counter_pointer = untag_slot_ptr(ctx, counter_value, "retain_counter")?;
    let c = ctx.llvm_context();
    let b = ctx.builder();
    let i64_type = c.i64_type();
    let i32_type = c.i32_type();
    let address = b
        .build_ptr_to_int(counter_pointer, i64_type, "retain_counter_word")
        .or_llvm_err()?;
    guard(
        ctx,
        b.build_is_not_null(source, "retain_handle_live")
            .or_llvm_err()?,
        "retain_copy has a null handle",
    )?;
    guard(
        ctx,
        b.build_is_not_null(counter_pointer, "retain_counter_live")
            .or_llvm_err()?,
        "retain_copy has a null counter",
    )?;
    let alignment = b
        .build_and(
            address,
            i64_type.const_int(3, false),
            "retain_counter_alignment",
        )
        .or_llvm_err()?;
    guard(
        ctx,
        b.build_int_compare(
            IntPredicate::EQ,
            alignment,
            i64_type.const_zero(),
            "retain_aligned",
        )
        .or_llvm_err()?,
        "retain_copy counter is misaligned",
    )?;
    // SAFETY: the actual reference carrier establishes a live object header.
    let owner_slot = unsafe {
        b.build_in_bounds_gep(
            c.i8_type(),
            source,
            &[i64_type.const_int(verum_common::layout::OBJECT_HEADER_TYPE_ID_OFFSET, false)],
            "retain_owner_slot",
        )
        .or_llvm_err()?
    };
    let actual_owner = b
        .build_load(i32_type, owner_slot, "retain_owner")
        .or_llvm_err()?
        .into_int_value();
    guard(
        ctx,
        b.build_int_compare(
            IntPredicate::EQ,
            actual_owner,
            i32_type.const_int(u64::from(layout.owner.0), false),
            "retain_exact_owner",
        )
        .or_llvm_err()?,
        "retain_copy has a different handle owner",
    )?;
    let bytes = RuntimeLowering::OBJECT_HEADER_SIZE
        + layout.fields as u64 * verum_common::layout::VALUE_SLOT_SIZE;
    // SAFETY: a live record's header contains its total allocation size.
    let size_slot = unsafe {
        b.build_in_bounds_gep(
            c.i8_type(),
            source,
            &[i64_type.const_int(verum_common::layout::OBJECT_HEADER_SIZE_OFFSET, false)],
            "retain_size_slot",
        )
        .or_llvm_err()?
    };
    let actual_size = b
        .build_load(i32_type, size_slot, "retain_size")
        .or_llvm_err()?
        .into_int_value();
    guard(
        ctx,
        b.build_int_compare(
            IntPredicate::EQ,
            actual_size,
            i32_type.const_int(bytes, false),
            "retain_exact_size",
        )
        .or_llvm_err()?,
        "retain_copy handle has an invalid extent",
    )?;
    let runtime = RuntimeLowering::new(c);
    // Use the canonical fresh-record allocator. The new wrapper must not
    // inherit generation, flags or reference bookkeeping from the source.
    let output = runtime.lower_new_object(b, ctx.get_module(), layout.fields as u32)?;
    b.build_store(output, actual_owner).or_llvm_err()?;
    // SAFETY: both source and destination have the checked declared extent.
    let (output_size, output_data, source_data) = unsafe {
        (
            b.build_in_bounds_gep(
                c.i8_type(),
                output,
                &[i64_type.const_int(verum_common::layout::OBJECT_HEADER_SIZE_OFFSET, false)],
                "retained_size",
            )
            .or_llvm_err()?,
            b.build_in_bounds_gep(
                c.i8_type(),
                output,
                &[i64_type.const_int(RuntimeLowering::OBJECT_HEADER_SIZE, false)],
                "retained_data",
            )
            .or_llvm_err()?,
            b.build_in_bounds_gep(
                c.i8_type(),
                source,
                &[i64_type.const_int(RuntimeLowering::OBJECT_HEADER_SIZE, false)],
                "retain_source_data",
            )
            .or_llvm_err()?,
        )
    };
    b.build_store(output_size, actual_size).or_llvm_err()?;
    b.build_call(
        runtime.get_or_declare_memcpy(ctx.get_module()),
        &[
            output_data.into(),
            source_data.into(),
            i64_type
                .const_int(bytes - RuntimeLowering::OBJECT_HEADER_SIZE, false)
                .into(),
        ],
        "",
    )
    .or_llvm_err()?;
    let cache = ctx.retained_counters.as_ref().or_internal("retain_copy requires module-owned lowering state")?;
    let (transition, _) = cache.get()?;
    let success = b.build_call(transition, &[counter_pointer.into()], "retained")
        .or_llvm_err()?.try_as_basic_value().basic().or_internal("retained counter result")?.into_int_value();
    let failed = c.append_basic_block(ctx.function(), "retain_count_refused");
    let done = c.append_basic_block(ctx.function(), "retain_done");
    b.build_conditional_branch(success, done, failed).or_llvm_err()?;
    b.position_at_end(failed);
    let release = super::super::error::get_or_declare_function(
        ctx.get_module(),
        "verum_os_free",
        c.void_type().fn_type(
            &[c.ptr_type(Default::default()).into(), i64_type.into()],
            false,
        ),
    );
    b.build_call(
        release,
        &[output.into(), i64_type.const_int(bytes, false).into()],
        "",
    )
    .or_llvm_err()?;
    emit_runtime_abort(
        ctx,
        "retain_copy requires a nonzero, non-overflowing UInt32 counter",
        "retain_count_failure",
    )?;
    b.build_unreachable().or_llvm_err()?;
    b.position_at_end(done);
    // Publication is infallible. No allocator or user callback follows the CAS.
    ctx.set_register(dst.0, output.into());
    Ok(())
}
