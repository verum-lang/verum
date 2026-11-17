//! Native aggregate allocation receipts. The producing operation records the
//! allocation domain and complete extent in the native header's padding word.
//! Source fields and the shared 24-byte header layout are unchanged.
use verum_llvm::{builder::Builder, context::Context, module::Module,
    values::{IntValue, PointerValue}, IntPredicate};

use super::error::{BuildExt, LlvmLoweringError, Result, get_or_declare_function};

// ObjectHeader ends with one u32 padding word. Other header words retain their
// existing meaning; in particular, historical size fields have mixed contracts.
const RECEIPT_OFFSET: u64 = verum_common::layout::OBJECT_HEADER_SIZE
    - std::mem::size_of::<u32>() as u64;
const OS_DOMAIN: u32 = 1 << (u32::BITS - 1);
const EXTENT_MASK: u32 = OS_DOMAIN - 1;

pub(super) fn check_extent(bytes: u64) -> Result<()> {
    if bytes < verum_common::layout::OBJECT_HEADER_SIZE
        || bytes > verum_common::layout::MAX_ALLOCATION_SIZE as u64
        || bytes > u64::from(EXTENT_MASK)
    {
        return Err(LlvmLoweringError::unsupported(
            "native aggregate allocation extent is not representable"));
    }
    Ok(())
}

fn slot<'ctx>(context: &'ctx Context, builder: &Builder<'ctx>,
    pointer: PointerValue<'ctx>) -> Result<PointerValue<'ctx>> {
    // SAFETY: callers provide the declared aggregate's complete native header.
    unsafe { builder.build_in_bounds_gep(context.i8_type(), pointer,
        &[context.i64_type().const_int(RECEIPT_OFFSET, false)], "object_storage_slot")
        .or_llvm_err() }
}

/// Issue only after a successful verum_os_alloc of this exact extent. Producers
/// bound the extent before allocation. Copying a source header is not issuance.
pub(super) fn stamp_os<'ctx>(context: &'ctx Context, builder: &Builder<'ctx>,
    pointer: PointerValue<'ctx>, bytes: IntValue<'ctx>) -> Result<()> {
    let extent = builder.build_int_truncate(bytes, context.i32_type(), "object_extent")
        .or_llvm_err()?;
    let receipt = builder.build_or(extent,
        context.i32_type().const_int(u64::from(OS_DOMAIN), false), "object_os_receipt")
        .or_llvm_err()?;
    builder.build_store(slot(context, builder, pointer)?, receipt).or_llvm_err()?;
    Ok(())
}

fn receipt<'ctx>(context: &'ctx Context, builder: &Builder<'ctx>,
    pointer: PointerValue<'ctx>) -> Result<IntValue<'ctx>> {
    Ok(builder.build_load(context.i32_type(), slot(context, builder, pointer)?,
        "object_storage_receipt").or_llvm_err()?.into_int_value())
}

pub(super) fn matches_os<'ctx>(context: &'ctx Context, builder: &Builder<'ctx>,
    pointer: PointerValue<'ctx>, bytes: u64) -> Result<IntValue<'ctx>> {
    check_extent(bytes)?;
    builder.build_int_compare(IntPredicate::EQ, receipt(context, builder, pointer)?,
        context.i32_type().const_int(bytes | u64::from(OS_DOMAIN), false),
        "object_exact_os_allocation").or_llvm_err()
}

/// A sum must prove that its allocation contains a tag before reading that tag.
/// The selected variant subsequently checks the exact complete extent.
pub(super) fn has_os_extent<'ctx>(context: &'ctx Context, builder: &Builder<'ctx>,
    pointer: PointerValue<'ctx>, minimum: u64) -> Result<IntValue<'ctx>> {
    check_extent(minimum)?;
    let value = receipt(context, builder, pointer)?;
    let domain = builder.build_and(value,
        context.i32_type().const_int(u64::from(OS_DOMAIN), false), "object_domain")
        .or_llvm_err()?;
    let domain = builder.build_int_compare(IntPredicate::EQ, domain,
        context.i32_type().const_int(u64::from(OS_DOMAIN), false), "object_os_domain")
        .or_llvm_err()?;
    let extent = builder.build_and(value,
        context.i32_type().const_int(u64::from(EXTENT_MASK), false), "object_extent")
        .or_llvm_err()?;
    let present = builder.build_int_compare(IntPredicate::UGE, extent,
        context.i32_type().const_int(minimum, false), "object_extent_present")
        .or_llvm_err()?;
    let bounded = builder.build_int_compare(IntPredicate::ULE, extent,
        context.i32_type().const_int(verum_common::layout::MAX_ALLOCATION_SIZE as u64, false),
        "object_extent_bounded").or_llvm_err()?;
    let valid = builder.build_and(domain, present, "object_storage_present").or_llvm_err()?;
    builder.build_and(valid, bounded, "object_storage_bounded").or_llvm_err()
}

/// The caller has checked the exact receipt. Consume it before component cleanup
/// can invoke source code; the eventual release uses that proven domain/extent.
pub(super) fn consume<'ctx>(context: &'ctx Context, builder: &Builder<'ctx>,
    pointer: PointerValue<'ctx>) -> Result<()> {
    builder.build_store(slot(context, builder, pointer)?, context.i32_type().const_zero())
        .or_llvm_err()?;
    Ok(())
}

pub(super) fn release_os<'ctx>(context: &'ctx Context, builder: &Builder<'ctx>,
    module: &Module<'ctx>, pointer: PointerValue<'ctx>, bytes: u64) -> Result<()> {
    check_extent(bytes)?;
    let release = get_or_declare_function(module, "verum_os_free",
        context.void_type().fn_type(&[pointer.get_type().into(), context.i64_type().into()], false));
    builder.build_call(release, &[pointer.into(), context.i64_type().const_int(bytes, false).into()], "")
        .or_llvm_err()?;
    Ok(())
}
