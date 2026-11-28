//! Physical storage of an indexed mutable borrow in the one-word native ABI.
//! Word slots are ordinary pointers. Sub-word references retain the selected
//! width while crossing calls or sums. Explicit native pointer conversions
//! expose the original byte address.

use super::context::FunctionContext;
use super::error::{BuildExt, CallSiteExt, Result};
use verum_llvm::IntPredicate;
use verum_llvm::values::IntValue;
use verum_llvm::{
    AddressSpace,
    builder::Builder,
    context::Context,
    module::{Linkage, Module},
};

struct TagLayout {
    shift: u64,
    mask: u64,
    base: u64,
}

impl TagLayout {
    fn for_target(module: &Module<'_>) -> Self {
        if super::target_triple::target_is_aarch64(module) {
            // Preserve the top byte of an AArch64 pointer, including an MTE tag.
            Self {
                shift: 52,
                mask: 0xf,
                base: 8,
            }
        } else {
            Self {
                shift: 56,
                mask: 0xff,
                base: 0xe0,
            }
        }
    }

    fn address_mask(&self) -> u64 {
        !(self.mask << self.shift)
    }
}

pub(super) fn address_fits<'ctx>(
    ctx: &FunctionContext<'_, 'ctx>,
    address: IntValue<'ctx>,
    width: IntValue<'ctx>,
) -> Result<IntValue<'ctx>> {
    let layout = TagLayout::for_target(ctx.get_module());
    let word = ctx.types().i64_type();
    let reserved = ctx
        .builder()
        .build_and(
            address,
            word.const_int(!layout.address_mask(), false),
            "native_address_header",
        )
        .or_llvm_err()?;
    let fits = ctx
        .builder()
        .build_int_compare(
            IntPredicate::EQ,
            reserved,
            word.const_zero(),
            "address_fits_element_reference",
        )
        .or_llvm_err()?;
    let full = ctx
        .builder()
        .build_int_compare(
            IntPredicate::EQ,
            width,
            word.const_int(8, false),
            "whole_slot",
        )
        .or_llvm_err()?;
    ctx.builder()
        .build_or(fits, full, "representable_reference")
        .or_llvm_err()
}

pub(super) fn encode<'ctx>(
    ctx: &FunctionContext<'_, 'ctx>,
    address: IntValue<'ctx>,
    width: IntValue<'ctx>,
) -> Result<IntValue<'ctx>> {
    let layout = TagLayout::for_target(ctx.get_module());
    let word = ctx.types().i64_type();
    let builder = ctx.builder();
    let tag = builder
        .build_or(width, word.const_int(layout.base, false), "element_tag")
        .or_llvm_err()?;
    let high = builder
        .build_left_shift(tag, word.const_int(layout.shift, false), "element_header")
        .or_llvm_err()?;
    let encoded = builder
        .build_or(address, high, "element_reference")
        .or_llvm_err()?;
    let full = builder
        .build_int_compare(
            IntPredicate::EQ,
            width,
            word.const_int(8, false),
            "word_element",
        )
        .or_llvm_err()?;
    Ok(builder
        .build_select(full, address, encoded, "indexed_reference")
        .or_llvm_err()?
        .into_int_value())
}

pub(super) fn decode<'ctx>(
    ctx: &FunctionContext<'_, 'ctx>,
    reference: IntValue<'ctx>,
) -> Result<(IntValue<'ctx>, IntValue<'ctx>)> {
    if !supported(ctx) {
        return Ok((reference, ctx.types().i64_type().const_int(8, false)));
    }
    decode_at(
        ctx.llvm_context(),
        ctx.get_module(),
        ctx.builder(),
        reference,
    )
}

pub(super) fn supported(ctx: &FunctionContext<'_, '_>) -> bool {
    ctx.vbc_module().is_some_and(|module| {
        module.header.version_minor >= verum_vbc::format::MUTABLE_ELEMENT_PROJECTION_VERSION
    })
}

fn decode_at<'ctx>(
    llvm: &'ctx Context,
    module: &Module<'ctx>,
    builder: &Builder<'ctx>,
    reference: IntValue<'ctx>,
) -> Result<(IntValue<'ctx>, IntValue<'ctx>)> {
    let layout = TagLayout::for_target(module);
    let word = llvm.i64_type();
    let tag = builder
        .build_right_shift(
            reference,
            word.const_int(layout.shift, false),
            false,
            "reference_header",
        )
        .or_llvm_err()?;
    let tag = builder
        .build_and(tag, word.const_int(layout.mask, false), "reference_tag")
        .or_llvm_err()?;
    let width = builder
        .build_xor(
            tag,
            word.const_int(layout.base, false),
            "reference_element_width",
        )
        .or_llvm_err()?;
    let is_width = |bytes| {
        builder
            .build_int_compare(
                IntPredicate::EQ,
                width,
                word.const_int(bytes, false),
                "narrow_element",
            )
            .or_llvm_err()
    };
    let small = builder
        .build_or(is_width(1)?, is_width(2)?, "small_element")
        .or_llvm_err()?;
    let tagged = builder
        .build_or(small, is_width(4)?, "packed_element")
        .or_llvm_err()?;
    let address = builder
        .build_and(
            reference,
            word.const_int(layout.address_mask(), false),
            "element_address",
        )
        .or_llvm_err()?;
    let selected_address = builder
        .build_select(tagged, address, reference, "reference_address")
        .or_llvm_err()?;
    let selected_width = builder
        .build_select(tagged, width, word.const_int(8, false), "reference_width")
        .or_llvm_err()?;
    Ok((
        selected_address.into_int_value(),
        selected_width.into_int_value(),
    ))
}

/// Keep each width switch in one internal function. A reference may have
/// crossed a generic call or a sum before a consumer reads its storage.
pub(super) fn load<'ctx>(
    llvm: &'ctx Context,
    module: &Module<'ctx>,
    caller: &Builder<'ctx>,
    reference: IntValue<'ctx>,
    name: &str,
) -> Result<IntValue<'ctx>> {
    let word = llvm.i64_type();
    let ty = word.fn_type(&[word.into()], false);
    let function =
        super::error::get_or_declare_function(module, "verum_indexed_reference_load", ty);
    if function.count_basic_blocks() == 0 {
        function.set_linkage(Linkage::Internal);
        let builder = llvm.create_builder();
        builder.position_at_end(llvm.append_basic_block(function, "entry"));
        let reference = function
            .get_first_param()
            .expect("declared reference")
            .into_int_value();
        let (address, width) = decode_at(llvm, module, &builder, reference)?;
        let pointer = storage_pointer(llvm, &builder, address, width)?;
        let storage = super::slice_cell::CellEnv {
            llvm,
            heap_floor: super::target_triple::heap_floor(module),
        };
        let value = storage.elem_load(&builder, width, pointer, "slot")?;
        builder.build_return(Some(&value)).or_llvm_err()?;
    }
    Ok(caller
        .build_call(function, &[reference.into()], name)
        .or_llvm_err()?
        .basic_value_or("indexed reference load")?
        .into_int_value())
}

pub(super) fn storage_pointer<'ctx>(
    llvm: &'ctx Context,
    builder: &Builder<'ctx>,
    address: IntValue<'ctx>,
    width: IntValue<'ctx>,
) -> Result<verum_llvm::values::PointerValue<'ctx>> {
    let address = if super::instruction::ref_tagging_enabled() {
        let word = llvm.i64_type();
        let full = builder
            .build_int_compare(
                IntPredicate::EQ,
                width,
                word.const_int(8, false),
                "word_reference",
            )
            .or_llvm_err()?;
        let slot = builder
            .build_and(address, word.const_int(!1, false), "word_address")
            .or_llvm_err()?;
        builder
            .build_select(full, slot, address, "storage_address")
            .or_llvm_err()?
            .into_int_value()
    } else {
        address
    };
    builder
        .build_int_to_ptr(
            address,
            llvm.ptr_type(AddressSpace::default()),
            "storage_pointer",
        )
        .or_llvm_err()
}

pub(super) fn check_values<'ctx>(
    ctx: &FunctionContext<'_, 'ctx>,
    replacements: &[(IntValue<'ctx>, IntValue<'ctx>)],
) -> Result<()> {
    if !supported(ctx) {
        return Ok(());
    }
    let llvm = ctx.llvm_context();
    let word = llvm.i64_type();
    let builder = ctx.builder();
    let mut valid = llvm.bool_type().const_all_ones();
    for &(width, value) in replacements {
        // Native packed Bytes hold an unsigned value. An ordinary Value slot
        // accepts the complete word, including pointers and signed values.
        let byte = builder
            .build_int_compare(
                IntPredicate::EQ,
                width,
                word.const_int(1, false),
                "packed_byte",
            )
            .or_llvm_err()?;
        let fits = builder
            .build_int_compare(
                IntPredicate::ULE,
                value,
                word.const_int(255, false),
                "fits_byte",
            )
            .or_llvm_err()?;
        let slot_valid = builder
            .build_select(
                byte,
                fits,
                llvm.bool_type().const_all_ones(),
                "fits_storage",
            )
            .or_llvm_err()?
            .into_int_value();
        valid = builder
            .build_and(valid, slot_valid, "valid_replacement")
            .or_llvm_err()?;
    }
    let accepted = llvm.append_basic_block(ctx.function(), "checked_store");
    let refused = llvm.append_basic_block(ctx.function(), "invalid_packed_value");
    builder
        .build_conditional_branch(valid, accepted, refused)
        .or_llvm_err()?;
    builder.position_at_end(refused);
    super::instruction::emit_runtime_abort(
        ctx,
        "value exceeds packed byte storage",
        "packed_byte_value",
    )?;
    builder.build_unreachable().or_llvm_err()?;
    builder.position_at_end(accepted);
    Ok(())
}

pub(super) fn store<'ctx>(
    ctx: &FunctionContext<'_, 'ctx>,
    pointer: verum_llvm::values::PointerValue<'ctx>,
    width: IntValue<'ctx>,
    value: IntValue<'ctx>,
) -> Result<()> {
    if !supported(ctx) {
        ctx.builder().build_store(pointer, value).or_llvm_err()?;
        return Ok(());
    }
    let llvm = ctx.llvm_context();
    let module = ctx.get_module();
    let word = llvm.i64_type();
    let ptr_type = llvm.ptr_type(AddressSpace::default());
    let ty = llvm
        .void_type()
        .fn_type(&[ptr_type.into(), word.into(), word.into()], false);
    let function =
        super::error::get_or_declare_function(module, "verum_indexed_reference_store", ty);
    if function.count_basic_blocks() == 0 {
        function.set_linkage(Linkage::Internal);
        let builder = llvm.create_builder();
        builder.position_at_end(llvm.append_basic_block(function, "entry"));
        let [pointer, width, value] = function.get_params()[..] else {
            unreachable!("declared store signature");
        };
        let storage = super::slice_cell::CellEnv {
            llvm,
            heap_floor: super::target_triple::heap_floor(module),
        };
        storage.elem_store(
            &builder,
            width.into_int_value(),
            pointer.into_pointer_value(),
            value.into_int_value(),
            "slot",
        )?;
        builder.build_return(None).or_llvm_err()?;
    }
    ctx.builder()
        .build_call(
            function,
            &[pointer.into(), width.into(), value.into()],
            "indexed_store",
        )
        .or_llvm_err()?;
    Ok(())
}
