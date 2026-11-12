//! Target queries lower to constants or byte comparisons; no runtime host shim.
use super::*;
use verum_common::{List, OrderedMap, Text};

fn capabilities(ctx: &FunctionContext<'_, '_>) -> Result<verum_vbc::target::TargetCapabilities> {
    let machine = ctx
        .target_machine
        .or_internal("target query requires the selected target machine")?;
    let info = machine
        .subtarget_info(ctx.function())
        .or_internal("LLVM supplied no target capability metadata")?;
    Ok(verum_vbc::target::TargetCapabilities {
        max_atomic_bits: info.max_atomic_bits,
        pointer_bits: machine.get_target_data().get_pointer_byte_size(None) * 8,
        features: info.features,
    })
}

pub(super) fn atomic(ctx: &mut FunctionContext<'_, '_>, dst: Reg, ty: &TypeRef) -> Result<()> {
    let facts = capabilities(ctx)?;
    let module = ctx
        .vbc_module()
        .or_internal("atomic type query requires declaration metadata")?;
    let value = match facts.has_atomic(module, ty) {
        Ok(supported) => ctx
            .types()
            .bool_type()
            .const_int(u64::from(supported), false),
        Err(error) => {
            // Unspecialized generic bodies may accompany their specializations;
            // entering such a body must never manufacture a successful answer.
            emit_runtime_abort(ctx, &error, "atomic_witness_missing")?;
            ctx.builder().build_unreachable().or_llvm_err()?;
            let dead = ctx
                .llvm_context()
                .append_basic_block(ctx.function(), "atomic_dead");
            ctx.builder().position_at_end(dead);
            ctx.types().bool_type().get_undef()
        }
    };
    ctx.set_register(dst.0, value.into());
    ctx.mark_bool_register(dst.0);
    Ok(())
}

pub(super) fn feature(ctx: &mut FunctionContext<'_, '_>, dst: Reg, operand: Reg) -> Result<()> {
    let facts = capabilities(ctx)?;
    let mut by_length: OrderedMap<usize, List<Text>> = OrderedMap::new();
    for (name, enabled) in facts.features {
        if enabled {
            by_length.entry(name.len()).or_default().push(name);
        }
    }
    let context = ctx.llvm_context();
    let builder = ctx.builder();
    let module = ctx.get_module();
    let i64_type = context.i64_type();
    let byte_type = context.i8_type();
    let bool_type = context.bool_type();
    let input = as_i64(ctx, ctx.get_register(operand.0)?, "feature_text")?;
    let ptr_type = ctx.types().ptr_type();
    let ptr_fn = super::super::error::get_or_declare_function(
        module,
        "verum_text_get_ptr",
        ptr_type.fn_type(&[i64_type.into()], false),
    );
    let len_fn = super::super::error::get_or_declare_function(
        module,
        "verum_text_byte_len",
        i64_type.fn_type(&[i64_type.into()], false),
    );
    let data = builder
        .build_call(ptr_fn, &[input.into()], "feature_bytes")
        .or_llvm_err()?
        .basic_value_or("feature data")?
        .into_pointer_value();
    let length = builder
        .build_call(len_fn, &[input.into()], "feature_length")
        .or_llvm_err()?
        .basic_value_or("feature length")?
        .into_int_value();
    let fallback = context.append_basic_block(ctx.function(), "feature_unknown");
    let done = context.append_basic_block(ctx.function(), "feature_done");
    let mut cases = List::new();
    let mut incoming = List::new();
    let start = builder
        .get_insert_block()
        .or_internal("feature query needs a block")?;
    // Dispatch by exact byte length before reading bytes. Each byte is loaded
    // once per length group; this is NUL-safe and folds for literal arguments.
    for (length, names) in by_length {
        let block = context.append_basic_block(ctx.function(), "feature_length_case");
        cases.push((i64_type.const_int(length as u64, false), block));
        builder.position_at_end(block);
        let mut bytes = List::new();
        for index in 0..length {
            // SAFETY: this block is reachable only for a Text of this length.
            let address = unsafe {
                builder.build_in_bounds_gep(
                    byte_type,
                    data,
                    &[i64_type.const_int(index as u64, false)],
                    "feature_byte_ptr",
                )
            }
            .or_llvm_err()?;
            bytes.push(
                builder
                    .build_load(byte_type, address, "feature_byte")
                    .or_llvm_err()?
                    .into_int_value(),
            );
        }
        let mut any = bool_type.const_zero();
        for name in names {
            let mut equal = bool_type.const_int(1, false);
            for (byte, expected) in bytes.iter().zip(name.as_bytes()) {
                let same = builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        *byte,
                        byte_type.const_int(u64::from(*expected), false),
                        "feature_byte_equal",
                    )
                    .or_llvm_err()?;
                equal = builder
                    .build_and(equal, same, "feature_equal")
                    .or_llvm_err()?;
            }
            any = builder
                .build_or(any, equal, "feature_enabled")
                .or_llvm_err()?;
        }
        builder.build_unconditional_branch(done).or_llvm_err()?;
        incoming.push((any, block));
    }
    builder.position_at_end(start);
    builder
        .build_switch(length, fallback, &cases)
        .or_llvm_err()?;
    builder.position_at_end(fallback);
    builder.build_unconditional_branch(done).or_llvm_err()?;
    incoming.push((bool_type.const_zero(), fallback));
    builder.position_at_end(done);
    let result = builder
        .build_phi(bool_type, "target_has_feature")
        .or_llvm_err()?;
    for (value, block) in &incoming {
        result.add_incoming(&[(value, *block)]);
    }
    ctx.set_register(dst.0, result.as_basic_value());
    ctx.mark_bool_register(dst.0);
    Ok(())
}
