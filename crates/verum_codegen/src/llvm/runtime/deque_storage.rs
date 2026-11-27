//! Checked address of a language-value slot in an owned native ring.
use super::{
    super::error::{BuildExt, OptionExt, Result},
    RuntimeLowering,
};
use verum_common::layout::{OBJECT_HEADER_SIZE, OBJECT_HEADER_SIZE_OFFSET};
use verum_llvm::{
    AddressSpace, IntPredicate,
    context::Context,
    module::{Linkage, Module},
};

pub(super) fn define<'ctx>(context: &'ctx Context, module: &Module<'ctx>) -> Result<()> {
    let runtime = RuntimeLowering::new(context);
    let word = context.i64_type();
    let i32_ty = context.i32_type();
    let pointer = context.ptr_type(AddressSpace::default());
    let function = super::super::error::get_or_declare_function(
        module,
        "verum_deque_storage_slot",
        pointer.fn_type(
            &[
                pointer.into(),
                word.into(),
                word.into(),
                context.bool_type().into(),
            ],
            false,
        ),
    );
    if function.count_basic_blocks() != 0 {
        return Ok(());
    }
    function.set_linkage(Linkage::Internal);
    let builder = context.create_builder();
    let entry = context.append_basic_block(function, "entry");
    let header = context.append_basic_block(function, "header");
    let ring = context.append_basic_block(function, "ring");
    let empty = context.append_basic_block(function, "empty");
    let allocated = context.append_basic_block(function, "allocated");
    let empty_ready = context.append_basic_block(function, "empty_ready");
    let extent = context.append_basic_block(function, "extent");
    let ready = context.append_basic_block(function, "ready");
    let invalid = context.append_basic_block(function, "invalid");
    let owner = function
        .get_nth_param(0)
        .or_internal("Deque owner")?
        .into_pointer_value();
    let index = function
        .get_nth_param(1)
        .or_internal("Deque physical index")?
        .into_int_value();
    let count = function
        .get_nth_param(2)
        .or_internal("Deque span length")?
        .into_int_value();
    let borrow = function
        .get_nth_param(3)
        .or_internal("Deque borrow selector")?
        .into_int_value();
    builder.position_at_end(entry);
    let present = builder
        .build_is_not_null(owner, "owner_present")
        .or_llvm_err()?;
    builder
        .build_conditional_branch(present, header, invalid)
        .or_llvm_err()?;

    builder.position_at_end(header);
    let id = builder
        .build_load(i32_ty, owner, "owner_id")
        .or_llvm_err()?
        .into_int_value();
    let id_ok = builder
        .build_int_compare(
            IntPredicate::EQ,
            id,
            i32_ty.const_int(verum_vbc::TypeId::DEQUE.0.into(), false),
            "declared_deque",
        )
        .or_llvm_err()?;
    let shape_slot =
        runtime.list_field_address(&builder, owner, OBJECT_HEADER_SIZE_OFFSET, "object_extent")?;
    let size = builder
        .build_load(i32_ty, shape_slot, "object_size")
        .or_llvm_err()?
        .into_int_value();
    let shape_ok = builder
        .build_int_compare(
            IntPredicate::EQ,
            size,
            i32_ty.const_int(OBJECT_HEADER_SIZE + 4 * 8, false),
            "owner_shape",
        )
        .or_llvm_err()?;
    let valid = builder
        .build_and(id_ok, shape_ok, "valid_owner")
        .or_llvm_err()?;
    builder
        .build_conditional_branch(valid, ring, invalid)
        .or_llvm_err()?;

    builder.position_at_end(ring);
    let load = |index: u64, name: &str| -> Result<_> {
        let field =
            runtime.list_field_address(&builder, owner, OBJECT_HEADER_SIZE + index * 8, name)?;
        Ok(builder
            .build_load(word, field, name)
            .or_llvm_err()?
            .into_int_value())
    };
    let address = load(0, "backing")?;
    let head = load(1, "head")?;
    let len = load(2, "length")?;
    let cap = load(3, "capacity")?;
    let zero = word.const_zero();
    let cap_zero = builder
        .build_int_compare(IntPredicate::EQ, cap, zero, "no_capacity")
        .or_llvm_err()?;
    builder
        .build_conditional_branch(cap_zero, empty, allocated)
        .or_llvm_err()?;
    builder.position_at_end(empty);
    let mut empty_ok = borrow;
    for value in [address, head, len, index, count] {
        let is_zero = builder
            .build_int_compare(IntPredicate::EQ, value, zero, "is_zero")
            .or_llvm_err()?;
        empty_ok = builder
            .build_and(empty_ok, is_zero, "valid_empty")
            .or_llvm_err()?;
    }
    builder
        .build_conditional_branch(empty_ok, empty_ready, invalid)
        .or_llvm_err()?;
    builder.position_at_end(empty_ready);
    builder
        .build_return(Some(&pointer.const_null()))
        .or_llvm_err()?;
    builder.position_at_end(allocated);
    let start_ok = builder
        .build_int_compare(IntPredicate::ULE, index, cap, "start_in_bounds")
        .or_llvm_err()?;
    let available = builder
        .build_int_sub(cap, index, "physical_remaining")
        .or_llvm_err()?;
    let count_ok = builder
        .build_int_compare(IntPredicate::ULE, count, available, "span_in_bounds")
        .or_llvm_err()?;
    let in_bounds = builder
        .build_and(start_ok, count_ok, "in_bounds")
        .or_llvm_err()?;
    let head_ok = builder
        .build_int_compare(IntPredicate::ULT, head, cap, "head_in_bounds")
        .or_llvm_err()?;
    let len_ok = builder
        .build_int_compare(IntPredicate::ULE, len, cap, "length_in_bounds")
        .or_llvm_err()?;
    let mask = builder
        .build_int_sub(cap, word.const_int(1, false), "capacity_mask")
        .or_llvm_err()?;
    let distance = builder
        .build_int_sub(index, head, "from_head")
        .or_llvm_err()?;
    let logical = builder
        .build_and(distance, mask, "logical_index")
        .or_llvm_err()?;
    let logical_ok = builder
        .build_int_compare(IntPredicate::ULE, logical, len, "logical_in_bounds")
        .or_llvm_err()?;
    let remaining = builder
        .build_int_sub(len, logical, "live_remaining")
        .or_llvm_err()?;
    let count_live = builder
        .build_int_compare(IntPredicate::ULE, count, remaining, "live_count")
        .or_llvm_err()?;
    let live_span = builder
        .build_and(logical_ok, count_live, "initialized_span")
        .or_llvm_err()?;
    let no_items = builder
        .build_int_compare(IntPredicate::EQ, count, zero, "no_items")
        .or_llvm_err()?;
    let live = builder
        .build_or(no_items, live_span, "initialized")
        .or_llvm_err()?;
    let transfer = builder.build_not(borrow, "is_transfer").or_llvm_err()?;
    let accessible = builder
        .build_or(live, transfer, "accessible")
        .or_llvm_err()?;
    let power_bits = builder.build_and(cap, mask, "power_bits").or_llvm_err()?;
    let power = builder
        .build_int_compare(
            IntPredicate::EQ,
            power_bits,
            word.const_zero(),
            "power_of_two",
        )
        .or_llvm_err()?;
    let data = builder
        .build_int_to_ptr(address, pointer, "data")
        .or_llvm_err()?;
    let mut valid = builder
        .build_is_not_null(data, "has_backing")
        .or_llvm_err()?;
    for condition in [in_bounds, head_ok, len_ok, power, accessible] {
        valid = builder
            .build_and(valid, condition, "ring_bound")
            .or_llvm_err()?;
    }
    builder
        .build_conditional_branch(valid, extent, invalid)
        .or_llvm_err()?;

    builder.position_at_end(extent);
    let allocation = runtime.emit_cbgr_get_header(&builder, data)?;
    let size_slot = runtime.list_field_address(
        &builder,
        allocation,
        RuntimeLowering::AOT_HDR_SIZE_OFFSET,
        "allocation_size_slot",
    )?;
    let bytes = builder
        .build_load(i32_ty, size_slot, "allocation_size")
        .or_llvm_err()?
        .into_int_value();
    let extent_bytes = builder
        .build_int_z_extend(bytes, word, "extent_bytes")
        .or_llvm_err()?;
    let slots = builder
        .build_int_unsigned_div(extent_bytes, word.const_int(8, false), "allocated_slots")
        .or_llvm_err()?;
    let rest = builder
        .build_int_unsigned_rem(extent_bytes, word.const_int(8, false), "partial_slot")
        .or_llvm_err()?;
    let aligned = builder
        .build_int_compare(IntPredicate::EQ, rest, word.const_zero(), "whole_slots")
        .or_llvm_err()?;
    let matches = builder
        .build_int_compare(IntPredicate::EQ, cap, slots, "matches_allocation")
        .or_llvm_err()?;
    let fits = builder
        .build_and(aligned, matches, "exact_extent")
        .or_llvm_err()?;
    builder
        .build_conditional_branch(fits, ready, invalid)
        .or_llvm_err()?;
    builder.position_at_end(ready);
    // SAFETY: index < cap and the complete capacity fits the owned allocation.
    let slot =
        unsafe { builder.build_in_bounds_gep(word, data, &[index], "value_slot") }.or_llvm_err()?;
    builder.build_return(Some(&slot)).or_llvm_err()?;
    builder.position_at_end(invalid);
    builder
        .build_call(
            runtime.get_or_declare_exit(module)?,
            &[word.const_int(1, false).into()],
            "",
        )
        .or_llvm_err()?;
    builder.build_unreachable().or_llvm_err()?;
    Ok(())
}
