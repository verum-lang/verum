//! Native Text storage receipts are issued by the allocating operation.
//! The first three words remain the source-visible ptr/len/cap payload.
use verum_common::Maybe;
use verum_llvm::{
    AddressSpace, IntPredicate,
    builder::Builder,
    context::Context,
    module::{Linkage, Module},
    values::{FunctionValue, IntValue, PointerValue},
};

use super::error::{BuildExt, CallSiteExt, OptionExt, Result, get_or_declare_function};

pub(super) const RECEIPT_OFFSET: u64 = verum_common::layout::TEXT_SIZE;
pub(super) const DESCRIPTOR_SIZE: u64 = RECEIPT_OFFSET + verum_common::layout::POINTER_SIZE;

/// A descriptor and its bytes can belong to different allocation domains.
/// Borrowed bytes remain owned by their producer, including after a release.
#[derive(Clone, Copy)]
#[repr(u64)]
pub(super) enum Receipt {
    Static = 1,
    ArenaBorrowed = 2,
    ArenaCbgr = 3,
    OsBorrowed = 4,
    OsCbgr = 5,
}

fn slot<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    descriptor: PointerValue<'ctx>,
    offset: u64,
) -> Result<PointerValue<'ctx>> {
    // SAFETY: the issuing producer allocated DESCRIPTOR_SIZE bytes. Each
    // addressed word is part of that descriptor's fixed native layout.
    unsafe {
        builder
            .build_in_bounds_gep(
                context.i8_type(),
                descriptor,
                &[context.i64_type().const_int(offset, false)],
                "text_storage_slot",
            )
            .or_llvm_err()
    }
}

pub(super) fn stamp<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    descriptor: PointerValue<'ctx>,
    receipt: Receipt,
) -> Result<()> {
    builder
        .build_store(
            slot(context, builder, descriptor, RECEIPT_OFFSET)?,
            context.i64_type().const_int(receipt as u64, false),
        )
        .or_llvm_err()?;
    Ok(())
}

fn trap<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &Module<'ctx>,
) -> Result<()> {
    let operation =
        get_or_declare_function(module, "llvm.trap", context.void_type().fn_type(&[], false));
    builder.build_call(operation, &[], "").or_llvm_err()?;
    builder.build_unreachable().or_llvm_err()?;
    Ok(())
}

fn require<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &Module<'ctx>,
    valid: IntValue<'ctx>,
    label: &str,
) -> Result<()> {
    let function = builder
        .get_insert_block()
        .or_internal("Text check has no block")?
        .get_parent()
        .or_internal("Text check has no function")?;
    let accepted = context.append_basic_block(function, label);
    let failure = context.append_basic_block(function, "text_storage_contract_failed");
    builder
        .build_conditional_branch(valid, accepted, failure)
        .or_llvm_err()?;
    builder.position_at_end(failure);
    trap(context, builder, module)?;
    builder.position_at_end(accepted);
    Ok(())
}

/// Validate representable dimensions before publishing a descriptor. An owned
/// adoption additionally requires the unsafe caller to supply the live base of
/// a CBGR allocation of at least cap + 1 bytes. Dimensions do not prove origin.
pub(super) fn validate_shape<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &Module<'ctx>,
    data: PointerValue<'ctx>,
    len: IntValue<'ctx>,
    capacity: Maybe<IntValue<'ctx>>,
) -> Result<()> {
    let word = context.i64_type();
    let extent = capacity.unwrap_or(len);
    let max = word.const_int(verum_common::layout::MAX_ALLOCATION_SIZE as u64 - 1, false);
    let bounded = builder
        .build_int_compare(IntPredicate::ULE, extent, max, "text_extent_bounded")
        .or_llvm_err()?;
    let fits = builder
        .build_int_compare(IntPredicate::ULE, len, extent, "text_length_fits")
        .or_llvm_err()?;
    let empty = builder
        .build_int_compare(IntPredicate::EQ, extent, word.const_zero(), "text_empty")
        .or_llvm_err()?;
    let present = builder
        .build_is_not_null(data, "text_data_present")
        .or_llvm_err()?;
    let backed = builder
        .build_or(empty, present, "text_storage_present")
        .or_llvm_err()?;
    let valid = builder
        .build_and(bounded, fits, "text_dimensions_valid")
        .or_llvm_err()?;
    let valid = builder
        .build_and(valid, backed, "text_shape_valid")
        .or_llvm_err()?;
    require(context, builder, module, valid, "text_shape_accepted")
}

/// Allocate the full byte extent, including any terminator, in the CBGR domain.
pub(super) fn allocate_bytes<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &Module<'ctx>,
    size: IntValue<'ctx>,
    name: &str,
) -> Result<PointerValue<'ctx>> {
    let word = context.i64_type();
    let within_limit = builder
        .build_int_compare(
            IntPredicate::ULE,
            size,
            word.const_int(verum_common::layout::MAX_ALLOCATION_SIZE as u64, false),
            "text_allocation_bounded",
        )
        .or_llvm_err()?;
    let positive = builder
        .build_int_compare(
            IntPredicate::NE,
            size,
            word.const_zero(),
            "text_allocation_positive",
        )
        .or_llvm_err()?;
    let valid = builder
        .build_and(within_limit, positive, "text_allocation_valid")
        .or_llvm_err()?;
    require(
        context,
        builder,
        module,
        valid,
        "text_allocation_size_accepted",
    )?;
    let allocate = get_or_declare_function(
        module,
        "verum_cbgr_allocate_aligned",
        context
            .ptr_type(AddressSpace::default())
            .fn_type(&[word.into(), word.into()], false),
    );
    let pointer = builder
        .build_call(
            allocate,
            &[size.into(), word.const_int(1, false).into()],
            name,
        )
        .or_llvm_err()?
        .basic_value_or("Text byte allocator returned void")?
        .into_pointer_value();
    let present = builder
        .build_is_not_null(pointer, "text_bytes_present")
        .or_llvm_err()?;
    require(context, builder, module, present, "text_bytes_allocated")?;
    Ok(pointer)
}

fn release_bytes<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &Module<'ctx>,
    descriptor: PointerValue<'ctx>,
) -> Result<()> {
    let word = builder
        .build_load(context.i64_type(), descriptor, "text_owned_bytes")
        .or_llvm_err()?
        .into_int_value();
    let bytes = builder
        .build_int_to_ptr(
            word,
            context.ptr_type(AddressSpace::default()),
            "text_bytes",
        )
        .or_llvm_err()?;
    let release = get_or_declare_function(
        module,
        "verum_cbgr_deallocate",
        context
            .void_type()
            .fn_type(&[bytes.get_type().into()], false),
    );
    let function = builder.get_insert_block().or_internal("Text byte release block")?
        .get_parent().or_internal("Text byte release function")?;
    let live = context.append_basic_block(function, "text_nonempty_allocation");
    let done = context.append_basic_block(function, "text_allocation_released");
    let present = builder.build_is_not_null(bytes, "text_allocation_present").or_llvm_err()?;
    builder.build_conditional_branch(present, live, done).or_llvm_err()?;
    builder.position_at_end(live);
    builder.build_call(release, &[bytes.into()], "").or_llvm_err()?;
    builder.build_unconditional_branch(done).or_llvm_err()?;
    builder.position_at_end(done);
    Ok(())
}

fn clear<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    descriptor: PointerValue<'ctx>,
    receipt: Receipt,
) -> Result<()> {
    for offset in [
        super::runtime::TEXT_PTR_OFFSET,
        super::runtime::TEXT_LEN_OFFSET,
        super::runtime::TEXT_CAP_OFFSET,
    ] {
        builder
            .build_store(
                slot(context, builder, descriptor, offset)?,
                context.i64_type().const_zero(),
            )
            .or_llvm_err()?;
    }
    stamp(context, builder, descriptor, receipt)
}

fn declaration<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    name: &str,
) -> FunctionValue<'ctx> {
    let function = get_or_declare_function(
        module,
        name,
        context
            .void_type()
            .fn_type(&[context.i64_type().into()], false),
    );
    function.set_linkage(Linkage::Internal);
    let attribute = verum_llvm::attributes::Attribute::get_named_enum_kind_id("noinline");
    function.add_attribute(
        verum_llvm::attributes::AttributeLoc::Function,
        context.create_enum_attribute(attribute, 0),
    );
    function
}

pub(super) fn define_release_functions<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
) -> Result<()> {
    let release = declaration(context, module, "verum_text_release_storage");
    let free = declaration(context, module, "verum_text_free");
    for (function, destroy_descriptor) in [(release, false), (free, true)] {
        if function.count_basic_blocks() > 0 {
            continue;
        }
        let builder = context.create_builder();
        let entry = context.append_basic_block(function, "entry");
        let live = context.append_basic_block(function, "text_live");
        let invalid = context.append_basic_block(function, "text_invalid_storage_receipt");
        let arena = context.append_basic_block(function, "text_arena_descriptor");
        let os = context.append_basic_block(function, "text_os_descriptor");
        let done = context.append_basic_block(function, "text_released");
        let word = context.i64_type();
        builder.position_at_end(entry);
        let value = function
            .get_nth_param(0)
            .or_internal("Text release receiver")?
            .into_int_value();
        let descriptor = builder
            .build_int_to_ptr(
                value,
                context.ptr_type(AddressSpace::default()),
                "text_descriptor",
            )
            .or_llvm_err()?;
        let empty = builder
            .build_is_null(descriptor, "text_null")
            .or_llvm_err()?;
        builder
            .build_conditional_branch(empty, done, live)
            .or_llvm_err()?;
        builder.position_at_end(live);
        if destroy_descriptor {
            builder
                .build_call(release, &[value.into()], "")
                .or_llvm_err()?;
        }
        let receipt = builder
            .build_load(
                word,
                slot(context, &builder, descriptor, RECEIPT_OFFSET)?,
                "text_receipt",
            )
            .or_llvm_err()?
            .into_int_value();
        let arena_cbgr = context.append_basic_block(function, "text_arena_cbgr_bytes");
        let os_cbgr = context.append_basic_block(function, "text_os_cbgr_bytes");
        builder
            .build_switch(
                receipt,
                invalid,
                &[
                    (word.const_int(Receipt::Static as u64, false), done),
                    (word.const_int(Receipt::ArenaBorrowed as u64, false), arena),
                    (word.const_int(Receipt::ArenaCbgr as u64, false), arena_cbgr),
                    (word.const_int(Receipt::OsBorrowed as u64, false), os),
                    (word.const_int(Receipt::OsCbgr as u64, false), os_cbgr),
                ],
            )
            .or_llvm_err()?;
        for (owned, next) in [(arena_cbgr, arena), (os_cbgr, os)] {
            builder.position_at_end(owned);
            if destroy_descriptor {
                // Releasing storage must have replaced every owned-byte receipt.
                builder.build_unconditional_branch(invalid).or_llvm_err()?;
            } else {
                release_bytes(context, &builder, module, descriptor)?;
                builder.build_unconditional_branch(next).or_llvm_err()?;
            }
        }
        for (block, receipt, allocator) in [
            (arena, Receipt::ArenaBorrowed, "verum_dealloc"),
            (os, Receipt::OsBorrowed, "verum_os_free"),
        ] {
            builder.position_at_end(block);
            if destroy_descriptor {
                let free_record = get_or_declare_function(
                    module,
                    allocator,
                    context
                        .void_type()
                        .fn_type(&[descriptor.get_type().into(), word.into()], false),
                );
                builder
                    .build_call(
                        free_record,
                        &[
                            descriptor.into(),
                            word.const_int(DESCRIPTOR_SIZE, false).into(),
                        ],
                        "",
                    )
                    .or_llvm_err()?;
            } else {
                clear(context, &builder, descriptor, receipt)?;
            }
            builder.build_unconditional_branch(done).or_llvm_err()?;
        }
        builder.position_at_end(invalid);
        trap(context, &builder, module)?;
        builder.position_at_end(done);
        builder.build_return(None).or_llvm_err()?;
    }
    define_replace_function(context, module)?;
    Ok(())
}

/// Replace the value behind an existing Text reference without changing that
/// descriptor's identity. Data ownership moves; the consumed source descriptor
/// retires in its own allocator domain.
fn define_replace_function<'ctx>(context: &'ctx Context, module: &Module<'ctx>) -> Result<()> {
    let word = context.i64_type();
    let pointer_type = context.ptr_type(AddressSpace::default());
    let function = get_or_declare_function(module, "verum_text_replace",
        context.void_type().fn_type(&[word.into(), word.into()], false));
    if function.count_basic_blocks() != 0 { return Ok(()); }
    function.set_linkage(Linkage::Internal);
    let builder = context.create_builder();
    builder.position_at_end(context.append_basic_block(function, "entry"));
    let destination = function.get_nth_param(0).or_internal("Text replacement destination")?.into_int_value();
    let source = function.get_nth_param(1).or_internal("Text replacement source")?.into_int_value();
    let mut states = verum_common::List::new();
    for value in [destination, source] {
        let descriptor = builder.build_int_to_ptr(value, pointer_type, "text_replacement_descriptor").or_llvm_err()?;
        let present = builder.build_is_not_null(descriptor, "text_replacement_present").or_llvm_err()?;
        require(context, &builder, module, present, "text_replacement_live")?;
        let load = |offset| -> Result<IntValue<'ctx>> {
            Ok(builder.build_load(word, slot(context, &builder, descriptor, offset)?, "text_replacement_field")
                .or_llvm_err()?.into_int_value())
        };
        let receipt = load(RECEIPT_OFFSET)?;
        let mut recognized = context.bool_type().const_zero();
        for kind in [Receipt::ArenaBorrowed, Receipt::ArenaCbgr, Receipt::OsBorrowed, Receipt::OsCbgr] {
            let same = builder.build_int_compare(IntPredicate::EQ, receipt, word.const_int(kind as u64, false), "text_replacement_domain").or_llvm_err()?;
            recognized = builder.build_or(recognized, same, "text_replacement_mutable_receipt").or_llvm_err()?;
        }
        require(context, &builder, module, recognized, "text_replacement_receipt_accepted")?;
        let arena = builder.build_int_compare(IntPredicate::ULE, receipt, word.const_int(Receipt::ArenaCbgr as u64, false), "text_replacement_arena").or_llvm_err()?;
        let borrowed = builder.build_select(arena, word.const_int(Receipt::ArenaBorrowed as u64, false), word.const_int(Receipt::OsBorrowed as u64, false), "text_replacement_borrowed_receipt").or_llvm_err()?.into_int_value();
        let owned = builder.build_int_compare(IntPredicate::NE, receipt, borrowed, "text_replacement_owns_bytes").or_llvm_err()?;
        let data = load(super::runtime::TEXT_PTR_OFFSET)?;
        let length = load(super::runtime::TEXT_LEN_OFFSET)?;
        let capacity = load(super::runtime::TEXT_CAP_OFFSET)?;
        let borrowed_capacity = builder.build_int_compare(IntPredicate::EQ, capacity, word.const_zero(), "text_borrowed_capacity_zero").or_llvm_err()?;
        let capacity_valid = builder.build_or(owned, borrowed_capacity, "text_replacement_capacity_valid").or_llvm_err()?;
        require(context, &builder, module, capacity_valid, "text_replacement_capacity_accepted")?;
        let extent = builder.build_select(owned, capacity, length, "text_replacement_extent").or_llvm_err()?.into_int_value();
        let bytes = builder.build_int_to_ptr(data, pointer_type, "text_replacement_bytes").or_llvm_err()?;
        validate_shape(context, &builder, module, bytes, length, Some(extent))?;
        states.push((descriptor, borrowed, owned, [data, length, capacity]));
    }
    let done = context.append_basic_block(function, "text_replacement_done");
    let replace = context.append_basic_block(function, "text_replace_distinct_value");
    let same = builder.build_int_compare(IntPredicate::EQ, destination, source, "text_same_descriptor").or_llvm_err()?;
    builder.build_conditional_branch(same, done, replace).or_llvm_err()?;
    builder.position_at_end(replace);
    let release = module.get_function("verum_text_release_storage").or_internal("Text storage release")?;
    builder.build_call(release, &[destination.into()], "").or_llvm_err()?;
    let (target, target_borrowed, _, _) = states[0];
    let (consumed, source_borrowed, source_owned, fields) = states[1];
    for (offset, value) in [super::runtime::TEXT_PTR_OFFSET, super::runtime::TEXT_LEN_OFFSET, super::runtime::TEXT_CAP_OFFSET].into_iter().zip(fields) {
        builder.build_store(slot(context, &builder, target, offset)?, value).or_llvm_err()?;
        builder.build_store(slot(context, &builder, consumed, offset)?, word.const_zero()).or_llvm_err()?;
    }
    let owned_receipt = builder.build_int_add(target_borrowed, word.const_int(1, false), "text_replacement_owned_receipt").or_llvm_err()?;
    let target_receipt = builder.build_select(source_owned, owned_receipt, target_borrowed, "text_replacement_receipt").or_llvm_err()?;
    builder.build_store(slot(context, &builder, target, RECEIPT_OFFSET)?, target_receipt).or_llvm_err()?;
    builder.build_store(slot(context, &builder, consumed, RECEIPT_OFFSET)?, source_borrowed).or_llvm_err()?;
    let free = module.get_function("verum_text_free").or_internal("Text descriptor release")?;
    builder.build_call(free, &[source.into()], "").or_llvm_err()?;
    builder.build_unconditional_branch(done).or_llvm_err()?;
    builder.position_at_end(done);
    builder.build_return(None).or_llvm_err()?;
    Ok(())
}
