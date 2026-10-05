//! Shared native output boundaries: a platform write and progress-checked write-all.
use super::error::{BuildExt, CallSiteExt, LlvmLoweringError, OptionExt, Result};
use verum_llvm::{
    AddressSpace, DLLStorageClass, IntPredicate,
    context::Context,
    module::{Linkage, Module},
    values::FunctionValue,
};

/// Synchronous standard-stream write on Windows. The runtime has no owned
/// fd-to-HANDLE table yet: only the declared standard descriptors 0/1/2 are
/// supported; arbitrary integers must not be reinterpreted as OS handles.
pub(super) fn emit_windows_write<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    function: FunctionValue<'ctx>,
) -> Result<()> {
    let i64 = context.i64_type();
    let i32 = context.i32_type();
    let ptr = context.ptr_type(AddressSpace::default());
    let std_type = ptr.fn_type(&[i32.into()], false);
    let write_type = i32.fn_type(
        &[ptr.into(), ptr.into(), i32.into(), ptr.into(), ptr.into()],
        false,
    );
    let declare = |name: &str, ty| -> Result<FunctionValue<'ctx>> {
        let function = module
            .get_function(name)
            .unwrap_or_else(|| module.add_function(name, ty, None));
        if function.get_type() != ty {
            return Err(LlvmLoweringError::internal(format!(
                "incompatible Windows output ABI for {name}"
            )));
        }
        if function.count_basic_blocks() > 0 {
            return Err(LlvmLoweringError::internal(format!(
                "Windows output import {name} already has a body"
            )));
        }
        // Explicit platform provenance survives the final bodyless-function
        // pass; DLL storage/linkage alone is not semantic declaration authority.
        super::error::mark_platform_extern(context, function);
        function
            .as_global_value()
            .set_dll_storage_class(DLLStorageClass::Import);
        Ok(function)
    };
    let get_std_handle = declare("GetStdHandle", std_type)?;
    let write_file = declare("WriteFile", write_type)?;
    let builder = context.create_builder();
    let entry = context.append_basic_block(function, "entry");
    let resolve = context.append_basic_block(function, "resolve_standard_handle");
    let write = context.append_basic_block(function, "write");
    let success = context.append_basic_block(function, "success");
    let fail = context.append_basic_block(function, "error");
    let empty = context.append_basic_block(function, "empty");
    builder.position_at_end(entry);
    let fd = function
        .get_nth_param(0)
        .or_internal("write fd")?
        .into_int_value();
    let bytes = function
        .get_nth_param(1)
        .or_internal("write buffer")?
        .into_pointer_value();
    let count = function
        .get_nth_param(2)
        .or_internal("write count")?
        .into_int_value();
    let written = builder.build_alloca(i32, "written").or_llvm_err()?;
    let valid_fd = builder
        .build_int_compare(
            IntPredicate::ULE,
            fd,
            i64.const_int(2, false),
            "supported_fd",
        )
        .or_llvm_err()?;
    let valid_count = builder
        .build_int_compare(IntPredicate::SGE, count, i64.const_zero(), "valid_count")
        .or_llvm_err()?;
    let valid = builder
        .build_and(valid_fd, valid_count, "valid_request")
        .or_llvm_err()?;
    let check_empty = context.append_basic_block(function, "check_empty");
    builder
        .build_conditional_branch(valid, check_empty, fail)
        .or_llvm_err()?;
    builder.position_at_end(check_empty);
    let zero = builder
        .build_int_compare(IntPredicate::EQ, count, i64.const_zero(), "zero_count")
        .or_llvm_err()?;
    builder
        .build_conditional_branch(zero, empty, resolve)
        .or_llvm_err()?;
    builder.position_at_end(empty);
    builder
        .build_return(Some(&i64.const_zero()))
        .or_llvm_err()?;
    builder.position_at_end(resolve);
    let fd32 = builder.build_int_truncate(fd, i32, "fd32").or_llvm_err()?;
    let selector = builder
        .build_int_sub(
            i32.const_int((-10_i32) as u64, true),
            fd32,
            "standard_selector",
        )
        .or_llvm_err()?;
    let handle = builder
        .build_call(get_std_handle, &[selector.into()], "handle")
        .or_llvm_err()?
        .basic_value_or("GetStdHandle must return HANDLE")?
        .into_pointer_value();
    let handle_bits = builder
        .build_ptr_to_int(handle, i64, "handle_bits")
        .or_llvm_err()?;
    let is_null = builder.build_is_null(handle, "null_handle").or_llvm_err()?;
    let invalid = builder
        .build_int_compare(
            IntPredicate::EQ,
            handle_bits,
            i64.const_all_ones(),
            "invalid_handle",
        )
        .or_llvm_err()?;
    let bad = builder
        .build_or(is_null, invalid, "bad_handle")
        .or_llvm_err()?;
    builder
        .build_conditional_branch(bad, fail, write)
        .or_llvm_err()?;
    builder.position_at_end(write);
    // WriteFile takes DWORD. One low-level write may return a short count;
    // the shared write-all loop advances by exactly that observed progress.
    let max = i64.const_int(u32::MAX as u64, false);
    let large = builder
        .build_int_compare(IntPredicate::UGT, count, max, "large_write")
        .or_llvm_err()?;
    let bounded = builder
        .build_select(large, max, count, "bounded_count")
        .or_llvm_err()?
        .into_int_value();
    let chunk = builder
        .build_int_truncate(bounded, i32, "chunk")
        .or_llvm_err()?;
    builder
        .build_store(written, i32.const_zero())
        .or_llvm_err()?;
    let result = builder
        .build_call(
            write_file,
            &[
                handle.into(),
                bytes.into(),
                chunk.into(),
                written.into(),
                ptr.const_null().into(),
            ],
            "write_ok",
        )
        .or_llvm_err()?
        .basic_value_or("WriteFile must return BOOL")?
        .into_int_value();
    let ok = builder
        .build_int_compare(IntPredicate::NE, result, i32.const_zero(), "succeeded")
        .or_llvm_err()?;
    builder
        .build_conditional_branch(ok, success, fail)
        .or_llvm_err()?;
    builder.position_at_end(success);
    let actual = builder
        .build_load(i32, written, "actual")
        .or_llvm_err()?
        .into_int_value();
    let amount = builder
        .build_int_z_extend(actual, i64, "amount")
        .or_llvm_err()?;
    builder.build_return(Some(&amount)).or_llvm_err()?;
    builder.position_at_end(fail);
    builder
        .build_return(Some(&i64.const_all_ones()))
        .or_llvm_err()?;
    Ok(())
}

/// Write every byte, advancing only by confirmed positive progress. Return
/// the requested count on success, -1 on errors, zero progress or bad counts.
/// The caller owns a readable buffer of `count` bytes for the whole operation.
pub(super) fn get_or_declare_write_all<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    write: FunctionValue<'ctx>,
) -> Result<FunctionValue<'ctx>> {
    let name = "verum_internal_write_all";
    let function = match module.get_function(name) {
        Some(function) if function.count_basic_blocks() > 0 => return Ok(function),
        Some(function) => function,
        None => module.add_function(name, write.get_type(), Some(Linkage::Internal)),
    };
    let i64 = context.i64_type();
    let builder = context.create_builder();
    let entry = context.append_basic_block(function, "entry");
    let check = context.append_basic_block(function, "check");
    let step = context.append_basic_block(function, "write");
    let progress = context.append_basic_block(function, "progress");
    let done = context.append_basic_block(function, "done");
    let fail = context.append_basic_block(function, "error");
    builder.position_at_end(entry);
    let fd = function.get_nth_param(0).or_internal("write-all fd")?;
    let buffer = function
        .get_nth_param(1)
        .or_internal("write-all buffer")?
        .into_pointer_value();
    let count = function
        .get_nth_param(2)
        .or_internal("write-all count")?
        .into_int_value();
    let valid = builder
        .build_int_compare(IntPredicate::SGE, count, i64.const_zero(), "valid_count")
        .or_llvm_err()?;
    builder
        .build_conditional_branch(valid, check, fail)
        .or_llvm_err()?;
    builder.position_at_end(check);
    let position = builder.build_phi(i64, "position").or_llvm_err()?;
    position.add_incoming(&[(&i64.const_zero(), entry)]);
    let pos = position.as_basic_value().into_int_value();
    let remaining = builder
        .build_int_sub(count, pos, "remaining")
        .or_llvm_err()?;
    let complete = builder
        .build_int_compare(IntPredicate::EQ, remaining, i64.const_zero(), "complete")
        .or_llvm_err()?;
    builder
        .build_conditional_branch(complete, done, step)
        .or_llvm_err()?;
    builder.position_at_end(step);
    // SAFETY: pos starts at zero and increases only by positive confirmed
    // progress <=remaining. It stays inside the caller's count-byte buffer.
    let bytes = unsafe {
        builder
            .build_gep(context.i8_type(), buffer, &[pos], "bytes")
            .or_llvm_err()?
    };
    let amount = builder
        .build_call(
            write,
            &[fd.into(), bytes.into(), remaining.into()],
            "written",
        )
        .or_llvm_err()?
        .basic_value_or("write must return a count")?
        .into_int_value();
    let positive = builder
        .build_int_compare(IntPredicate::SGT, amount, i64.const_zero(), "positive")
        .or_llvm_err()?;
    let bounded = builder
        .build_int_compare(IntPredicate::ULE, amount, remaining, "bounded")
        .or_llvm_err()?;
    let valid = builder
        .build_and(positive, bounded, "valid_progress")
        .or_llvm_err()?;
    builder
        .build_conditional_branch(valid, progress, fail)
        .or_llvm_err()?;
    builder.position_at_end(progress);
    let next = builder.build_int_add(pos, amount, "next").or_llvm_err()?;
    builder.build_unconditional_branch(check).or_llvm_err()?;
    position.add_incoming(&[(&next, progress)]);
    builder.position_at_end(done);
    builder.build_return(Some(&count)).or_llvm_err()?;
    builder.position_at_end(fail);
    builder
        .build_return(Some(&i64.const_all_ones()))
        .or_llvm_err()?;
    Ok(function)
}
