//! Native callable ownership uses exact formation evidence and the shared
//! checked counter transitions. Capture copies and destruction use ordinary
//! declaration plans; an erased function signature is never ownership proof.
use super::*;

fn counter<'ctx>(ctx: &FunctionContext<'_, 'ctx>, value: BasicValueEnum<'ctx>)
    -> Result<(PointerValue<'ctx>, PointerValue<'ctx>)>
{
    let pointer = as_ptr(ctx, value, "callable_owner")?;
    // SAFETY: callers carry the exact owned NewClosure producer, which allocates
    // the complete prefix and initializes this aligned UInt32 counter.
    let slot = unsafe { ctx.builder().build_in_bounds_gep(ctx.types().i8_type(), pointer,
        &[ctx.types().i64_type().const_int(RuntimeLowering::CLOSURE_OWNER_OFFSET, false)],
        "callable_owner_count").or_llvm_err()? };
    Ok((pointer, slot))
}

pub(super) fn guard(ctx: &FunctionContext<'_, '_>, valid: IntValue<'_>, message: &str) -> Result<()> {
    let accepted = ctx.llvm_context().append_basic_block(ctx.function(), "callable_owner_valid");
    let refused = ctx.llvm_context().append_basic_block(ctx.function(), "callable_owner_refused");
    ctx.builder().build_conditional_branch(valid, accepted, refused).or_llvm_err()?;
    ctx.builder().position_at_end(refused);
    emit_runtime_abort(ctx, message, "callable_ownership")?;
    ctx.builder().build_unreachable().or_llvm_err()?;
    ctx.builder().position_at_end(accepted);
    Ok(())
}

pub(super) fn retain(ctx: &FunctionContext<'_, '_>, src: Reg) -> Result<()> {
    let producer = ctx.callable_producer(src)
        .or_internal("callable retain requires its checked producer")?;
    if producer.owned_captures.is_none() { return Ok(()); }
    let (_, slot) = counter(ctx, ctx.get_register(src.0)?)?;
    let (retain, _) = ctx.retained_counters.as_ref()
        .or_internal("callable retain requires module-owned counters")?.get()?;
    let valid = ctx.builder().build_call(retain, &[slot.into()], "callable_retained")
        .or_llvm_err()?.basic_value_or("callable retain result")?.into_int_value();
    guard(ctx, valid, "callable owner count is zero or would overflow")
}

/// Only exact locally represented producers enter this path. Aggregate or
/// indirect native carriers continue through their existing explicit refusal.
pub(super) fn release<'ctx>(ctx: &mut FunctionContext<'_, 'ctx>, src: Reg,
    expected: &TypeRef) -> Result<bool>
{
    let Some(producer) = ctx.callable_producer(src) else { return Ok(false); };
    let Some(declarations) = producer.owned_captures else { return Ok(false); };
    ctx.require_callable_copy(src, expected)?;
    let module = ctx.vbc_module().or_internal("callable cleanup requires declarations")?;
    let plans = declarations.iter().map(|ty| verum_vbc::drop_plan::build(module, ty)
        .map_err(LlvmLoweringError::UnprovenOwnedDestruction))
        .collect::<Result<verum_common::List<_>>>()?;
    let (pointer, slot) = counter(ctx, ctx.get_register(src.0)?)?;
    let release = ctx.retained_counters.as_ref()
        .or_internal("callable release requires module-owned counters")?.owner_release()?;
    // Consume this handle before any captured destructor can invoke user code.
    ctx.set_register(src.0, ctx.types().i64_type().const_zero().into());
    let previous = ctx.builder().build_call(release, &[slot.into()], "callable_released")
        .or_llvm_err()?.basic_value_or("callable release result")?.into_int_value();
    let i32_type = ctx.llvm_context().i32_type();
    let live = ctx.builder().build_int_compare(IntPredicate::NE, previous,
        i32_type.const_zero(), "callable_had_owner").or_llvm_err()?;
    guard(ctx, live, "callable cleanup has no live owner")?;
    let last = ctx.builder().build_int_compare(IntPredicate::EQ, previous,
        i32_type.const_int(1, false), "callable_last_owner").or_llvm_err()?;
    let destroy = ctx.llvm_context().append_basic_block(ctx.function(), "callable_destroy");
    let done = ctx.llvm_context().append_basic_block(ctx.function(), "callable_release_done");
    ctx.builder().build_conditional_branch(last, destroy, done).or_llvm_err()?;
    ctx.builder().position_at_end(destroy);
    for (index, plan) in plans.iter().enumerate() {
        if matches!(plan, verum_vbc::drop_plan::DropPlan::Ignore) { continue; }
        // SAFETY: the exact producer allocated this declared capture slot; a
        // successful final release grants its unique cleanup ownership.
        let field = unsafe { ctx.builder().build_in_bounds_gep(ctx.types().i8_type(), pointer,
            &[ctx.types().i64_type().const_int(RuntimeLowering::OWNED_CLOSURE_PREFIX + index as u64 * 8, false)],
            "callable_drop_capture").or_llvm_err()? };
        let value = ctx.builder().build_load(ctx.types().i64_type(), field, "callable_owned_capture")
            .or_llvm_err()?;
        ctx.builder().build_store(field, ctx.types().i64_type().const_zero()).or_llvm_err()?;
        super::drop_typed::emit(ctx, value, plan)?;
    }
    super::super::object_storage::release_os(ctx.llvm_context(), ctx.builder(),
        ctx.get_module(), pointer, RuntimeLowering::OWNED_CLOSURE_PREFIX + plans.len() as u64 * 8)?;
    ctx.builder().build_unconditional_branch(done).or_llvm_err()?;
    ctx.builder().position_at_end(done);
    Ok(true)
}
