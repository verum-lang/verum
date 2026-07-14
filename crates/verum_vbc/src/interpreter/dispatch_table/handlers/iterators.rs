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

// ============================================================================
// Iterator + Range Operations
// ============================================================================

/// IterNew (0xC0) - Create iterator from iterable.
///

/// Format: `IterNew dst, src`
/// Creates an iterator over src and stores in dst.
///

/// Type discrimination is performed by examining the ObjectHeader's type_id:
/// - TypeId::LIST (512) → ITER_TYPE_LIST
/// - TypeId::MAP (513) → ITER_TYPE_MAP
/// - TypeId::SET (513) → ITER_TYPE_MAP (same iteration pattern)
/// - TypeId::ARRAY (518) → ITER_TYPE_ARRAY
/// - TypeId::RANGE (517) → ITER_TYPE_RANGE (special handling)
///

/// Iterator protocol: creates an iterator object from a collection or range. The iterator
/// holds a type tag (LIST/SET/MAP/ARRAY/RANGE), the source reference, and a cursor index.
/// Each call to IterNext advances the cursor and returns the next element or nil.
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
    let source = super::cbgr_helpers::resolve_arg_value(state, source);

    // Check for generator values first (NaN-boxed generator tag, not pointer)
    if source.is_generator() {
        // Generator iterator: store generator value directly in iterator object.
        // IterNext will detect ITER_TYPE_GENERATOR and resume the generator.
        let iter_obj = state
            .heap
            .alloc(TypeId::UNIT, 3 * std::mem::size_of::<Value>())?;
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

    // Pre-built iterator blob conversion — if the source is already a
    // 4-value iterator blob (TypeId::UNIT + 4 × Value with a valid iter_type
    // tag in slot 3, produced by explicit `.iter()` on builtin Map/Set/
    // List/Array), unwrap it to the collection pointer the 3-value blob
    // layout expects, using the tagged iter_type from slot 3 instead of
    // re-deriving it from the wrapper blob's own header (which is
    // `TypeId::UNIT` and would otherwise misclassify as
    // `ITER_TYPE_LIST`).
    let (source, forced_iter_type) = if source.is_ptr() {
        let src_ptr = source.as_ptr::<u8>();
        if !src_ptr.is_null()
            && (src_ptr as usize).is_multiple_of(std::mem::align_of::<heap::ObjectHeader>())
        {
            let src_header = unsafe { heap::ObjectHeader::ref_or_stub(src_ptr) };
            if src_header.type_id == TypeId::UNIT
                && src_header.size as usize == 4 * std::mem::size_of::<Value>()
            {
                let data = unsafe { src_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
                let tag = unsafe { (*data.add(3)).as_i64() };
                if (ITER_TYPE_LIST..=ITER_TYPE_BYTE_LIST).contains(&tag) {
                    let inner_source = unsafe { *data };
                    (inner_source, Some(tag))
                } else {
                    (source, None)
                }
            } else {
                (source, None)
            }
        } else {
            (source, None)
        }
    } else {
        (source, None)
    };

    // Determine collection type by examining the object header.
    // For non-builtin records, `protocol_next_fid` carries the resolved
    // `<Type>.next` FunctionId into blob slot 1 (see ITER_TYPE_PROTOCOL).
    let mut protocol_next_fid: Option<crate::module::FunctionId> = None;
    let iter_type = if source.is_fat_ref() {
        // FATREF-ITER-1 (#40): general slice iteration. FatRefs fail
        // `is_ptr()` and previously fell to the non-pointer LIST
        // default; IterNext then dereferenced the FAT_REF_MARKER
        // payload bits as a List header → SIGSEGV. Keep the encoded
        // Value in slot 0; IterNext resolves per step.
        ITER_TYPE_FATREF_SLICE
    } else if let Some(tag) = forced_iter_type {
        // Already-built iterator blob — use the blob's tag rather than
        // re-deriving from `source`'s (now unwrapped) header.
        tag
    } else if source.is_ptr() {
        let source_ptr = source.as_ptr::<u8>();
        if !source_ptr.is_null() {
            // Read object header to get type_id
            let header = unsafe { heap::ObjectHeader::ref_or_stub(source_ptr) };
            match header.type_id {
                TypeId::MAP | TypeId::SET => ITER_TYPE_MAP,
                TypeId::ARRAY => ITER_TYPE_ARRAY,
                TypeId::RANGE => ITER_TYPE_RANGE,
                TypeId::BYTE_LIST => ITER_TYPE_BYTE_LIST,
                TypeId::BYTE_SLICE => ITER_TYPE_BYTE_SLICE,
                TypeId::LIST => ITER_TYPE_LIST,
                // Packed typed arrays — scalar-TypeId raw buffers.
                TypeId::U8
                | TypeId::U16
                | TypeId::U32
                | TypeId::U64
                | TypeId::F32
                | TypeId::F64 => ITER_TYPE_TYPED_ARRAY,
                // Non-builtin heap object.  If its type is an Iterator
                // record (resolvable 1-arg `<Type>.next` with a real
                // body), iterate through the protocol — reading an
                // arbitrary record as a List header is memory-unsafe
                // (PROTOCOL-ITER-1).  Types without a `next` keep the
                // historic list treatment (synthetic list-shaped blobs
                // from runtime intercepts rely on it).
                other => match resolve_protocol_next(state, other) {
                    Some(fid) => {
                        protocol_next_fid = Some(fid);
                        ITER_TYPE_PROTOCOL
                    }
                    None => ITER_TYPE_LIST,
                },
            }
        } else {
            // Null pointer - default to list (will fail on IterNext)
            ITER_TYPE_LIST
        }
    } else {
        // Non-pointer value - could be a range encoded in value bits
        // For now, default to list
        ITER_TYPE_LIST
    };

    // Allocate iterator object: [source_ptr, current_idx, iter_type]
    let iter_obj = state
        .heap
        .alloc(TypeId::UNIT, 3 * std::mem::size_of::<Value>())?;
    state.record_allocation();

    let iter_ptr =
        unsafe { (iter_obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value };

    // Initialize iterator.  Slot 1 is the cursor for indexed iteration
    // and the `next` FunctionId for protocol iteration.
    let slot1 = match protocol_next_fid {
        Some(fid) => Value::from_i64(fid.0 as i64),
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

    let iter_ptr = state.get_reg(iter_reg).as_ptr::<u8>();
    if iter_ptr.is_null() {
        return Err(InterpreterError::NullPointer);
    }

    // Read iterator state
    let iter_data = unsafe { iter_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value };
    let source = unsafe { *iter_data };
    let current_idx = unsafe { (*iter_data.add(1)).as_i64() } as usize;
    let iter_type = unsafe { (*iter_data.add(2)).as_i64() };

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
                if !entry_key.is_unit() {
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
            // Read array length from header (arrays store len in first slot after object header)
            let array_header = unsafe { source_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
            // For arrays, we use a simpler layout: elements directly after header
            // The length is stored separately or passed as metadata
            // For now, treat similarly to list
            let len = unsafe { (*array_header).as_i64() } as usize;

            if current_idx >= len {
                state.set_reg(dst, Value::unit());
                state.set_reg(has_next_dst, Value::from_bool(false));
                return Ok(DispatchResult::Continue);
            }

            let elem_ptr = unsafe { array_header.add(1 + current_idx) };
            let element = unsafe { *elem_ptr };

            unsafe {
                *iter_data.add(1) = Value::from_i64((current_idx + 1) as i64);
            }

            state.set_reg(dst, element);
            state.set_reg(has_next_dst, Value::from_bool(true));
        }
        ITER_TYPE_RANGE => {
            // Range layout: [current: i64, end: i64, inclusive: bool]
            // For IterNew, we store the source range pointer
            // For IterNext, current_idx is used as the current value
            //

            // Range objects have layout: [start, end, step, inclusive_flag]
            // We read these on first iteration
            let range_header = unsafe { source_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };

            // On first iteration (current_idx == 0), initialize from range
            let current_val = if current_idx == 0 {
                // Read start value
                unsafe { (*range_header).as_i64() }
            } else {
                // current_idx holds the current value after adjustment
                current_idx as i64
            };

            let end_val = unsafe { (*range_header.add(1)).as_i64() };
            let inclusive = unsafe { (*range_header.add(2)).as_bool() };

            // Check if we've reached the end
            let at_end = if inclusive {
                current_val > end_val
            } else {
                current_val >= end_val
            };

            if at_end {
                state.set_reg(dst, Value::unit());
                state.set_reg(has_next_dst, Value::from_bool(false));
                return Ok(DispatchResult::Continue);
            }

            // Return current value and advance
            state.set_reg(dst, Value::from_i64(current_val));
            state.set_reg(has_next_dst, Value::from_bool(true));

            // Store next value in current_idx slot
            unsafe {
                *iter_data.add(1) = Value::from_i64(current_val + 1);
            }
        }
        _ => {
            // Unknown iterator type
            state.set_reg(dst, Value::unit());
            state.set_reg(has_next_dst, Value::from_bool(false));
        }
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
