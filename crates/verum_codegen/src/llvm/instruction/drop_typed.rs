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
    if let Ok(DropPlan::Callable { signature }) = &plan
        && super::callable_lifecycle::release(ctx, src, signature)? {
        return Ok(());
    }
    let value = ctx.get_register(src.0)?;
    if matches!(plan, Ok(DropPlan::Text)) {
        // DropTyped names the consumed register. Comparing independently
        // emitted LLVM loads cannot identify its lifetime in alloca mode.
        ctx.unmark_owned_text_register(src.0);
    }
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

fn require_storage<'ctx>(ctx: &mut FunctionContext<'_, 'ctx>,
    valid: IntValue<'ctx>) -> Result<()> {
    let accepted = ctx.llvm_context().append_basic_block(ctx.function(), "drop_storage_valid");
    let refused = ctx.llvm_context().append_basic_block(ctx.function(), "drop_storage_refused");
    ctx.builder().build_conditional_branch(valid, accepted, refused).or_llvm_err()?;
    ctx.builder().position_at_end(refused);
    emit_runtime_abort(ctx, "typed destruction requires its exact producer-issued object receipt",
        "drop_object_receipt")?;
    ctx.builder().build_unreachable().or_llvm_err()?;
    ctx.builder().position_at_end(accepted);
    Ok(())
}

fn retire_fields<'ctx>(ctx: &mut FunctionContext<'_, 'ctx>, pointer: PointerValue<'ctx>,
    offset: u64, declared: &[DropPlan]) -> Result<()> {
    let bytes = offset + declared.len() as u64 * verum_common::layout::VALUE_SLOT_SIZE;
    let valid = super::super::object_storage::matches_os(
        ctx.llvm_context(), ctx.builder(), pointer, bytes)?;
    require_storage(ctx, valid)?;
    super::super::object_storage::consume(ctx.llvm_context(), ctx.builder(), pointer)?;
    fields(ctx, pointer, offset, declared)?;
    super::super::object_storage::release_os(ctx.llvm_context(), ctx.builder(),
        ctx.get_module(), pointer, bytes)
}

/// The bootstrap List has a concrete element witness but no callable source
/// destructor. Its storage producer owns the same initialized prefix as NewList.
fn retire_list_storage<'ctx>(
    ctx: &mut FunctionContext<'_, 'ctx>,
    pointer: PointerValue<'ctx>,
    element: &DropPlan,
) -> Result<()> {
    let context = ctx.llvm_context();
    let word = context.i64_type();
    let length_slot = slot(ctx, pointer, verum_common::layout::LIST_LEN_OFFSET)?;
    if matches!(element, DropPlan::Ignore) {
        ctx.builder()
            .build_store(length_slot, word.const_zero())
            .or_llvm_err()?;
    } else {
        let test = context.append_basic_block(ctx.function(), "drop_list_test");
        let body = context.append_basic_block(ctx.function(), "drop_list_element");
        let done = context.append_basic_block(ctx.function(), "drop_list_empty");
        ctx.builder()
            .build_unconditional_branch(test)
            .or_llvm_err()?;
        ctx.builder().position_at_end(test);
        let length = ctx
            .builder()
            .build_load(word, length_slot, "drop_list_remaining")
            .or_llvm_err()?
            .into_int_value();
        let empty = ctx
            .builder()
            .build_int_compare(
                IntPredicate::EQ,
                length,
                word.const_zero(),
                "drop_list_empty",
            )
            .or_llvm_err()?;
        ctx.builder()
            .build_conditional_branch(empty, done, body)
            .or_llvm_err()?;
        ctx.builder().position_at_end(body);
        let index = ctx
            .builder()
            .build_int_sub(length, word.const_int(1, false), "drop_list_last")
            .or_llvm_err()?;
        let read = ctx
            .get_module()
            .get_function("verum_list_storage_read")
            .or_missing_fn("verum_list_storage_read")?;
        let value = ctx
            .builder()
            .build_call(read, &[pointer.into(), index.into()], "drop_list_value")
            .or_llvm_err()?
            .basic_value_or("List storage read returned void")?;
        ctx.builder()
            .build_store(length_slot, index)
            .or_llvm_err()?;
        emit(ctx, value, element)?;
        ctx.builder()
            .build_unconditional_branch(test)
            .or_llvm_err()?;
        ctx.builder().position_at_end(done);
    }
    let resize = ctx
        .get_module()
        .get_function("verum_list_try_resize_storage")
        .or_missing_fn("verum_list_try_resize_storage")?;
    let released = ctx
        .builder()
        .build_call(
            resize,
            &[pointer.into(), word.const_zero().into()],
            "drop_list_backing",
        )
        .or_llvm_err()?
        .basic_value_or("List resize returned void")?
        .into_int_value();
    let valid = ctx
        .builder()
        .build_int_compare(
            IntPredicate::NE,
            released,
            word.const_zero(),
            "drop_list_released",
        )
        .or_llvm_err()?;
    require_storage(ctx, valid)?;
    super::super::object_storage::release_os(
        context,
        ctx.builder(),
        ctx.get_module(),
        pointer,
        verum_common::layout::LIST_OBJECT_SIZE,
    )
}

pub(super) fn emit<'ctx>(
    ctx: &mut FunctionContext<'_, 'ctx>,
    value: BasicValueEnum<'ctx>,
    plan: &DropPlan,
) -> Result<()> {
    if matches!(plan, DropPlan::Ignore) {
        return Ok(());
    }
    if matches!(plan, DropPlan::Text) {
        let word = as_i64(ctx, value, "drop_text_value")?;
        let release = super::super::error::get_or_declare_function(
            ctx.get_module(),
            "verum_text_free",
            ctx.types().void_type().fn_type(&[ctx.types().i64_type().into()], false),
        );
        ctx.builder().build_call(release, &[word.into()], "").or_llvm_err()?;
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
    if matches!(plan, DropPlan::Callable { .. }) {
        // Native headerless closure carriers do not retain the producer's
        // environment ownership receipt. Refuse on the executing path rather
        // than trusting pointer bits or allowing a fallback body to omit drop.
        emit_runtime_abort(ctx,
            "native callable cleanup requires a producer-issued environment ownership receipt",
            "drop_callable_receipt")?;
        ctx.builder().build_unreachable().or_llvm_err()?;
        ctx.builder().position_at_end(end);
        return Ok(());
    }
    let owner = match plan {
        DropPlan::ListStorage { .. } => TypeId::LIST,
        DropPlan::Glue { owner, .. }
        | DropPlan::Record { owner, .. }
        | DropPlan::Sum { owner, .. } => *owner,
        DropPlan::Ignore | DropPlan::Text | DropPlan::Callable { .. } => unreachable!(),
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
    let list_element = if matches!(plan, DropPlan::Glue { .. } | DropPlan::ListStorage { .. }) {
        plan.list_element(ctx.vbc_module().or_internal("DropTyped requires module")?)
            .map_err(LlvmLoweringError::UnprovenOwnedDestruction)?
    } else {
        None
    };
    let storage_list = list_element.is_some();
    if let Some(element) = list_element {
        // NewList and its packed-byte producer carry a physical encoding in
        // the header. Only the standard declaration's concrete destructor
        // may consume it, after checking the allocation and initialized range.
        let context = ctx.llvm_context();
        let alternate = context.append_basic_block(ctx.function(), "drop_list_encoding");
        let checked = context.append_basic_block(ctx.function(), "drop_list_allocation");
        // Reserved encoding IDs always refer to physical storage, even when
        // the linked source descriptor shares that ID.
        if owner.is_list_like() {
            ctx.builder()
                .build_unconditional_branch(alternate)
                .or_llvm_err()?;
        } else {
            ctx.builder()
                .build_conditional_branch(matches, valid, alternate)
                .or_llvm_err()?;
        }
        ctx.builder().position_at_end(alternate);
        let encoding_is = |id: TypeId| {
            ctx.builder()
                .build_int_compare(
                    IntPredicate::EQ,
                    actual,
                    context.i32_type().const_int(u64::from(id.0), false),
                    "drop_list_encoding_matches",
                )
                .or_llvm_err()
        };
        let slots = encoding_is(TypeId::LIST)?;
        let permitted = if element == &TypeRef::Concrete(TypeId::BYTE) {
            ctx.builder()
                .build_or(slots, encoding_is(TypeId::BYTE_LIST)?, "drop_byte_list")
                .or_llvm_err()?
        } else {
            slots
        };
        ctx.builder()
            .build_conditional_branch(permitted, checked, invalid)
            .or_llvm_err()?;
        ctx.builder().position_at_end(checked);
        let receipt = super::super::object_storage::matches_os(
            context,
            ctx.builder(),
            pointer,
            verum_common::layout::LIST_OBJECT_SIZE,
        )?;
        require_storage(ctx, receipt)?;
        let length = ctx
            .builder()
            .build_load(
                context.i64_type(),
                slot(ctx, pointer, verum_common::layout::LIST_LEN_OFFSET)?,
                "drop_list_length",
            )
            .or_llvm_err()?
            .into_int_value();
        let address = ctx
            .get_module()
            .get_function("verum_list_storage_address")
            .or_missing_fn("verum_list_storage_address")?;
        ctx.builder()
            .build_call(
                address,
                &[
                    pointer.into(),
                    context.i64_type().const_zero().into(),
                    length.into(),
                ],
                "drop_list_checked_prefix",
            )
            .or_llvm_err()?;
        super::super::object_storage::consume(context, ctx.builder(), pointer)?;
        ctx.builder()
            .build_unconditional_branch(valid)
            .or_llvm_err()?;
    } else {
        ctx.builder()
            .build_conditional_branch(matches, valid, invalid)
            .or_llvm_err()?;
    }
    ctx.builder().position_at_end(invalid);
    let name = ctx
        .vbc_module()
        .and_then(|module| module.get_type_name(owner));
    let function = ctx.function();
    emit_runtime_abort(
        ctx,
        &format!(
            "typed destruction nominal owner mismatch in {}: expected {} [#{}]",
            function.get_name().to_string_lossy(), name.as_deref().unwrap_or("<unnamed>"), owner.0,
        ),
        "drop_invalid_owner",
    )?;
    ctx.builder().build_unreachable().or_llvm_err()?;
    ctx.builder().position_at_end(valid);
    match plan {
        DropPlan::Ignore | DropPlan::Text | DropPlan::Callable { .. } => unreachable!(),
        DropPlan::ListStorage { element, .. } => retire_list_storage(ctx, pointer, element)?,
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
            if storage_list {
                // The source destructor retires the elements and their backing.
                // The native allocator separately owns the descriptor itself.
                if !owner.is_list_like() {
                    let release = ctx.llvm_context().append_basic_block(ctx.function(), "drop_list_owner");
                    ctx.builder().build_conditional_branch(matches, end, release).or_llvm_err()?;
                    ctx.builder().position_at_end(release);
                }
                super::super::object_storage::release_os(
                    ctx.llvm_context(), ctx.builder(), ctx.get_module(), pointer,
                    verum_common::layout::LIST_OBJECT_SIZE,
                )?;
            }
        }
        DropPlan::Record {
            fields: declared, ..
        } => retire_fields(ctx, pointer, RuntimeLowering::OBJECT_HEADER_SIZE, declared)?,
        DropPlan::Sum { variants, .. } => {
            let valid = super::super::object_storage::has_os_extent(ctx.llvm_context(),
                ctx.builder(), pointer, RuntimeLowering::OBJECT_HEADER_SIZE + 8)?;
            require_storage(ctx, valid)?;
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
                retire_fields(
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
