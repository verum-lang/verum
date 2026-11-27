//! Construct the declared allocation Result on every returning native edge.
use super::{
    BuildExt, FunctionContext, IntValue, OptionExt, PointerValue, Result, RuntimeLowering, TypeId,
    emit_runtime_abort,
};
use verum_common::well_known_types::{result_error_tag, result_success_tag};
use verum_llvm::basic_block::BasicBlock;
use verum_vbc::allocation_result::{AllocatingOperation, AllocationFailure, error_variant};

fn case<'ctx>(
    ctx: &FunctionContext<'_, 'ctx>,
    owner: TypeId,
    tag: u32,
    payload: IntValue<'ctx>,
) -> Result<IntValue<'ctx>> {
    let runtime = RuntimeLowering::new(ctx.llvm_context());
    let builder = ctx.builder();
    let value = runtime.lower_make_typed_variant(builder, ctx.get_module(), owner, tag, 1)?;
    runtime.lower_set_variant_data(builder, value, 0, payload)?;
    Ok(builder
        .build_ptr_to_int(value, ctx.types().i64_type(), "allocation_case")
        .or_llvm_err()?)
}

/// Enter on success with a live allocation. Each failure block either returns
/// its declared error or terminates if the bytecode lacks its declaration.
pub(super) fn finish<'ctx>(
    ctx: &mut FunctionContext<'_, 'ctx>,
    dst: u16,
    intrinsic: &str,
    pointer: PointerValue<'ctx>,
    requested: IntValue<'ctx>,
    invalid: BasicBlock<'ctx>,
    exhausted: BasicBlock<'ctx>,
) -> Result<()> {
    let builder = ctx.builder();
    let runtime = RuntimeLowering::new(ctx.llvm_context());
    let word = ctx.types().i64_type();
    let (generation, epoch) = runtime.lower_header_gen_epoch(builder, pointer)?;
    let address = builder
        .build_ptr_to_int(pointer, word, "allocation_address")
        .or_llvm_err()?;
    let tuple = runtime.lower_pack_typed(
        builder,
        ctx.get_module(),
        &[address, generation, epoch],
        TypeId::TUPLE.0,
    )?;
    let payload = builder
        .build_ptr_to_int(tuple, word, "allocation_tuple")
        .or_llvm_err()?;
    let success = case(ctx, TypeId::RESULT, result_success_tag(), payload)?;
    let success_block = builder
        .get_insert_block()
        .or_internal("allocation success has no block")?;
    let done = ctx
        .llvm_context()
        .append_basic_block(ctx.function(), "allocation_result");
    builder.build_unconditional_branch(done).or_llvm_err()?;
    builder.position_at_end(done);
    let result = builder.build_phi(word, "allocation_value").or_llvm_err()?;
    result.add_incoming(&[(&success, success_block)]);

    let module = ctx
        .vbc_module()
        .or_internal("allocation requires its VBC module")?;
    for (block, failure) in [
        (invalid, AllocationFailure::InvalidSize),
        (exhausted, AllocationFailure::OutOfMemory),
    ] {
        builder.position_at_end(block);
        match error_variant(module, AllocatingOperation::Intrinsic(intrinsic), failure) {
            Ok((owner, tag)) => {
                let error = case(ctx, owner, tag, requested)?;
                let value = case(ctx, TypeId::RESULT, result_error_tag(), error)?;
                let end = builder
                    .get_insert_block()
                    .or_internal("allocation failure has no block")?;
                builder.build_unconditional_branch(done).or_llvm_err()?;
                result.add_incoming(&[(&value, end)]);
            }
            Err(message) => {
                emit_runtime_abort(ctx, &message, "allocation_error_declaration")?;
                builder.build_unreachable().or_llvm_err()?;
            }
        }
    }
    builder.position_at_end(done);
    ctx.set_register(dst, result.as_basic_value());
    ctx.mark_variant_register(dst);
    ctx.set_obj_alloc_size(dst, RuntimeLowering::OBJECT_HEADER_SIZE + 16);
    Ok(())
}
