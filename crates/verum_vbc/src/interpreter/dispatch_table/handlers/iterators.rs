//! Iterator and range instruction handlers for VBC interpreter.

use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::heap;
use super::super::super::state::{GeneratorId, GeneratorStatus, InterpreterState};
use super::super::DispatchResult;
use super::super::dispatch_loop_table_with_entry_depth;
use super::bytecode_io::*;
use crate::instruction::Reg;
use crate::types::TypeId;
use crate::value::Value;

// ── Iterator type constants ──
const ITER_TYPE_LIST: i64 = 0;
const ITER_TYPE_MAP: i64 = 1;
const ITER_TYPE_ARRAY: i64 = 2;
const ITER_TYPE_RANGE: i64 = 3;
const ITER_TYPE_GENERATOR: i64 = 4;
/// `List<Byte>` packed-byte iteration (red-team §4 fix).
/// Element stride is 1 byte instead of `sizeof(Value)`; reads
/// zero-extend the byte into a NaN-boxed Value.
const ITER_TYPE_BYTE_LIST: i64 = 5;
/// **Iterator-protocol fallback** (PROTOCOL-ITER-1, closes the
/// hazard/reclaim SIGSEGV class).  A for-in whose iterable is a
/// stdlib/user Iterator RECORD (`Drain<T>`, adapter chains, …) may
/// still be lowered to native IterNew/IterNext when codegen could not
/// classify the iterable's type.  Pre-fix the type-discrimination
/// mapped every non-builtin `type_id` to ITER_TYPE_LIST and IterNext
/// then read the record's fields as a `List` header — a memory-unsafe
/// misinterpretation whose outcome depended on the record's field
/// values (immediate exit, garbage elements, or SIGSEGV).
///
/// With this tag, IterNew detects a record whose type has a
/// resolvable 1-arg `<Type>.next` method and stores that FunctionId
/// in blob slot 1; IterNext dispatches `next(&mut self)` through
/// `call_function_sync` and unpacks the returned `Maybe<T>`.
/// Iteration semantics are therefore IDENTICAL between the native
/// lowering and the codegen protocol-loop lowering.
const ITER_TYPE_PROTOCOL: i64 = 6;
/// BYTE_SLICE (528) byte-view iteration (ARCH-P5): `for b in
/// text.as_bytes()`.  The source object's payload is TWO RAW i64
/// slots `{ptr, len}` — NOT NaN-boxed Values — so IterNext reads them
/// as raw words and yields one zero-extended byte per step.  Before
/// the typed byte view existed, a byte-slice iterable reached the
/// ITER_TYPE_LIST fallback and its FatRef marker payload was read as
/// a List header → SIGSEGV.
const ITER_TYPE_BYTE_SLICE: i64 = 7;
/// FATREF-ITER-1 (#40): iteration over a general FatRef slice —
/// `for c in &list[..]`, `for x in slice_param`.  A FatRef Value's
/// 48-bit payload is `FAT_REF_MARKER | table-index` (bits 47-45 set),
/// NOT a heap pointer, and `is_ptr()` EXCLUDES FatRefs — so the
/// legacy classifier fell through to "non-pointer → ITER_TYPE_LIST"
/// and IterNext dereferenced the marker bits as a List header
/// (SIGSEGV at 0xE000_0000_00xx).  Slot 0 keeps the ENCODED FatRef
/// Value; each IterNext re-resolves it and reads the element through
/// `fat_ref_read_element` — the same authority `GetE` uses, honoring
/// the `reserved` elem-size dispatch (0=Value, 1/2/4/8=raw).
const ITER_TYPE_FATREF_SLICE: i64 = 8;

/// TYPED-ARRAY-ITER-1: packed `[T; N]` typed arrays (`NewTypedArray`'s
/// raw buffers stamped with SCALAR TypeIds — U8/U16/U32/U64/F32/F64).
/// Without this leg the classifier fell through to `ITER_TYPE_LIST`,
/// read the raw element bytes as a `[len, cap, backing]` list header
/// and dereferenced a garbage backing pointer (`for item in values`
/// inside `Set.from([1, 2, 3])` → SIGSEGV).  Element geometry/decode
/// authority: `heap::typed_array_element{_spec,}`.
const ITER_TYPE_TYPED_ARRAY: i64 = 9;
const ITER_TYPE_TEXT: i64 = 10;

// ============================================================================
// Iterator + Range Operations
// ============================================================================

/// Allocate the cursor used by `IterNext` for a supported source value.
/// The source representation or its declared iterator method determines access.
pub(in super::super) fn handle_iter_new(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;

    let source = state.get_reg(src);

    // If source is a reference (e.g. `&List<Int>` parameter, or a
    // FIELD reference from `for x in &h.items`), deref it to the
    // actual collection pointer. REFFIELD-LIST-FORITER-EMPTY-1: the
    // pre-fix arm handled ONLY the register-ref encoding
    // (`cbgr_helpers::is_cbgr_ref` — the inline-negative-int shape);
    // a field reference (ThinRef / `cbgr_mutable_ptrs` heap-slot
    // pointer from `CbgrExtended RefField`) fell through unchanged,
    // and the slot ADDRESS was then read as a List header → SIGSEGV.
    // `resolve_arg_value` handles all three reference shapes.
    let source = super::cbgr_helpers::resolve_arg_value(state, source)?;

    // Check for generator values first (NaN-boxed generator tag, not pointer)
    if source.is_generator() {
        // Generator iterator: store generator value directly in iterator object.
        // IterNext will detect ITER_TYPE_GENERATOR and resume the generator.
        let iter_obj = state
            .heap
            .alloc_native_iterator::<3>()?;
        state.record_allocation();
        let iter_ptr =
            unsafe { (iter_obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value };
        unsafe {
            *iter_ptr = source; // generator value
            *iter_ptr.add(1) = Value::from_i64(0); // unused for generators
            *iter_ptr.add(2) = Value::from_i64(ITER_TYPE_GENERATOR); // iter_type
        }
        state.set_reg(dst, Value::from_ptr(iter_obj.as_ptr() as *mut u8));
        return Ok(DispatchResult::Continue);
    }

    // Native method iterators keep front/back cursors. Ordinary records can
    // have the same size and contents, so require the producer's layout flag.
    let source = if source.is_regular_ptr() && state.heap.contains(source.as_ptr()) {
        let ptr = source.as_ptr::<heap::ObjectHeader>();
        // SAFETY: the exact allocation belongs to this interpreter.
        if unsafe { (*ptr).is_native_iterator(4) } {
            unsafe { *ptr.cast::<u8>().add(heap::OBJECT_HEADER_SIZE).cast::<Value>() }
        } else {
            source
        }
    } else {
        source
    };

    // Determine collection type by examining the object header.
    // For non-builtin records, `protocol_next_fid` carries the resolved
    // `<Type>.next` FunctionId into blob slot 1 (see ITER_TYPE_PROTOCOL).
    let mut protocol_next_fid: Option<crate::module::FunctionId> = None;
    let iter_type = if source.is_small_string() {
        ITER_TYPE_TEXT
    } else if source.is_fat_ref() {
        // FATREF-ITER-1 (#40): general slice iteration. FatRefs fail
        // `is_ptr()` and previously fell to the non-pointer LIST
        // default; IterNext then dereferenced the FAT_REF_MARKER
        // payload bits as a List header → SIGSEGV. Keep the encoded
        // Value in slot 0; IterNext resolves per step.
        ITER_TYPE_FATREF_SLICE
    } else if source.is_regular_ptr() && state.heap.contains(source.as_ptr()) {
        // SAFETY: contains checks an exact allocation base.
        let header = unsafe { &*source.as_ptr::<heap::ObjectHeader>() };
        if !header.is_valid() {
            return Err(InterpreterError::InvalidOperand { message: "IterNew requires a live iterable".into() });
        }
        match header.type_id {
            TypeId::MAP | TypeId::SET => ITER_TYPE_MAP,
            TypeId::ARRAY => ITER_TYPE_ARRAY,
            TypeId::RANGE => ITER_TYPE_RANGE,
            TypeId::BYTE_LIST => ITER_TYPE_BYTE_LIST,
            TypeId::BYTE_SLICE => ITER_TYPE_BYTE_SLICE,
            TypeId::LIST => ITER_TYPE_LIST,
            TypeId::TEXT if header.size as usize >= heap::TEXT_RECORD_SIZE => ITER_TYPE_TEXT,
            // Packed typed arrays — scalar-TypeId raw buffers.
            TypeId::U8
            | TypeId::U16
            | TypeId::U32
            | TypeId::U64
            | TypeId::F32
            | TypeId::F64 => ITER_TYPE_TYPED_ARRAY,
            other => {
                let fid = resolve_protocol_next(state, other).ok_or_else(|| {
                    InterpreterError::InvalidOperand {
                        message: format!("IterNew: type {} has no iterable representation or next method", other.0).into(),
                    }
                })?;
                protocol_next_fid = Some(fid);
                ITER_TYPE_PROTOCOL
            }
        }
    } else {
        return Err(InterpreterError::InvalidOperand { message: "IterNew requires an iterable value".into() });
    };

    // Allocate iterator object: [source_ptr, current_idx, iter_type]
    let iter_obj = state
        .heap
        .alloc_native_iterator::<3>()?;
    state.record_allocation();

    let iter_ptr =
        unsafe { (iter_obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value };

    // Initialize iterator.  Slot 1 is the cursor for indexed iteration
    // and the `next` FunctionId for protocol iteration.
    let slot1 = match protocol_next_fid {
        Some(fid) => Value::from_i64(fid.0 as i64),
        None if iter_type == ITER_TYPE_RANGE => unsafe {
            *(source.as_ptr::<u8>().add(heap::OBJECT_HEADER_SIZE) as *const Value)
        },
        None => Value::from_i64(0),
    };
    unsafe {
        *iter_ptr = source; // source_ptr
        *iter_ptr.add(1) = slot1; // current_idx = 0 | protocol next fid
        *iter_ptr.add(2) = Value::from_i64(iter_type); // iter_type
    }

    state.set_reg(dst, Value::from_ptr(iter_obj.as_ptr() as *mut u8));
    Ok(DispatchResult::Continue)
}

/// Resolve the Iterator-protocol `next` method for a non-builtin
/// record type (PROTOCOL-ITER-1).  Accepts only a 1-arg (`&mut self`)
/// function with a real body whose name is exactly `<Type>.next` or
/// ends with `.<Type>.next` (the module-qualified bundled form).  The
/// tight name discipline mirrors the protocol-default fallback in
/// `method_dispatch.rs` — loose `.next` suffix matches would route a
/// record to a SIBLING iterator's `next` (the Chars/Rev collision
/// class).
fn resolve_protocol_next(
    state: &InterpreterState,
    type_id: TypeId,
) -> Option<crate::module::FunctionId> {
    let td = state.module.get_type(type_id)?;
    let ty_name = state.module.strings.get(td.name)?;
    if ty_name.is_empty() {
        return None;
    }
    let qualified = format!("{}.next", ty_name);
    let dotted = format!(".{}.next", ty_name);
    state
        .module
        .functions
        .iter()
        .find(|f| {
            let n = state.module.strings.get(f.name).unwrap_or("");
            (n == qualified || n.ends_with(&dotted))
                && f.params.len() == 1
                && f.bytecode_length > 0
        })
        .map(|f| f.id)
}

/// IterNext (0xC1) - Get next element from iterator.
///
/// Format: `IterNext dst, has_next_dst, iter`
/// Advances iterator, sets dst to next value (or unit if exhausted),
/// and sets has_next_dst to bool indicating if there was a value.
pub(in super::super) fn handle_iter_next(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let has_next_dst = read_reg(state)?;
    let iter_reg = read_reg(state)?;

    let iter = state.get_reg(iter_reg);
    if !iter.is_regular_ptr() || !state.heap.contains(iter.as_ptr()) {
        return Err(InterpreterError::InvalidOperand { message: "IterNext requires a VM iterator".into() });
    }
    let iter_ptr = iter.as_ptr::<u8>();
    // SAFETY: exact heap membership was checked, and the flag is set only
    // when allocating the three-slot state consumed below.
    if !unsafe { (*iter_ptr.cast::<heap::ObjectHeader>()).is_native_iterator(3) } {
        return Err(InterpreterError::InvalidOperand { message: "IterNext requires a VM iterator".into() });
    }

    // Read iterator state
    let iter_data = unsafe { iter_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value };
    let source = unsafe { *iter_data };
    let cursor = unsafe { *iter_data.add(1) };
    let iter_type = unsafe { (*iter_data.add(2)).as_i64() };
    if iter_type == ITER_TYPE_RANGE {
        return advance_range(state, source, cursor, iter_data, dst, has_next_dst);
    }
    let current_idx = cursor.as_i64() as usize;

    // Generator iteration: resume via nested dispatch loop, get yielded value
    if iter_type == ITER_TYPE_GENERATOR {
        let gen_val = source;
        if !gen_val.is_generator() {
            state.set_reg(dst, Value::unit());
            state.set_reg(has_next_dst, Value::from_bool(false));
            return Ok(DispatchResult::Continue);
        }

        let gen_id = GeneratorId(gen_val.as_generator_id());

        if !state
            .generators
            .get(gen_id)
            .map(|g| g.can_resume())
            .unwrap_or(false)
        {
            state.set_reg(dst, Value::unit());
            state.set_reg(has_next_dst, Value::from_bool(false));
            return Ok(DispatchResult::Continue);
        }

        let (func_id, status, reg_count) = {
            let generator =
                state
                    .generators
                    .get(gen_id)
                    .ok_or(InterpreterError::InvalidGeneratorId {
                        generator_id: gen_id,
                    })?;
            (generator.func_id, generator.status, generator.reg_count)
        };

        let _func = state
            .module
            .get_function(func_id)
            .ok_or(InterpreterError::FunctionNotFound(func_id))?;

        // PC is relative to function start (matching handle_call which sets pc=0)
        let (resume_pc, restore_registers, restore_contexts) =
            match status {
                GeneratorStatus::Created => {
                    let generator = state.generators.get(gen_id).ok_or(
                        InterpreterError::InvalidGeneratorId {
                            generator_id: gen_id,
                        },
                    )?;
                    (0u32, generator.saved_registers.clone(), Vec::new())
                }
                GeneratorStatus::Yielded => {
                    let generator = state.generators.get(gen_id).ok_or(
                        InterpreterError::InvalidGeneratorId {
                            generator_id: gen_id,
                        },
                    )?;
                    (
                        generator.saved_pc,
                        generator.saved_registers.clone(),
                        generator.saved_contexts.clone(),
                    )
                }
                _ => {
                    state.set_reg(dst, Value::unit());
                    state.set_reg(has_next_dst, Value::from_bool(false));
                    return Ok(DispatchResult::Continue);
                }
            };

        if let Some(g) = state.generators.get_mut(gen_id) {
            g.status = GeneratorStatus::Running;
        }

        let entry_depth = state.call_stack.depth();
        let return_pc = state.pc();
        state
            .call_stack
            .push_frame(func_id, reg_count, return_pc, dst)?;
        state.registers.push_frame(reg_count);

        let new_reg_base = state.reg_base();
        for (i, val) in restore_registers.iter().enumerate() {
            state.registers.set(new_reg_base, Reg(i as u16), *val);
        }
        if !restore_contexts.is_empty() {
            state.context_stack.restore_entries(restore_contexts);
        }

        state.current_generator = Some(gen_id);
        state.set_pc(resume_pc);

        // Run generator until yield or completion
        let result = dispatch_loop_table_with_entry_depth(state, entry_depth);

        {
            let value = result?;
            if state
                .generators
                .get(gen_id)
                .map(|g| g.status == GeneratorStatus::Yielded)
                .unwrap_or(false)
            {
                let yielded = state
                    .generators
                    .get(gen_id)
                    .and_then(|g| g.yielded_value)
                    .unwrap_or(value);
                state.set_reg(dst, yielded);
                state.set_reg(has_next_dst, Value::from_bool(true));
            } else {
                state.set_reg(dst, Value::unit());
                state.set_reg(has_next_dst, Value::from_bool(false));
            }
        }

        return Ok(DispatchResult::Continue);
    }

    // Iterator-protocol dispatch (PROTOCOL-ITER-1): slot 1 carries the
    // resolved `<Type>.next` FunctionId.  Call `next(&mut self)` and
    // unpack the returned `Maybe<T>`: a heap variant carries its tag at
    // OBJECT_HEADER_SIZE (None ⇒ exhausted, Some ⇒ payload slot 0); a
    // non-pointer result (nil / unit) is the payload-less None shape.
    if iter_type == ITER_TYPE_PROTOCOL {
        let next_fid = crate::module::FunctionId(current_idx as u32);
        let maybe = super::super::call_function_sync(state, next_fid, &[source])?;
        if maybe.is_ptr() && !maybe.is_nil() {
            let p = maybe.as_ptr::<u8>();
            if !p.is_null() {
                // SAFETY: `next` returns a Maybe variant object; every
                // heap object begins with an ObjectHeader and variants
                // carry their tag immediately after it.
                let tag = unsafe { heap::variant_tag(p) };
                if tag == verum_common::well_known_types::maybe_none_tag() {
                    state.set_reg(dst, Value::unit());
                    state.set_reg(has_next_dst, Value::from_bool(false));
                } else {
                    let payload = unsafe { heap::variant_payload(p, 0) };
                    state.set_reg(dst, payload);
                    state.set_reg(has_next_dst, Value::from_bool(true));
                }
                return Ok(DispatchResult::Continue);
            }
        }
        state.set_reg(dst, Value::unit());
        state.set_reg(has_next_dst, Value::from_bool(false));
        return Ok(DispatchResult::Continue);
    }

    // FATREF-ITER-1 (#40): FatRef slice iteration — MUST run before the
    // `as_ptr` extraction below (a FatRef Value's payload is
    // FAT_REF_MARKER|table-index, not a heap pointer). Element reads go
    // through `fat_ref_read_element`, the same authority as `GetE`, so
    // indexing and iteration honor the identical `reserved` elem-size
    // dispatch.
    if iter_type == ITER_TYPE_FATREF_SLICE {
        if !source.is_fat_ref() {
            return Err(InterpreterError::Panic {
                message: format!(
                    "IterNext: FATREF_SLICE iterator holds a non-FatRef source (bits {:#x})",
                    source.to_bits()
                ),
            });
        }
        let fat_ref = source.as_fat_ref();
        let len = fat_ref.len() as usize;
        if current_idx >= len {
            state.set_reg(dst, Value::unit());
            state.set_reg(has_next_dst, Value::from_bool(false));
            return Ok(DispatchResult::Continue);
        }
        if fat_ref.ptr().is_null() {
            return Err(InterpreterError::NullPointer);
        }
        let element =
            super::memory_collections::fat_ref_read_element(&fat_ref, current_idx);
        unsafe {
            *iter_data.add(1) = Value::from_i64((current_idx + 1) as i64);
        }
        state.set_reg(dst, element);
        state.set_reg(has_next_dst, Value::from_bool(true));
        return Ok(DispatchResult::Continue);
    }

    if iter_type == ITER_TYPE_TEXT {
        let byte = if source.is_small_string() {
            source.as_small_string().as_bytes().get(current_idx).copied()
        } else {
            let base = source.as_ptr::<u8>();
            // IterNew checks the tracked TEXT carrier and its record extent.
            let (data, len) = unsafe { heap::text_record_payload(base) }.ok_or_else(|| {
                InterpreterError::InvalidOperand { message: "IterNext: invalid Text record".into() }
            })?;
            if current_idx >= len { None } else {
                if data.is_null() { return Err(InterpreterError::NullPointer); }
                // The Text value owns the live UTF-8 buffer throughout the loop.
                Some(unsafe { *data.add(current_idx) })
            }
        };
        if let Some(byte) = byte {
            unsafe { *iter_data.add(1) = Value::from_i64((current_idx + 1) as i64) };
            state.set_reg(dst, Value::from_i64(i64::from(byte)));
        } else {
            state.set_reg(dst, Value::unit());
        }
        state.set_reg(has_next_dst, Value::from_bool(byte.is_some()));
        return Ok(DispatchResult::Continue);
    }
    let source_ptr = source.as_ptr::<u8>();
    if source_ptr.is_null() {
        state.set_reg(dst, Value::unit());
        state.set_reg(has_next_dst, Value::from_bool(false));
        return Ok(DispatchResult::Continue);
    }

    match iter_type {
        ITER_TYPE_LIST => {
            // Read list header: [len, cap, backing_ptr]
            let list_header = unsafe { source_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
            let len = unsafe { (*list_header).as_i64() } as usize;

            if current_idx >= len {
                // Exhausted
                state.set_reg(dst, Value::unit());
                state.set_reg(has_next_dst, Value::from_bool(false));
                return Ok(DispatchResult::Continue);
            }

            // Get element from backing array
            let backing_ptr = unsafe { (*list_header.add(2)).as_ptr::<u8>() };
            let elem_ptr = unsafe {
                backing_ptr
                    .add(heap::OBJECT_HEADER_SIZE + current_idx * std::mem::size_of::<Value>())
                    as *const Value
            };
            let element = unsafe { *elem_ptr };

            // Advance iterator
            unsafe {
                *iter_data.add(1) = Value::from_i64((current_idx + 1) as i64);
            }

            state.set_reg(dst, element);
            state.set_reg(has_next_dst, Value::from_bool(true));
        }
        ITER_TYPE_TYPED_ARRAY => {
            // Packed typed array — element count derives from the
            // header size and the scalar TypeId's stride; decode via
            // the ONE authority shared with indexed reads.
            // SAFETY: IterNew classified on the header TypeId, which
            // proves the raw-buffer shape; bounds checked below.
            let header = unsafe { heap::ObjectHeader::ref_or_stub(source_ptr) };
            let spec = heap::typed_array_element_spec(header.type_id);
            let (stride, _is_float) = match spec {
                Some(sf) => sf,
                None => {
                    return Err(InterpreterError::Panic {
                        message: format!(
                            "IterNext: ITER_TYPE_TYPED_ARRAY over non-typed-array TypeId {}",
                            header.type_id.0
                        ),
                    });
                }
            };
            let count = header.size as usize / stride;
            if current_idx >= count {
                state.set_reg(dst, Value::unit());
                state.set_reg(has_next_dst, Value::from_bool(false));
                return Ok(DispatchResult::Continue);
            }
            let data_ptr = unsafe { source_ptr.add(heap::OBJECT_HEADER_SIZE) };
            // SAFETY: bounds-checked against header.size/stride above.
            let element = unsafe {
                heap::typed_array_element(header.type_id, data_ptr, current_idx)
            }
            .expect("spec checked above");

            unsafe {
                *iter_data.add(1) = Value::from_i64((current_idx + 1) as i64);
            }
            state.set_reg(dst, element);
            state.set_reg(has_next_dst, Value::from_bool(true));
        }
        ITER_TYPE_BYTE_SLICE => {
            // BYTE_SLICE byte-view iteration (ARCH-P5).  Payload is
            // TWO RAW i64 slots `{ptr, len}` — read as raw words, then
            // yield the byte at `current_idx`, zero-extended.
            // SAFETY: IterNew type-discriminated on the BYTE_SLICE
            // header stamp, which proves the 16-byte raw payload shape.
            let (base, len) = unsafe { heap::byte_slice_payload(source_ptr) };

            if current_idx >= len as usize {
                state.set_reg(dst, Value::unit());
                state.set_reg(has_next_dst, Value::from_bool(false));
                return Ok(DispatchResult::Continue);
            }

            // SAFETY: bounds-checked above; the view addresses `len`
            // bytes at `base` (never-null producer contract).
            let byte_value = unsafe { *base.add(current_idx) };

            unsafe {
                *iter_data.add(1) = Value::from_i64((current_idx + 1) as i64);
            }

            state.set_reg(dst, Value::from_i64(byte_value as i64));
            state.set_reg(has_next_dst, Value::from_bool(true));
        }
        ITER_TYPE_BYTE_LIST => {
            // Packed-byte list iteration: same 3-Value header shape as
            // LIST but the backing is `[u8; cap]`.  Read 1 byte and
            // zero-extend into a NaN-boxed Value (red-team §4 fix).
            let list_header = unsafe { source_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
            let len = unsafe { (*list_header).as_i64() } as usize;

            if current_idx >= len {
                state.set_reg(dst, Value::unit());
                state.set_reg(has_next_dst, Value::from_bool(false));
                return Ok(DispatchResult::Continue);
            }

            let backing_ptr = unsafe { (*list_header.add(2)).as_ptr::<u8>() };
            let elem_ptr =
                unsafe { backing_ptr.add(heap::OBJECT_HEADER_SIZE + current_idx) };
            let byte_value = unsafe { *elem_ptr };

            unsafe {
                *iter_data.add(1) = Value::from_i64((current_idx + 1) as i64);
            }

            state.set_reg(dst, Value::from_i64(byte_value as i64));
            state.set_reg(has_next_dst, Value::from_bool(true));
        }
        ITER_TYPE_MAP => {
            // Read map/set header: [count, capacity, entries_ptr]
            let map_header = unsafe { source_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
            let capacity = unsafe { (*map_header.add(1)).as_i64() } as usize;
            let entries_val = unsafe { *map_header.add(2) };

            // A never-inserted Map/Set is lazily allocated: `Map.new()` /
            // `Set.new()` store `entries: null_ptr(), cap: 0` and only
            // materialise the entry buffer on first insert.  Iterating that
            // state must report exhaustion, not dereference the null/non-
            // pointer entries slot (was an ICE: `Expected pointer, got
            // Some(1)` — empty-map-iteration defect).
            if capacity == 0 || !entries_val.is_ptr() {
                unsafe {
                    *iter_data.add(1) = Value::from_i64(capacity as i64);
                }
                state.set_reg(dst, Value::unit());
                state.set_reg(has_next_dst, Value::from_bool(false));
                return Ok(DispatchResult::Continue);
            }

            let entries_ptr = entries_val.as_ptr::<u8>();
            let entries_data = unsafe { entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };

            // Inspect the source's type_id to decide element shape:
            //  SET → yield key only (matches `implement<T> Iterator for
            //  SetIter<T> { type Item = T; … }` in
            //  core/collections/set.vr)
            //  MAP → yield `(key, value)` 2-tuple for destructuring
            //  `for (k, v) in map { … }`
            // Everything else (historical builtins piping through
            // TypeId::UNIT) defaults to the MAP pair shape.
            let source_is_set = {
                let header = unsafe { heap::ObjectHeader::ref_or_stub(source_ptr) };
                header.type_id == TypeId::SET
            };

            // Find next non-empty entry starting from current_idx.
            let mut idx = current_idx;
            while idx < capacity {
                let entry_key = unsafe { *entries_data.add(idx * 2) };
                if !entry_key.is_vacant_hash_key() {
                    // Advance iterator to next slot.
                    unsafe {
                        *iter_data.add(1) = Value::from_i64((idx + 1) as i64);
                    }

                    let element = if source_is_set {
                        entry_key
                    } else {
                        let entry_val = unsafe { *entries_data.add(idx * 2 + 1) };
                        let tuple_obj = state
                            .heap
                            .alloc(TypeId::TUPLE, 2 * std::mem::size_of::<Value>())?;
                        let tuple_data = unsafe {
                            (tuple_obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE)
                                as *mut Value
                        };
                        unsafe {
                            *tuple_data = entry_key;
                            *tuple_data.add(1) = entry_val;
                        }
                        Value::from_ptr(tuple_obj.as_ptr() as *mut u8)
                    };

                    state.set_reg(dst, element);
                    state.set_reg(has_next_dst, Value::from_bool(true));
                    return Ok(DispatchResult::Continue);
                }
                idx += 1;
            }

            // Exhausted
            unsafe {
                *iter_data.add(1) = Value::from_i64(capacity as i64);
            }
            state.set_reg(dst, Value::unit());
            state.set_reg(has_next_dst, Value::from_bool(false));
        }
        ITER_TYPE_ARRAY => {
            let header = unsafe { &*source_ptr.cast::<heap::ObjectHeader>() };
            let length = super::super::get_array_length(source_ptr, header)?;
            if current_idx >= length {
                state.set_reg(dst, Value::unit());
                state.set_reg(has_next_dst, Value::from_bool(false));
                return Ok(DispatchResult::Continue);
            }
            let value = super::super::get_array_element(state, source_ptr, header, current_idx)?;
            unsafe { *iter_data.add(1) = Value::from_i64((current_idx + 1) as i64) };
            state.set_reg(dst, value);
            state.set_reg(has_next_dst, Value::from_bool(true));
        }
        _ => {
            // Unknown iterator type
            state.set_reg(dst, Value::unit());
            state.set_reg(has_next_dst, Value::from_bool(false));
        }
    }

    Ok(DispatchResult::Continue)
}

/// A range cursor is its current integer or Unit after exhaustion. Initializing
/// the cursor at creation keeps zero available as an ordinary element; consuming
/// the endpoint never computes a successor outside the integer domain.
fn advance_range(
    state: &mut InterpreterState,
    source: Value,
    cursor: Value,
    iter_data: *mut Value,
    dst: Reg,
    has_next_dst: Reg,
) -> InterpreterResult<DispatchResult> {
    if cursor.is_unit() {
        state.set_reg(dst, Value::unit());
        state.set_reg(has_next_dst, Value::from_bool(false));
        return Ok(DispatchResult::Continue);
    }
    let source_ptr = source.as_ptr::<u8>();
    if source_ptr.is_null() { return Err(InterpreterError::NullPointer); }
    let range = unsafe { source_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
    let current = cursor.as_i64();
    let end = unsafe { (*range.add(1)).as_i64() };
    let inclusive = unsafe { (*range.add(2)).as_bool() };
    if current > end || (!inclusive && current == end) {
        unsafe { *iter_data.add(1) = Value::unit(); }
        state.set_reg(dst, Value::unit());
        state.set_reg(has_next_dst, Value::from_bool(false));
    } else {
        // In the second branch current < end, so current + 1 cannot overflow.
        unsafe { *iter_data.add(1) = if current == end { Value::unit() }
            else { Value::from_i64(current + 1) }; }
        state.set_reg(dst, cursor);
        state.set_reg(has_next_dst, Value::from_bool(true));
    }
    Ok(DispatchResult::Continue)
}

/// NewRange (0xCC) - Create a new range for iteration.
///
/// Encoding: opcode + dst + start + end + inclusive (1 byte)
/// Effect: Creates a Range object that can be iterated with IterNew/IterNext.
///
/// Range layout in memory (3 Values at data offset) - must match IterNext expectations:
///  [0] start: Starting value (Int)
///  [1] end: Ending value (Int)
///  [2] inclusive: Whether end is included (Bool: 0 or 1)
pub(in super::super) fn handle_new_range(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let start_reg = read_reg(state)?;
    let end_reg = read_reg(state)?;
    let inclusive_byte = read_u8(state)?;
    let inclusive = inclusive_byte != 0;

    let start_val = state.get_reg(start_reg);
    let end_val = state.get_reg(end_reg);

    // Get integer values
    let start_int = start_val.as_i64();
    let end_int = end_val.as_i64();

    // Allocate Range object: ObjectHeader + 3 Values (start, end, inclusive)
    let obj = state.heap.alloc(
        crate::types::TypeId::RANGE,
        3 * std::mem::size_of::<Value>(),
    )?;
    state.record_allocation();

    let base_ptr = obj.as_ptr() as *mut u8;
    let data_ptr = unsafe { base_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value };

    // Write range data - must match IterNext's expected layout
    unsafe {
        *data_ptr = Value::from_i64(start_int); // [0] start
        *data_ptr.add(1) = Value::from_i64(end_int); // [1] end
        *data_ptr.add(2) = Value::from_bool(inclusive); // [2] inclusive flag
    }

    state.set_reg(dst, Value::from_ptr(base_ptr));
    Ok(DispatchResult::Continue)
}
