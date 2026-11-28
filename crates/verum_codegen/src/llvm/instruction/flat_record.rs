use super::*;

pub(super) fn write<'ctx>(
    ctx: &mut FunctionContext<'_, 'ctx>,
    address: Reg,
    value: Reg,
    declared: &verum_vbc::types::TypeRef,
) -> Result<()> {
    let record = declared
        .base_type_id()
        .and_then(|id| ctx.vbc_module()?.get_type(id))
        .filter(|decl| decl.kind == verum_vbc::types::TypeKind::Record && decl.fields.len() >= 2)
        .or_internal("PtrWriteRecord requires a declared multi-field record")?;
    let owner = record.id.0;
    let payload = record.fields.len() as u64 * verum_common::layout::VALUE_SLOT_SIZE;
    let header = verum_common::layout::OBJECT_HEADER_SIZE;
    let total = header + payload;
    let c = ctx.llvm_context();
    let b = ctx.builder();
    let word = c.i64_type();
    let n = |size| word.const_int(size, false);
    let destination = as_ptr(ctx, ctx.get_register(address.0)?, "record_destination")?;
    let source = as_ptr(ctx, ctx.get_register(value.0)?, "record_owner")?;
    let from = b
        .build_ptr_to_int(source, word, "record_owner_address")
        .or_llvm_err()?;
    let to = b
        .build_ptr_to_int(destination, word, "record_destination_address")
        .or_llvm_err()?;
    let present = b
        .build_and(
            b.build_is_not_null(source, "record_owner_present")
                .or_llvm_err()?,
            b.build_is_not_null(destination, "record_destination_present")
                .or_llvm_err()?,
            "record_present",
        )
        .or_llvm_err()?;
    let low_bits = b
        .build_and(
            to,
            n(verum_common::layout::VALUE_SLOT_SIZE - 1),
            "record_alignment",
        )
        .or_llvm_err()?;
    let aligned = b
        .build_int_compare(IntPredicate::EQ, low_bits, n(0), "record_aligned")
        .or_llvm_err()?;
    // Both unsigned distances meet their respective extents exactly when the
    // two intervals are disjoint, independently of their order in memory.
    let before = b
        .build_int_sub(from, to, "record_left_distance")
        .or_llvm_err()?;
    let after = b
        .build_int_sub(to, from, "record_right_distance")
        .or_llvm_err()?;
    let left = b
        .build_int_compare(IntPredicate::UGE, before, n(payload), "record_left_room")
        .or_llvm_err()?;
    let right = b
        .build_int_compare(IntPredicate::UGE, after, n(total), "record_right_room")
        .or_llvm_err()?;
    let disjoint = b.build_and(left, right, "record_disjoint").or_llvm_err()?;
    let usable = b
        .build_and(present, aligned, "record_address_valid")
        .or_llvm_err()?;
    let usable = b
        .build_and(usable, disjoint, "record_ranges_valid")
        .or_llvm_err()?;
    let inspect = c.append_basic_block(ctx.function(), "record_inspect");
    let rejected = c.append_basic_block(ctx.function(), "record_rejected");
    let transfer = c.append_basic_block(ctx.function(), "record_transfer");
    b.build_conditional_branch(usable, inspect, rejected)
        .or_llvm_err()?;
    b.position_at_end(inspect);
    let receipt = super::super::object_storage::matches_os(c, b, source, total)?;
    let found = b
        .build_load(c.i32_type(), source, "record_owner_type")
        .or_llvm_err()?
        .into_int_value();
    let same = b
        .build_int_compare(
            IntPredicate::EQ,
            found,
            c.i32_type().const_int(u64::from(owner), false),
            "record_nominal_match",
        )
        .or_llvm_err()?;
    let valid = b
        .build_and(receipt, same, "record_storage_valid")
        .or_llvm_err()?;
    b.build_conditional_branch(valid, transfer, rejected)
        .or_llvm_err()?;
    b.position_at_end(rejected);
    emit_runtime_abort(
        ctx,
        "ptr_write requires a complete owned record and disjoint writable storage",
        "record_write_refused",
    )?;
    b.build_unreachable().or_llvm_err()?;
    b.position_at_end(transfer);
    // SAFETY: the exact producer receipt covers every declared source slot.
    // A raw destination's writable extent is the ptr_write caller's contract.
    let fields = unsafe {
        b.build_in_bounds_gep(c.i8_type(), source, &[n(header)], "record_payload")
            .or_llvm_err()?
    };
    let mut ffi = FfiLowering::new(c);
    ffi.lower_memcpy(b, ctx.get_module(), destination, fields, n(payload))?;
    super::super::object_storage::consume(c, b, source)?;
    super::super::object_storage::release_os(c, b, ctx.get_module(), source, total)?;
    ctx.set_register(value.0, n(0).into());
    Ok(())
}
