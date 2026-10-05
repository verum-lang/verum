//! Capacity-checked operations on canonical List-owned CBGR backing.
use super::super::{
    error::{BuildExt, CallSiteExt, OptionExt, Result},
    slice_cell::CellEnv,
};
use super::{LIST_CAP_OFFSET, LIST_PTR_OFFSET, RuntimeLowering};
use verum_llvm::{
    AddressSpace, IntPredicate,
    context::Context,
    module::{Linkage, Module},
};

pub(super) fn define<'ctx>(context: &'ctx Context, module: &Module<'ctx>) -> Result<()> {
    let runtime = RuntimeLowering::new(context);
    let i64_ty = context.i64_type();
    let ptr_ty = context.ptr_type(AddressSpace::default());
    let address = super::super::error::get_or_declare_function(
        module,
        "verum_list_storage_address",
        ptr_ty.fn_type(&[ptr_ty.into(), i64_ty.into(), i64_ty.into()], false),
    );
    if address.count_basic_blocks() != 0 {
        return Ok(());
    }
    address.set_linkage(Linkage::Internal);
    let builder = context.create_builder();
    let entry = context.append_basic_block(address, "entry");
    let shape = context.append_basic_block(address, "shape");
    let range_ok = context.append_basic_block(address, "range_ok");
    let empty = context.append_basic_block(address, "empty");
    let backing = context.append_basic_block(address, "nonempty_address");
    let missing_backing = context.append_basic_block(address, "missing_backing");
    let extent = context.append_basic_block(address, "extent");
    let ready = context.append_basic_block(address, "ready");
    let invalid = context.append_basic_block(address, "invalid");
    builder.position_at_end(entry);
    let list = address
        .get_nth_param(0)
        .or_internal("storage owner")?
        .into_pointer_value();
    let index = address
        .get_nth_param(1)
        .or_internal("storage index")?
        .into_int_value();
    let count = address
        .get_nth_param(2)
        .or_internal("storage count")?
        .into_int_value();
    let width = runtime.lower_list_element_width(&builder, module, list)?;
    let object_size_ptr = runtime.list_field_address(&builder, list, 12, "object_size_ptr")?;
    let object_size = builder
        .build_load(context.i32_type(), object_size_ptr, "object_size")
        .or_llvm_err()?
        .into_int_value();
    let valid_shape = builder
        .build_int_compare(
            IntPredicate::UGE,
            object_size,
            context.i32_type().const_int(super::LIST_OBJECT_SIZE, false),
            "valid_object_shape",
        )
        .or_llvm_err()?;
    builder
        .build_conditional_branch(valid_shape, shape, invalid)
        .or_llvm_err()?;
    builder.position_at_end(shape);
    let cap_ptr = runtime.list_field_address(&builder, list, LIST_CAP_OFFSET, "capacity_ptr")?;
    let capacity = builder
        .build_load(i64_ty, cap_ptr, "capacity")
        .or_llvm_err()?
        .into_int_value();
    let limit = builder
        .build_int_unsigned_div(
            i64_ty.const_int(u32::MAX as u64, false),
            width,
            "capacity_limit",
        )
        .or_llvm_err()?;
    let valid_cap = builder
        .build_int_compare(IntPredicate::ULE, capacity, limit, "valid_capacity")
        .or_llvm_err()?;
    let valid_index = builder
        .build_int_compare(IntPredicate::ULE, index, capacity, "valid_index")
        .or_llvm_err()?;
    let remaining = builder
        .build_int_sub(capacity, index, "remaining_capacity")
        .or_llvm_err()?;
    let valid_count = builder
        .build_int_compare(IntPredicate::ULE, count, remaining, "valid_count")
        .or_llvm_err()?;
    let valid = builder
        .build_and(valid_cap, valid_index, "valid_owner_index")
        .or_llvm_err()?;
    let valid = builder
        .build_and(valid, valid_count, "valid_range")
        .or_llvm_err()?;
    builder
        .build_conditional_branch(valid, range_ok, invalid)
        .or_llvm_err()?;
    builder.position_at_end(range_ok);
    let data_slot = runtime.list_field_address(&builder, list, LIST_PTR_OFFSET, "data_slot")?;
    let data_word = builder
        .build_load(i64_ty, data_slot, "data_word")
        .or_llvm_err()?
        .into_int_value();
    let data = builder
        .build_int_to_ptr(data_word, ptr_ty, "data")
        .or_llvm_err()?;
    let missing = builder
        .build_is_null(data, "missing_storage")
        .or_llvm_err()?;
    builder
        .build_conditional_branch(missing, missing_backing, extent)
        .or_llvm_err()?;
    builder.position_at_end(missing_backing);
    let empty_owner = builder
        .build_int_compare(
            IntPredicate::EQ,
            capacity,
            i64_ty.const_zero(),
            "empty_owner",
        )
        .or_llvm_err()?;
    builder
        .build_conditional_branch(empty_owner, empty, invalid)
        .or_llvm_err()?;
    builder.position_at_end(empty);
    builder
        .build_return(Some(&ptr_ty.const_null()))
        .or_llvm_err()?;
    builder.position_at_end(extent);
    // The canonical List constructor/resize contract owns CBGR backing.
    // Read its declared allocator header, never inspect arbitrary payloads to
    // guess whether they have this layout.
    let header = runtime.emit_cbgr_get_header(&builder, data)?;
    let size_ptr = runtime.list_field_address(
        &builder,
        header,
        RuntimeLowering::AOT_HDR_SIZE_OFFSET,
        "extent_ptr",
    )?;
    let size = builder
        .build_load(context.i32_type(), size_ptr, "allocation_extent")
        .or_llvm_err()?
        .into_int_value();
    let size = builder
        .build_int_z_extend(size, i64_ty, "allocation_extent64")
        .or_llvm_err()?;
    let capacity_bytes = builder
        .build_int_mul(capacity, width, "capacity_bytes")
        .or_llvm_err()?;
    let fits = builder
        .build_int_compare(
            IntPredicate::ULE,
            capacity_bytes,
            size,
            "capacity_fits_allocation",
        )
        .or_llvm_err()?;
    builder
        .build_conditional_branch(fits, ready, invalid)
        .or_llvm_err()?;
    builder.position_at_end(ready);
    let is_empty = builder
        .build_int_compare(IntPredicate::EQ, count, i64_ty.const_zero(), "empty_range")
        .or_llvm_err()?;
    builder
        .build_conditional_branch(is_empty, empty, backing)
        .or_llvm_err()?;
    builder.position_at_end(backing);
    let offset = builder
        .build_int_mul(index, width, "element_offset")
        .or_llvm_err()?;
    // SAFETY: range and allocation extent were checked before this GEP.
    let pointer = unsafe {
        builder
            .build_in_bounds_gep(context.i8_type(), data, &[offset], "element_address")
            .or_llvm_err()?
    };
    builder.build_return(Some(&pointer)).or_llvm_err()?;
    builder.position_at_end(invalid);
    builder
        .build_call(
            runtime.get_or_declare_exit(module),
            &[i64_ty.const_int(1, false).into()],
            "",
        )
        .or_llvm_err()?;
    builder.build_unreachable().or_llvm_err()?;

    let cell = CellEnv {
        llvm: context,
        heap_floor: super::super::target_triple::heap_floor(module),
    };
    for write in [false, true] {
        let name = if write {
            "verum_list_storage_write"
        } else {
            "verum_list_storage_read"
        };
        let function_type = if write {
            context
                .void_type()
                .fn_type(&[ptr_ty.into(), i64_ty.into(), i64_ty.into()], false)
        } else {
            i64_ty.fn_type(&[ptr_ty.into(), i64_ty.into()], false)
        };
        let function = super::super::error::get_or_declare_function(module, name, function_type);
        function.set_linkage(Linkage::Internal);
        builder.position_at_end(context.append_basic_block(function, "entry"));
        let list = function
            .get_nth_param(0)
            .or_internal("storage access owner")?
            .into_pointer_value();
        let index = function
            .get_nth_param(1)
            .or_internal("storage access index")?
            .into_int_value();
        let pointer = builder
            .build_call(
                address,
                &[list.into(), index.into(), i64_ty.const_int(1, false).into()],
                "checked_address",
            )
            .or_llvm_err()?
            .basic_value_or("storage address returned void")?
            .into_pointer_value();
        let width = runtime.lower_list_element_width(&builder, module, list)?;
        if write {
            let value = function
                .get_nth_param(2)
                .or_internal("storage write value")?
                .into_int_value();
            cell.elem_store(&builder, width, pointer, value, "storage_write")?;
            builder.build_return(None).or_llvm_err()?;
        } else {
            let value = cell.elem_load(&builder, width, pointer, "storage_read")?;
            builder.build_return(Some(&value)).or_llvm_err()?;
        }
    }
    let function = super::super::error::get_or_declare_function(
        module,
        "verum_list_storage_move",
        context.void_type().fn_type(
            &[ptr_ty.into(), i64_ty.into(), i64_ty.into(), i64_ty.into()],
            false,
        ),
    );
    function.set_linkage(Linkage::Internal);
    let entry = context.append_basic_block(function, "entry");
    let setup = context.append_basic_block(function, "setup");
    let body = context.append_basic_block(function, "copy_byte");
    let done = context.append_basic_block(function, "done");
    builder.position_at_end(entry);
    let list = function
        .get_nth_param(0)
        .or_internal("storage move owner")?
        .into_pointer_value();
    let source = function
        .get_nth_param(1)
        .or_internal("storage move source")?
        .into_int_value();
    let target = function
        .get_nth_param(2)
        .or_internal("storage move target")?
        .into_int_value();
    let count = function
        .get_nth_param(3)
        .or_internal("storage move count")?
        .into_int_value();
    let src = builder
        .build_call(
            address,
            &[list.into(), source.into(), count.into()],
            "source_address",
        )
        .or_llvm_err()?
        .basic_value_or("storage address returned void")?
        .into_pointer_value();
    let dst = builder
        .build_call(
            address,
            &[list.into(), target.into(), count.into()],
            "target_address",
        )
        .or_llvm_err()?
        .basic_value_or("storage address returned void")?
        .into_pointer_value();
    let empty = builder
        .build_int_compare(IntPredicate::EQ, count, i64_ty.const_zero(), "empty")
        .or_llvm_err()?;
    builder
        .build_conditional_branch(empty, done, setup)
        .or_llvm_err()?;
    builder.position_at_end(setup);
    let width = runtime.lower_list_element_width(&builder, module, list)?;
    let bytes = builder
        .build_int_mul(count, width, "copy_bytes")
        .or_llvm_err()?;
    let last = builder
        .build_int_sub(bytes, i64_ty.const_int(1, false), "last_byte")
        .or_llvm_err()?;
    let backward = builder
        .build_int_compare(IntPredicate::UGT, target, source, "copy_backward")
        .or_llvm_err()?;
    builder.build_unconditional_branch(body).or_llvm_err()?;
    builder.position_at_end(body);
    let step = builder.build_phi(i64_ty, "step").or_llvm_err()?;
    step.add_incoming(&[(&i64_ty.const_zero(), setup)]);
    let offset = step.as_basic_value().into_int_value();
    let reverse = builder
        .build_int_sub(last, offset, "reverse_offset")
        .or_llvm_err()?;
    let offset = builder
        .build_select(backward, reverse, offset, "copy_offset")
        .or_llvm_err()?
        .into_int_value();
    // SAFETY: both complete source/target ranges were validated above. Each
    // loop offset is less than bytes; direction preserves overlap semantics.
    let (s, d) = unsafe {
        (
            builder
                .build_in_bounds_gep(context.i8_type(), src, &[offset], "source_byte")
                .or_llvm_err()?,
            builder
                .build_in_bounds_gep(context.i8_type(), dst, &[offset], "target_byte")
                .or_llvm_err()?,
        )
    };
    let byte = builder
        .build_load(context.i8_type(), s, "byte")
        .or_llvm_err()?;
    builder.build_store(d, byte).or_llvm_err()?;
    let next = builder
        .build_int_add(
            step.as_basic_value().into_int_value(),
            i64_ty.const_int(1, false),
            "next_step",
        )
        .or_llvm_err()?;
    step.add_incoming(&[(&next, body)]);
    let more = builder
        .build_int_compare(IntPredicate::ULT, next, bytes, "more")
        .or_llvm_err()?;
    builder
        .build_conditional_branch(more, body, done)
        .or_llvm_err()?;
    builder.position_at_end(done);
    builder.build_return(None).or_llvm_err()?;
    Ok(())
}
