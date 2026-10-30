//! Native execution of the same declaration-owned cleanup plan as the VM.
use super::*;
use verum_vbc::drop_plan::DropPlan;

pub(super) fn lower<'ctx>(
    ctx: &mut FunctionContext<'_, 'ctx>,
    src: Reg,
    ty: &TypeRef,
) -> Result<()> {
    let module = ctx
        .vbc_module()
        .or_internal("DropTyped requires declaration metadata")?;
    let plan = verum_vbc::drop_plan::build(module, ty);
    let value = ctx.get_register(src.0)?;
    // Consume before invoking any user code, as in the interpreter.
    ctx.set_register(src.0, ctx.types().i64_type().const_zero().into());
    match plan {
        Ok(plan) => emit(ctx, value, &plan),
        Err(reason) if !ty.is_generic() => Err(LlvmLoweringError::UnprovenOwnedDestruction(reason)),
        Err(reason) => {
            // Erased originals coexist with monomorphized bodies. Calling an
            // unresolved original must refuse, never silently omit cleanup.
            emit_runtime_abort(ctx, &reason, "drop_unresolved")?;
            ctx.builder().build_unreachable().or_llvm_err()?;
            let dead = ctx
                .llvm_context()
                .append_basic_block(ctx.function(), "drop_dead");
            ctx.builder().position_at_end(dead);
            Ok(())
        }
    }
}

fn slot<'ctx>(
    ctx: &FunctionContext<'_, 'ctx>,
    pointer: PointerValue<'ctx>,
    offset: u64,
) -> Result<PointerValue<'ctx>> {
    // The plan addresses only declared fields of a compiler-created object.
    unsafe {
        ctx.builder()
            .build_in_bounds_gep(
                ctx.llvm_context().i8_type(),
                pointer,
                &[ctx.types().i64_type().const_int(offset, false)],
                "drop_slot",
            )
            .or_llvm_err()
    }
}

fn fields<'ctx>(
    ctx: &mut FunctionContext<'_, 'ctx>,
    pointer: PointerValue<'ctx>,
    offset: u64,
    fields: &[DropPlan],
) -> Result<()> {
    for (index, plan) in fields.iter().enumerate() {
        if matches!(plan, DropPlan::Ignore) {
            continue;
        }
        let pointer = slot(ctx, pointer, offset + index as u64 * 8)?;
        let value = ctx
            .builder()
            .build_load(ctx.types().i64_type(), pointer, "drop_component")
            .or_llvm_err()?;
        ctx.builder()
            .build_store(pointer, ctx.types().i64_type().const_zero())
            .or_llvm_err()?;
        emit(ctx, value, plan)?;
    }
    Ok(())
}

fn emit<'ctx>(
    ctx: &mut FunctionContext<'_, 'ctx>,
    value: BasicValueEnum<'ctx>,
    plan: &DropPlan,
) -> Result<()> {
    if matches!(plan, DropPlan::Ignore) {
        return Ok(());
    }
    let pointer = as_ptr(ctx, value, "drop_owner")?;
    let end = ctx
        .llvm_context()
        .append_basic_block(ctx.function(), "drop_done");
    let live = ctx
        .llvm_context()
        .append_basic_block(ctx.function(), "drop_live");
    let empty = ctx
        .builder()
        .build_is_null(pointer, "drop_consumed")
        .or_llvm_err()?;
    ctx.builder()
        .build_conditional_branch(empty, end, live)
        .or_llvm_err()?;
    ctx.builder().position_at_end(live);
    let owner = match plan {
        DropPlan::Glue { owner, .. }
        | DropPlan::Record { owner, .. }
        | DropPlan::Sum { owner, .. } => *owner,
        DropPlan::Ignore => unreachable!(),
    };
    let actual = ctx
        .builder()
        .build_load(ctx.llvm_context().i32_type(), pointer, "drop_nominal_owner")
        .or_llvm_err()?
        .into_int_value();
    let matches = ctx
        .builder()
        .build_int_compare(
            IntPredicate::EQ,
            actual,
            ctx.llvm_context()
                .i32_type()
                .const_int(u64::from(owner.0), false),
            "drop_owner_matches",
        )
        .or_llvm_err()?;
    let valid = ctx
        .llvm_context()
        .append_basic_block(ctx.function(), "drop_declared_owner");
    let invalid = ctx
        .llvm_context()
        .append_basic_block(ctx.function(), "drop_invalid_owner");
    ctx.builder()
        .build_conditional_branch(matches, valid, invalid)
        .or_llvm_err()?;
    ctx.builder().position_at_end(invalid);
    emit_runtime_abort(
        ctx,
        "typed destruction nominal owner mismatch",
        "drop_invalid_owner",
    )?;
    ctx.builder().build_unreachable().or_llvm_err()?;
    ctx.builder().position_at_end(valid);
    match plan {
        DropPlan::Ignore => unreachable!(),
        DropPlan::Glue {
            function,
            type_args,
            ..
        } => {
            let module = ctx.vbc_module().or_internal("DropTyped requires module")?;
            let target = verum_vbc::drop_plan::native_glue(module, *function, type_args)
                .and_then(|id| ctx.resolve_func_id(id.0))
                .ok_or_else(|| {
                    LlvmLoweringError::UnprovenOwnedDestruction(
                        "DropTyped has no exact native destructor specialization".into(),
                    )
                })?;
            let parameters = target.get_type().get_param_types();
            if parameters.len() != 1 {
                return Err(LlvmLoweringError::UnprovenOwnedDestruction(
                    "DropTyped destructor must have exactly one receiver".into(),
                ));
            }
            let expected =
                meta_type_to_basic(parameters[0]).or_internal("DropTyped invalid receiver ABI")?;
            let argument = coerce_value(ctx, pointer.into(), expected, "drop_receiver")?;
            if argument.get_type() != expected {
                return Err(LlvmLoweringError::UnprovenOwnedDestruction(
                    "DropTyped unsupported destructor receiver ABI".into(),
                ));
            }
            let call = ctx
                .builder()
                .build_call(target, &[argument.into()], "")
                .or_llvm_err()?;
            call.set_call_convention(target.get_call_conventions());
        }
        DropPlan::Record {
            fields: declared, ..
        } => fields(ctx, pointer, RuntimeLowering::OBJECT_HEADER_SIZE, declared)?,
        DropPlan::Sum { variants, .. } => {
            let tag_slot = slot(ctx, pointer, RuntimeLowering::OBJECT_HEADER_SIZE)?;
            let tag = ctx
                .builder()
                .build_load(ctx.llvm_context().i32_type(), tag_slot, "drop_tag")
                .or_llvm_err()?
                .into_int_value();
            let bad = ctx
                .llvm_context()
                .append_basic_block(ctx.function(), "drop_invalid_tag");
            let branches: Vec<_> = variants
                .iter()
                .map(|(tag, _)| {
                    (
                        ctx.llvm_context()
                            .i32_type()
                            .const_int(u64::from(*tag), false),
                        ctx.llvm_context()
                            .append_basic_block(ctx.function(), "drop_variant"),
                    )
                })
                .collect();
            ctx.builder()
                .build_switch(tag, bad, &branches)
                .or_llvm_err()?;
            for ((_, declared), (_, block)) in variants.iter().zip(&branches) {
                ctx.builder().position_at_end(*block);
                fields(
                    ctx,
                    pointer,
                    RuntimeLowering::OBJECT_HEADER_SIZE + 8,
                    declared,
                )?;
                ctx.builder()
                    .build_unconditional_branch(end)
                    .or_llvm_err()?;
            }
            ctx.builder().position_at_end(bad);
            emit_runtime_abort(
                ctx,
                "typed destruction sum tag is not declared",
                "drop_invalid_tag",
            )?;
            ctx.builder().build_unreachable().or_llvm_err()?;
            ctx.builder().position_at_end(end);
            return Ok(());
        }
    }
    ctx.builder()
        .build_unconditional_branch(end)
        .or_llvm_err()?;
    ctx.builder().position_at_end(end);
    Ok(())
}
