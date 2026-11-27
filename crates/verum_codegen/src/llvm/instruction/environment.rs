//! Acquire the target process environment and copy it into owned Verum storage.
use super::*;
use crate::llvm::error::get_or_declare_function;
use crate::llvm::target_triple::{target_is_freebsd, target_is_linux, target_is_windows};
use verum_llvm::module::Linkage;

/// Darwin's environ table and its strings can be replaced by setenv and unsetenv.
pub(super) fn darwin_lock(ctx: &FunctionContext<'_, '_>, acquire: bool) -> Result<()> {
    let module = ctx.get_module();
    if !target_is_darwin(module) {
        return Ok(());
    }
    let lock = module
        .get_global("__verum_environment_lock")
        .unwrap_or_else(|| {
            let ty = ctx.types().i32_type();
            let lock = module.add_global(ty, None, "__verum_environment_lock");
            lock.set_linkage(Linkage::Internal);
            lock.set_initializer(&ty.const_zero());
            lock
        });
    let name = if acquire {
        "os_unfair_lock_lock"
    } else {
        "os_unfair_lock_unlock"
    };
    let op = get_or_declare_function(
        module,
        name,
        ctx.types()
            .void_type()
            .fn_type(&[ctx.types().ptr_type().into()], false),
    );
    ctx.builder()
        .build_call(op, &[lock.as_pointer_value().into()], "")
        .or_llvm_err()?;
    Ok(())
}

/// The OS writes a byte prefix into ordinary mutable List storage.
fn snapshot_storage<'ctx>(
    ctx: &FunctionContext<'_, 'ctx>,
    length: IntValue<'ctx>,
) -> Result<(PointerValue<'ctx>, PointerValue<'ctx>)> {
    let b = ctx.builder();
    let rt = RuntimeLowering::new(ctx.llvm_context());
    let owner = rt.lower_new_list_storage(b, ctx.get_module(), length)?;
    let len = rt.list_field_address(
        b,
        owner,
        super::super::runtime::LIST_LEN_OFFSET,
        "environment_len",
    )?;
    let data = rt.list_field_address(
        b,
        owner,
        super::super::runtime::LIST_PTR_OFFSET,
        "environment_data",
    )?;
    b.build_store(len, length).or_llvm_err()?;
    let data = b
        .build_load(ctx.types().ptr_type(), data, "env_buffer")
        .or_llvm_err()?
        .into_pointer_value();
    Ok((owner, data))
}

/// Expand the OS byte prefix into its owning List's value slots in reverse
/// order so each unread input byte survives the wider stores.
fn initialize_byte_slots<'ctx>(
    ctx: &FunctionContext<'_, 'ctx>,
    buffer: PointerValue<'ctx>,
    length: IntValue<'ctx>,
) -> Result<()> {
    let b = ctx.builder();
    let llvm = ctx.llvm_context();
    let word = ctx.types().i64_type();
    let start = b
        .get_insert_block()
        .or_internal("environment buffer entry")?;
    let next = llvm.append_basic_block(ctx.function(), "env_expand_next");
    let copy = llvm.append_basic_block(ctx.function(), "env_expand_byte");
    let done = llvm.append_basic_block(ctx.function(), "env_values_ready");
    b.build_unconditional_branch(next).or_llvm_err()?;
    b.position_at_end(next);
    let remaining = b.build_phi(word, "remaining_bytes").or_llvm_err()?;
    remaining.add_incoming(&[(&length, start)]);
    let count = remaining.as_basic_value().into_int_value();
    let empty = b
        .build_int_compare(
            IntPredicate::EQ,
            count,
            word.const_zero(),
            "all_bytes_stored",
        )
        .or_llvm_err()?;
    b.build_conditional_branch(empty, done, copy)
        .or_llvm_err()?;
    b.position_at_end(copy);
    let index = b
        .build_int_sub(count, word.const_int(1, false), "byte_index")
        .or_llvm_err()?;
    // SAFETY: index is in 0..length and the allocator reserved length value slots.
    let (source, dest) = unsafe {
        (
            b.build_in_bounds_gep(llvm.i8_type(), buffer, &[index], "env_byte")
                .or_llvm_err()?,
            b.build_in_bounds_gep(word, buffer, &[index], "env_value_slot")
                .or_llvm_err()?,
        )
    };
    let byte = b
        .build_load(llvm.i8_type(), source, "environment_byte")
        .or_llvm_err()?
        .into_int_value();
    let value = b
        .build_int_z_extend(byte, word, "byte_value")
        .or_llvm_err()?;
    b.build_store(dest, value).or_llvm_err()?;
    remaining.add_incoming(&[(&index, copy)]);
    b.build_unconditional_branch(next).or_llvm_err()?;
    b.position_at_end(done);
    Ok(())
}

/// With the table held stable, measure or copy every C string including its terminator.
fn unix_entries<'ctx>(
    ctx: &FunctionContext<'_, 'ctx>,
    table: PointerValue<'ctx>,
    output: Option<PointerValue<'ctx>>,
) -> Result<IntValue<'ctx>> {
    let llvm = ctx.llvm_context();
    let b = ctx.builder();
    let word = ctx.types().i64_type();
    let ptr = ctx.types().ptr_type();
    let start = b
        .get_insert_block()
        .or_internal("environment table entry")?;
    let next = llvm.append_basic_block(ctx.function(), "env_next");
    let read = llvm.append_basic_block(ctx.function(), "env_read");
    let done = llvm.append_basic_block(ctx.function(), "env_done");
    let empty = b.build_is_null(table, "null_environment").or_llvm_err()?;
    b.build_conditional_branch(empty, done, next)
        .or_llvm_err()?;
    b.position_at_end(next);
    let slot = b.build_phi(ptr, "env_slot").or_llvm_err()?;
    let offset = b.build_phi(word, "env_offset").or_llvm_err()?;
    slot.add_incoming(&[(&table, start)]);
    offset.add_incoming(&[(&word.const_zero(), start)]);
    let cursor = slot.as_basic_value().into_pointer_value();
    let size = offset.as_basic_value().into_int_value();
    let entry = b
        .build_load(ptr, cursor, "env_entry")
        .or_llvm_err()?
        .into_pointer_value();
    let end = b.build_is_null(entry, "end_of_environment").or_llvm_err()?;
    b.build_conditional_branch(end, done, read).or_llvm_err()?;
    b.position_at_end(read);
    let runtime = RuntimeLowering::new(llvm);
    let strlen = runtime.get_or_declare_strlen(ctx.get_module())?;
    let count = b
        .build_call(strlen, &[entry.into()], "env_text_len")
        .or_llvm_err()?
        .basic_value_or("missing environment entry length")?
        .into_int_value();
    let count = b
        .build_int_add(count, word.const_int(1, false), "env_entry_size")
        .or_llvm_err()?;
    if let Some(buffer) = output {
        // SAFETY: the first traversal reserved the sum of all entry sizes.
        let dst = unsafe {
            b.build_in_bounds_gep(llvm.i8_type(), buffer, &[size], "env_dst")
                .or_llvm_err()?
        };
        b.build_call(
            runtime.get_or_declare_memcpy(ctx.get_module()),
            &[dst.into(), entry.into(), count.into()],
            "",
        )
        .or_llvm_err()?;
    }
    let total = b.build_int_add(size, count, "env_total").or_llvm_err()?;
    // SAFETY: a non-null entry is followed by another slot or the final null slot.
    let following = unsafe {
        b.build_in_bounds_gep(ptr, cursor, &[word.const_int(1, false)], "next_env_slot")
            .or_llvm_err()?
    };
    slot.add_incoming(&[(&following, read)]);
    offset.add_incoming(&[(&total, read)]);
    b.build_unconditional_branch(next).or_llvm_err()?;
    b.position_at_end(done);
    let length = b.build_phi(word, "env_length").or_llvm_err()?;
    length.add_incoming(&[(&word.const_zero(), start), (&size, next)]);
    Ok(length.as_basic_value().into_int_value())
}

fn windows_snapshot<'ctx>(ctx: &FunctionContext<'_, 'ctx>) -> Result<PointerValue<'ctx>> {
    let llvm = ctx.llvm_context();
    let module = ctx.get_module();
    let b = ctx.builder();
    let ptr = ctx.types().ptr_type();
    let int = ctx.types().i32_type();
    let word = ctx.types().i64_type();
    let get = get_or_declare_function(module, "GetEnvironmentStringsW", ptr.fn_type(&[], false));
    let free = get_or_declare_function(
        module,
        "FreeEnvironmentStringsW",
        int.fn_type(&[ptr.into()], false),
    );
    let convert = get_or_declare_function(
        module,
        "WideCharToMultiByte",
        int.fn_type(
            &[
                int.into(),
                int.into(),
                ptr.into(),
                int.into(),
                ptr.into(),
                int.into(),
                ptr.into(),
                ptr.into(),
            ],
            false,
        ),
    );
    let source = b
        .build_call(get, &[], "wide_environment")
        .or_llvm_err()?
        .basic_value_or("missing environment block")?
        .into_pointer_value();
    let block = |name| llvm.append_basic_block(ctx.function(), name);
    let start = b
        .get_insert_block()
        .or_internal("Windows environment entry")?;
    let scan = block("env_scan_utf16");
    let advance = block("env_advance_utf16");
    let measured = block("env_wide_measured");
    let allocate = block("env_allocate_utf8");
    let write = block("env_write_utf8");
    let done = block("env_snapshot_done");
    let invalid = block("env_conversion_failure");
    let error = block("env_snapshot_failure");
    let missing = b
        .build_is_null(source, "missing_wide_environment")
        .or_llvm_err()?;
    b.build_conditional_branch(missing, error, scan)
        .or_llvm_err()?;
    b.position_at_end(scan);
    let count = b.build_phi(word, "wide_length").or_llvm_err()?;
    let entry_start = b
        .build_phi(llvm.bool_type(), "env_entry_start")
        .or_llvm_err()?;
    count.add_incoming(&[(&word.const_zero(), start)]);
    entry_start.add_incoming(&[(&llvm.bool_type().const_int(1, false), start)]);
    let length = count.as_basic_value().into_int_value();
    // SAFETY: GetEnvironmentStringsW owns a double-NUL-terminated UTF-16 block.
    let slot = unsafe {
        b.build_in_bounds_gep(llvm.i16_type(), source, &[length], "wide_env_slot")
            .or_llvm_err()?
    };
    let unit = b
        .build_load(llvm.i16_type(), slot, "wide_unit")
        .or_llvm_err()?
        .into_int_value();
    let nul = b
        .build_int_compare(
            IntPredicate::EQ,
            unit,
            llvm.i16_type().const_zero(),
            "wide_nul",
        )
        .or_llvm_err()?;
    let end = b
        .build_and(
            entry_start.as_basic_value().into_int_value(),
            nul,
            "end_of_block",
        )
        .or_llvm_err()?;
    b.build_conditional_branch(end, measured, advance)
        .or_llvm_err()?;
    b.position_at_end(advance);
    let next = b
        .build_int_add(length, word.const_int(1, false), "wide_next")
        .or_llvm_err()?;
    count.add_incoming(&[(&next, advance)]);
    entry_start.add_incoming(&[(&nul, advance)]);
    let fits = b
        .build_int_compare(
            IntPredicate::ULE,
            next,
            word.const_int(i32::MAX as u64, false),
            "valid_wide_length",
        )
        .or_llvm_err()?;
    b.build_conditional_branch(fits, scan, invalid)
        .or_llvm_err()?;
    b.position_at_end(measured);
    let empty = b
        .build_int_compare(
            IntPredicate::EQ,
            length,
            word.const_zero(),
            "empty_environment",
        )
        .or_llvm_err()?;
    let measure = block("env_measure_utf8");
    b.build_conditional_branch(empty, allocate, measure)
        .or_llvm_err()?;
    b.position_at_end(measure);
    let length32 = b
        .build_int_truncate(length, int, "wide_length_i32")
        .or_llvm_err()?;
    let convert_utf8 = |length32: IntValue<'ctx>,
                        dest: PointerValue<'ctx>,
                        capacity: IntValue<'ctx>|
     -> Result<IntValue<'ctx>> {
        Ok(b.build_call(
            convert,
            &[
                int.const_int(65001, false).into(),
                int.const_zero().into(),
                source.into(),
                length32.into(),
                dest.into(),
                capacity.into(),
                ptr.const_null().into(),
                ptr.const_null().into(),
            ],
            "environment_utf8_bytes",
        )
        .or_llvm_err()?
        .basic_value_or("missing converted length")?
        .into_int_value())
    };
    let required = convert_utf8(length32, ptr.const_null(), int.const_zero())?;
    let valid = b
        .build_int_compare(
            IntPredicate::SGT,
            required,
            int.const_zero(),
            "env_size_valid",
        )
        .or_llvm_err()?;
    b.build_conditional_branch(valid, allocate, invalid)
        .or_llvm_err()?;
    b.position_at_end(allocate);
    let result_size = b.build_phi(int, "env_result_size").or_llvm_err()?;
    result_size.add_incoming(&[(&int.const_zero(), measured), (&required, measure)]);
    let capacity = result_size.as_basic_value().into_int_value();
    let size64 = b
        .build_int_z_extend(capacity, word, "env_byte_capacity")
        .or_llvm_err()?;
    let (owner, dest) = snapshot_storage(ctx, size64)?;
    b.build_conditional_branch(empty, done, write)
        .or_llvm_err()?;
    b.position_at_end(write);
    let length32 = b
        .build_int_truncate(length, int, "env_char_count")
        .or_llvm_err()?;
    let written = convert_utf8(length32, dest, capacity)?;
    let complete = b
        .build_int_compare(IntPredicate::EQ, written, capacity, "environment_complete")
        .or_llvm_err()?;
    b.build_conditional_branch(complete, done, invalid)
        .or_llvm_err()?;
    b.position_at_end(invalid);
    b.build_call(free, &[source.into()], "").or_llvm_err()?;
    b.build_unconditional_branch(error).or_llvm_err()?;
    b.position_at_end(error);
    emit_runtime_abort(
        ctx,
        "could not snapshot the process environment",
        "environment_failure",
    )?;
    b.build_unreachable().or_llvm_err()?;
    b.position_at_end(done);
    b.build_call(free, &[source.into()], "").or_llvm_err()?;
    initialize_byte_slots(ctx, dest, size64)?;
    Ok(owner)
}

pub(super) fn snapshot<'ctx>(ctx: &FunctionContext<'_, 'ctx>) -> Result<PointerValue<'ctx>> {
    let module = ctx.get_module();
    let b = ctx.builder();
    let ptr = ctx.types().ptr_type();
    if target_is_windows(module) {
        return windows_snapshot(ctx);
    }
    let cell = if target_is_darwin(module) {
        darwin_lock(ctx, true)?;
        let get = get_or_declare_function(module, "_NSGetEnviron", ptr.fn_type(&[], false));
        b.build_call(get, &[], "environment_cell")
            .or_llvm_err()?
            .basic_value_or("missing environ cell")?
            .into_pointer_value()
    } else if target_is_linux(module) || target_is_freebsd(module) {
        module
            .get_global("__verum_envp")
            .or_internal("process startup environment is missing")?
            .as_pointer_value()
    } else {
        return Err(LlvmLoweringError::UnsupportedInstruction(
            format!(
                "process environment is unavailable for {}",
                module.get_triple().as_str().to_string_lossy()
            )
            .into(),
        ));
    };
    let table = b
        .build_load(ptr, cell, "environment_table")
        .or_llvm_err()?
        .into_pointer_value();
    let length = unix_entries(ctx, table, None)?;
    let (owner, output) = snapshot_storage(ctx, length)?;
    unix_entries(ctx, table, Some(output))?;
    darwin_lock(ctx, false)?;
    initialize_byte_slots(ctx, output, length)?;
    Ok(owner)
}
