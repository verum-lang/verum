//! Method dispatch handlers for VBC interpreter.
//!
//! This module contains the CallM instruction handler and all supporting
//! method dispatch functions, including:
//! - handle_call_method: the main CallM opcode handler
//! - dispatch_primitive_method: built-in methods on Int, Float, Bool, Text, etc.
//! - dispatch_array_method: higher-order methods on arrays/lists (map, filter, fold, etc.)
//! - dispatch_variant_method: methods on variant types
//! - Helper functions: call_closure_sync, call_function_sync, alloc_list_from_values, etc.

use crate::instruction::{Reg, RegRange};
use crate::module::FunctionId;
use crate::types::{TypeId, StringId};
use crate::interpreter::state::GeneratorId;
use crate::value::Value;
use verum_common::well_known_types::WellKnownType as WKT;
use crate::interpreter::error::{InterpreterError, InterpreterResult};
use crate::interpreter::state::InterpreterState;
use crate::interpreter::heap;
use crate::value::{ThinRef, Capabilities};
use super::super::DispatchResult;

// Re-import bytecode I/O functions
use super::bytecode_io::{read_reg, read_varint, read_reg_range};

// Re-import string helper functions
use super::string_helpers::{extract_string, alloc_string_value, is_heap_string};

// Re-import CBGR helper functions
use super::cbgr_helpers::{is_cbgr_ref, decode_cbgr_ref, is_cbgr_ref_mutable};

// Re-import debug helpers
use super::debug::format_value_for_print;

// Import helper functions that remain in dispatch_table/mod.rs
use super::super::{deep_value_eq, value_hash, value_eq, dispatch_loop_table_with_entry_depth};

// ── Iterator type constants ──
const ITER_TYPE_LIST: i64 = 0;
const ITER_TYPE_MAP: i64 = 1;
const ITER_TYPE_ARRAY: i64 = 2;
const ITER_TYPE_RANGE: i64 = 3;

// ============================================================================
// Method Dispatch Handlers
// ============================================================================

/// Call method: `dst = receiver.method(args...)`
pub(in super::super) fn handle_call_method(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let receiver_reg = read_reg(state)?;
    let method_id = read_varint(state)? as u32;
    let args = read_reg_range(state)?;

    let receiver = state.get_reg(receiver_reg);

    // Resolve method name from string table.
    //
    // `mut` because the Shared-deref auto-forwarder below
    // (`_ => {}` arm of the Shared TypeId match) re-qualifies
    // a `"Shared.foo"` name to `"AtomicInt.foo"` (or whatever
    // the inner type is) so the qualified-lookup walker finds
    // the user-compiled body on the inner type.
    let mut method_name = state.module.strings.get(StringId(method_id))
        .unwrap_or("")
        .to_string();
    // Extract bare method name by stripping type prefix (e.g., "List.pop" -> "pop").
    // VBC codegen qualifies method names with the receiver type for dispatch,
    // but builtin handlers match on bare names. Keep the original for compiled
    // function lookup, use bare name for builtin dispatch.
    let bare_method_name: String = if let Some(dot_pos) = method_name.rfind('.') {
        method_name[dot_pos + 1..].to_string()
    } else {
        method_name.clone()
    };

    // Try generator methods (next, has_next, collect) first
    // These are dispatched to the corresponding GenNext/GenHasNext handlers
    if receiver.is_generator() {
        let gen_id = GeneratorId(receiver.as_generator_id());
        match bare_method_name.as_str() {
            "next" => {
                // Generator.next() -> Option<T>
                // Reuse the GenNext handler logic
                let _gen_val = receiver;

                // Check generator status
                let (func_id, status, reg_count) = {
                    let generator = state.generators.get(gen_id)
                        .ok_or(InterpreterError::InvalidGeneratorId { generator_id: gen_id })?;

                    if generator.is_completed() {
                        // Return None - generator exhausted
                        state.set_reg(dst, Value::nil());
                        return Ok(DispatchResult::Continue);
                    }

                    (generator.func_id, generator.status, generator.reg_count)
                };

                // Get function info
                let func = state.module.get_function(func_id)
                    .ok_or(InterpreterError::FunctionNotFound(func_id))?;
                let bytecode_offset = func.bytecode_offset;

                use crate::interpreter::state::GeneratorStatus;

                // Check if we need to restore state from a previous yield
                let (resume_pc, restore_registers, restore_contexts): (u32, Vec<Value>, Vec<crate::interpreter::state::ContextEntry>) = match status {
                    GeneratorStatus::Created => {
                        // First resume - restore initial arguments
                        let generator = state.generators.get(gen_id)
                            .ok_or(InterpreterError::InvalidGeneratorId { generator_id: gen_id })?;
                        let initial_args = generator.saved_registers.clone();
                        (bytecode_offset, initial_args, Vec::new())
                    }
                    GeneratorStatus::Yielded => {
                        let generator = state.generators.get(gen_id)
                            .ok_or(InterpreterError::InvalidGeneratorId { generator_id: gen_id })?;
                        let resume_pc = if generator.saved_pc > 0 { generator.saved_pc } else { bytecode_offset };
                        let restore_registers = generator.saved_registers.clone();
                        let restore_contexts = generator.saved_contexts.clone();
                        (resume_pc, restore_registers, restore_contexts)
                    }
                    GeneratorStatus::Running => {
                        return Err(InterpreterError::GeneratorNotResumable {
                            generator_id: gen_id,
                            status: "Running",
                        });
                    }
                    GeneratorStatus::Completed => {
                        state.set_reg(dst, Value::nil());
                        return Ok(DispatchResult::Continue);
                    }
                };

                // Push generator frame
                state.call_stack.push_frame(func_id, reg_count, resume_pc, dst)?;
                state.registers.push_frame(reg_count);

                // Restore registers
                let new_reg_base = state.reg_base();
                for (i, val) in restore_registers.iter().enumerate() {
                    state.registers.set(new_reg_base, Reg(i as u16), *val);
                }

                // Restore contexts
                if !restore_contexts.is_empty() {
                    state.context_stack.restore_entries(restore_contexts);
                }

                // Mark generator as running
                if let Some(g) = state.generators.get_mut(gen_id) {
                    g.status = GeneratorStatus::Running;
                }
                state.current_generator = Some(gen_id);
                state.set_pc(resume_pc);

                return Ok(DispatchResult::Continue);
            }
            "has_next" => {
                // Generator.has_next() -> Bool
                let generator = state.generators.get(gen_id)
                    .ok_or(InterpreterError::InvalidGeneratorId { generator_id: gen_id })?;
                let has_more = generator.can_resume();
                state.set_reg(dst, Value::from_bool(has_more));
                return Ok(DispatchResult::Continue);
            }
            "collect" => {
                // Generator.collect() -> List<T>
                // Run the generator to completion, collecting all yielded values into a list.
                use crate::interpreter::state::GeneratorStatus;
                let mut values = Vec::new();
                let entry_depth = state.call_stack.depth();

                loop {
                    // Check if generator can resume
                    if !state.generators.get(gen_id).map(|g| g.can_resume()).unwrap_or(false) {
                        break;
                    }

                    let (func_id, status, reg_count) = {
                        let generator = state.generators.get(gen_id)
                            .ok_or(InterpreterError::InvalidGeneratorId { generator_id: gen_id })?;
                        (generator.func_id, generator.status, generator.reg_count)
                    };

                    let (resume_pc, restore_regs, restore_contexts) = match status {
                        GeneratorStatus::Created => {
                            let generator = state.generators.get(gen_id)
                                .ok_or(InterpreterError::InvalidGeneratorId { generator_id: gen_id })?;
                            (0u32, generator.saved_registers.clone(), Vec::new())
                        }
                        GeneratorStatus::Yielded => {
                            let generator = state.generators.get(gen_id)
                                .ok_or(InterpreterError::InvalidGeneratorId { generator_id: gen_id })?;
                            (generator.saved_pc, generator.saved_registers.clone(), generator.saved_contexts.clone())
                        }
                        _ => break,
                    };

                    // Mark as Running
                    if let Some(g) = state.generators.get_mut(gen_id) {
                        g.status = GeneratorStatus::Running;
                    }

                    // Set up the generator's frame (mirroring IterNext generator path)
                    let return_pc = state.pc();
                    state.call_stack.push_frame(func_id, reg_count, return_pc, dst)?;
                    state.registers.push_frame(reg_count);

                    let new_reg_base = state.reg_base();
                    for (i, val) in restore_regs.iter().enumerate() {
                        state.registers.set(new_reg_base, Reg(i as u16), *val);
                    }
                    if !restore_contexts.is_empty() {
                        state.context_stack.restore_entries(restore_contexts);
                    }

                    state.current_generator = Some(gen_id);
                    state.set_pc(resume_pc);

                    // Run until yield or return
                    let _result = dispatch_loop_table_with_entry_depth(state, entry_depth);

                    // Check if the generator yielded a value
                    if let Some(gen_ref) = state.generators.get(gen_id) {
                        if gen_ref.status == GeneratorStatus::Yielded {
                            if let Some(val) = gen_ref.yielded_value {
                                values.push(val);
                            }
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }

                // Build a heap-allocated list from collected values
                let count = values.len();
                let header_size = 3 * std::mem::size_of::<i64>();
                let obj = state.heap.alloc(TypeId::LIST, header_size)?;
                state.record_allocation();
                let data_ptr = unsafe {
                    (obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut i64
                };
                let backing_layout = std::alloc::Layout::from_size_align(
                    count.max(1) * std::mem::size_of::<Value>(), 8
                ).map_err(|_| InterpreterError::Panic {
                    message: "collect list layout overflow".into(),
                })?;
                let backing_ptr = unsafe { std::alloc::alloc_zeroed(backing_layout) };
                let value_ptr = backing_ptr as *mut Value;
                for (i, val) in values.iter().enumerate() {
                    unsafe { *value_ptr.add(i) = *val };
                }
                unsafe {
                    *data_ptr = count as i64;
                    *data_ptr.add(1) = count as i64;
                    *data_ptr.add(2) = backing_ptr as i64;
                }

                state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
                return Ok(DispatchResult::Continue);
            }
            _ => {
                // Unknown generator method - fall through to other dispatchers
            }
        }
    }

    // Handle CBGR ref-specific methods BEFORE unwrapping. Methods like
    // `can_write`, `can_read`, `capabilities`, `epoch_caps_raw`, `stored_generation`,
    // `is_valid` operate on the reference metadata, not the referent. Unwrapping
    // to the referent would hide the mutability bit and break the dispatch.
    if is_cbgr_ref(&receiver)
        && let Some(result) = dispatch_primitive_method(state, &receiver, &method_name, &args)? {
            state.set_reg(dst, result);
            return Ok(DispatchResult::Continue);
        }

    // Deref CBGR references to get the actual value for builtin dispatch.
    // RefMut creates register-based refs for &mut self method calls.
    //
    // `mut` because the Shared-deref auto-forwarder below may rebind
    // `dispatch_receiver` to the inner Value when an unrecognised
    // method is called on a `Shared<T>` carrier — see the `_ => {}`
    // arm of the Shared TypeId match.
    let mut dispatch_receiver = if is_cbgr_ref(&receiver) {
        let (abs_index, _) = decode_cbgr_ref(receiver.as_i64());
        state.registers.get_absolute(abs_index)
    } else {
        receiver
    };

    // Try built-in primitive method dispatch first.
    // Pass the full qualified method_name so dispatch_primitive_method can
    // inspect the type prefix (e.g., "Stats.add") and skip builtin dispatch
    // when the prefix refers to a user-defined type. The builtin handlers
    // internally strip the prefix to match the bare method name.
    if let Some(result) = dispatch_primitive_method(state, &dispatch_receiver, &method_name, &args)? {
        state.set_reg(dst, result);
        return Ok(DispatchResult::Continue);
    }

    // Try built-in array/list methods (map, filter, fold, etc.)
    if dispatch_receiver.is_ptr() && !dispatch_receiver.is_nil()
        && let Some(result) = dispatch_array_method(state, dispatch_receiver, &bare_method_name, &args)? {
            state.set_reg(dst, result);
            return Ok(DispatchResult::Continue);
        }

    // Try variant methods (unwrap, is_ok, is_err, etc.) on heap-allocated variants.
    //
    // The primitive variant dispatch CANNOT distinguish `Maybe.Some(v)` from
    // `Result.Err(v)` from raw runtime data — both share `(tag>=1, fc=1,
    // type_id == 0x8000+tag)` after `MakeVariant` (see comment block in
    // `dispatch_variant_method`'s `unwrap` arm). The historic resolution was
    // to silently return the payload on `Result.Err.unwrap()` rather than
    // panic — convenient for `Maybe.Some.unwrap()` but a CRITICAL silent
    // failure for `Result.Err.unwrap()` which masked an entire class of
    // downstream bugs (parse(...).unwrap() chains producing malformed
    // values that bubbled up as seemingly-unrelated panics — see #79).
    //
    // Fix: when the codegen emitted a *qualified* method name like
    // `Result.unwrap` AND the user-compiled function exists for that
    // qualified name, skip the primitive fallback so the compiled
    // `match self { Ok(v) => v, Err(e) => panic(...) }` body actually
    // runs. The bare-name route ("unwrap" without a type prefix —
    // typically generic-parameter dispatch) keeps the historic
    // behaviour for back-compatibility with code that doesn't have
    // type information at codegen time.
    let prefer_user_compiled = method_name != bare_method_name
        && state.module.find_function_by_name(&method_name).is_some();
    if !prefer_user_compiled
        && dispatch_receiver.is_ptr() && !dispatch_receiver.is_nil()
        && let Some(result) = dispatch_variant_method(state, dispatch_receiver, &bare_method_name, &args, &method_name)? {
            state.set_reg(dst, result);
            return Ok(DispatchResult::Continue);
        }

    // Try Shared<T> instance methods (borrow, borrow_mut, clone)
    if receiver.is_ptr() && !receiver.is_nil() {
        let ptr = receiver.as_ptr::<u8>();
        // Check if this is a Shared object by reading the type_id from ObjectHeader
        let header = unsafe { &*(ptr as *const heap::ObjectHeader) };
        if header.type_id == TypeId::SHARED {
            // Shared layout: [ObjectHeader][refcount: i64][value: Value]
            let data_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value };

            // Strip type prefix if present (e.g., "Shared.borrow" -> "borrow").
            // Support both "." (new convention) and "::" (legacy) for backwards compatibility.
            //
            // Owned `String` rather than borrowed `&str` so the
            // auto-deref arm below can re-qualify `method_name` to
            // the inner type without running into the borrow
            // checker — `base_method` would otherwise hold an
            // immutable reference into `method_name` for the entire
            // match scope.
            let base_method: String = if let Some(idx) = method_name.rfind('.') {
                method_name[idx + 1..].to_string()
            } else if let Some(idx) = method_name.rfind("::") {
                method_name[idx + 2..].to_string()
            } else {
                method_name.clone()
            };

            match base_method.as_str() {
                "borrow" | "borrow_mut" => {
                    // Return the inner value (or a reference to it)
                    // In VBC, we simplify by returning the value itself since
                    // we're single-threaded and don't need actual borrow checking
                    let inner = unsafe { *data_ptr.add(1) };
                    state.set_reg(dst, inner);
                    return Ok(DispatchResult::Continue);
                }
                "clone" => {
                    // Increment refcount and return the same Shared pointer
                    unsafe {
                        let refcount = (*data_ptr).as_i64();
                        *data_ptr = Value::from_i64(refcount + 1);
                    }
                    state.set_reg(dst, receiver);
                    return Ok(DispatchResult::Continue);
                }
                _ => {
                    // Auto-deref: any other method on `Shared<T>` is
                    // forwarded to the inner `T`. Covers
                    // `Shared<AtomicInt>::load / store / fetch_add`,
                    // `Shared<AtomicBool>::load / store`, and any
                    // user-defined `impl T { … }` reached through a
                    // `Shared<T>` carrier — without monomorphising
                    // every `Shared<T>` permutation. Mirrors the
                    // earlier CBGR-ref deref above.
                    //
                    // Concrete callers that depend on this:
                    // `core/net/weft/dst.vr` wraps state in
                    // `Shared<AtomicInt>` / `Shared<AtomicBool>`
                    // (SeededRng, WeftSimulator, TestClock); pre-fix
                    // every `.load()` / `.store()` / `.fetch_add()`
                    // panicked with "method not found".
                    let inner = unsafe { *data_ptr.add(1) };
                    dispatch_receiver = inner;
                    // `receiver` itself stays as the Shared pointer
                    // for any code that explicitly checks Shared
                    // identity. All builtin dispatchers below
                    // operate on `dispatch_receiver`.
                    //
                    // Re-qualify method_name with the inner type's
                    // name so the qualified-lookup walker at
                    // ~line 1014 finds e.g. `"AtomicInt.load"`
                    // instead of the original `"Shared.load"`. The
                    // codegen emits the receiver-type-prefixed form
                    // for `self.x.method()` calls, so without this
                    // rewrite we'd hit the catch-all panic even
                    // though the user-compiled body is registered
                    // for the inner type.
                    if dispatch_receiver.is_ptr() && !dispatch_receiver.is_nil() {
                        let inner_ptr = dispatch_receiver.as_ptr::<u8>();
                        if !inner_ptr.is_null()
                            && (inner_ptr as usize)
                                .is_multiple_of(std::mem::align_of::<heap::ObjectHeader>())
                        {
                            // SAFETY: alignment verified; heap
                            // objects begin with an ObjectHeader.
                            let inner_header =
                                unsafe { &*(inner_ptr as *const heap::ObjectHeader) };
                            if let Some(td) =
                                state.module.get_type(inner_header.type_id)
                                && let Some(inner_type_name) =
                                    state.module.strings.get(td.name)
                                && !inner_type_name.is_empty()
                            {
                                method_name = format!(
                                    "{}.{}",
                                    inner_type_name, base_method
                                );
                                // `bare_method_name` already equals
                                // `base_method`, so no recompute
                                // needed.
                            }
                        }
                    }
                }
            }
        }
    }

    // Extract type name from receiver (supports both SmallString and heap-allocated strings)
    let receiver_type_name: Option<String> = if receiver.is_small_string() {
        Some(receiver.as_small_string().as_str().to_string())
    } else if receiver.is_ptr() && !receiver.is_nil() {
        // Try reading as heap-allocated string: [ObjectHeader][len: u64][bytes...]
        let base_ptr = receiver.as_ptr::<u8>();
        if !base_ptr.is_null() {
            unsafe {
                let data_offset = heap::OBJECT_HEADER_SIZE;
                let len_ptr = base_ptr.add(data_offset) as *const u64;
                let len = *len_ptr as usize;
                if len <= 256 {
                    let bytes_ptr = base_ptr.add(data_offset + 8);
                    let bytes = std::slice::from_raw_parts(bytes_ptr, len);
                    std::str::from_utf8(bytes).ok().map(|s| s.to_string())
                } else {
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    // Handle Shared.new(value) - reference-counted shared ownership
    // VBC-internal: interpreter runtime dispatch — Shared.new() allocates a
    // refcounted heap object. Must match the WKT::Shared name to trigger this
    // intrinsic path instead of falling through to compiled method lookup.
    if bare_method_name == "new"
        && let Some(ref name) = receiver_type_name
            && WKT::Shared.matches(name) {
                let caller_base = state.reg_base();
                let value = if args.count > 0 {
                    state.registers.get(caller_base, Reg(args.start.0))
                } else {
                    Value::unit()
                };

                // Allocate Shared object: [ObjectHeader][refcount: i64][value: Value]
                // We store the inner value directly for simplicity
                let obj = state.heap.alloc(TypeId::SHARED, 2 * std::mem::size_of::<Value>())?;
                state.record_allocation();
                let data_ptr = unsafe {
                    (obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
                };
                unsafe {
                    *data_ptr = Value::from_i64(1);      // refcount = 1
                    *data_ptr.add(1) = value;            // inner value
                }
                state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
                return Ok(DispatchResult::Continue);
            }

    // Handle Heap.new(value) - CBGR allocation
    // Check for both bare receiver (receiver_type_name = "Heap") and qualified method name (dyn:Heap.new).
    // The qualified form occurs when codegen emits a dyn: dispatch, in which case the receiver register
    // holds the value being allocated (not the "Heap" type string).
    let is_heap_new = (bare_method_name == "new" && receiver_type_name.as_deref() == Some("Heap"))
        || method_name == "dyn:Heap.new"
        || method_name == "Heap.new";
    if is_heap_new {
        {
            {
                // For dyn:Heap.new dispatch: receiver holds the inner value (Heap is a wrapper,
                // not a real receiver object). For bare Heap.new: receiver is "Heap" string,
                // inner value is in args[0].
                let caller_base = state.reg_base();
                let value = if receiver_type_name.as_deref() == Some("Heap") {
                    // Bare receiver path: value is in args[0]
                    if args.count > 0 {
                        state.registers.get(caller_base, Reg(args.start.0))
                    } else {
                        Value::unit()
                    }
                } else {
                    // dyn: dispatch path: receiver is the value itself
                    receiver
                };

                // Allocate CBGR object: 32-byte AllocationHeader + 8-byte Value
                // Layout: [size:4][align:4][generation:4][epoch:2][caps:2][type_id:4][flags:4][reserved:8]
                const CBGR_HEADER_SIZE: usize = 32;
                let data_size = std::mem::size_of::<Value>() as u32;
                let alloc_size = CBGR_HEADER_SIZE + data_size as usize;
                let layout = std::alloc::Layout::from_size_align(alloc_size, 32)
                    .map_err(|_| InterpreterError::OutOfMemory {
                        requested: alloc_size,
                        available: 0,
                    })?;
                let raw_ptr = unsafe { std::alloc::alloc_zeroed(layout) };
                if raw_ptr.is_null() {
                    return Err(InterpreterError::OutOfMemory {
                        requested: alloc_size,
                        available: 0,
                    });
                }

                // Initialize CBGR AllocationHeader (32 bytes): [size:u32, align:u32,
                // generation:u32, epoch:u16, flags:u16, type_id:u32, padding:u32, reserved:u64]
                // The generation counter enables use-after-free detection at ~15ns per check.
                let generation = state.heap.next_generation();
                let epoch = state.cbgr_epoch as u16;
                unsafe {
                    *(raw_ptr as *mut u32) = data_size;                     // offset 0: size
                    *(raw_ptr.add(4) as *mut u32) = 8;                      // offset 4: alignment
                    *(raw_ptr.add(8) as *mut u32) = generation;            // offset 8: generation
                    *(raw_ptr.add(12) as *mut u16) = epoch;                 // offset 12: epoch (u16)
                    *(raw_ptr.add(14) as *mut u16) = 0x03;                  // offset 14: capabilities (read+write)
                    *(raw_ptr.add(16) as *mut u32) = 0;                     // offset 16: type_id
                    *(raw_ptr.add(20) as *mut u32) = 0;                     // offset 20: flags (0 = allocated)
                    // offsets 24-31: reserved (already zeroed)
                    // Write user data value after the header
                    *(raw_ptr.add(CBGR_HEADER_SIZE) as *mut Value) = value;
                }
                // Track this as a CBGR allocation for raw field access in GetField
                state.cbgr_allocations.insert(raw_ptr as usize);

                // Return pointer to data portion (after header)
                let data_ptr = unsafe { raw_ptr.add(CBGR_HEADER_SIZE) };
                state.set_reg(dst, Value::from_ptr(data_ptr));
                state.record_allocation();
                return Ok(DispatchResult::Continue);
            }
        }
    }

    // Handle Text.from(string) - string conversion
    // VBC-internal: interpreter runtime dispatch — Text.from() is a no-op in
    // VBC when the argument is already a Text value (small string or heap
    // string). For any other argument shape (Char, Int, &Byte buffer,
    // user-defined type with a `From` impl) fall through to the regular
    // user-function dispatch so the stdlib `impl From<T> for Text` body
    // runs.
    if bare_method_name == "from"
        && args.count > 0
        && let Some(ref name) = receiver_type_name
            && WKT::Text.matches(name) {
                let caller_base = state.reg_base();
                let value = state.registers.get(caller_base, Reg(args.start.0));
                // Only short-circuit when the arg is already a Text
                // representation. Anything else (Char, Int, raw byte slice,
                // user-defined type) must go through the compiled stdlib
                // `From<T>::from` body.
                let is_already_text = value.is_small_string() || {
                    value.is_ptr()
                        && !value.is_nil()
                        && !value.is_boxed_int()
                        && {
                            let p = value.as_ptr::<u8>();
                            if p.is_null() {
                                false
                            } else {
                                let header = unsafe { &*(p as *const heap::ObjectHeader) };
                                header.type_id == TypeId::TEXT
                                    || header.type_id == TypeId(0x0001)
                            }
                        }
                };
                if is_already_text {
                    state.set_reg(dst, value);
                    return Ok(DispatchResult::Continue);
                }
                // Otherwise fall through to user-function lookup.
            }

    // Handle static constructor methods (e.g., List.new(), Set.new(), Map.new())
    // ALWAYS use builtin handlers for collection types. The stdlib user-defined
    // constructors (e.g., core.collections.map.Map.new) create plain struct records,
    // but ALL builtin instance methods (insert, get, len, etc.) expect the specific
    // memory layout with the correct TypeId (LIST, MAP, SET, CHANNEL, DEQUE).
    // This is the same reasoning as Channel (see original note below).
    if bare_method_name == "new" {
        let is_list = receiver_type_name.as_deref() == Some("List");
        let is_set = receiver_type_name.as_deref() == Some("Set");
        let is_map = receiver_type_name.as_deref() == Some("Map");
        let is_deque = receiver_type_name.as_deref() == Some("Deque");
        let is_channel = receiver_type_name.as_deref() == Some("Channel");

        if is_list {
            // Create empty List: [len, cap, backing_ptr] with TypeId::LIST
            const DEFAULT_CAP: usize = 16;
            let obj = state.heap.alloc(TypeId::LIST, 3 * std::mem::size_of::<Value>())?;
            state.record_allocation();
            let data_ptr = unsafe {
                (obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
            };
            let backing = state.heap.alloc_array(TypeId::LIST, DEFAULT_CAP)?;
            state.record_allocation();
            unsafe {
                *data_ptr = Value::from_i64(0);                                    // len = 0
                *data_ptr.add(1) = Value::from_i64(DEFAULT_CAP as i64);            // cap
                *data_ptr.add(2) = Value::from_ptr(backing.as_ptr() as *mut u8);   // backing_ptr
            }
            state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
            return Ok(DispatchResult::Continue);
        } else if is_set || is_map {
            // Create empty Set/Map: [count, capacity, entries_ptr]
            const DEFAULT_CAP: usize = 16;
            let type_id = if is_set { TypeId::SET } else { TypeId::MAP };
            let obj = state.heap.alloc(type_id, 3 * std::mem::size_of::<Value>())?;
            state.record_allocation();
            let header_ptr = unsafe {
                (obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
            };
            let entries = state.heap.alloc_array(TypeId::UNIT, DEFAULT_CAP * 2)?;
            state.record_allocation();
            let entries_ptr = entries.as_ptr() as *mut u8;
            let entries_data = unsafe {
                entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
            };
            for i in 0..(DEFAULT_CAP * 2) {
                unsafe { *entries_data.add(i) = Value::unit(); }
            }
            unsafe {
                *header_ptr = Value::from_i64(0);
                *header_ptr.add(1) = Value::from_i64(DEFAULT_CAP as i64);
                *header_ptr.add(2) = Value::from_ptr(entries_ptr);
            }
            state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
            return Ok(DispatchResult::Continue);
        } else if is_deque {
            // Create empty Deque: [data(0), head(1), len(2), cap(3)]
            // Layout matches stdlib: type Deque<T> is { data, head, len, cap }
            const DEFAULT_CAP: usize = 16;
            let obj = state.heap.alloc(TypeId::DEQUE, 4 * std::mem::size_of::<Value>())?;
            state.record_allocation();
            let header_ptr = unsafe {
                (obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
            };
            let buffer = state.heap.alloc_array(TypeId::UNIT, DEFAULT_CAP)?;
            state.record_allocation();
            let buffer_ptr = buffer.as_ptr() as *mut u8;
            let buf_data = unsafe {
                buffer_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
            };
            for i in 0..DEFAULT_CAP {
                unsafe { *buf_data.add(i) = Value::unit(); }
            }
            unsafe {
                *header_ptr = Value::from_ptr(buffer_ptr);       // data (index 0)
                *header_ptr.add(1) = Value::from_i64(0);         // head (index 1)
                *header_ptr.add(2) = Value::from_i64(0);         // len  (index 2)
                *header_ptr.add(3) = Value::from_i64(DEFAULT_CAP as i64); // cap  (index 3)
            }
            state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
            return Ok(DispatchResult::Continue);
        } else if is_channel {
            // Create bounded Channel: [len, cap, head, buffer_ptr, closed]
            let caller_base = state.reg_base();
            let cap = if args.count > 0 {
                state.registers.get(caller_base, Reg(args.start.0)).as_i64().max(1) as usize
            } else {
                16
            };
            let obj = state.heap.alloc(TypeId::CHANNEL, 5 * std::mem::size_of::<Value>())?;
            state.record_allocation();
            let header_ptr = unsafe {
                (obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
            };
            let buffer = state.heap.alloc_array(TypeId::UNIT, cap)?;
            state.record_allocation();
            let buffer_ptr = buffer.as_ptr() as *mut u8;
            let buf_data = unsafe {
                buffer_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
            };
            for i in 0..cap {
                unsafe { *buf_data.add(i) = Value::unit(); }
            }
            unsafe {
                *header_ptr = Value::from_i64(0);
                *header_ptr.add(1) = Value::from_i64(cap as i64);
                *header_ptr.add(2) = Value::from_i64(0);
                *header_ptr.add(3) = Value::from_ptr(buffer_ptr);
                *header_ptr.add(4) = Value::from_i64(0);
            }
            state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
            return Ok(DispatchResult::Continue);
        }
    }

    // Handle List.with_capacity(n) static method
    if bare_method_name == "with_capacity" {
        let is_list = receiver_type_name.as_deref() == Some("List");
        if is_list && args.count == 1 {
            let caller_base = state.reg_base();
            let capacity_val = state.registers.get(caller_base, Reg(args.start.0));
            let capacity = capacity_val.as_i64().max(0) as usize;
            let actual_cap = if capacity == 0 { 16 } else { capacity };

            // Create List: [len, cap, backing_ptr] with TypeId::LIST
            let obj = state.heap.alloc(TypeId::LIST, 3 * std::mem::size_of::<Value>())?;
            state.record_allocation();
            let data_ptr = unsafe {
                (obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
            };
            let backing = state.heap.alloc_array(TypeId::LIST, actual_cap)?;
            state.record_allocation();
            unsafe {
                *data_ptr = Value::from_i64(0);                                    // len = 0
                *data_ptr.add(1) = Value::from_i64(actual_cap as i64);            // cap
                *data_ptr.add(2) = Value::from_ptr(backing.as_ptr() as *mut u8);  // backing_ptr
            }
            state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
            return Ok(DispatchResult::Continue);
        }
    }

    // Handle Numeric protocol static methods (Float.zero(), Float.one(), Int.zero(), etc.)
    // These are commonly used in generic code like Vector<T>.zeros() which calls T.zero()
    if (bare_method_name == "zero" || bare_method_name == "one" || bare_method_name == "epsilon")
        && let Some(ref name) = receiver_type_name {
            match name.as_str() {
                "Float" | "Float64" | "f64" => {
                    let value = match bare_method_name.as_str() {
                        "zero" => 0.0,
                        "one" => 1.0,
                        "epsilon" => f64::EPSILON,
                        _ => 0.0,
                    };
                    state.set_reg(dst, Value::from_f64(value));
                    return Ok(DispatchResult::Continue);
                }
                "Float32" | "f32" => {
                    let value = match bare_method_name.as_str() {
                        "zero" => 0.0f32 as f64,
                        "one" => 1.0f32 as f64,
                        "epsilon" => f32::EPSILON as f64,
                        _ => 0.0,
                    };
                    state.set_reg(dst, Value::from_f64(value));
                    return Ok(DispatchResult::Continue);
                }
                "Int" | "Int64" | "i64" => {
                    let value = match bare_method_name.as_str() {
                        "zero" => 0,
                        "one" => 1,
                        "epsilon" => 0, // integers don't have epsilon
                        _ => 0,
                    };
                    state.set_reg(dst, Value::from_i64(value));
                    return Ok(DispatchResult::Continue);
                }
                _ => {}
            }
        }

    // Handle static from_le_bytes / from_be_bytes (e.g., Int.from_le_bytes(bytes))
    if bare_method_name == "from_le_bytes" || bare_method_name == "from_be_bytes" {
        let caller_base = state.reg_base();
        let bytes_val = state.registers.get(caller_base, Reg(args.start.0));
        let bytes_ptr = bytes_val.as_ptr::<u8>();
        if !bytes_ptr.is_null() {
            let data = unsafe { bytes_ptr.add(heap::OBJECT_HEADER_SIZE) };
            let header = unsafe { &*(bytes_ptr as *const heap::ObjectHeader) };
            let byte_count = header.size as usize;
            let mut buf = [0u8; 8];
            let n = byte_count.min(8);
            for (i, byte) in buf.iter_mut().enumerate().take(n) {
                *byte = unsafe { *data.add(i) };
            }
            let result = if bare_method_name == "from_le_bytes" {
                i64::from_le_bytes(buf)
            } else {
                i64::from_be_bytes(buf)
            };
            state.set_reg(dst, Value::from_i64(result));
            return Ok(DispatchResult::Continue);
        }
    }

    // Handle Runtime.* static methods for CBGR epoch system.
    // Match by bare method name since the receiver may be a non-string value
    // (e.g., when called from a compiled stdlib function body via dyn: dispatch,
    // the receiver is an opaque heap pointer, not the "Runtime" type string).
    if bare_method_name == "current_epoch" {
        state.set_reg(dst, Value::from_i64(state.cbgr_epoch as i64));
        return Ok(DispatchResult::Continue);
    }
    if bare_method_name == "advance_epoch" {
        state.cbgr_epoch = state.cbgr_epoch.wrapping_add(1);
        state.set_reg(dst, Value::unit());
        return Ok(DispatchResult::Continue);
    }
    if let Some(ref name) = receiver_type_name {
        match name.as_str() {
            "Runtime" => {
                match bare_method_name.as_str() {
                    "current_epoch" => {
                        state.set_reg(dst, Value::from_i64(state.cbgr_epoch as i64));
                        return Ok(DispatchResult::Continue);
                    }
                    "advance_epoch" => {
                        state.cbgr_epoch = state.cbgr_epoch.wrapping_add(1);
                        state.set_reg(dst, Value::unit());
                        return Ok(DispatchResult::Continue);
                    }
                    _ => {}
                }
            }
            "Epoch" => {
                match bare_method_name.as_str() {
                    "current" => {
                        state.set_reg(dst, Value::from_i64(state.cbgr_epoch as i64));
                        return Ok(DispatchResult::Continue);
                    }
                    "advance" => {
                        state.cbgr_epoch = state.cbgr_epoch.wrapping_add(1);
                        state.set_reg(dst, Value::unit());
                        return Ok(DispatchResult::Continue);
                    }
                    "max_value" => {
                        state.set_reg(dst, Value::from_i64(u32::MAX as i64));
                        return Ok(DispatchResult::Continue);
                    }
                    _ => {}
                }
            }
            "Time"
                if method_name.as_str() == "now" => {
                    // Return current time in nanoseconds since epoch
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos() as i64;
                    state.set_reg(dst, Value::from_i64(now));
                    return Ok(DispatchResult::Continue);
                }
            _ => {}
        }
    }

    // Handle eq/ne for Text strings - must be before function search to avoid incorrect
    // dispatch to Maybe.eq or other imported type's eq methods
    if (bare_method_name == "eq" || bare_method_name == "ne") && args.count == 1 {
        let is_string_receiver = receiver.is_small_string() ||
            (receiver.is_ptr() && !receiver.is_nil() && {
                let ptr = receiver.as_ptr::<u8>();
                if !ptr.is_null() {
                    let header = unsafe { &*(ptr as *const heap::ObjectHeader) };
                    header.type_id == crate::types::TypeId::TEXT || header.type_id.0 == 0x0001
                } else {
                    false
                }
            });

        if is_string_receiver {
            let caller_base = state.reg_base();
            let other_val = state.registers.get(caller_base, Reg(args.start.0));

            // Handle CBGR reference
            let other = if is_cbgr_ref(&other_val) {
                let (abs_index, _) = decode_cbgr_ref(other_val.as_i64());
                state.registers.get_absolute(abs_index)
            } else {
                other_val
            };

            let recv = if is_cbgr_ref(&receiver) {
                let (abs_index, _) = decode_cbgr_ref(receiver.as_i64());
                state.registers.get_absolute(abs_index)
            } else {
                receiver
            };

            let result = deep_value_eq(&recv, &other, state);
            let final_result = if bare_method_name == "eq" { result } else { !result };
            state.set_reg(dst, Value::from_bool(final_result));
            return Ok(DispatchResult::Continue);
        }
    }

    // Check if receiver is a builtin collection (Map, Set, List). If so, skip user-defined
    // method lookup to ensure builtin methods are used. This prevents issues where user-defined
    // methods from core/collections/map.vr try to call private methods on builtin objects.
    // dispatch_receiver already has CBGR refs dereffed.
    let is_builtin_collection = if dispatch_receiver.is_ptr() && !dispatch_receiver.is_nil() {
        let ptr = dispatch_receiver.as_ptr::<u8>();
        let header = unsafe { &*(ptr as *const heap::ObjectHeader) };
        
        header.type_id == TypeId::MAP || header.type_id == TypeId::SET || header.type_id == TypeId::LIST
            || header.type_id == TypeId::DEQUE || header.type_id == TypeId::CHANNEL
    } else {
        false
    };

    // Fallback: try to find a user-defined impl method by searching for "Type.method_name"
    // in the module's function table. This handles methods defined in `implement Type { ... }` blocks.
    //
    // If method_name already contains "." or "::" (e.g., "MapFlags.to_unix_flags"), it's already qualified.
    // Otherwise, we search for functions ending with ".method_name".
    // Skip this for builtin collections to ensure builtin methods are used.
    // Strip "dyn:" or "ctx:" prefix from context/protocol dispatch methods.
    // VBC codegen emits "dyn:Protocol.method" for context method calls,
    // but registered function names are "Type.method" without the prefix.
    let method_name = if method_name.starts_with("dyn:") || method_name.starts_with("ctx:") {
        let rest = &method_name[4..];
        if let Some(dot) = rest.rfind('.') {
            rest[dot + 1..].to_string()
        } else {
            method_name
        }
    } else {
        method_name
    };

    let is_already_qualified = method_name.contains('.') || method_name.contains("::");
    let method_suffix = if is_already_qualified {
        method_name.clone()
    } else {
        format!(".{}", method_name)
    };
    let mut found_func_id: Option<FunctionId> = None;

    // Method dispatch cache: check if we've resolved this (method, receiver-type)
    // pair before. The cache key includes the receiver's `type_id` so two
    // different types implementing the same method name (e.g.,
    // `MockDatabase.name` and `InMemoryDatabase.name`, both implementing
    // `protocol Named`) don't collide — keying by method_id alone silently
    // pinned the first resolution to every later receiver of any type.
    // For non-pointer receivers (primitives, nil), the type_id slot is 0.
    let receiver_type_id_for_cache: u32 = if receiver.is_ptr() && !receiver.is_nil() {
        let ptr = receiver.as_ptr::<u8>();
        if !ptr.is_null()
            && (ptr as usize).is_multiple_of(std::mem::align_of::<heap::ObjectHeader>())
        {
            // SAFETY: pointer alignment verified; every heap object begins with
            // an ObjectHeader, read-only.
            unsafe { (*(ptr as *const heap::ObjectHeader)).type_id.0 }
        } else { 0 }
    } else { 0 };
    let cache_key = (method_id, receiver_type_id_for_cache);

    // For builtin collections (List/Map/Set/Deque/Channel), NEVER use compiled
    // functions because the interpreter has its own optimized handlers with
    // correct memory layout. The is_already_qualified flag is irrelevant for
    // builtin collections.
    if !is_builtin_collection
        && let Some(&cached_fid) = state.method_cache.get(&cache_key) {
            // Verify the cached function still exists (should always be true within a module)
            if state.module.get_function(cached_fid).is_some() {
                found_func_id = Some(cached_fid);
            }
        }

    // Skip user-defined method lookup for builtin collections (Map, Set, List).
    // The interpreter has optimized builtin handlers for these types with correct
    // memory layout. Using compiled stdlib functions would fail because they expect
    // different internal representations (e.g., `self.inner.insert(…)` on a
    // stdlib `Set<T>` assumes a struct `{ inner: Map<T,()> }` layout, but
    // `Set.new()` returns the builtin `[count, capacity, entries_ptr]`
    // layout — dereferencing a non-existent field panics with
    // "field access out of bounds"). New collection methods belong as
    // builtin handlers alongside the existing `insert` / `contains` / …
    // dispatchers above (see the `"filter" if is_set` arm below).
    if found_func_id.is_none() && !is_builtin_collection {
        // First try the old approach: treat method_id as function_id for backwards compatibility
        let func_id = FunctionId(method_id);
        if let Some(func) = state.module.get_function(func_id) {
            // Verify the function name actually ends with the expected method suffix
            // to prevent false matches when method_id is a string table index
            let func_name = state.module.strings.get(func.name).unwrap_or("");
            if is_already_qualified {
                // For qualified names, do exact match
                if func_name == method_name {
                    found_func_id = Some(func_id);
                }
            } else if func_name.ends_with(&method_suffix) {
                found_func_id = Some(func_id);
            }
        }

        // If that didn't work, search for any registered function named "SomeType::method_name"
        if found_func_id.is_none() {
            let expected_param_count = args.count as usize + 1; // +1 for self

            // Receiver-type-aware dispatch — before scanning the whole function
            // table, peek at the receiver's heap-object header to recover the
            // concrete type name. When a qualified function
            // "<ReceiverType>.<method>" exists, prefer it over same-named
            // methods on other types (core.math.elementary.min, List.min, …)
            // which would otherwise be picked by "first suffix match wins"
            // and lead to wrong-layout return values.
            // See vcs/specs/L0-critical/vbc/struct_layout/ for reproducers.
            // Use `dispatch_receiver` (post-deref) instead of the
            // raw `receiver` so a `Shared<T>` carrier whose
            // unrecognised method triggered the auto-deref above
            // resolves to the inner `T`'s type name (e.g.,
            // `Shared<AtomicInt>::load` → `receiver_type = "AtomicInt"`),
            // which lets the qualified lookup at line ~1014
            // build `"AtomicInt.load"` and find the user-compiled
            // method. Pre-fix this saw `TypeId::SHARED` and
            // produced no type name, falling through to the
            // catch-all panic.
            let receiver_type: Option<String> =
                if dispatch_receiver.is_ptr() && !dispatch_receiver.is_nil() && !is_already_qualified {
                    let ptr = dispatch_receiver.as_ptr::<u8>();
                    if ptr.is_null() {
                        None
                    } else if (ptr as usize)
                        .is_multiple_of(std::mem::align_of::<heap::ObjectHeader>())
                    {
                        // SAFETY: alignment verified; every heap object
                        // begins with an ObjectHeader.
                        let header = unsafe { &*(ptr as *const heap::ObjectHeader) };
                        state
                            .module
                            .get_type(header.type_id)
                            .map(|td| state.module.strings.get(td.name).unwrap_or("").to_string())
                            .filter(|s| !s.is_empty())
                    } else {
                        None
                    }
                } else {
                    None
                };

            // First pass: look for an exact receiver-type match
            // (e.g., receiver is FlexItem → prefer "FlexItem.min" over any
            // other "*.min"). Only runs when we recovered a type name.
            if let Some(ref ty_name) = receiver_type {
                // Support both qualified registrations (e.g.,
                // "core.term.layout.FlexItem.min") and bare ones
                // ("FlexItem.min") — the prefix must end with the type name.
                let dotted = format!(".{}.{}", ty_name, method_name);
                let bare = format!("{}.{}", ty_name, method_name);
                for func in &state.module.functions {
                    let func_name = state.module.strings.get(func.name).unwrap_or("");
                    let type_match = func_name == bare || func_name.ends_with(&dotted);
                    if type_match
                        && (func.params.len() == expected_param_count
                            || func.register_count > 0)
                    {
                        found_func_id = Some(func.id);
                        break;
                    }
                }
            }

            // Second pass: original suffix-match (no receiver-type constraint)
            // — kept for the many call sites where we either have no heap
            // receiver (small-string type names, primitives) or the type
            // doesn't itself declare the method (protocol-default body,
            // inherited impl, …). Preserves all existing passing tests.
            if found_func_id.is_none() {
                for func in &state.module.functions {
                    let func_name = state.module.strings.get(func.name).unwrap_or("");
                    let matches = if is_already_qualified {
                        // For qualified names, require the qualified suffix to match
                        // (prevents "Result.unwrap_or" matching when looking for
                        // "Maybe.unwrap_or").
                        func_name == method_name || func_name.ends_with(&method_suffix)
                    } else {
                        func_name.ends_with(&method_suffix) && func_name.contains('.')
                    };
                    if matches {
                        // Prefer exact parameter count match
                        if func.params.len() == expected_param_count
                            || func.register_count > 0
                        {
                            found_func_id = Some(func.id);
                            break;
                        }
                    }
                }
            }
        }
    }

    if let Some(target_func_id) = found_func_id {
        // Store in method dispatch cache keyed by (method_id, receiver_type_id)
        // so distinct receiver types preserve distinct resolutions.
        state.method_cache.insert(cache_key, target_func_id);

        if let Some(func) = state.module.get_function(target_func_id) {
            let reg_count = func.register_count;
            let return_pc = state.pc();
            let caller_base = state.reg_base();

            // Some method-shaped calls dispatch to functions that DON'T take
            // self — most commonly context methods declared without `&self`
            // (`context Logger { fn log(tag: Text, message: Text); }`). For
            // those, prepending the receiver as r0 shifts every real arg one
            // slot too far and produces "tag={empty record}" / "message=<the
            // intended tag>" symptoms. Detect by inspecting the callee's
            // first param name: if it's not `self`, copy args directly into
            // r0+ without prepending the receiver.
            let takes_self = func.params.first()
                .and_then(|p| state.module.strings.get(p.name))
                .map(|n| n == "self")
                .unwrap_or(true);

            let new_base = state.call_stack.push_frame(target_func_id, reg_count, return_pc, dst)?;
            state.registers.push_frame(reg_count);

            if takes_self {
                // First arg is receiver (self)
                state.registers.set(new_base, Reg(0), receiver);
                // Copy remaining arguments
                for i in 0..args.count {
                    let arg_value = state.registers.get(caller_base, Reg(args.start.0 + i as u16));
                    state.registers.set(new_base, Reg(i as u16 + 1), arg_value);
                }
            } else {
                // Static method dispatched via CallM (e.g., context method
                // with no `self`). Skip the receiver and copy args into r0+.
                for i in 0..args.count {
                    let arg_value = state.registers.get(caller_base, Reg(args.start.0 + i as u16));
                    state.registers.set(new_base, Reg(i as u16), arg_value);
                }
            }

            state.set_pc(0);
            state.record_call();
            return Ok(DispatchResult::Continue);
        }
    }

    // Fallback for "eq" method: when no custom Eq implementation is found,
    // use structural deep equality comparison. This supports the pattern where
    // `==` compiles to CallM("eq") for non-primitive types, and types without
    // a custom `implement Eq` still get correct structural equality.
    if bare_method_name == "eq" && args.count == 1 {
        let caller_base = state.reg_base();
        let other_val = state.registers.get(caller_base, Reg(args.start.0));

        // Handle CBGR reference: the argument might be &T (CBGR ref), need to deref
        let other = if is_cbgr_ref(&other_val) {
            let (abs_index, _) = decode_cbgr_ref(other_val.as_i64());
            state.registers.get_absolute(abs_index)
        } else {
            other_val
        };

        // Also handle receiver being a CBGR reference
        let recv = if is_cbgr_ref(&receiver) {
            let (abs_index, _) = decode_cbgr_ref(receiver.as_i64());
            state.registers.get_absolute(abs_index)
        } else {
            receiver
        };

        let result = deep_value_eq(&recv, &other, state);
        state.set_reg(dst, Value::from_bool(result));
        return Ok(DispatchResult::Continue);
    }

    // Fallback for "ne" method: default implementation is !eq(other)
    // This implements the Eq protocol's default ne() method.
    if bare_method_name == "ne" && args.count == 1 {
        let caller_base = state.reg_base();
        let other_val = state.registers.get(caller_base, Reg(args.start.0));

        // Handle CBGR reference: the argument might be &T (CBGR ref), need to deref
        let other = if is_cbgr_ref(&other_val) {
            let (abs_index, _) = decode_cbgr_ref(other_val.as_i64());
            state.registers.get_absolute(abs_index)
        } else {
            other_val
        };

        // Also handle receiver being a CBGR reference
        let recv = if is_cbgr_ref(&receiver) {
            let (abs_index, _) = decode_cbgr_ref(receiver.as_i64());
            state.registers.get_absolute(abs_index)
        } else {
            receiver
        };

        // ne is the inverse of eq
        let result = !deep_value_eq(&recv, &other, state);
        state.set_reg(dst, Value::from_bool(result));
        return Ok(DispatchResult::Continue);
    }

    #[cfg(debug_assertions)]
    if method_name.contains("ensure_capacity") {
        eprintln!("DEBUG: Looking for method '{}', is_builtin_collection={}", method_name, is_builtin_collection);
        eprintln!("DEBUG: All Map-related functions in module:");
        for func in &state.module.functions {
            let func_name = state.module.strings.get(func.name).unwrap_or("");
            if func_name.contains("Map") || func_name.contains("map") {
                eprintln!("  - {} (id={}, params={})", func_name, func.id.0, func.params.len());
            }
        }
    }

    // Last-chance native variant dispatch — applies when
    // `prefer_user_compiled` short-circuited the early native check
    // (line ~325) AND the user-compiled lookup itself fell through
    // every codepath without finding a runtime-typed instance. The
    // generic `Result<T, E>::map_err<F>` definition exists in
    // `core/base/result.vr` and gets registered as a function, but
    // VBC monomorphisation does not always produce a callable body
    // for the concrete `Result<i32, IoError>` instances synthesised
    // at runtime (see e.g. `core/net/tcp.vr:321`
    // `socket(...).map_err(IoError.from_os)?`). The native variant
    // handler in `dispatch_variant_method` is monomorphisation-free
    // — the variant layout is type-erased and the closure carries
    // the type-specific work — so it is sound to retry here as a
    // safety net before panicking.
    if dispatch_receiver.is_ptr() && !dispatch_receiver.is_nil()
        && let Some(result) = dispatch_variant_method(
            state,
            dispatch_receiver,
            &bare_method_name,
            &args,
            &method_name,
        )?
    {
        state.set_reg(dst, result);
        return Ok(DispatchResult::Continue);
    }

    // Reflexive `.into()` identity safety-net.  The stdlib defines
    // blanket `implement<T> From<T> for T` (core/base/protocols.vr:339)
    // plus `implement<T, U: From<T>> Into<U> for T` so any
    // `value.into()` call whose source and target type coincide
    // SHOULD reduce to the identity function.  VBC monomorphisation
    // does not always synthesise the concrete `T::into() -> T`
    // instance — same gap the `Result.map_err` safety-net handles
    // for fallible combinators above.  We only fall back to identity
    // when EVERY other dispatch path missed; cross-type `.into()`
    // calls (where a real `From::from` was monomorphised) hit one
    // of the dispatchers above and never reach this branch.  Pre-
    // fix every `"…".into()` call on a literal Text panicked with
    // "method 'Text.into' not found on value" — broke shell-script
    // execution the moment scripts actually ran past type-check.
    if bare_method_name == "into" && args.count == 0 {
        state.set_reg(dst, receiver);
        return Ok(DispatchResult::Continue);
    }

    Err(InterpreterError::Panic {
        message: format!("method '{}' not found on value", method_name),
    })
}

/// Returns monotonic nanosecond timestamp using a shared thread-local epoch.
/// All time operations in the interpreter MUST use this to ensure consistency
/// between FfiExtended sub-opcodes and method dispatch.
pub(super) fn monotonic_nanos_shared() -> i64 {
    use std::time::Instant;
    thread_local! {
        static EPOCH: Instant = Instant::now();
    }
    EPOCH.with(|epoch| epoch.elapsed().as_nanos() as i64)
}

/// Returns wall-clock nanoseconds since Unix epoch.
pub(super) fn realtime_nanos_shared() -> i64 {
    use std::time::SystemTime;
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as i64
}

/// Dispatch built-in methods on primitive types (Int, Float, Bool, Char, Byte).
pub(super) fn dispatch_primitive_method(
    state: &mut InterpreterState,
    receiver: &Value,
    method: &str,
    args: &RegRange,
) -> InterpreterResult<Option<Value>> {
    // (removed leftover debug eprintln! that flooded stderr on every method
    // dispatch in debug builds — that noise polluted test stdout/stderr and
    // broke expected-stdout comparisons in the VCS runner.)
    // If method name is qualified (contains "."), check if the type prefix is a known builtin.
    // For user-defined types like "SimpleVec.len", skip builtin dispatch so user method is called.
    // EXCEPTION: If the receiver's actual ObjectHeader TypeId is a builtin collection (Map, Set,
    // List, Deque, Channel), always dispatch builtin methods regardless of static type prefix.
    // This handles cases where VBC codegen assigns wrong static type (e.g., "Maybe.contains_key"
    // when the actual runtime object is a Map returned from filter()).
    let receiver_is_actually_builtin = receiver.is_ptr() && !receiver.is_nil() && {
        let ptr = receiver.as_ptr::<u8>();
        if !ptr.is_null() {
            let header = unsafe { &*(ptr as *const heap::ObjectHeader) };
            // Builtin collection types always use builtin dispatch regardless of static prefix.
            let is_builtin_collection = header.type_id == TypeId::MAP || header.type_id == TypeId::SET
                || header.type_id == TypeId::LIST || header.type_id == TypeId::DEQUE
                || header.type_id == TypeId::CHANNEL;
            // Iterator objects (UNIT type_id, 4-value layout) have builtin iterator methods
            // (next, fold, map, filter, collect, etc.) that must be dispatched even when
            // the static type prefix is a user-defined iterator type like "ListIter".
            let is_iterator_object = header.type_id == TypeId::UNIT
                && header.size as usize == 4 * std::mem::size_of::<Value>();
            is_builtin_collection || is_iterator_object
        } else {
            false
        }
    };

    if let Some(pos) = method.rfind('.') {
        let type_prefix = &method[..pos];
        // VBC-internal: interpreter method routing gate. Uses WKT registry to check
        // if the type prefix names a stdlib type with built-in interpreter dispatch.
        // User-defined types must fall through to compiled method lookup.
        // Additional interpreter-specific aliases (String, Byte, numeric variants,
        // timer types) that are not in WKT are also checked.
        let is_builtin_prefix = WKT::from_name(type_prefix).is_some()
            || matches!(type_prefix,
                "String" | "Byte" | "UInt64" | "Int32" | "Float32" | "Float64"
                | "Stopwatch" | "PerfCounter" | "DeadlineTimer"
            );
        if !is_builtin_prefix && !receiver_is_actually_builtin {
            // User-defined type and not a builtin collection - let user method be called
            return Ok(None);
        }
    }
    // VBC-internal: same routing gate for "::" path separator (static method syntax).
    if let Some(pos) = method.rfind("::") {
        let type_prefix = &method[..pos];
        let is_builtin_prefix = WKT::from_name(type_prefix).is_some()
            || matches!(type_prefix,
                "String" | "Byte" | "UInt64" | "Int32" | "Float32" | "Float64"
                | "Stopwatch" | "PerfCounter" | "DeadlineTimer"
            );
        if !is_builtin_prefix && !receiver_is_actually_builtin {
            return Ok(None);
        }
    }

    // Extract unqualified method name from qualified names like "Heap.generation" -> "generation"
    // This handles cases where codegen emits fully qualified method names for struct methods.
    // Support both "." (new convention) and "::" (legacy) for backwards compatibility.
    let method = if let Some(pos) = method.rfind('.') {
        &method[pos + 1..]
    } else if let Some(pos) = method.rfind("::") {
        &method[pos + 2..]
    } else {
        method
    };

    // CBGR register reference unwrapping: when a method is called on a CBGR register
    // reference (negative Int encoding), deref the reference to get the inner value
    // and dispatch the method on it (e.g., `.sub()` on a reference to a pointer).
    // This must be checked BEFORE the Int section, which has a catch-all `_ => return Ok(None)`.
    if is_cbgr_ref(receiver) {
        let (abs_index, generation) = decode_cbgr_ref(receiver.as_i64());

        // CBGR reference-specific methods
        match method {
            "stored_generation" => {
                return Ok(Some(Value::from_i64(generation as i64)));
            }
            "is_valid" => {
                let current_gen = state.registers.get_generation(abs_index);
                return Ok(Some(Value::from_bool(generation == current_gen)));
            }
            "epoch" => {
                // Return the epoch from the interpreter state (register refs don't store epoch inline)
                return Ok(Some(Value::from_i64(state.cbgr_epoch as i64)));
            }
            "epoch_caps" | "epoch_caps_raw" | "raw_epoch_caps" => {
                // Return packed epoch + capabilities for register-based reference
                let epoch = state.cbgr_epoch as u32;
                let is_mut = is_cbgr_ref_mutable(receiver.as_i64());
                let cap_bits: u32 = if is_mut { 0x03 } else { 0x01 }; // read+write or read-only
                let packed = ((epoch & 0x00FF_FFFF) << 8) | cap_bits;
                return Ok(Some(Value::from_i64(packed as i64)));
            }
            "capabilities" => {
                let epoch = state.cbgr_epoch as u32;
                let is_mut = is_cbgr_ref_mutable(receiver.as_i64());
                let cap_bits: u32 = if is_mut { 0x03 } else { 0x01 };
                let packed = ((epoch & 0x00FF_FFFF) << 8) | cap_bits;
                return Ok(Some(Value::from_i64(packed as i64)));
            }
            "can_read" => {
                return Ok(Some(Value::from_bool(true)));
            }
            "can_write" => {
                let is_mut = is_cbgr_ref_mutable(receiver.as_i64());
                return Ok(Some(Value::from_bool(is_mut)));
            }
            "generation" => {
                return Ok(Some(Value::from_i64(generation as i64)));
            }
            "raw_ptr" => {
                // Return the absolute register index as a pseudo-pointer
                return Ok(Some(Value::from_i64(abs_index as i64)));
            }
            "is_epoch_valid" => {
                // Check if reference epoch is within validity window of current epoch
                // Uses a default validity window of 1,000,000 epochs
                let _current = state.cbgr_epoch;
                // The reference epoch is the current epoch at creation time
                // For register refs, we use the current epoch (they're always "fresh")
                return Ok(Some(Value::from_bool(true)));
            }
            "clone" => {
                // Clone on a reference should return a clone of the inner value
                // This handles cases like ref_text.clone() where ref_text: &Text
                let inner_val = state.registers.get_absolute(abs_index);
                // For primitives and small strings, just return the value (copy semantics)
                // For heap-allocated objects, we would need deep clone
                return Ok(Some(inner_val));
            }
            _ => {}
        }

        let inner_val = state.registers.get_absolute(abs_index);
        if inner_val.is_ptr() && !inner_val.is_nil() {
            // The inner value is a pointer - dispatch pointer methods on it
            return dispatch_primitive_method(state, &inner_val, method, args);
        }
    }

    // CBGR data pointer methods: methods on pointer-based references obtained from
    // `&*heap_value` or direct Heap.new() allocations. These pointers point to the
    // data area (16 bytes after the AllocationHeader). We detect them by checking
    // whether (data_ptr - 16) is a known CBGR allocation.
    if receiver.is_ptr() && !receiver.is_nil() {
        let data_ptr = receiver.as_ptr::<u8>() as usize;
        let header_addr = data_ptr.wrapping_sub(32); // 32-byte AllocationHeader
        if state.cbgr_allocations.contains(&header_addr) {
            match method {
                // AllocationHeader layout: [size:4][align:4][generation:4][epoch:2][caps:2][type_id:4][flags:4][reserved:8]
                "generation" | "stored_generation" => {
                    let generation = unsafe { *((header_addr + 8) as *const u32) };
                    return Ok(Some(Value::from_i64(generation as i64)));
                }
                "header_generation" => {
                    let generation = unsafe { *((header_addr + 8) as *const u32) };
                    return Ok(Some(Value::from_i64(generation as i64)));
                }
                "header_size" => {
                    let size = unsafe { *(header_addr as *const u32) };
                    return Ok(Some(Value::from_i64(size as i64)));
                }
                "header_epoch" => {
                    let epoch = unsafe { *((header_addr + 12) as *const u16) };
                    return Ok(Some(Value::from_i64(epoch as i64)));
                }
                "is_allocated" => {
                    let flags = unsafe { *((header_addr + 20) as *const u32) };
                    return Ok(Some(Value::from_bool(flags & 1 == 0)));
                }
                "is_freed" => {
                    let flags = unsafe { *((header_addr + 20) as *const u32) };
                    return Ok(Some(Value::from_bool(flags & 1 != 0)));
                }
                "epoch" => {
                    // Return the reference creation epoch if tracked, else fallback to header
                    let epoch = state.cbgr_ref_creation_epoch.get(&data_ptr)
                        .map(|&e| e as u32)
                        .unwrap_or_else(|| unsafe { *((header_addr + 12) as *const u16) as u32 });
                    return Ok(Some(Value::from_i64(epoch as i64)));
                }
                "is_epoch_valid" => {
                    let ref_epoch = state.cbgr_ref_creation_epoch.get(&data_ptr)
                        .map(|&e| e as u32)
                        .unwrap_or_else(|| unsafe { *((header_addr + 12) as *const u16) as u32 });
                    let current = state.cbgr_epoch as u32;
                    let diff = current.wrapping_sub(ref_epoch);
                    return Ok(Some(Value::from_bool(diff < 1_000_000)));
                }
                "capabilities" | "epoch_caps" => {
                    let ref_epoch = state.cbgr_ref_creation_epoch.get(&data_ptr)
                        .map(|&e| e as u32)
                        .unwrap_or_else(|| unsafe { *((header_addr + 12) as *const u16) as u32 });
                    let is_mut = state.cbgr_mutable_ptrs.contains(&data_ptr);
                    let cap_bits: u32 = if is_mut { 0x03 } else { 0x01 };
                    let packed = ((ref_epoch & 0x00FF_FFFF) << 8) | cap_bits;
                    return Ok(Some(Value::from_i64(packed as i64)));
                }
                "raw_ptr" => {
                    return Ok(Some(Value::from_i64(data_ptr as i64)));
                }
                "epoch_caps_raw" | "raw_epoch_caps" => {
                    let ref_epoch = state.cbgr_ref_creation_epoch.get(&data_ptr)
                        .map(|&e| e as u32)
                        .unwrap_or_else(|| unsafe { *((header_addr + 12) as *const u16) as u32 });
                    let is_mut = state.cbgr_mutable_ptrs.contains(&data_ptr);
                    let cap_bits: u32 = if is_mut { 0x03 } else { 0x01 };
                    let packed = ((ref_epoch & 0x00FF_FFFF) << 8) | cap_bits;
                    return Ok(Some(Value::from_i64(packed as i64)));
                }
                "can_read" => {
                    return Ok(Some(Value::from_bool(true)));
                }
                "can_write" => {
                    let is_mut = state.cbgr_mutable_ptrs.contains(&data_ptr);
                    return Ok(Some(Value::from_bool(is_mut)));
                }
                _ => {}
            }
        }
    }

    // Universal methods available on all primitive types
    if method == "clone" {
        // Check if this is a variant data pointer (from ref pattern in match)
        // If so, dereference it to get the actual value to clone
        if receiver.is_ptr() && !receiver.is_nil() {
            let ptr_addr = receiver.as_ptr::<u8>() as usize;
            if state.cbgr_mutable_ptrs.contains(&ptr_addr) {
                // This is a pointer to a Value (from GetVariantDataRef)
                // Read the actual value and return it (clone for Copy types)
                let actual_value = unsafe { *(ptr_addr as *const Value) };
                return Ok(Some(actual_value));
            }
        }
        // All primitives are Copy — clone returns the value itself
        return Ok(Some(*receiver));
    }
    // `to_text` is the Verum-native spelling (`Text` instead of `String`);
    // route through the same allocator as `to_string` so every primitive
    // reports a real `Text` value instead of hitting the generic method-
    // not-found fall-through.
    if method == "to_string" || method == "to_text" {
        let string_repr = format_value_for_print(state, *receiver);
        if let Some(small_str_value) = Value::from_small_string(&string_repr) {
            return Ok(Some(small_str_value));
        } else {
            let bytes = string_repr.as_bytes();
            let len = bytes.len();
            let alloc_size = 8 + len;
            let obj = state.heap.alloc(crate::types::TypeId(0x0001), alloc_size)?;
            state.record_allocation();
            let base_ptr = obj.as_ptr() as *mut u8;
            unsafe {
                let data_offset = heap::OBJECT_HEADER_SIZE;
                let len_ptr = base_ptr.add(data_offset) as *mut u64;
                *len_ptr = len as u64;
                let bytes_ptr = base_ptr.add(data_offset + 8);
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), bytes_ptr, len);
            }
            return Ok(Some(Value::from_ptr(obj.as_ptr() as *mut u8)));
        }
    }

    // Int methods
    if receiver.is_int() {
        let v = receiver.as_i64();
        let result = match method {
            "abs" => Value::from_i64(v.abs()),
            "signum" => Value::from_i64(v.signum()),
            "is_positive" => Value::from_bool(v > 0),
            "is_negative" => Value::from_bool(v < 0),
            "is_zero" => Value::from_bool(v == 0),
            "min" => {
                let other = state.get_reg(Reg(args.start.0)).as_i64();
                Value::from_i64(v.min(other))
            }
            "max" => {
                let other = state.get_reg(Reg(args.start.0)).as_i64();
                Value::from_i64(v.max(other))
            }
            "clamp" => {
                let lo = state.get_reg(Reg(args.start.0)).as_i64();
                let hi = state.get_reg(Reg(args.start.0 + 1)).as_i64();
                Value::from_i64(v.clamp(lo, hi))
            }
            "pow" => {
                let exp = state.get_reg(Reg(args.start.0)).as_i64();
                Value::from_i64(v.pow(exp as u32))
            }
            "checked_add" => {
                let other = state.get_reg(Reg(args.start.0)).as_i64();
                return Ok(Some(make_maybe_int(state, v.checked_add(other))?));
            }
            "checked_sub" => {
                let other = state.get_reg(Reg(args.start.0)).as_i64();
                return Ok(Some(make_maybe_int(state, v.checked_sub(other))?));
            }
            "checked_mul" => {
                let other = state.get_reg(Reg(args.start.0)).as_i64();
                return Ok(Some(make_maybe_int(state, v.checked_mul(other))?));
            }
            "checked_div" => {
                let other = state.get_reg(Reg(args.start.0)).as_i64();
                return Ok(Some(make_maybe_int(state, v.checked_div(other))?));
            }
            // UInt64-specific checked arithmetic (uses unsigned overflow detection)
            "checked_add_u64" => {
                let other = state.get_reg(Reg(args.start.0)).as_i64() as u64;
                let result = (v as u64).checked_add(other).map(|r| r as i64);
                return Ok(Some(make_maybe_int(state, result)?));
            }
            "checked_sub_u64" => {
                let other = state.get_reg(Reg(args.start.0)).as_i64() as u64;
                let result = (v as u64).checked_sub(other).map(|r| r as i64);
                return Ok(Some(make_maybe_int(state, result)?));
            }
            "checked_mul_u64" => {
                let other = state.get_reg(Reg(args.start.0)).as_i64() as u64;
                let result = (v as u64).checked_mul(other).map(|r| r as i64);
                return Ok(Some(make_maybe_int(state, result)?));
            }
            // Byte-width (u8) arithmetic methods — emitted by codegen for Byte-typed variables
            "byte$saturating_add" => {
                let other = state.get_reg(Reg(args.start.0)).as_i64();
                Value::from_i64((v as u8).saturating_add(other as u8) as i64)
            }
            "byte$saturating_sub" => {
                let other = state.get_reg(Reg(args.start.0)).as_i64();
                Value::from_i64((v as u8).saturating_sub(other as u8) as i64)
            }
            "byte$wrapping_add" => {
                let other = state.get_reg(Reg(args.start.0)).as_i64();
                Value::from_i64((v as u8).wrapping_add(other as u8) as i64)
            }
            "byte$wrapping_sub" => {
                let other = state.get_reg(Reg(args.start.0)).as_i64();
                Value::from_i64((v as u8).wrapping_sub(other as u8) as i64)
            }
            "byte$wrapping_mul" => {
                let other = state.get_reg(Reg(args.start.0)).as_i64();
                Value::from_i64((v as u8).wrapping_mul(other as u8) as i64)
            }
            "byte$checked_add" => {
                let other = state.get_reg(Reg(args.start.0)).as_i64();
                return Ok(Some(make_maybe_int(state, (v as u8).checked_add(other as u8).map(|r| r as i64))?));
            }
            "byte$checked_sub" => {
                let other = state.get_reg(Reg(args.start.0)).as_i64();
                return Ok(Some(make_maybe_int(state, (v as u8).checked_sub(other as u8).map(|r| r as i64))?));
            }
            "byte$checked_mul" => {
                let other = state.get_reg(Reg(args.start.0)).as_i64();
                return Ok(Some(make_maybe_int(state, (v as u8).checked_mul(other as u8).map(|r| r as i64))?));
            }
            "saturating_add" => {
                let other = state.get_reg(Reg(args.start.0)).as_i64();
                Value::from_i64(v.saturating_add(other))
            }
            "saturating_sub" => {
                let other = state.get_reg(Reg(args.start.0)).as_i64();
                Value::from_i64(v.saturating_sub(other))
            }
            "saturating_mul" => {
                let other = state.get_reg(Reg(args.start.0)).as_i64();
                Value::from_i64(v.saturating_mul(other))
            }
            "wrapping_add" => {
                let other = state.get_reg(Reg(args.start.0)).as_i64();
                Value::from_i64(v.wrapping_add(other))
            }
            "wrapping_sub" => {
                let other = state.get_reg(Reg(args.start.0)).as_i64();
                Value::from_i64(v.wrapping_sub(other))
            }
            "wrapping_mul" => {
                let other = state.get_reg(Reg(args.start.0)).as_i64();
                Value::from_i64(v.wrapping_mul(other))
            }
            "to_float" | "to_f64" => Value::from_f64(v as f64),
            "to_int" => Value::from_i64(v), // identity
            "count_ones" => Value::from_i64(v.count_ones() as i64),
            "count_zeros" => Value::from_i64(v.count_zeros() as i64),
            "leading_zeros" => Value::from_i64(v.leading_zeros() as i64),
            "trailing_zeros" => Value::from_i64(v.trailing_zeros() as i64),
            "reverse_bits" => Value::from_i64(v.reverse_bits()),
            "swap_bytes" => Value::from_i64(v.swap_bytes()),
            "rotate_left" => {
                let n = state.get_reg(Reg(args.start.0)).as_i64();
                Value::from_i64(v.rotate_left(n as u32))
            }
            "rotate_right" => {
                let n = state.get_reg(Reg(args.start.0)).as_i64();
                Value::from_i64(v.rotate_right(n as u32))
            }
            "in_range" => {
                let lo = state.get_reg(Reg(args.start.0)).as_i64();
                let hi = state.get_reg(Reg(args.start.0 + 1)).as_i64();
                Value::from_bool(v >= lo && v < hi)
            }
            "max_value" => Value::from_i64(i64::MAX),
            "min_value" => Value::from_i64(i64::MIN),
            // CBGR epoch_caps bit inspection methods (packed capability integer)
            // Encoding: ((epoch & 0x00FF_FFFF) << 8) | capabilities
            // Bit 0 = read capability (0x01), Bit 1 = write capability (0x02)
            "can_read" => Value::from_bool((v & 0x01) != 0),
            "can_write" => Value::from_bool((v & 0x02) != 0),
            "can_extend" => Value::from_bool((v & 0x04) != 0),
            "is_unique" => Value::from_bool((v & 0x08) != 0),
            "epoch" => Value::from_i64((v >> 8) & 0x00FF_FFFF), // Extract epoch from packed value
            "raw" => Value::from_i64(v), // Identity: return raw integer value
            "to_hex_string" => {
                let s = format!("{:x}", v);
                return Ok(Some(alloc_string_value(state, &s)?));
            }
            "to_binary_string" => {
                let s = format!("{:b}", v);
                return Ok(Some(alloc_string_value(state, &s)?));
            }
            "to_octal_string" => {
                let s = format!("{:o}", v);
                return Ok(Some(alloc_string_value(state, &s)?));
            }
            // Byte/ASCII methods (operate on the low 8 bits as a u8 value)
            "is_ascii_alphabetic" => Value::from_bool((v as u8).is_ascii_alphabetic()),
            "is_ascii_alphanumeric" => Value::from_bool((v as u8).is_ascii_alphanumeric()),
            "is_ascii_digit" => Value::from_bool((v as u8).is_ascii_digit()),
            "is_ascii_whitespace" => Value::from_bool((v as u8).is_ascii_whitespace()),
            "is_ascii_lowercase" => Value::from_bool((v as u8).is_ascii_lowercase()),
            "is_ascii_uppercase" => Value::from_bool((v as u8).is_ascii_uppercase()),
            "is_ascii_control" => Value::from_bool((v as u8).is_ascii_control()),
            "is_ascii_punctuation" => Value::from_bool((v as u8).is_ascii_punctuation()),
            "is_ascii_graphic" => Value::from_bool((v as u8).is_ascii_graphic()),
            "is_ascii_hexdigit" => Value::from_bool((v as u8).is_ascii_hexdigit()),
            "is_ascii" => Value::from_bool((0..=127).contains(&v)),
            "to_ascii_lowercase" => Value::from_i64((v as u8).to_ascii_lowercase() as i64),
            "to_ascii_uppercase" => Value::from_i64((v as u8).to_ascii_uppercase() as i64),
            // Byte-prefixed ASCII methods (codegen emits byte$ prefix for Byte-typed vars)
            "byte$is_ascii_alphabetic" => Value::from_bool((v as u8).is_ascii_alphabetic()),
            "byte$is_ascii_alphanumeric" => Value::from_bool((v as u8).is_ascii_alphanumeric()),
            "byte$is_ascii_digit" => Value::from_bool((v as u8).is_ascii_digit()),
            "byte$is_ascii_whitespace" => Value::from_bool((v as u8).is_ascii_whitespace()),
            "byte$is_ascii_lowercase" => Value::from_bool((v as u8).is_ascii_lowercase()),
            "byte$is_ascii_uppercase" => Value::from_bool((v as u8).is_ascii_uppercase()),
            "byte$is_ascii_control" => Value::from_bool((v as u8).is_ascii_control()),
            "byte$is_ascii_punctuation" => Value::from_bool((v as u8).is_ascii_punctuation()),
            "byte$is_ascii_graphic" => Value::from_bool((v as u8).is_ascii_graphic()),
            "byte$is_ascii_hexdigit" => Value::from_bool((v as u8).is_ascii_hexdigit()),
            "byte$is_ascii" => Value::from_bool((0..=127).contains(&v)),
            "byte$to_ascii_lowercase" => Value::from_i64((v as u8).to_ascii_lowercase() as i64),
            "byte$to_ascii_uppercase" => Value::from_i64((v as u8).to_ascii_uppercase() as i64),
            "byte$to_int" => Value::from_i64(v),  // Byte -> Int conversion
            // Char (Unicode) methods — chars stored as i64 codepoints
            "is_alphabetic" => {
                if let Some(c) = char::from_u32(v as u32) { Value::from_bool(c.is_alphabetic()) }
                else { Value::from_bool(false) }
            }
            "is_numeric" => {
                if let Some(c) = char::from_u32(v as u32) { Value::from_bool(c.is_numeric()) }
                else { Value::from_bool(false) }
            }
            "is_alphanumeric" => {
                if let Some(c) = char::from_u32(v as u32) { Value::from_bool(c.is_alphanumeric()) }
                else { Value::from_bool(false) }
            }
            "is_whitespace" => {
                if let Some(c) = char::from_u32(v as u32) { Value::from_bool(c.is_whitespace()) }
                else { Value::from_bool(false) }
            }
            "is_uppercase" => {
                if let Some(c) = char::from_u32(v as u32) { Value::from_bool(c.is_uppercase()) }
                else { Value::from_bool(false) }
            }
            "is_lowercase" => {
                if let Some(c) = char::from_u32(v as u32) { Value::from_bool(c.is_lowercase()) }
                else { Value::from_bool(false) }
            }
            "is_control" => {
                if let Some(c) = char::from_u32(v as u32) { Value::from_bool(c.is_control()) }
                else { Value::from_bool(false) }
            }
            "to_uppercase" => {
                if let Some(c) = char::from_u32(v as u32) {
                    // Return first char of uppercase mapping
                    let upper: char = c.to_uppercase().next().unwrap_or(c);
                    Value::from_i64(upper as i64)
                } else { Value::from_i64(v) }
            }
            "to_lowercase" => {
                if let Some(c) = char::from_u32(v as u32) {
                    let lower: char = c.to_lowercase().next().unwrap_or(c);
                    Value::from_i64(lower as i64)
                } else { Value::from_i64(v) }
            }
            "to_digit" => {
                let radix = state.get_reg(Reg(args.start.0)).as_i64() as u32;
                let digit_opt = char::from_u32(v as u32).and_then(|c| c.to_digit(radix)).map(|d| d as i64);
                return Ok(Some(make_maybe_int(state, digit_opt)?));
            }
            "from_digit" => {
                // Char.from_digit(digit, radix) — static-style, receiver ignored
                let digit = state.get_reg(Reg(args.start.0)).as_i64() as u32;
                let radix = state.get_reg(Reg(args.start.0 + 1)).as_i64() as u32;
                let ch_opt = char::from_digit(digit, radix).map(|c| {
                    // Verum convention: hex digits are uppercase (A-F, not a-f)
                    if c.is_ascii_lowercase() { c.to_ascii_uppercase() as i64 } else { c as i64 }
                });
                return Ok(Some(make_maybe_int(state, ch_opt)?));
            }
            "len_utf8" => {
                if let Some(c) = char::from_u32(v as u32) {
                    Value::from_i64(c.len_utf8() as i64)
                } else { Value::from_i64(0) }
            }
            "len_utf16" => {
                if let Some(c) = char::from_u32(v as u32) {
                    Value::from_i64(c.len_utf16() as i64)
                } else { Value::from_i64(0) }
            }
            // Byte conversion methods
            "to_le_bytes" => {
                let bytes = v.to_le_bytes();
                let vals: Vec<Value> = bytes.iter().map(|&b| Value::from_i64(b as i64)).collect();
                return Ok(Some(alloc_list_from_values(state, vals)?));
            }
            "to_be_bytes" => {
                let bytes = v.to_be_bytes();
                let vals: Vec<Value> = bytes.iter().map(|&b| Value::from_i64(b as i64)).collect();
                return Ok(Some(alloc_list_from_values(state, vals)?));
            }
            "from_le_bytes" | "from_be_bytes" => {
                // Static-style: Int.from_le_bytes(bytes_list)
                let list_val = state.get_reg(Reg(args.start.0));
                let list_ptr = list_val.as_ptr::<u8>();
                let list_header = unsafe { &*(list_ptr as *const heap::ObjectHeader) };
                let mut byte_arr = [0u8; 8];
                for (i, byte) in byte_arr.iter_mut().enumerate() {
                    let elem = get_array_element(list_ptr, list_header, i)?;
                    *byte = elem.as_i64() as u8;
                }
                if method == "from_le_bytes" {
                    Value::from_i64(i64::from_le_bytes(byte_arr))
                } else {
                    Value::from_i64(i64::from_be_bytes(byte_arr))
                }
            }
            // ── Int32 (i32-width) methods ──
            "int32$abs" => Value::from_i64((v as i32).wrapping_abs() as i64),
            "int32$signum" => Value::from_i64((v as i32).signum() as i64),
            "int32$checked_add" => {
                let rhs = state.get_reg(Reg(args.start.0)).as_i64() as i32;
                let result = (v as i32).checked_add(rhs).map(|r| r as i64);
                return Ok(Some(make_maybe_int(state, result)?));
            }
            "int32$checked_sub" => {
                let rhs = state.get_reg(Reg(args.start.0)).as_i64() as i32;
                let result = (v as i32).checked_sub(rhs).map(|r| r as i64);
                return Ok(Some(make_maybe_int(state, result)?));
            }
            "int32$checked_mul" => {
                let rhs = state.get_reg(Reg(args.start.0)).as_i64() as i32;
                let result = (v as i32).checked_mul(rhs).map(|r| r as i64);
                return Ok(Some(make_maybe_int(state, result)?));
            }
            "int32$wrapping_add" => {
                let rhs = state.get_reg(Reg(args.start.0)).as_i64() as i32;
                Value::from_i64((v as i32).wrapping_add(rhs) as i64)
            }
            "int32$wrapping_sub" => {
                let rhs = state.get_reg(Reg(args.start.0)).as_i64() as i32;
                Value::from_i64((v as i32).wrapping_sub(rhs) as i64)
            }
            "int32$wrapping_mul" => {
                let rhs = state.get_reg(Reg(args.start.0)).as_i64() as i32;
                Value::from_i64((v as i32).wrapping_mul(rhs) as i64)
            }
            "int32$saturating_add" => {
                let rhs = state.get_reg(Reg(args.start.0)).as_i64() as i32;
                Value::from_i64((v as i32).saturating_add(rhs) as i64)
            }
            "int32$saturating_sub" => {
                let rhs = state.get_reg(Reg(args.start.0)).as_i64() as i32;
                Value::from_i64((v as i32).saturating_sub(rhs) as i64)
            }
            "int32$leading_zeros" => Value::from_i64((v as i32 as u32).leading_zeros() as i64),
            "int32$trailing_zeros" => Value::from_i64((v as i32 as u32).trailing_zeros() as i64),
            "int32$count_ones" => Value::from_i64((v as i32 as u32).count_ones() as i64),
            "int32$rotate_left" => {
                let n = state.get_reg(Reg(args.start.0)).as_i64() as u32;
                Value::from_i64((v as u32).rotate_left(n) as i32 as i64)
            }
            "int32$rotate_right" => {
                let n = state.get_reg(Reg(args.start.0)).as_i64() as u32;
                Value::from_i64((v as u32).rotate_right(n) as i32 as i64)
            }
            "int32$swap_bytes" => Value::from_i64((v as i32).swap_bytes() as i64),
            "int32$to_int" => Value::from_i64(v as i32 as i64),
            "int32$MAX" => Value::from_i64(i32::MAX as i64),
            "int32$MIN" => Value::from_i64(i32::MIN as i64),
            "int32$to_le_bytes" => {
                let bytes = (v as i32).to_le_bytes();
                let vals: Vec<Value> = bytes.iter().map(|&b| Value::from_i64(b as i64)).collect();
                return Ok(Some(alloc_list_from_values(state, vals)?));
            }
            "int32$to_be_bytes" => {
                let bytes = (v as i32).to_be_bytes();
                let vals: Vec<Value> = bytes.iter().map(|&b| Value::from_i64(b as i64)).collect();
                return Ok(Some(alloc_list_from_values(state, vals)?));
            }
            "int32$from_le_bytes" | "int32$from_be_bytes" => {
                let list_val = state.get_reg(Reg(args.start.0));
                let list_ptr = list_val.as_ptr::<u8>();
                let list_header = unsafe { &*(list_ptr as *const heap::ObjectHeader) };
                let mut byte_arr = [0u8; 4];
                for (i, byte) in byte_arr.iter_mut().enumerate() {
                    let elem = get_array_element(list_ptr, list_header, i)?;
                    *byte = elem.as_i64() as u8;
                }
                if method == "int32$from_le_bytes" {
                    Value::from_i64(i32::from_le_bytes(byte_arr) as i64)
                } else {
                    Value::from_i64(i32::from_be_bytes(byte_arr) as i64)
                }
            }

            // ── UInt64 (u64-width) methods ──
            "uint64$checked_add" => {
                let rhs = state.get_reg(Reg(args.start.0)).as_i64() as u64;
                let result = (v as u64).checked_add(rhs).map(|r| r as i64);
                return Ok(Some(make_maybe_int(state, result)?));
            }
            "uint64$checked_sub" => {
                let rhs = state.get_reg(Reg(args.start.0)).as_i64() as u64;
                let result = (v as u64).checked_sub(rhs).map(|r| r as i64);
                return Ok(Some(make_maybe_int(state, result)?));
            }
            "uint64$checked_mul" => {
                let rhs = state.get_reg(Reg(args.start.0)).as_i64() as u64;
                let result = (v as u64).checked_mul(rhs).map(|r| r as i64);
                return Ok(Some(make_maybe_int(state, result)?));
            }
            "uint64$wrapping_add" => {
                let rhs = state.get_reg(Reg(args.start.0)).as_i64() as u64;
                Value::from_i64((v as u64).wrapping_add(rhs) as i64)
            }
            "uint64$wrapping_sub" => {
                let rhs = state.get_reg(Reg(args.start.0)).as_i64() as u64;
                Value::from_i64((v as u64).wrapping_sub(rhs) as i64)
            }
            "uint64$saturating_add" => {
                let rhs = state.get_reg(Reg(args.start.0)).as_i64() as u64;
                Value::from_i64((v as u64).saturating_add(rhs) as i64)
            }
            "uint64$saturating_sub" => {
                let rhs = state.get_reg(Reg(args.start.0)).as_i64() as u64;
                Value::from_i64((v as u64).saturating_sub(rhs) as i64)
            }
            "uint64$leading_zeros" => Value::from_i64((v as u64).leading_zeros() as i64),
            "uint64$trailing_zeros" => Value::from_i64((v as u64).trailing_zeros() as i64),
            "uint64$count_ones" => Value::from_i64((v as u64).count_ones() as i64),
            "uint64$rotate_left" => {
                let n = state.get_reg(Reg(args.start.0)).as_i64() as u32;
                Value::from_i64((v as u64).rotate_left(n) as i64)
            }
            "uint64$rotate_right" => {
                let n = state.get_reg(Reg(args.start.0)).as_i64() as u32;
                Value::from_i64((v as u64).rotate_right(n) as i64)
            }
            "uint64$swap_bytes" => Value::from_i64((v as u64).swap_bytes() as i64),
            "uint64$to_int" => Value::from_i64(v),
            "uint64$MAX" => Value::from_i64(u64::MAX as i64),
            "uint64$MIN" => Value::from_i64(u64::MIN as i64),
            "uint64$to_le_bytes" => {
                let bytes = (v as u64).to_le_bytes();
                let vals: Vec<Value> = bytes.iter().map(|&b| Value::from_i64(b as i64)).collect();
                return Ok(Some(alloc_list_from_values(state, vals)?));
            }
            "uint64$to_be_bytes" => {
                let bytes = (v as u64).to_be_bytes();
                let vals: Vec<Value> = bytes.iter().map(|&b| Value::from_i64(b as i64)).collect();
                return Ok(Some(alloc_list_from_values(state, vals)?));
            }
            "uint64$from_le_bytes" | "uint64$from_be_bytes" => {
                let list_val = state.get_reg(Reg(args.start.0));
                let list_ptr = list_val.as_ptr::<u8>();
                let list_header = unsafe { &*(list_ptr as *const heap::ObjectHeader) };
                let mut byte_arr = [0u8; 8];
                for (i, byte) in byte_arr.iter_mut().enumerate() {
                    let elem = get_array_element(list_ptr, list_header, i)?;
                    *byte = elem.as_i64() as u8;
                }
                if method == "uint64$from_le_bytes" {
                    Value::from_i64(u64::from_le_bytes(byte_arr) as i64)
                } else {
                    Value::from_i64(u64::from_be_bytes(byte_arr) as i64)
                }
            }

            // ── UInt32 (u32-width) methods ──
            "uint32$to_le_bytes" => {
                let bytes = (v as u32).to_le_bytes();
                let vals: Vec<Value> = bytes.iter().map(|&b| Value::from_i64(b as i64)).collect();
                return Ok(Some(alloc_list_from_values(state, vals)?));
            }
            "uint32$to_be_bytes" => {
                let bytes = (v as u32).to_be_bytes();
                let vals: Vec<Value> = bytes.iter().map(|&b| Value::from_i64(b as i64)).collect();
                return Ok(Some(alloc_list_from_values(state, vals)?));
            }
            "uint32$from_le_bytes" | "uint32$from_be_bytes" => {
                let list_val = state.get_reg(Reg(args.start.0));
                let list_ptr = list_val.as_ptr::<u8>();
                let list_header = unsafe { &*(list_ptr as *const heap::ObjectHeader) };
                let mut byte_arr = [0u8; 4];
                for (i, byte) in byte_arr.iter_mut().enumerate() {
                    let elem = get_array_element(list_ptr, list_header, i)?;
                    *byte = elem.as_i64() as u8;
                }
                if method == "uint32$from_le_bytes" {
                    Value::from_i64(u32::from_le_bytes(byte_arr) as i64)
                } else {
                    Value::from_i64(u32::from_be_bytes(byte_arr) as i64)
                }
            }

            // ── UInt16 (u16-width) methods ──
            "uint16$to_le_bytes" => {
                let bytes = (v as u16).to_le_bytes();
                let vals: Vec<Value> = bytes.iter().map(|&b| Value::from_i64(b as i64)).collect();
                return Ok(Some(alloc_list_from_values(state, vals)?));
            }
            "uint16$to_be_bytes" => {
                let bytes = (v as u16).to_be_bytes();
                let vals: Vec<Value> = bytes.iter().map(|&b| Value::from_i64(b as i64)).collect();
                return Ok(Some(alloc_list_from_values(state, vals)?));
            }
            "uint16$from_le_bytes" | "uint16$from_be_bytes" => {
                let list_val = state.get_reg(Reg(args.start.0));
                let list_ptr = list_val.as_ptr::<u8>();
                let list_header = unsafe { &*(list_ptr as *const heap::ObjectHeader) };
                let mut byte_arr = [0u8; 2];
                for (i, byte) in byte_arr.iter_mut().enumerate() {
                    let elem = get_array_element(list_ptr, list_header, i)?;
                    *byte = elem.as_i64() as u8;
                }
                if method == "uint16$from_le_bytes" {
                    Value::from_i64(u16::from_le_bytes(byte_arr) as i64)
                } else {
                    Value::from_i64(u16::from_be_bytes(byte_arr) as i64)
                }
            }

            // Duration methods (Duration stored as nanoseconds in Int)
            "as_secs" => Value::from_i64(v / 1_000_000_000),
            "as_millis" => Value::from_i64(v / 1_000_000),
            "as_micros" => Value::from_i64(v / 1_000),
            "as_nanos" => Value::from_i64(v),
            "subsec_nanos" => Value::from_i64(v % 1_000_000_000),
            "add" => {
                let other = state.get_reg(Reg(args.start.0)).as_i64();
                Value::from_i64(v + other)
            }

            // Instant methods (Instant stored as nanoseconds since epoch in Int)
            "duration_since" => {
                let other = state.get_reg(Reg(args.start.0)).as_i64();
                Value::from_i64((v - other).max(0))
            }
            "elapsed" => {
                Value::from_i64((monotonic_nanos_shared() - v).max(0))
            }

            // PerfCounter methods
            "elapsed_since" => {
                let other = state.get_reg(Reg(args.start.0)).as_i64();
                Value::from_i64((v - other).max(0))
            }

            // Ordering comparison method (Ord protocol)
            "cmp" => {
                let other_val = state.get_reg(Reg(args.start.0));
                // Handle reference: the argument might be &Int (CBGR ref), need to deref
                // Check CBGR ref FIRST since CBGR refs also pass is_int()
                let other = if is_cbgr_ref(&other_val) {
                    let (abs_index, _) = decode_cbgr_ref(other_val.as_i64());
                    state.registers.get_absolute(abs_index).as_i64()
                } else {
                    other_val.as_i64()
                };
                return Ok(Some(make_ordering(state, v.cmp(&other))?));
            }

            // Eq protocol methods - handle eq/ne directly for primitives to avoid
            // incorrect dispatch to Maybe.eq or other imported type's eq methods
            "eq" => {
                let other_val = state.get_reg(Reg(args.start.0));
                // Handle CBGR reference: the argument might be &Int, need to deref
                let other = if is_cbgr_ref(&other_val) {
                    let (abs_index, _) = decode_cbgr_ref(other_val.as_i64());
                    state.registers.get_absolute(abs_index).as_i64()
                } else {
                    other_val.as_i64()
                };
                Value::from_bool(v == other)
            }
            "ne" => {
                let other_val = state.get_reg(Reg(args.start.0));
                // Handle CBGR reference: the argument might be &Int, need to deref
                let other = if is_cbgr_ref(&other_val) {
                    let (abs_index, _) = decode_cbgr_ref(other_val.as_i64());
                    state.registers.get_absolute(abs_index).as_i64()
                } else {
                    other_val.as_i64()
                };
                Value::from_bool(v != other)
            }
            // Ord protocol comparison methods - handle directly to avoid incorrect dispatch
            "lt" => {
                let other_val = state.get_reg(Reg(args.start.0));
                let other = if is_cbgr_ref(&other_val) {
                    let (abs_index, _) = decode_cbgr_ref(other_val.as_i64());
                    state.registers.get_absolute(abs_index).as_i64()
                } else {
                    other_val.as_i64()
                };
                Value::from_bool(v < other)
            }
            "le" => {
                let other_val = state.get_reg(Reg(args.start.0));
                let other = if is_cbgr_ref(&other_val) {
                    let (abs_index, _) = decode_cbgr_ref(other_val.as_i64());
                    state.registers.get_absolute(abs_index).as_i64()
                } else {
                    other_val.as_i64()
                };
                Value::from_bool(v <= other)
            }
            "gt" => {
                let other_val = state.get_reg(Reg(args.start.0));
                let other = if is_cbgr_ref(&other_val) {
                    let (abs_index, _) = decode_cbgr_ref(other_val.as_i64());
                    state.registers.get_absolute(abs_index).as_i64()
                } else {
                    other_val.as_i64()
                };
                Value::from_bool(v > other)
            }
            "ge" => {
                let other_val = state.get_reg(Reg(args.start.0));
                let other = if is_cbgr_ref(&other_val) {
                    let (abs_index, _) = decode_cbgr_ref(other_val.as_i64());
                    state.registers.get_absolute(abs_index).as_i64()
                } else {
                    other_val.as_i64()
                };
                Value::from_bool(v >= other)
            }

            _ => return Ok(None),
        };
        return Ok(Some(result));
    }

    // Float methods (including NaN which is stored with TAG_NAN)
    if let Some(v) = receiver.try_as_f64() {
        let result = match method {
            "abs" => Value::from_f64(v.abs()),
            "ceil" => Value::from_f64(v.ceil()),
            "floor" => Value::from_f64(v.floor()),
            "round" => Value::from_f64(v.round()),
            "trunc" => Value::from_f64(v.trunc()),
            "fract" => Value::from_f64(v.fract()),
            "sqrt" => Value::from_f64(v.sqrt()),
            "cbrt" => Value::from_f64(v.cbrt()),
            "sin" => Value::from_f64(v.sin()),
            "cos" => Value::from_f64(v.cos()),
            "tan" => Value::from_f64(v.tan()),
            "asin" => Value::from_f64(v.asin()),
            "acos" => Value::from_f64(v.acos()),
            "atan" => Value::from_f64(v.atan()),
            "atan2" => {
                let other = state.get_reg(Reg(args.start.0)).as_f64();
                Value::from_f64(v.atan2(other))
            }
            "ln" => Value::from_f64(v.ln()),
            "log2" => Value::from_f64(v.log2()),
            "log10" => Value::from_f64(v.log10()),
            "log" => {
                let base = state.get_reg(Reg(args.start.0)).as_f64();
                Value::from_f64(v.log(base))
            }
            "exp" => Value::from_f64(v.exp()),
            "exp2" => Value::from_f64(v.exp2()),
            "signum" => {
                // Verum semantics: NaN→NaN, >0→1.0, <0→-1.0, 0→0.0
                if v.is_nan() { Value::from_f64(f64::NAN) }
                else if v > 0.0 { Value::from_f64(1.0) }
                else if v < 0.0 { Value::from_f64(-1.0) }
                else { Value::from_f64(0.0) }
            }
            "is_nan" => Value::from_bool(v.is_nan()),
            "is_infinite" => Value::from_bool(v.is_infinite()),
            "is_finite" => Value::from_bool(v.is_finite()),
            "is_positive" => Value::from_bool(v > 0.0),
            "is_negative" => Value::from_bool(v < 0.0),
            "is_zero" => Value::from_bool(v == 0.0),
            "to_int" | "to_i64" => Value::from_i64(v as i64),
            "to_degrees" => Value::from_f64(v.to_degrees()),
            "to_radians" => Value::from_f64(v.to_radians()),
            "min" => {
                let other = state.get_reg(Reg(args.start.0)).as_f64();
                Value::from_f64(v.min(other))
            }
            "max" => {
                let other = state.get_reg(Reg(args.start.0)).as_f64();
                Value::from_f64(v.max(other))
            }
            "clamp" => {
                let lo = state.get_reg(Reg(args.start.0)).as_f64();
                let hi = state.get_reg(Reg(args.start.0 + 1)).as_f64();
                Value::from_f64(v.clamp(lo, hi))
            }
            "pow" | "powi" => {
                let exp = state.get_reg(Reg(args.start.0));
                if exp.is_int() {
                    Value::from_f64(v.powi(exp.as_i64() as i32))
                } else {
                    Value::from_f64(v.powf(exp.as_f64()))
                }
            }
            "hypot" => {
                let other = state.get_reg(Reg(args.start.0)).as_f64();
                Value::from_f64(v.hypot(other))
            }
            // Constants as methods
            "pi" => Value::from_f64(std::f64::consts::PI),
            "e" => Value::from_f64(std::f64::consts::E),
            "epsilon" => Value::from_f64(f64::EPSILON),
            "infinity" => Value::from_f64(f64::INFINITY),
            "neg_infinity" => Value::from_f64(f64::NEG_INFINITY),
            "nan" => Value::from_f64(f64::NAN),
            "max_value" => Value::from_f64(f64::MAX),
            "min_value" => Value::from_f64(f64::MIN),
            // Byte conversion methods
            "to_le_bytes" => {
                let bytes = v.to_le_bytes();
                let vals: Vec<Value> = bytes.iter().map(|&b| Value::from_i64(b as i64)).collect();
                return Ok(Some(alloc_list_from_values(state, vals)?));
            }
            "to_be_bytes" => {
                let bytes = v.to_be_bytes();
                let vals: Vec<Value> = bytes.iter().map(|&b| Value::from_i64(b as i64)).collect();
                return Ok(Some(alloc_list_from_values(state, vals)?));
            }
            "from_le_bytes" | "from_be_bytes" => {
                let list_val = state.get_reg(Reg(args.start.0));
                let list_ptr = list_val.as_ptr::<u8>();
                let list_header = unsafe { &*(list_ptr as *const heap::ObjectHeader) };
                let mut byte_arr = [0u8; 8];
                for (i, byte) in byte_arr.iter_mut().enumerate() {
                    let elem = get_array_element(list_ptr, list_header, i)?;
                    *byte = elem.as_i64() as u8;
                }
                if method == "from_le_bytes" {
                    Value::from_f64(f64::from_le_bytes(byte_arr))
                } else {
                    Value::from_f64(f64::from_be_bytes(byte_arr))
                }
            }
            // Eq protocol methods - handle eq/ne directly for Float primitives
            "eq" => {
                let other_val = state.get_reg(Reg(args.start.0));
                // Handle CBGR reference: the argument might be &Float, need to deref
                let other = if is_cbgr_ref(&other_val) {
                    let (abs_index, _) = decode_cbgr_ref(other_val.as_i64());
                    state.registers.get_absolute(abs_index).as_f64()
                } else {
                    other_val.as_f64()
                };
                // Use partial equality for floats (NaN != NaN)
                Value::from_bool(v == other)
            }
            "ne" => {
                let other_val = state.get_reg(Reg(args.start.0));
                // Handle CBGR reference: the argument might be &Float, need to deref
                let other = if is_cbgr_ref(&other_val) {
                    let (abs_index, _) = decode_cbgr_ref(other_val.as_i64());
                    state.registers.get_absolute(abs_index).as_f64()
                } else {
                    other_val.as_f64()
                };
                Value::from_bool(v != other)
            }
            _ => return Ok(None),
        };
        return Ok(Some(result));
    }

    // Array/List methods (non-closure: len, is_empty, contains, etc.)
    if receiver.is_ptr() && !receiver.is_nil() {
        // These are simple non-closure array methods handled here.
        // Higher-order methods (map, filter, fold) are in dispatch_array_method.
        let ptr = receiver.as_ptr::<u8>();
        let header = unsafe { &*(ptr as *const heap::ObjectHeader) };

        // Check for heap string (Text) type - these have special layout: [len: u64][bytes...]
        let is_heap_string = header.type_id == crate::types::TypeId::TEXT
            || header.type_id == crate::types::TypeId(0x0001);
        if is_heap_string {
            match method {
                "len" => {
                    // Heap string layout: [ObjectHeader][len: u64][bytes...]
                    let len_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *const u64 };
                    let len = unsafe { *len_ptr } as i64;
                    return Ok(Some(Value::from_i64(len)));
                }
                "is_empty" => {
                    let len_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *const u64 };
                    let len = unsafe { *len_ptr };
                    return Ok(Some(Value::from_bool(len == 0)));
                }
                _ => {} // Fall through to other method handling
            }
        }

        let is_value_array = header.type_id != TypeId::U8 && header.type_id != TypeId::LIST
            && header.type_id != TypeId::MAP && header.type_id != TypeId::SET
            && header.type_id != TypeId::DEQUE && header.type_id != TypeId::CHANNEL
            && !is_heap_string;
        let is_list = header.type_id == TypeId::LIST;

        // IMPORTANT: For user-defined types (type_id >= FIRST_USER but < 256),
        // skip the builtin array handlers. User-defined structs like core.collections.map.Map
        // have their own len()/is_empty() methods that read from struct fields, not from the
        // generic array memory layout. The type_id ranges are:
        // - 0-15: primitives
        // - 16-255: user-defined types (FIRST_USER to before meta types)
        // - 256-511: meta system types
        // - 512+: well-known collection types (LIST, MAP, SET, etc.)
        let is_user_defined_struct = header.type_id.0 >= crate::types::TypeId::FIRST_USER
            && header.type_id.0 < 256;

        if (is_value_array && !is_user_defined_struct) || is_list {
            match method {
                "len" => {
                    let len = get_array_length(ptr, header)?;
                    return Ok(Some(Value::from_i64(len as i64)));
                }
                "is_empty" => {
                    let len = get_array_length(ptr, header)?;
                    return Ok(Some(Value::from_bool(len == 0)));
                }
                "push" if is_list => {
                    let caller_base = state.reg_base();
                    let new_val = state.registers.get(caller_base, Reg(args.start.0));
                    // eprintln!("[DEBUG List::push] receiver={:?}, new_val={:?}", *receiver, new_val);
                    list_push(state, *receiver, new_val)?;
                    return Ok(Some(Value::unit()));
                }
                "contains" => {
                    let caller_base = state.reg_base();
                    let needle = state.registers.get(caller_base, Reg(args.start.0));
                    let len = get_array_length(ptr, header)?;
                    let mut found = false;
                    for i in 0..len {
                        let elem = get_array_element(ptr, header, i)?;
                        if elem.to_bits() == needle.to_bits() {
                            found = true;
                            break;
                        }
                    }
                    return Ok(Some(Value::from_bool(found)));
                }
                "iter" => {
                    // Create an iterator object for arrays/lists
                    // Iterator layout: [source_ptr, front_idx, back_idx, iter_type] - 4 values for double-ended
                    let iter_type = if is_list { ITER_TYPE_LIST } else { ITER_TYPE_ARRAY };
                    let len = get_array_length(ptr, header)?;

                    let iter_obj = state.heap.alloc(TypeId::UNIT, 4 * std::mem::size_of::<Value>())?;
                    state.record_allocation();

                    let iter_ptr = unsafe {
                        (iter_obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };

                    // Initialize iterator
                    unsafe {
                        *iter_ptr = *receiver;                         // source_ptr
                        *iter_ptr.add(1) = Value::from_i64(0);         // front_idx = 0
                        *iter_ptr.add(2) = Value::from_i64(len as i64); // back_idx = len (exclusive)
                        *iter_ptr.add(3) = Value::from_i64(iter_type); // iter_type
                    }

                    return Ok(Some(Value::from_ptr(iter_obj.as_ptr() as *mut u8)));
                }
                "iter_mut" => {
                    // Create a mutable iterator object for arrays/lists
                    let iter_type = if is_list { ITER_TYPE_LIST } else { ITER_TYPE_ARRAY };
                    let len = get_array_length(ptr, header)?;

                    let iter_obj = state.heap.alloc(TypeId::UNIT, 4 * std::mem::size_of::<Value>())?;
                    state.record_allocation();

                    let iter_ptr = unsafe {
                        (iter_obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };

                    // Initialize iterator (same as iter, mutability tracked by type system)
                    unsafe {
                        *iter_ptr = *receiver;
                        *iter_ptr.add(1) = Value::from_i64(0);
                        *iter_ptr.add(2) = Value::from_i64(len as i64);
                        *iter_ptr.add(3) = Value::from_i64(iter_type);
                    }

                    return Ok(Some(Value::from_ptr(iter_obj.as_ptr() as *mut u8)));
                }
                _ => {} // fall through
            }
        }

        // Iterator methods - detect iterator objects by checking for 4-value layout with iter_type
        // Iterator layout: [source_ptr, front_idx, back_idx, iter_type]
        if header.type_id == TypeId::UNIT && header.size as usize == 4 * std::mem::size_of::<Value>() {
            let iter_data = unsafe {
                ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
            };
            let iter_type = unsafe { (*iter_data.add(3)).as_i64() };

            // Strip the type prefix if present (e.g., "ListIter.fold" -> "fold") so that
            // user-defined iterator type prefixes dispatch to the builtin iterator methods.
            let iter_method = method.rsplit('.').next()
                .or_else(|| method.rsplit("::").next())
                .unwrap_or(method);

            // Check if this is a valid iterator type
            if (ITER_TYPE_LIST..=ITER_TYPE_RANGE).contains(&iter_type) {
                match iter_method {
                    "next" => {
                        let source_ptr = unsafe { (*iter_data).as_ptr::<u8>() };
                        let front_idx = unsafe { (*iter_data.add(1)).as_i64() } as usize;
                        let back_idx = unsafe { (*iter_data.add(2)).as_i64() } as usize;

                        // Check if iterator is exhausted (front meets or passes back)
                        if front_idx >= back_idx {
                            return Ok(Some(make_none_value(state)?));
                        }

                        // ITER_TYPE_MAP covers both Map (yields `(key, value)`
                        // tuples) and Set (yields keys only). Branch on the
                        // source object's type_id: SET → key only, MAP/other
                        // → 2-tuple. Matches the stdlib's
                        // `implement<T> Iterator for SetIter<T> { type Item = T; … }`
                        // and `implement<K, V> Iterator for MapIter<K, V> { type Item = (K, V); … }`.
                        if iter_type == ITER_TYPE_MAP {
                            let source_is_set = {
                                let header = unsafe { &*(source_ptr as *const heap::ObjectHeader) };
                                header.type_id == TypeId::SET
                            };
                            let map_header = unsafe {
                                source_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                            };
                            let capacity = unsafe { (*map_header.add(1)).as_i64() } as usize;
                            let entries_ptr = unsafe { (*map_header.add(2)).as_ptr::<u8>() };
                            let entries_data = unsafe {
                                entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                            };
                            let scan_end = capacity.min(back_idx);
                            let mut idx = front_idx;
                            while idx < scan_end {
                                let entry_key = unsafe { *entries_data.add(idx * 2) };
                                if !entry_key.is_unit() {
                                    unsafe { *iter_data.add(1) = Value::from_i64((idx + 1) as i64); }
                                    let element = if source_is_set {
                                        entry_key
                                    } else {
                                        let entry_val = unsafe { *entries_data.add(idx * 2 + 1) };
                                        let tuple_obj = state.heap.alloc(
                                            TypeId::TUPLE,
                                            2 * std::mem::size_of::<Value>(),
                                        )?;
                                        let tuple_data = unsafe {
                                            (tuple_obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
                                        };
                                        unsafe {
                                            *tuple_data = entry_key;
                                            *tuple_data.add(1) = entry_val;
                                        }
                                        Value::from_ptr(tuple_obj.as_ptr() as *mut u8)
                                    };
                                    return Ok(Some(make_some_value(state, element)?));
                                }
                                idx += 1;
                            }
                            unsafe { *iter_data.add(1) = Value::from_i64(scan_end as i64); }
                            return Ok(Some(make_none_value(state)?));
                        }

                        // Get element reference based on iterator type
                        let elem_ptr = match iter_type {
                            ITER_TYPE_LIST => {
                                let list_header = unsafe {
                                    source_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                                };
                                let backing_ptr = unsafe { (*list_header.add(2)).as_ptr::<u8>() };
                                (backing_ptr as usize)
                                    + heap::OBJECT_HEADER_SIZE
                                    + front_idx * std::mem::size_of::<Value>()
                            }
                            ITER_TYPE_ARRAY => {
                                (source_ptr as usize)
                                    + heap::OBJECT_HEADER_SIZE
                                    + front_idx * std::mem::size_of::<Value>()
                            }
                            _ => return Ok(Some(make_none_value(state)?)),
                        };

                        let thin_ref = ThinRef::new(
                            elem_ptr as *mut u8,
                            state.cbgr_epoch as u32,
                            state.cbgr_epoch as u16,
                            Capabilities::READ_ONLY,
                        );
                        let ref_val = Value::from_thin_ref(thin_ref);

                        // Advance front index
                        unsafe { *iter_data.add(1) = Value::from_i64((front_idx + 1) as i64); }

                        return Ok(Some(make_some_value(state, ref_val)?));
                    }
                    "next_back" => {
                        let source_ptr = unsafe { (*iter_data).as_ptr::<u8>() };
                        let front_idx = unsafe { (*iter_data.add(1)).as_i64() } as usize;
                        let back_idx = unsafe { (*iter_data.add(2)).as_i64() } as usize;

                        // Check if iterator is exhausted
                        if front_idx >= back_idx {
                            return Ok(Some(make_none_value(state)?));
                        }

                        // Decrement back_idx to get the element at the back
                        let new_back_idx = back_idx - 1;

                        // Get element reference based on iterator type
                        let elem_ptr = match iter_type {
                            ITER_TYPE_LIST => {
                                let list_header = unsafe {
                                    source_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                                };
                                let backing_ptr = unsafe { (*list_header.add(2)).as_ptr::<u8>() };
                                (backing_ptr as usize)
                                    + heap::OBJECT_HEADER_SIZE
                                    + new_back_idx * std::mem::size_of::<Value>()
                            }
                            ITER_TYPE_ARRAY => {
                                (source_ptr as usize)
                                    + heap::OBJECT_HEADER_SIZE
                                    + new_back_idx * std::mem::size_of::<Value>()
                            }
                            _ => return Ok(Some(make_none_value(state)?)),
                        };

                        let thin_ref = ThinRef::new(
                            elem_ptr as *mut u8,
                            state.cbgr_epoch as u32,
                            state.cbgr_epoch as u16,
                            Capabilities::READ_ONLY,
                        );
                        let ref_val = Value::from_thin_ref(thin_ref);

                        // Update back index
                        unsafe { *iter_data.add(2) = Value::from_i64(new_back_idx as i64); }

                        return Ok(Some(make_some_value(state, ref_val)?));
                    }
                    "count" => {
                        // Count remaining elements in the iterator
                        let front_idx = unsafe { (*iter_data.add(1)).as_i64() } as usize;
                        let back_idx = unsafe { (*iter_data.add(2)).as_i64() } as usize;

                        let remaining = back_idx.saturating_sub(front_idx);
                        // Consume the iterator (set front_idx to back_idx)
                        unsafe { *iter_data.add(1) = Value::from_i64(back_idx as i64); }
                        return Ok(Some(Value::from_i64(remaining as i64)));
                    }
                    "fold" => {
                        // fold(init, closure) -> accumulator
                        let source_ptr = unsafe { (*iter_data).as_ptr::<u8>() };
                        let front_idx = unsafe { (*iter_data.add(1)).as_i64() } as usize;
                        let back_idx = unsafe { (*iter_data.add(2)).as_i64() } as usize;
                        let caller_base = state.reg_base();
                        let mut acc = state.registers.get(caller_base, Reg(args.start.0));
                        let closure_val = state.registers.get(caller_base, Reg(args.start.0 + 1));

                        for i in front_idx..back_idx {
                            // Read element value (not a ref) - closures expect values directly
                            let elem = match iter_type {
                                ITER_TYPE_LIST => {
                                    let list_header = unsafe {
                                        source_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                                    };
                                    let backing_ptr = unsafe { (*list_header.add(2)).as_ptr::<u8>() };
                                    let elem_ptr = (backing_ptr as usize)
                                        + heap::OBJECT_HEADER_SIZE
                                        + i * std::mem::size_of::<Value>();
                                    unsafe { *(elem_ptr as *const Value) }
                                }
                                ITER_TYPE_ARRAY => {
                                    let elem_ptr = (source_ptr as usize)
                                        + heap::OBJECT_HEADER_SIZE
                                        + i * std::mem::size_of::<Value>();
                                    unsafe { *(elem_ptr as *const Value) }
                                }
                                _ => Value::unit(),
                            };
                            // Call closure with (acc, elem)
                            acc = call_closure_sync(state, closure_val, &[acc, elem])?;
                        }
                        // Consume the iterator
                        unsafe { *iter_data.add(1) = Value::from_i64(back_idx as i64); }
                        return Ok(Some(acc));
                    }
                    "map" => {
                        // map(closure) -> List - eagerly collects mapped values into a List
                        let source_ptr = unsafe { (*iter_data).as_ptr::<u8>() };
                        let front_idx = unsafe { (*iter_data.add(1)).as_i64() } as usize;
                        let back_idx = unsafe { (*iter_data.add(2)).as_i64() } as usize;
                        let caller_base = state.reg_base();
                        let closure_val = state.registers.get(caller_base, Reg(args.start.0));

                        let mut results = Vec::with_capacity(back_idx.saturating_sub(front_idx));
                        for i in front_idx..back_idx {
                            let elem = match iter_type {
                                ITER_TYPE_LIST => {
                                    let list_header = unsafe {
                                        source_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                                    };
                                    let backing_ptr = unsafe { (*list_header.add(2)).as_ptr::<u8>() };
                                    let elem_ptr = (backing_ptr as usize)
                                        + heap::OBJECT_HEADER_SIZE
                                        + i * std::mem::size_of::<Value>();
                                    unsafe { *(elem_ptr as *const Value) }
                                }
                                ITER_TYPE_ARRAY => {
                                    let elem_ptr = (source_ptr as usize)
                                        + heap::OBJECT_HEADER_SIZE
                                        + i * std::mem::size_of::<Value>();
                                    unsafe { *(elem_ptr as *const Value) }
                                }
                                _ => Value::unit(),
                            };
                            let mapped = call_closure_sync(state, closure_val, &[elem])?;
                            results.push(mapped);
                        }
                        // Consume the iterator
                        unsafe { *iter_data.add(1) = Value::from_i64(back_idx as i64); }
                        let list_val = alloc_list_from_values(state, results)?;
                        return Ok(Some(list_val));
                    }
                    "filter" => {
                        // filter(predicate) -> List - eagerly collects matching values
                        let source_ptr = unsafe { (*iter_data).as_ptr::<u8>() };
                        let front_idx = unsafe { (*iter_data.add(1)).as_i64() } as usize;
                        let back_idx = unsafe { (*iter_data.add(2)).as_i64() } as usize;
                        let caller_base = state.reg_base();
                        let predicate = state.registers.get(caller_base, Reg(args.start.0));

                        let mut results = Vec::new();
                        for i in front_idx..back_idx {
                            let elem = match iter_type {
                                ITER_TYPE_LIST => {
                                    let list_header = unsafe {
                                        source_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                                    };
                                    let backing_ptr = unsafe { (*list_header.add(2)).as_ptr::<u8>() };
                                    let elem_ptr = (backing_ptr as usize)
                                        + heap::OBJECT_HEADER_SIZE
                                        + i * std::mem::size_of::<Value>();
                                    unsafe { *(elem_ptr as *const Value) }
                                }
                                ITER_TYPE_ARRAY => {
                                    let elem_ptr = (source_ptr as usize)
                                        + heap::OBJECT_HEADER_SIZE
                                        + i * std::mem::size_of::<Value>();
                                    unsafe { *(elem_ptr as *const Value) }
                                }
                                _ => Value::unit(),
                            };
                            let keep = call_closure_sync(state, predicate, &[elem])?;
                            if keep.as_bool() {
                                results.push(elem);
                            }
                        }
                        // Consume the iterator
                        unsafe { *iter_data.add(1) = Value::from_i64(back_idx as i64); }
                        let list_val = alloc_list_from_values(state, results)?;
                        return Ok(Some(list_val));
                    }
                    "collect" => {
                        // collect() -> List - drains remaining iterator elements into a List.
                        // In this Tier 0 interpreter, map/filter already return Lists eagerly,
                        // but collect() may be called on a raw iterator from .iter().
                        let source_ptr = unsafe { (*iter_data).as_ptr::<u8>() };
                        let front_idx = unsafe { (*iter_data.add(1)).as_i64() } as usize;
                        let back_idx = unsafe { (*iter_data.add(2)).as_i64() } as usize;

                        let mut results = Vec::with_capacity(back_idx.saturating_sub(front_idx));
                        for i in front_idx..back_idx {
                            let elem = match iter_type {
                                ITER_TYPE_LIST => {
                                    let list_header = unsafe {
                                        source_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                                    };
                                    let backing_ptr = unsafe { (*list_header.add(2)).as_ptr::<u8>() };
                                    let elem_ptr = (backing_ptr as usize)
                                        + heap::OBJECT_HEADER_SIZE
                                        + i * std::mem::size_of::<Value>();
                                    unsafe { *(elem_ptr as *const Value) }
                                }
                                ITER_TYPE_ARRAY => {
                                    let elem_ptr = (source_ptr as usize)
                                        + heap::OBJECT_HEADER_SIZE
                                        + i * std::mem::size_of::<Value>();
                                    unsafe { *(elem_ptr as *const Value) }
                                }
                                _ => Value::unit(),
                            };
                            results.push(elem);
                        }
                        // Consume the iterator
                        unsafe { *iter_data.add(1) = Value::from_i64(back_idx as i64); }
                        let list_val = alloc_list_from_values(state, results)?;
                        return Ok(Some(list_val));
                    }
                    "all" => {
                        // all(predicate) -> bool
                        let source_ptr = unsafe { (*iter_data).as_ptr::<u8>() };
                        let front_idx = unsafe { (*iter_data.add(1)).as_i64() } as usize;
                        let back_idx = unsafe { (*iter_data.add(2)).as_i64() } as usize;
                        let caller_base = state.reg_base();
                        let predicate = state.registers.get(caller_base, Reg(args.start.0));

                        let mut result = true;
                        for i in front_idx..back_idx {
                            let elem_ptr = match iter_type {
                                ITER_TYPE_LIST => {
                                    let list_header = unsafe {
                                        source_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                                    };
                                    let backing_ptr = unsafe { (*list_header.add(2)).as_ptr::<u8>() };
                                    (backing_ptr as usize)
                                        + heap::OBJECT_HEADER_SIZE
                                        + i * std::mem::size_of::<Value>()
                                }
                                ITER_TYPE_ARRAY => {
                                    (source_ptr as usize)
                                        + heap::OBJECT_HEADER_SIZE
                                        + i * std::mem::size_of::<Value>()
                                }
                                _ => 0,
                            };
                            let thin_ref = ThinRef::new(
                                elem_ptr as *mut u8,
                                state.cbgr_epoch as u32,
                                state.cbgr_epoch as u16,
                                Capabilities::READ_ONLY,
                            );
                            let elem_ref = Value::from_thin_ref(thin_ref);
                            let test_result = call_closure_sync(state, predicate, &[elem_ref])?;
                            if !test_result.as_bool() {
                                result = false;
                                break;
                            }
                        }
                        // Consume the iterator
                        unsafe { *iter_data.add(1) = Value::from_i64(back_idx as i64); }
                        return Ok(Some(Value::from_bool(result)));
                    }
                    "any" => {
                        // any(predicate) -> bool
                        let source_ptr = unsafe { (*iter_data).as_ptr::<u8>() };
                        let front_idx = unsafe { (*iter_data.add(1)).as_i64() } as usize;
                        let back_idx = unsafe { (*iter_data.add(2)).as_i64() } as usize;
                        let caller_base = state.reg_base();
                        let predicate = state.registers.get(caller_base, Reg(args.start.0));

                        let mut result = false;
                        for i in front_idx..back_idx {
                            let elem_ptr = match iter_type {
                                ITER_TYPE_LIST => {
                                    let list_header = unsafe {
                                        source_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                                    };
                                    let backing_ptr = unsafe { (*list_header.add(2)).as_ptr::<u8>() };
                                    (backing_ptr as usize)
                                        + heap::OBJECT_HEADER_SIZE
                                        + i * std::mem::size_of::<Value>()
                                }
                                ITER_TYPE_ARRAY => {
                                    (source_ptr as usize)
                                        + heap::OBJECT_HEADER_SIZE
                                        + i * std::mem::size_of::<Value>()
                                }
                                _ => 0,
                            };
                            let thin_ref = ThinRef::new(
                                elem_ptr as *mut u8,
                                state.cbgr_epoch as u32,
                                state.cbgr_epoch as u16,
                                Capabilities::READ_ONLY,
                            );
                            let elem_ref = Value::from_thin_ref(thin_ref);
                            let test_result = call_closure_sync(state, predicate, &[elem_ref])?;
                            if test_result.as_bool() {
                                result = true;
                                break;
                            }
                        }
                        // Consume the iterator
                        unsafe { *iter_data.add(1) = Value::from_i64(back_idx as i64); }
                        return Ok(Some(Value::from_bool(result)));
                    }
                    "for_each" => {
                        // for_each(closure) - calls closure on each element
                        let source_ptr = unsafe { (*iter_data).as_ptr::<u8>() };
                        let front_idx = unsafe { (*iter_data.add(1)).as_i64() } as usize;
                        let back_idx = unsafe { (*iter_data.add(2)).as_i64() } as usize;
                        let caller_base = state.reg_base();
                        let closure = state.registers.get(caller_base, Reg(args.start.0));

                        for i in front_idx..back_idx {
                            let elem_ptr = match iter_type {
                                ITER_TYPE_LIST => {
                                    let list_header = unsafe {
                                        source_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                                    };
                                    let backing_ptr = unsafe { (*list_header.add(2)).as_ptr::<u8>() };
                                    (backing_ptr as usize)
                                        + heap::OBJECT_HEADER_SIZE
                                        + i * std::mem::size_of::<Value>()
                                }
                                ITER_TYPE_ARRAY => {
                                    (source_ptr as usize)
                                        + heap::OBJECT_HEADER_SIZE
                                        + i * std::mem::size_of::<Value>()
                                }
                                _ => 0,
                            };
                            let thin_ref = ThinRef::new(
                                elem_ptr as *mut u8,
                                state.cbgr_epoch as u32,
                                state.cbgr_epoch as u16,
                                Capabilities::READ_ONLY,
                            );
                            let elem_ref = Value::from_thin_ref(thin_ref);
                            call_closure_sync(state, closure, &[elem_ref])?;
                        }
                        // Consume the iterator
                        unsafe { *iter_data.add(1) = Value::from_i64(back_idx as i64); }
                        return Ok(Some(Value::unit()));
                    }
                    _ => {} // fall through
                }
            }
        }

        // Map methods
        let is_map = header.type_id == TypeId::MAP;
        let is_set = header.type_id == TypeId::SET;
        if is_map || is_set {
            match method {
                "len" => {
                    let data_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let count = unsafe { (*data_ptr).as_i64() } as usize;
                    return Ok(Some(Value::from_i64(count as i64)));
                }
                "iter" => {
                    // Return a proper 4-value iterator blob
                    // [source_ptr, front_idx=0, back_idx=capacity, iter_type=ITER_TYPE_MAP]
                    // so explicit `.next()` / `.size_hint()` / `.fold()` calls
                    // work through the iterator-blob dispatch path at
                    // `TypeId::UNIT + 4 × Value` layout below.
                    //
                    // For-loops continue to work: `IterNew` detects a
                    // pre-built iterator blob (TypeId::UNIT + 4-value) and
                    // passes it through unchanged — see the updated
                    // `handle_iter_new` in `iterators.rs`.
                    //
                    // `back_idx` is set to `capacity` (not `len`) because
                    // Map/Set iteration scans the raw `entries` array and
                    // `next` on an iterator-blob skips empty slots.
                    let header_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let capacity = unsafe { (*header_ptr.add(1)).as_i64() };
                    let iter_obj = state.heap.alloc(TypeId::UNIT, 4 * std::mem::size_of::<Value>())?;
                    state.record_allocation();
                    let iter_ptr = unsafe {
                        (iter_obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    unsafe {
                        *iter_ptr = *receiver;                         // source_ptr (Map/Set)
                        *iter_ptr.add(1) = Value::from_i64(0);         // front_idx = 0
                        *iter_ptr.add(2) = Value::from_i64(capacity);  // back_idx = capacity
                        *iter_ptr.add(3) = Value::from_i64(ITER_TYPE_MAP); // iter_type
                    }
                    return Ok(Some(Value::from_ptr(iter_obj.as_ptr() as *mut u8)));
                }
                "contains" | "contains_key" if is_map => {
                    let caller_base = state.reg_base();
                    let key = state.registers.get(caller_base, Reg(args.start.0));
                    let header_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
                    let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };
                    let entries_data = unsafe {
                        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let hash = value_hash(key);
                    let mut idx = hash % capacity;
                    let start = idx;
                    let mut found = false;
                    loop {
                        let entry_key = unsafe { *entries_data.add(idx * 2) };
                        if entry_key.is_unit() { break; }
                        if value_eq(entry_key, key) { found = true; break; }
                        idx = (idx + 1) % capacity;
                        if idx == start { break; }
                    }
                    return Ok(Some(Value::from_bool(found)));
                }
                "insert" if is_set => {
                    let caller_base = state.reg_base();
                    let val = state.registers.get(caller_base, Reg(args.start.0));
                    // Set uses same layout as Map: [count, capacity, entries_ptr]
                    // entries are [key, Value::unit()] pairs
                    let header_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    let mut count = unsafe { (*header_ptr).as_i64() } as usize;
                    let mut capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
                    let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };
                    let mut entries_data = unsafe {
                        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };

                    // Resize if load factor >= 75%
                    if count * 4 >= capacity * 3 {
                        let new_cap = capacity * 2;
                        let new_entries = state.heap.alloc_array(TypeId::UNIT, new_cap * 2)?;
                        state.record_allocation();
                        let new_entries_ptr = new_entries.as_ptr() as *mut u8;
                        let new_entries_data = unsafe {
                            new_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                        };
                        // Initialize all slots to unit
                        for i in 0..(new_cap * 2) {
                            unsafe { *new_entries_data.add(i) = Value::unit(); }
                        }
                        // Rehash existing entries
                        for i in 0..capacity {
                            let old_key = unsafe { *entries_data.add(i * 2) };
                            if !old_key.is_unit() {
                                let h = value_hash(old_key);
                                let mut ni = h % new_cap;
                                loop {
                                    if unsafe { (*new_entries_data.add(ni * 2)).is_unit() } {
                                        unsafe {
                                            *new_entries_data.add(ni * 2) = old_key;
                                            // Set's value-slot must be unit to match the
                                            // stdlib contract (`Map<T, ()>`); ITER_TYPE_MAP
                                            // inspects the source type_id to decide whether
                                            // to yield keys (Set) or pairs (Map).
                                            *new_entries_data.add(ni * 2 + 1) = Value::unit();
                                        }
                                        break;
                                    }
                                    ni = (ni + 1) % new_cap;
                                }
                            }
                        }
                        capacity = new_cap;
                        entries_data = new_entries_data;
                        unsafe {
                            *header_ptr.add(1) = Value::from_i64(new_cap as i64);
                            *header_ptr.add(2) = Value::from_ptr(new_entries_ptr);
                        }
                    }

                    let hash = value_hash(val);
                    let mut idx = hash % capacity;
                    loop {
                        let entry_key = unsafe { *entries_data.add(idx * 2) };
                        if entry_key.is_unit() {
                            unsafe {
                                *entries_data.add(idx * 2) = val;
                                // Set's value-slot is unit (`Map<T, ()>`).
                                *entries_data.add(idx * 2 + 1) = Value::unit();
                            }
                            count += 1;
                            unsafe { *header_ptr = Value::from_i64(count as i64); }
                            break;
                        }
                        if value_eq(entry_key, val) {
                            break; // duplicate
                        }
                        idx = (idx + 1) % capacity;
                    }
                    return Ok(Some(Value::unit()));
                }
                "contains" if is_set => {
                    let caller_base = state.reg_base();
                    let val = state.registers.get(caller_base, Reg(args.start.0));
                    let header_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
                    let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };
                    let entries_data = unsafe {
                        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let hash = value_hash(val);
                    let mut idx = hash % capacity;
                    let start = idx;
                    let mut found = false;
                    loop {
                        let entry_key = unsafe { *entries_data.add(idx * 2) };
                        if entry_key.is_unit() { break; }
                        if value_eq(entry_key, val) { found = true; break; }
                        idx = (idx + 1) % capacity;
                        if idx == start { break; }
                    }
                    return Ok(Some(Value::from_bool(found)));
                }
                "insert" if is_map => {
                    // Map.insert(key, value) -> Maybe<V>
                    // Returns Some(old_value) if key existed, None otherwise
                    let caller_base = state.reg_base();
                    let key = state.registers.get(caller_base, Reg(args.start.0));
                    let value = state.registers.get(caller_base, Reg(args.start.0 + 1));
                    let header_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    let mut count = unsafe { (*header_ptr).as_i64() } as usize;
                    let mut capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
                    let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };
                    let mut entries_data = unsafe {
                        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };

                    // Resize if load factor >= 75%
                    if count * 4 >= capacity * 3 {
                        let new_cap = capacity * 2;
                        let new_entries = state.heap.alloc_array(TypeId::UNIT, new_cap * 2)?;
                        state.record_allocation();
                        let new_entries_ptr = new_entries.as_ptr() as *mut u8;
                        let new_entries_data = unsafe {
                            new_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                        };
                        // Initialize all slots to unit
                        for i in 0..(new_cap * 2) {
                            unsafe { *new_entries_data.add(i) = Value::unit(); }
                        }
                        // Rehash existing entries
                        for i in 0..capacity {
                            let old_key = unsafe { *entries_data.add(i * 2) };
                            if !old_key.is_unit() {
                                let old_val = unsafe { *entries_data.add(i * 2 + 1) };
                                let h = value_hash(old_key);
                                let mut ni = h % new_cap;
                                loop {
                                    if unsafe { (*new_entries_data.add(ni * 2)).is_unit() } {
                                        unsafe {
                                            *new_entries_data.add(ni * 2) = old_key;
                                            *new_entries_data.add(ni * 2 + 1) = old_val;
                                        }
                                        break;
                                    }
                                    ni = (ni + 1) % new_cap;
                                }
                            }
                        }
                        capacity = new_cap;
                        entries_data = new_entries_data;
                        unsafe {
                            *header_ptr.add(1) = Value::from_i64(new_cap as i64);
                            *header_ptr.add(2) = Value::from_ptr(new_entries_ptr);
                        }
                    }

                    let hash = value_hash(key);
                    let mut idx = hash % capacity;
                    let start = idx;
                    let mut old_value: Option<Value> = None;
                    loop {
                        let entry_key = unsafe { *entries_data.add(idx * 2) };
                        if entry_key.is_unit() {
                            // Empty slot - insert new entry
                            unsafe {
                                *entries_data.add(idx * 2) = key;
                                *entries_data.add(idx * 2 + 1) = value;
                            }
                            count += 1;
                            unsafe { *header_ptr = Value::from_i64(count as i64); }
                            break;
                        }
                        if value_eq(entry_key, key) {
                            // Key exists - replace value
                            old_value = Some(unsafe { *entries_data.add(idx * 2 + 1) });
                            unsafe { *entries_data.add(idx * 2 + 1) = value; }
                            break;
                        }
                        idx = (idx + 1) % capacity;
                        if idx == start {
                            // Should not happen if load factor is maintained
                            break;
                        }
                    }
                    // Return Maybe: None if no old value, Some(old_value) if key existed
                    let result = match old_value {
                        None => make_none_value(state)?,
                        Some(v) => make_some_value(state, v)?,
                    };
                    return Ok(Some(result));
                }
                "get" if is_map => {
                    // Map.get(key) -> V (returns default 0 for missing keys)
                    // Matches MapGet opcode behavior for interpreter/AOT consistency.
                    let caller_base = state.reg_base();
                    let key = state.registers.get(caller_base, Reg(args.start.0));
                    let header_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
                    let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };
                    let entries_data = unsafe {
                        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let hash = value_hash(key);
                    let mut idx = hash % capacity;
                    let start = idx;
                    loop {
                        let entry_key = unsafe { *entries_data.add(idx * 2) };
                        if entry_key.is_unit() {
                            // Not found - return default (0)
                            return Ok(Some(Value::from_i64(0)));
                        }
                        if value_eq(entry_key, key) {
                            let val = unsafe { *entries_data.add(idx * 2 + 1) };
                            return Ok(Some(val));
                        }
                        idx = (idx + 1) % capacity;
                        if idx == start {
                            return Ok(Some(Value::from_i64(0)));
                        }
                    }
                }
                "ensure_capacity" if is_map => {
                    // Map.ensure_capacity() - resize if load factor is too high
                    // This is called by user-defined Map.insert from core/collections
                    let header_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    let count = unsafe { (*header_ptr).as_i64() } as usize;
                    let capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;

                    // Handle initial capacity (0 means uninitialized)
                    if capacity == 0 {
                        const INITIAL_CAP: usize = 16;
                        let new_entries = state.heap.alloc_array(TypeId::UNIT, INITIAL_CAP * 2)?;
                        state.record_allocation();
                        let new_entries_ptr = new_entries.as_ptr() as *mut u8;
                        let new_entries_data = unsafe {
                            new_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                        };
                        for i in 0..(INITIAL_CAP * 2) {
                            unsafe { *new_entries_data.add(i) = Value::unit(); }
                        }
                        unsafe {
                            *header_ptr.add(1) = Value::from_i64(INITIAL_CAP as i64);
                            *header_ptr.add(2) = Value::from_ptr(new_entries_ptr);
                        }
                        return Ok(Some(Value::unit()));
                    }

                    // Resize if load factor >= 75%
                    if count * 4 >= capacity * 3 {
                        let new_cap = capacity * 2;
                        let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };
                        let entries_data = unsafe {
                            entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                        };
                        let new_entries = state.heap.alloc_array(TypeId::UNIT, new_cap * 2)?;
                        state.record_allocation();
                        let new_entries_ptr = new_entries.as_ptr() as *mut u8;
                        let new_entries_data = unsafe {
                            new_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                        };
                        // Initialize all slots to unit
                        for i in 0..(new_cap * 2) {
                            unsafe { *new_entries_data.add(i) = Value::unit(); }
                        }
                        // Rehash existing entries
                        for i in 0..capacity {
                            let old_key = unsafe { *entries_data.add(i * 2) };
                            if !old_key.is_unit() {
                                let old_val = unsafe { *entries_data.add(i * 2 + 1) };
                                let h = value_hash(old_key);
                                let mut ni = h % new_cap;
                                loop {
                                    if unsafe { (*new_entries_data.add(ni * 2)).is_unit() } {
                                        unsafe {
                                            *new_entries_data.add(ni * 2) = old_key;
                                            *new_entries_data.add(ni * 2 + 1) = old_val;
                                        }
                                        break;
                                    }
                                    ni = (ni + 1) % new_cap;
                                }
                            }
                        }
                        unsafe {
                            *header_ptr.add(1) = Value::from_i64(new_cap as i64);
                            *header_ptr.add(2) = Value::from_ptr(new_entries_ptr);
                        }
                    }
                    return Ok(Some(Value::unit()));
                }
                "new" => {
                    // Static constructor: create a new empty set/map
                    // This handles Set.new() and Map.new()
                    // But actually, "new" is called on the type value, not on an instance.
                    // This won't match here since receiver would be a type marker, not a Set/Map object.
                    // Fall through.
                }
                "remove" if is_map => {
                    // Map.remove(key) -> Maybe<V>
                    // Returns Some(old_value) if key existed, None otherwise
                    let caller_base = state.reg_base();
                    let key = state.registers.get(caller_base, Reg(args.start.0));
                    let header_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    let mut count = unsafe { (*header_ptr).as_i64() } as usize;
                    let capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
                    let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };
                    let entries_data = unsafe {
                        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    let hash = value_hash(key);
                    let mut idx = hash % capacity;
                    let start = idx;
                    let mut found_value: Option<Value> = None;
                    loop {
                        let entry_key = unsafe { *entries_data.add(idx * 2) };
                        if entry_key.is_unit() { break; }
                        if value_eq(entry_key, key) {
                            found_value = Some(unsafe { *entries_data.add(idx * 2 + 1) });
                            // Clear the slot
                            unsafe {
                                *entries_data.add(idx * 2) = Value::unit();
                                *entries_data.add(idx * 2 + 1) = Value::unit();
                            }
                            count -= 1;
                            unsafe { *header_ptr = Value::from_i64(count as i64); }
                            // Backward-shift rehash: fix up the probe chain
                            let mut gap = idx;
                            let mut j = (idx + 1) % capacity;
                            loop {
                                let jk = unsafe { *entries_data.add(j * 2) };
                                if jk.is_unit() { break; }
                                let jh = value_hash(jk) % capacity;
                                // Check if j's natural slot is at or before the gap
                                // (accounting for wraparound)
                                let should_move = if gap <= j {
                                    jh <= gap || jh > j
                                } else {
                                    jh <= gap && jh > j
                                };
                                if should_move {
                                    let jv = unsafe { *entries_data.add(j * 2 + 1) };
                                    unsafe {
                                        *entries_data.add(gap * 2) = jk;
                                        *entries_data.add(gap * 2 + 1) = jv;
                                        *entries_data.add(j * 2) = Value::unit();
                                        *entries_data.add(j * 2 + 1) = Value::unit();
                                    }
                                    gap = j;
                                }
                                j = (j + 1) % capacity;
                                if j == start { break; }
                            }
                            break;
                        }
                        idx = (idx + 1) % capacity;
                        if idx == start { break; }
                    }
                    let result = match found_value {
                        None => make_none_value(state)?,
                        Some(v) => make_some_value(state, v)?,
                    };
                    return Ok(Some(result));
                }
                "keys" if is_map => {
                    // Map.keys() -> List<K>
                    let header_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
                    let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };
                    let entries_data = unsafe {
                        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let mut keys = Vec::new();
                    for i in 0..capacity {
                        let k = unsafe { *entries_data.add(i * 2) };
                        if !k.is_unit() {
                            keys.push(k);
                        }
                    }
                    let result = alloc_list_from_values(state, keys)?;
                    return Ok(Some(result));
                }
                "values" if is_map => {
                    // Map.values() -> List<V>
                    let header_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
                    let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };
                    let entries_data = unsafe {
                        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let mut vals = Vec::new();
                    for i in 0..capacity {
                        let k = unsafe { *entries_data.add(i * 2) };
                        if !k.is_unit() {
                            vals.push(unsafe { *entries_data.add(i * 2 + 1) });
                        }
                    }
                    let result = alloc_list_from_values(state, vals)?;
                    return Ok(Some(result));
                }
                "entries" if is_map => {
                    // Map.entries() -> List<(K, V)>
                    let header_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
                    let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };
                    let entries_data = unsafe {
                        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let mut tuples = Vec::new();
                    for i in 0..capacity {
                        let k = unsafe { *entries_data.add(i * 2) };
                        if !k.is_unit() {
                            let v = unsafe { *entries_data.add(i * 2 + 1) };
                            // Allocate a 2-element tuple: [key, value]
                            let tuple_obj = state.heap.alloc(TypeId::TUPLE, 2 * std::mem::size_of::<Value>())?;
                            state.record_allocation();
                            let tuple_data = unsafe {
                                (tuple_obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
                            };
                            unsafe {
                                *tuple_data = k;
                                *tuple_data.add(1) = v;
                            }
                            tuples.push(Value::from_ptr(tuple_obj.as_ptr() as *mut u8));
                        }
                    }
                    let result = alloc_list_from_values(state, tuples)?;
                    return Ok(Some(result));
                }
                "clear" if is_map => {
                    // Map.clear() - remove all entries
                    let header_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    let capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
                    let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };
                    let entries_data = unsafe {
                        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    // Set count to 0
                    unsafe { *header_ptr = Value::from_i64(0); }
                    // Clear all slots
                    for i in 0..(capacity * 2) {
                        unsafe { *entries_data.add(i) = Value::unit(); }
                    }
                    return Ok(Some(Value::unit()));
                }
                "is_empty" if is_map => {
                    // Map.is_empty() -> Bool
                    let header_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let count = unsafe { (*header_ptr).as_i64() } as usize;
                    return Ok(Some(Value::from_bool(count == 0)));
                }
                "get_or_insert" if is_map => {
                    // Map.get_or_insert(key, default_value) -> V
                    // Returns existing value if key present, else inserts default and returns it
                    let caller_base = state.reg_base();
                    let key = state.registers.get(caller_base, Reg(args.start.0));
                    let default_value = state.registers.get(caller_base, Reg(args.start.0 + 1));
                    let header_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    let mut count = unsafe { (*header_ptr).as_i64() } as usize;
                    let mut capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
                    let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };
                    let mut entries_data = unsafe {
                        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };

                    // Resize if load factor >= 75%
                    if count * 4 >= capacity * 3 {
                        let new_cap = capacity * 2;
                        let new_entries = state.heap.alloc_array(TypeId::UNIT, new_cap * 2)?;
                        state.record_allocation();
                        let new_entries_ptr = new_entries.as_ptr() as *mut u8;
                        let new_entries_data = unsafe {
                            new_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                        };
                        for i in 0..(new_cap * 2) {
                            unsafe { *new_entries_data.add(i) = Value::unit(); }
                        }
                        for i in 0..capacity {
                            let old_key = unsafe { *entries_data.add(i * 2) };
                            if !old_key.is_unit() {
                                let old_val = unsafe { *entries_data.add(i * 2 + 1) };
                                let h = value_hash(old_key);
                                let mut ni = h % new_cap;
                                loop {
                                    if unsafe { (*new_entries_data.add(ni * 2)).is_unit() } {
                                        unsafe {
                                            *new_entries_data.add(ni * 2) = old_key;
                                            *new_entries_data.add(ni * 2 + 1) = old_val;
                                        }
                                        break;
                                    }
                                    ni = (ni + 1) % new_cap;
                                }
                            }
                        }
                        capacity = new_cap;
                        entries_data = new_entries_data;
                        unsafe {
                            *header_ptr.add(1) = Value::from_i64(new_cap as i64);
                            *header_ptr.add(2) = Value::from_ptr(new_entries_ptr);
                        }
                    }

                    let hash = value_hash(key);
                    let mut idx = hash % capacity;
                    let start = idx;
                    loop {
                        let entry_key = unsafe { *entries_data.add(idx * 2) };
                        if entry_key.is_unit() {
                            // Not found - insert default
                            unsafe {
                                *entries_data.add(idx * 2) = key;
                                *entries_data.add(idx * 2 + 1) = default_value;
                            }
                            count += 1;
                            unsafe { *header_ptr = Value::from_i64(count as i64); }
                            return Ok(Some(default_value));
                        }
                        if value_eq(entry_key, key) {
                            // Found - return existing value
                            let existing = unsafe { *entries_data.add(idx * 2 + 1) };
                            return Ok(Some(existing));
                        }
                        idx = (idx + 1) % capacity;
                        if idx == start {
                            // Table full (shouldn't happen with resize)
                            return Ok(Some(default_value));
                        }
                    }
                }
                "remove" if is_set => {
                    // Set.remove(element) -> Bool
                    // Returns true if element was present, false otherwise
                    let caller_base = state.reg_base();
                    let val = state.registers.get(caller_base, Reg(args.start.0));
                    let header_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    let mut count = unsafe { (*header_ptr).as_i64() } as usize;
                    let capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
                    let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };
                    let entries_data = unsafe {
                        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    let hash = value_hash(val);
                    let mut idx = hash % capacity;
                    let start = idx;
                    let mut found = false;
                    loop {
                        let entry_key = unsafe { *entries_data.add(idx * 2) };
                        if entry_key.is_unit() { break; }
                        if value_eq(entry_key, val) {
                            found = true;
                            // Clear the slot
                            unsafe {
                                *entries_data.add(idx * 2) = Value::unit();
                                *entries_data.add(idx * 2 + 1) = Value::unit();
                            }
                            count -= 1;
                            unsafe { *header_ptr = Value::from_i64(count as i64); }
                            // Backward-shift rehash
                            let mut gap = idx;
                            let mut j = (idx + 1) % capacity;
                            loop {
                                let jk = unsafe { *entries_data.add(j * 2) };
                                if jk.is_unit() { break; }
                                let jh = value_hash(jk) % capacity;
                                let should_move = if gap <= j {
                                    jh <= gap || jh > j
                                } else {
                                    jh <= gap && jh > j
                                };
                                if should_move {
                                    let jv = unsafe { *entries_data.add(j * 2 + 1) };
                                    unsafe {
                                        *entries_data.add(gap * 2) = jk;
                                        *entries_data.add(gap * 2 + 1) = jv;
                                        *entries_data.add(j * 2) = Value::unit();
                                        *entries_data.add(j * 2 + 1) = Value::unit();
                                    }
                                    gap = j;
                                }
                                j = (j + 1) % capacity;
                                if j == start { break; }
                            }
                            break;
                        }
                        idx = (idx + 1) % capacity;
                        if idx == start { break; }
                    }
                    return Ok(Some(Value::from_bool(found)));
                }
                "clear" if is_set => {
                    // Set.clear() - remove all elements
                    let header_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    let capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
                    let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };
                    let entries_data = unsafe {
                        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    unsafe { *header_ptr = Value::from_i64(0); }
                    for i in 0..(capacity * 2) {
                        unsafe { *entries_data.add(i) = Value::unit(); }
                    }
                    return Ok(Some(Value::unit()));
                }
                "is_empty" if is_set => {
                    // Set.is_empty() -> Bool
                    let header_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let count = unsafe { (*header_ptr).as_i64() } as usize;
                    return Ok(Some(Value::from_bool(count == 0)));
                }
                "union" if is_set => {
                    // Set.union(other) -> Set  (new set with elements from both)
                    let caller_base = state.reg_base();
                    let other_val = state.registers.get(caller_base, Reg(args.start.0));

                    // Read self entries
                    let self_header = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let self_count = unsafe { (*self_header).as_i64() } as usize;
                    let self_cap = unsafe { (*self_header.add(1)).as_i64() } as usize;
                    let self_entries_ptr = unsafe { (*self_header.add(2)).as_ptr::<u8>() };
                    let self_entries = unsafe {
                        self_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };

                    // Read other entries
                    let other_ptr = other_val.as_ptr::<u8>();
                    let other_header = unsafe {
                        other_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let other_count = unsafe { (*other_header).as_i64() } as usize;
                    let other_cap = unsafe { (*other_header.add(1)).as_i64() } as usize;
                    let other_entries_ptr = unsafe { (*other_header.add(2)).as_ptr::<u8>() };
                    let other_entries = unsafe {
                        other_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };

                    // Collect all unique elements
                    let mut elements = Vec::with_capacity(self_count + other_count);
                    for i in 0..self_cap {
                        let k = unsafe { *self_entries.add(i * 2) };
                        if !k.is_unit() {
                            elements.push(k);
                        }
                    }
                    for i in 0..other_cap {
                        let k = unsafe { *other_entries.add(i * 2) };
                        if !k.is_unit() {
                            // Check if already in elements
                            let mut dup = false;
                            for existing in &elements {
                                if value_eq(*existing, k) { dup = true; break; }
                            }
                            if !dup {
                                elements.push(k);
                            }
                        }
                    }

                    // Allocate new set
                    let new_cap = (elements.len() * 2).max(16);
                    let new_obj = state.heap.alloc(TypeId::SET, 3 * std::mem::size_of::<Value>())?;
                    state.record_allocation();
                    let new_backing = state.heap.alloc_array(TypeId::UNIT, new_cap * 2)?;
                    state.record_allocation();
                    let new_backing_ptr = new_backing.as_ptr() as *mut u8;
                    let new_data = unsafe {
                        new_backing_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    for i in 0..(new_cap * 2) {
                        unsafe { *new_data.add(i) = Value::unit(); }
                    }
                    // Insert all elements
                    for elem in &elements {
                        let h = value_hash(*elem);
                        let mut ni = h % new_cap;
                        loop {
                            if unsafe { (*new_data.add(ni * 2)).is_unit() } {
                                unsafe {
                                    *new_data.add(ni * 2) = *elem;
                                    // Set's value-slot is unit (`Map<T, ()>`
                                    // contract; iteration branches on
                                    // source type_id).
                                    *new_data.add(ni * 2 + 1) = Value::unit();
                                }
                                break;
                            }
                            ni = (ni + 1) % new_cap;
                        }
                    }
                    // Initialize header
                    let new_header = unsafe {
                        (new_obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    unsafe {
                        *new_header = Value::from_i64(elements.len() as i64);
                        *new_header.add(1) = Value::from_i64(new_cap as i64);
                        *new_header.add(2) = Value::from_ptr(new_backing_ptr);
                    }
                    return Ok(Some(Value::from_ptr(new_obj.as_ptr() as *mut u8)));
                }
                "intersection" if is_set => {
                    // Set.intersection(other) -> Set  (elements in both sets)
                    let caller_base = state.reg_base();
                    let other_val = state.registers.get(caller_base, Reg(args.start.0));

                    // Read self entries
                    let self_header = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let self_cap = unsafe { (*self_header.add(1)).as_i64() } as usize;
                    let self_entries_ptr = unsafe { (*self_header.add(2)).as_ptr::<u8>() };
                    let self_entries = unsafe {
                        self_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };

                    // Read other entries (for lookup)
                    let other_ptr = other_val.as_ptr::<u8>();
                    let other_header = unsafe {
                        other_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let other_cap = unsafe { (*other_header.add(1)).as_i64() } as usize;
                    let other_entries_ptr = unsafe { (*other_header.add(2)).as_ptr::<u8>() };
                    let other_entries = unsafe {
                        other_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };

                    // Collect elements that are in both sets
                    let mut elements = Vec::new();
                    for i in 0..self_cap {
                        let k = unsafe { *self_entries.add(i * 2) };
                        if !k.is_unit() {
                            // Check if k is in other set
                            let h = value_hash(k);
                            let mut oi = h % other_cap;
                            let ostart = oi;
                            let mut in_other = false;
                            loop {
                                let ok = unsafe { *other_entries.add(oi * 2) };
                                if ok.is_unit() { break; }
                                if value_eq(ok, k) { in_other = true; break; }
                                oi = (oi + 1) % other_cap;
                                if oi == ostart { break; }
                            }
                            if in_other {
                                elements.push(k);
                            }
                        }
                    }

                    // Allocate new set
                    let new_cap = (elements.len() * 2).max(16);
                    let new_obj = state.heap.alloc(TypeId::SET, 3 * std::mem::size_of::<Value>())?;
                    state.record_allocation();
                    let new_backing = state.heap.alloc_array(TypeId::UNIT, new_cap * 2)?;
                    state.record_allocation();
                    let new_backing_ptr = new_backing.as_ptr() as *mut u8;
                    let new_data = unsafe {
                        new_backing_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    for i in 0..(new_cap * 2) {
                        unsafe { *new_data.add(i) = Value::unit(); }
                    }
                    for elem in &elements {
                        let h = value_hash(*elem);
                        let mut ni = h % new_cap;
                        loop {
                            if unsafe { (*new_data.add(ni * 2)).is_unit() } {
                                unsafe {
                                    *new_data.add(ni * 2) = *elem;
                                    // Set's value-slot is unit (`Map<T, ()>`
                                    // contract; iteration branches on
                                    // source type_id).
                                    *new_data.add(ni * 2 + 1) = Value::unit();
                                }
                                break;
                            }
                            ni = (ni + 1) % new_cap;
                        }
                    }
                    let new_header = unsafe {
                        (new_obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    unsafe {
                        *new_header = Value::from_i64(elements.len() as i64);
                        *new_header.add(1) = Value::from_i64(new_cap as i64);
                        *new_header.add(2) = Value::from_ptr(new_backing_ptr);
                    }
                    return Ok(Some(Value::from_ptr(new_obj.as_ptr() as *mut u8)));
                }
                "difference" if is_set => {
                    // Set.difference(other) -> Set  (elements in self but not in other)
                    let caller_base = state.reg_base();
                    let other_val = state.registers.get(caller_base, Reg(args.start.0));

                    // Read self entries
                    let self_header = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let self_cap = unsafe { (*self_header.add(1)).as_i64() } as usize;
                    let self_entries_ptr = unsafe { (*self_header.add(2)).as_ptr::<u8>() };
                    let self_entries = unsafe {
                        self_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };

                    // Read other entries (for lookup)
                    let other_ptr = other_val.as_ptr::<u8>();
                    let other_header = unsafe {
                        other_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let other_cap = unsafe { (*other_header.add(1)).as_i64() } as usize;
                    let other_entries_ptr = unsafe { (*other_header.add(2)).as_ptr::<u8>() };
                    let other_entries = unsafe {
                        other_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };

                    // Collect elements in self but not in other
                    let mut elements = Vec::new();
                    for i in 0..self_cap {
                        let k = unsafe { *self_entries.add(i * 2) };
                        if !k.is_unit() {
                            // Check if k is in other set
                            let h = value_hash(k);
                            let mut oi = h % other_cap;
                            let ostart = oi;
                            let mut in_other = false;
                            loop {
                                let ok = unsafe { *other_entries.add(oi * 2) };
                                if ok.is_unit() { break; }
                                if value_eq(ok, k) { in_other = true; break; }
                                oi = (oi + 1) % other_cap;
                                if oi == ostart { break; }
                            }
                            if !in_other {
                                elements.push(k);
                            }
                        }
                    }

                    // Allocate new set
                    let new_cap = (elements.len() * 2).max(16);
                    let new_obj = state.heap.alloc(TypeId::SET, 3 * std::mem::size_of::<Value>())?;
                    state.record_allocation();
                    let new_backing = state.heap.alloc_array(TypeId::UNIT, new_cap * 2)?;
                    state.record_allocation();
                    let new_backing_ptr = new_backing.as_ptr() as *mut u8;
                    let new_data = unsafe {
                        new_backing_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    for i in 0..(new_cap * 2) {
                        unsafe { *new_data.add(i) = Value::unit(); }
                    }
                    for elem in &elements {
                        let h = value_hash(*elem);
                        let mut ni = h % new_cap;
                        loop {
                            if unsafe { (*new_data.add(ni * 2)).is_unit() } {
                                unsafe {
                                    *new_data.add(ni * 2) = *elem;
                                    // Set's value-slot is unit (`Map<T, ()>`
                                    // contract; iteration branches on
                                    // source type_id).
                                    *new_data.add(ni * 2 + 1) = Value::unit();
                                }
                                break;
                            }
                            ni = (ni + 1) % new_cap;
                        }
                    }
                    let new_header = unsafe {
                        (new_obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    unsafe {
                        *new_header = Value::from_i64(elements.len() as i64);
                        *new_header.add(1) = Value::from_i64(new_cap as i64);
                        *new_header.add(2) = Value::from_ptr(new_backing_ptr);
                    }
                    return Ok(Some(Value::from_ptr(new_obj.as_ptr() as *mut u8)));
                }
                "is_subset" if is_set => {
                    // Set.is_subset(other) -> Bool  (all self elements are in other)
                    let caller_base = state.reg_base();
                    let other_val = state.registers.get(caller_base, Reg(args.start.0));

                    let self_header = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let self_cap = unsafe { (*self_header.add(1)).as_i64() } as usize;
                    let self_entries_ptr = unsafe { (*self_header.add(2)).as_ptr::<u8>() };
                    let self_entries = unsafe {
                        self_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };

                    let other_ptr = other_val.as_ptr::<u8>();
                    let other_header = unsafe {
                        other_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let other_cap = unsafe { (*other_header.add(1)).as_i64() } as usize;
                    let other_entries_ptr = unsafe { (*other_header.add(2)).as_ptr::<u8>() };
                    let other_entries = unsafe {
                        other_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };

                    let mut is_subset = true;
                    for i in 0..self_cap {
                        let k = unsafe { *self_entries.add(i * 2) };
                        if !k.is_unit() {
                            let h = value_hash(k);
                            let mut oi = h % other_cap;
                            let ostart = oi;
                            let mut in_other = false;
                            loop {
                                let ok = unsafe { *other_entries.add(oi * 2) };
                                if ok.is_unit() { break; }
                                if value_eq(ok, k) { in_other = true; break; }
                                oi = (oi + 1) % other_cap;
                                if oi == ostart { break; }
                            }
                            if !in_other {
                                is_subset = false;
                                break;
                            }
                        }
                    }
                    return Ok(Some(Value::from_bool(is_subset)));
                }
                "is_superset" if is_set => {
                    // Set.is_superset(other) -> Bool  (all other elements are in self)
                    let caller_base = state.reg_base();
                    let other_val = state.registers.get(caller_base, Reg(args.start.0));

                    let self_header = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let self_cap = unsafe { (*self_header.add(1)).as_i64() } as usize;
                    let self_entries_ptr = unsafe { (*self_header.add(2)).as_ptr::<u8>() };
                    let self_entries = unsafe {
                        self_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };

                    let other_ptr = other_val.as_ptr::<u8>();
                    let other_header = unsafe {
                        other_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let other_cap = unsafe { (*other_header.add(1)).as_i64() } as usize;
                    let other_entries_ptr = unsafe { (*other_header.add(2)).as_ptr::<u8>() };
                    let other_entries = unsafe {
                        other_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };

                    // Check all elements in other are in self
                    let mut is_superset = true;
                    for i in 0..other_cap {
                        let k = unsafe { *other_entries.add(i * 2) };
                        if !k.is_unit() {
                            let h = value_hash(k);
                            let mut si = h % self_cap;
                            let sstart = si;
                            let mut in_self = false;
                            loop {
                                let sk = unsafe { *self_entries.add(si * 2) };
                                if sk.is_unit() { break; }
                                if value_eq(sk, k) { in_self = true; break; }
                                si = (si + 1) % self_cap;
                                if si == sstart { break; }
                            }
                            if !in_self {
                                is_superset = false;
                                break;
                            }
                        }
                    }
                    return Ok(Some(Value::from_bool(is_superset)));
                }
                "symmetric_difference" if is_set => {
                    // Set.symmetric_difference(other) -> Set (elements in either but not both)
                    let caller_base = state.reg_base();
                    let other_val = state.registers.get(caller_base, Reg(args.start.0));

                    let self_header = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let self_cap = unsafe { (*self_header.add(1)).as_i64() } as usize;
                    let self_entries_ptr = unsafe { (*self_header.add(2)).as_ptr::<u8>() };
                    let self_entries = unsafe {
                        self_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };

                    let other_ptr = other_val.as_ptr::<u8>();
                    let other_header = unsafe {
                        other_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let other_cap = unsafe { (*other_header.add(1)).as_i64() } as usize;
                    let other_entries_ptr = unsafe { (*other_header.add(2)).as_ptr::<u8>() };
                    let other_entries = unsafe {
                        other_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };

                    let mut elements = Vec::new();
                    // Elements in self but not in other
                    for i in 0..self_cap {
                        let k = unsafe { *self_entries.add(i * 2) };
                        if !k.is_unit() {
                            let h = value_hash(k);
                            let mut oi = h % other_cap;
                            let ostart = oi;
                            let mut in_other = false;
                            loop {
                                let ok = unsafe { *other_entries.add(oi * 2) };
                                if ok.is_unit() { break; }
                                if value_eq(ok, k) { in_other = true; break; }
                                oi = (oi + 1) % other_cap;
                                if oi == ostart { break; }
                            }
                            if !in_other { elements.push(k); }
                        }
                    }
                    // Elements in other but not in self
                    for i in 0..other_cap {
                        let k = unsafe { *other_entries.add(i * 2) };
                        if !k.is_unit() {
                            let h = value_hash(k);
                            let mut si = h % self_cap;
                            let sstart = si;
                            let mut in_self = false;
                            loop {
                                let sk = unsafe { *self_entries.add(si * 2) };
                                if sk.is_unit() { break; }
                                if value_eq(sk, k) { in_self = true; break; }
                                si = (si + 1) % self_cap;
                                if si == sstart { break; }
                            }
                            if !in_self { elements.push(k); }
                        }
                    }

                    let new_cap = (elements.len() * 2).max(16);
                    let new_obj = state.heap.alloc(TypeId::SET, 3 * std::mem::size_of::<Value>())?;
                    state.record_allocation();
                    let new_backing = state.heap.alloc_array(TypeId::UNIT, new_cap * 2)?;
                    state.record_allocation();
                    let new_backing_ptr = new_backing.as_ptr() as *mut u8;
                    let new_data = unsafe {
                        new_backing_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    for i in 0..(new_cap * 2) {
                        unsafe { *new_data.add(i) = Value::unit(); }
                    }
                    for elem in &elements {
                        let h = value_hash(*elem);
                        let mut ni = h % new_cap;
                        loop {
                            if unsafe { (*new_data.add(ni * 2)).is_unit() } {
                                unsafe {
                                    *new_data.add(ni * 2) = *elem;
                                    // Set's value-slot is unit (`Map<T, ()>`
                                    // contract; iteration branches on
                                    // source type_id).
                                    *new_data.add(ni * 2 + 1) = Value::unit();
                                }
                                break;
                            }
                            ni = (ni + 1) % new_cap;
                        }
                    }
                    let new_header = unsafe {
                        (new_obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    unsafe {
                        *new_header = Value::from_i64(elements.len() as i64);
                        *new_header.add(1) = Value::from_i64(new_cap as i64);
                        *new_header.add(2) = Value::from_ptr(new_backing_ptr);
                    }
                    return Ok(Some(Value::from_ptr(new_obj.as_ptr() as *mut u8)));
                }
                "disjoint" | "is_disjoint" if is_set => {
                    // Set.is_disjoint(other) -> Bool (no elements in common)
                    let caller_base = state.reg_base();
                    let other_val = state.registers.get(caller_base, Reg(args.start.0));

                    let self_header = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let self_cap = unsafe { (*self_header.add(1)).as_i64() } as usize;
                    let self_entries_ptr = unsafe { (*self_header.add(2)).as_ptr::<u8>() };
                    let self_entries = unsafe {
                        self_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };

                    let other_ptr = other_val.as_ptr::<u8>();
                    let other_header = unsafe {
                        other_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let other_cap = unsafe { (*other_header.add(1)).as_i64() } as usize;
                    let other_entries_ptr = unsafe { (*other_header.add(2)).as_ptr::<u8>() };
                    let other_entries = unsafe {
                        other_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };

                    let mut disjoint = true;
                    for i in 0..self_cap {
                        let k = unsafe { *self_entries.add(i * 2) };
                        if !k.is_unit() {
                            let h = value_hash(k);
                            let mut oi = h % other_cap;
                            let ostart = oi;
                            loop {
                                let ok = unsafe { *other_entries.add(oi * 2) };
                                if ok.is_unit() { break; }
                                if value_eq(ok, k) { disjoint = false; break; }
                                oi = (oi + 1) % other_cap;
                                if oi == ostart { break; }
                            }
                            if !disjoint { break; }
                        }
                    }
                    return Ok(Some(Value::from_bool(disjoint)));
                }
                "to_list" if is_set => {
                    // Set.to_list() -> List<T> (collect all elements into a list)
                    let self_header = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let self_cap = unsafe { (*self_header.add(1)).as_i64() } as usize;
                    let self_entries_ptr = unsafe { (*self_header.add(2)).as_ptr::<u8>() };
                    let self_entries = unsafe {
                        self_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let mut elems = Vec::new();
                    for i in 0..self_cap {
                        let k = unsafe { *self_entries.add(i * 2) };
                        if !k.is_unit() { elems.push(k); }
                    }
                    let result = alloc_list_from_values(state, elems)?;
                    return Ok(Some(result));
                }
                "for_each" if is_set => {
                    // Set.for_each(closure) - call closure(element) for each element
                    let caller_base = state.reg_base();
                    let closure_val = state.registers.get(caller_base, Reg(args.start.0));
                    let self_header = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let self_cap = unsafe { (*self_header.add(1)).as_i64() } as usize;
                    let self_entries_ptr = unsafe { (*self_header.add(2)).as_ptr::<u8>() };
                    let self_entries = unsafe {
                        self_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    for i in 0..self_cap {
                        let k = unsafe { *self_entries.add(i * 2) };
                        if !k.is_unit() {
                            call_closure_sync(state, closure_val, &[k])?;
                        }
                    }
                    return Ok(Some(Value::unit()));
                }
                "filter" if is_set => {
                    // Set.filter(closure) -> Set (new set with elements where closure(element) is true)
                    let caller_base = state.reg_base();
                    let closure_val = state.registers.get(caller_base, Reg(args.start.0));
                    let self_header = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let self_cap = unsafe { (*self_header.add(1)).as_i64() } as usize;
                    let self_entries_ptr = unsafe { (*self_header.add(2)).as_ptr::<u8>() };
                    let self_entries = unsafe {
                        self_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let mut kept = Vec::new();
                    for i in 0..self_cap {
                        let k = unsafe { *self_entries.add(i * 2) };
                        if !k.is_unit() {
                            let result = call_closure_sync(state, closure_val, &[k])?;
                            if result.as_bool() {
                                kept.push(k);
                            }
                        }
                    }
                    return Ok(Some(build_set_from_values(state, kept)?));
                }
                "map" if is_set => {
                    // Set.map(closure) -> Set (new set of closure(element) values)
                    let caller_base = state.reg_base();
                    let closure_val = state.registers.get(caller_base, Reg(args.start.0));
                    let self_header = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let self_cap = unsafe { (*self_header.add(1)).as_i64() } as usize;
                    let self_entries_ptr = unsafe { (*self_header.add(2)).as_ptr::<u8>() };
                    let self_entries = unsafe {
                        self_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let mut mapped = Vec::new();
                    for i in 0..self_cap {
                        let k = unsafe { *self_entries.add(i * 2) };
                        if !k.is_unit() {
                            let out = call_closure_sync(state, closure_val, &[k])?;
                            mapped.push(out);
                        }
                    }
                    return Ok(Some(build_set_from_values(state, mapped)?));
                }
                "fold" if is_set => {
                    // Set.fold(init, closure) -> U (fold over elements)
                    let caller_base = state.reg_base();
                    let mut acc = state.registers.get(caller_base, Reg(args.start.0));
                    let closure_val = state.registers.get(caller_base, Reg(args.start.0 + 1));
                    let self_header = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let self_cap = unsafe { (*self_header.add(1)).as_i64() } as usize;
                    let self_entries_ptr = unsafe { (*self_header.add(2)).as_ptr::<u8>() };
                    let self_entries = unsafe {
                        self_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    for i in 0..self_cap {
                        let k = unsafe { *self_entries.add(i * 2) };
                        if !k.is_unit() {
                            acc = call_closure_sync(state, closure_val, &[acc, k])?;
                        }
                    }
                    return Ok(Some(acc));
                }
                "count_where" if is_set => {
                    // Set.count_where(closure) -> Int
                    let caller_base = state.reg_base();
                    let closure_val = state.registers.get(caller_base, Reg(args.start.0));
                    let self_header = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let self_cap = unsafe { (*self_header.add(1)).as_i64() } as usize;
                    let self_entries_ptr = unsafe { (*self_header.add(2)).as_ptr::<u8>() };
                    let self_entries = unsafe {
                        self_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let mut count: i64 = 0;
                    for i in 0..self_cap {
                        let k = unsafe { *self_entries.add(i * 2) };
                        if !k.is_unit() {
                            let r = call_closure_sync(state, closure_val, &[k])?;
                            if r.as_bool() { count += 1; }
                        }
                    }
                    return Ok(Some(Value::from_i64(count)));
                }
                "filter_map" if is_set => {
                    // Set.filter_map(closure: fn(&T) -> Maybe<U>) -> Set<U>
                    let caller_base = state.reg_base();
                    let closure_val = state.registers.get(caller_base, Reg(args.start.0));
                    let self_header = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let self_cap = unsafe { (*self_header.add(1)).as_i64() } as usize;
                    let self_entries_ptr = unsafe { (*self_header.add(2)).as_ptr::<u8>() };
                    let self_entries = unsafe {
                        self_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let mut kept = Vec::new();
                    for i in 0..self_cap {
                        let k = unsafe { *self_entries.add(i * 2) };
                        if !k.is_unit() {
                            let maybe = call_closure_sync(state, closure_val, &[k])?;
                            // Extract Some payload from Maybe variant. None → skip.
                            if let Some(inner) = unwrap_maybe_some(maybe) {
                                kept.push(inner);
                            }
                        }
                    }
                    return Ok(Some(build_set_from_values(state, kept)?));
                }
                "for_each" if is_map => {
                    // Map.for_each(closure) - call closure(key, value) for each entry
                    let caller_base = state.reg_base();
                    let closure_val = state.registers.get(caller_base, Reg(args.start.0));
                    let header_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
                    let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };
                    let entries_data = unsafe {
                        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    for i in 0..capacity {
                        let k = unsafe { *entries_data.add(i * 2) };
                        if !k.is_unit() {
                            let v = unsafe { *entries_data.add(i * 2 + 1) };
                            call_closure_sync(state, closure_val, &[k, v])?;
                        }
                    }
                    return Ok(Some(Value::unit()));
                }
                "fold" if is_map => {
                    // Map.fold(init, closure) -> U — fold over (key, value) entries
                    let caller_base = state.reg_base();
                    let mut acc = state.registers.get(caller_base, Reg(args.start.0));
                    let closure_val = state.registers.get(caller_base, Reg(args.start.0 + 1));
                    let header_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
                    let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };
                    let entries_data = unsafe {
                        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    for i in 0..capacity {
                        let k = unsafe { *entries_data.add(i * 2) };
                        if !k.is_unit() {
                            let v = unsafe { *entries_data.add(i * 2 + 1) };
                            acc = call_closure_sync(state, closure_val, &[acc, k, v])?;
                        }
                    }
                    return Ok(Some(acc));
                }
                "filter" if is_map => {
                    // Map.filter(closure) -> Map (new map with entries where closure(k,v) is true)
                    let caller_base = state.reg_base();
                    let closure_val = state.registers.get(caller_base, Reg(args.start.0));
                    let header_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
                    let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };
                    let entries_data = unsafe {
                        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let mut kept_keys = Vec::new();
                    let mut kept_vals = Vec::new();
                    for i in 0..capacity {
                        let k = unsafe { *entries_data.add(i * 2) };
                        if !k.is_unit() {
                            let v = unsafe { *entries_data.add(i * 2 + 1) };
                            let result = call_closure_sync(state, closure_val, &[k, v])?;
                            if result.as_bool() {
                                kept_keys.push(k);
                                kept_vals.push(v);
                            }
                        }
                    }
                    // Build new map
                    let new_cap = (kept_keys.len() * 2).max(16);
                    let new_obj = state.heap.alloc(TypeId::MAP, 3 * std::mem::size_of::<Value>())?;
                    state.record_allocation();
                    let new_backing = state.heap.alloc_array(TypeId::UNIT, new_cap * 2)?;
                    state.record_allocation();
                    let new_backing_ptr = new_backing.as_ptr() as *mut u8;
                    let new_data = unsafe {
                        new_backing_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    for j in 0..(new_cap * 2) {
                        unsafe { *new_data.add(j) = Value::unit(); }
                    }
                    for j in 0..kept_keys.len() {
                        let h = value_hash(kept_keys[j]);
                        let mut ni = h % new_cap;
                        loop {
                            if unsafe { (*new_data.add(ni * 2)).is_unit() } {
                                unsafe {
                                    *new_data.add(ni * 2) = kept_keys[j];
                                    *new_data.add(ni * 2 + 1) = kept_vals[j];
                                }
                                break;
                            }
                            ni = (ni + 1) % new_cap;
                        }
                    }
                    let new_header = unsafe {
                        (new_obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    unsafe {
                        *new_header = Value::from_i64(kept_keys.len() as i64);
                        *new_header.add(1) = Value::from_i64(new_cap as i64);
                        *new_header.add(2) = Value::from_ptr(new_backing_ptr);
                    }
                    return Ok(Some(Value::from_ptr(new_obj.as_ptr() as *mut u8)));
                }
                "any" if is_map => {
                    // Map.any(closure) -> Bool (true if closure(k,v) is true for any entry)
                    let caller_base = state.reg_base();
                    let closure_val = state.registers.get(caller_base, Reg(args.start.0));
                    let header_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
                    let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };
                    let entries_data = unsafe {
                        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let mut found = false;
                    for i in 0..capacity {
                        let k = unsafe { *entries_data.add(i * 2) };
                        if !k.is_unit() {
                            let v = unsafe { *entries_data.add(i * 2 + 1) };
                            let result = call_closure_sync(state, closure_val, &[k, v])?;
                            if result.as_bool() { found = true; break; }
                        }
                    }
                    return Ok(Some(Value::from_bool(found)));
                }
                "all" if is_map => {
                    // Map.all(closure) -> Bool (true if closure(k,v) is true for all entries)
                    let caller_base = state.reg_base();
                    let closure_val = state.registers.get(caller_base, Reg(args.start.0));
                    let header_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
                    let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };
                    let entries_data = unsafe {
                        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let mut all_match = true;
                    for i in 0..capacity {
                        let k = unsafe { *entries_data.add(i * 2) };
                        if !k.is_unit() {
                            let v = unsafe { *entries_data.add(i * 2 + 1) };
                            let result = call_closure_sync(state, closure_val, &[k, v])?;
                            if !result.as_bool() { all_match = false; break; }
                        }
                    }
                    return Ok(Some(Value::from_bool(all_match)));
                }
                "find" if is_map => {
                    // Map.find(closure) -> Maybe<(K,V)> (first entry where closure(k,v) is true)
                    let caller_base = state.reg_base();
                    let closure_val = state.registers.get(caller_base, Reg(args.start.0));
                    let header_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
                    let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };
                    let entries_data = unsafe {
                        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    for i in 0..capacity {
                        let k = unsafe { *entries_data.add(i * 2) };
                        if !k.is_unit() {
                            let v = unsafe { *entries_data.add(i * 2 + 1) };
                            let result = call_closure_sync(state, closure_val, &[k, v])?;
                            if result.as_bool() {
                                // Return Some((k, v)) - allocate tuple
                                let tuple_obj = state.heap.alloc(TypeId::TUPLE, 2 * std::mem::size_of::<Value>())?;
                                state.record_allocation();
                                let tuple_data = unsafe {
                                    (tuple_obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
                                };
                                unsafe {
                                    *tuple_data = k;
                                    *tuple_data.add(1) = v;
                                }
                                let tuple_val = Value::from_ptr(tuple_obj.as_ptr() as *mut u8);
                                let result = make_some_value(state, tuple_val)?;
                                return Ok(Some(result));
                            }
                        }
                    }
                    let result = make_none_value(state)?;
                    return Ok(Some(result));
                }
                "retain" if is_map => {
                    // Map.retain(closure) - keep only entries where closure(k,v) is true (mutating)
                    let caller_base = state.reg_base();
                    let closure_val = state.registers.get(caller_base, Reg(args.start.0));
                    let header_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    let mut count = unsafe { (*header_ptr).as_i64() } as usize;
                    let capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
                    let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };
                    let entries_data = unsafe {
                        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    for i in 0..capacity {
                        let k = unsafe { *entries_data.add(i * 2) };
                        if !k.is_unit() {
                            let v = unsafe { *entries_data.add(i * 2 + 1) };
                            let result = call_closure_sync(state, closure_val, &[k, v])?;
                            if !result.as_bool() {
                                // Remove this entry
                                unsafe {
                                    *entries_data.add(i * 2) = Value::unit();
                                    *entries_data.add(i * 2 + 1) = Value::unit();
                                }
                                count -= 1;
                            }
                        }
                    }
                    unsafe { *header_ptr = Value::from_i64(count as i64); }
                    return Ok(Some(Value::unit()));
                }
                "retain" if is_set => {
                    // Set.retain(closure) - keep only elements where closure(elem) is true (mutating)
                    let caller_base = state.reg_base();
                    let closure_val = state.registers.get(caller_base, Reg(args.start.0));
                    let header_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    let mut count = unsafe { (*header_ptr).as_i64() } as usize;
                    let capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
                    let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };
                    let entries_data = unsafe {
                        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    for i in 0..capacity {
                        let k = unsafe { *entries_data.add(i * 2) };
                        if !k.is_unit() {
                            let result = call_closure_sync(state, closure_val, &[k])?;
                            if !result.as_bool() {
                                unsafe {
                                    *entries_data.add(i * 2) = Value::unit();
                                    *entries_data.add(i * 2 + 1) = Value::unit();
                                }
                                count -= 1;
                            }
                        }
                    }
                    unsafe { *header_ptr = Value::from_i64(count as i64); }
                    return Ok(Some(Value::unit()));
                }
                "map_values" if is_map => {
                    // Map.map_values(closure) -> Map (new map with values transformed by closure(k,v))
                    let caller_base = state.reg_base();
                    let closure_val = state.registers.get(caller_base, Reg(args.start.0));
                    let header_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
                    let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };
                    let entries_data = unsafe {
                        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let mut keys = Vec::new();
                    let mut new_vals = Vec::new();
                    for i in 0..capacity {
                        let k = unsafe { *entries_data.add(i * 2) };
                        if !k.is_unit() {
                            let v = unsafe { *entries_data.add(i * 2 + 1) };
                            let mapped = call_closure_sync(state, closure_val, &[k, v])?;
                            keys.push(k);
                            new_vals.push(mapped);
                        }
                    }
                    let new_cap = (keys.len() * 2).max(16);
                    let new_obj = state.heap.alloc(TypeId::MAP, 3 * std::mem::size_of::<Value>())?;
                    state.record_allocation();
                    let new_backing = state.heap.alloc_array(TypeId::UNIT, new_cap * 2)?;
                    state.record_allocation();
                    let new_backing_ptr = new_backing.as_ptr() as *mut u8;
                    let new_data = unsafe {
                        new_backing_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    for j in 0..(new_cap * 2) {
                        unsafe { *new_data.add(j) = Value::unit(); }
                    }
                    for j in 0..keys.len() {
                        let h = value_hash(keys[j]);
                        let mut ni = h % new_cap;
                        loop {
                            if unsafe { (*new_data.add(ni * 2)).is_unit() } {
                                unsafe {
                                    *new_data.add(ni * 2) = keys[j];
                                    *new_data.add(ni * 2 + 1) = new_vals[j];
                                }
                                break;
                            }
                            ni = (ni + 1) % new_cap;
                        }
                    }
                    let new_header = unsafe {
                        (new_obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    unsafe {
                        *new_header = Value::from_i64(keys.len() as i64);
                        *new_header.add(1) = Value::from_i64(new_cap as i64);
                        *new_header.add(2) = Value::from_ptr(new_backing_ptr);
                    }
                    return Ok(Some(Value::from_ptr(new_obj.as_ptr() as *mut u8)));
                }
                "contains_value" if is_map => {
                    // Map.contains_value(val) -> Bool (check if any entry has this value)
                    let caller_base = state.reg_base();
                    let target = state.registers.get(caller_base, Reg(args.start.0));
                    let header_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
                    let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };
                    let entries_data = unsafe {
                        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let mut found = false;
                    for i in 0..capacity {
                        let k = unsafe { *entries_data.add(i * 2) };
                        if !k.is_unit() {
                            let v = unsafe { *entries_data.add(i * 2 + 1) };
                            if value_eq(v, target) { found = true; break; }
                        }
                    }
                    return Ok(Some(Value::from_bool(found)));
                }
                "count" if is_map || is_set => {
                    // Alias for len()
                    let data_ptr = unsafe {
                        ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let count = unsafe { (*data_ptr).as_i64() } as usize;
                    return Ok(Some(Value::from_i64(count as i64)));
                }
                _ => {} // fall through
            }
        }

        // ============================================================
        // Deque methods (ring buffer: [data(0), head(1), len(2), cap(3)])
        // Layout matches stdlib: type Deque<T> is { data, head, len, cap }
        // ============================================================
        let is_deque = header.type_id == TypeId::DEQUE;
        if is_deque {
            let header_ptr = unsafe {
                ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
            };
            // Field indices matching stdlib layout
            const DEQUE_DATA: usize = 0;
            const DEQUE_HEAD: usize = 1;
            const DEQUE_LEN: usize = 2;
            const DEQUE_CAP: usize = 3;
            match method {
                "len" | "count" => {
                    let len = unsafe { (*header_ptr.add(DEQUE_LEN)).as_i64() };
                    return Ok(Some(Value::from_i64(len)));
                }
                "is_empty" => {
                    let len = unsafe { (*header_ptr.add(DEQUE_LEN)).as_i64() };
                    return Ok(Some(Value::from_bool(len == 0)));
                }
                "push_back" => {
                    let caller_base = state.reg_base();
                    let val = state.registers.get(caller_base, Reg(args.start.0));
                    let mut len = unsafe { (*header_ptr.add(DEQUE_LEN)).as_i64() } as usize;
                    let cap = unsafe { (*header_ptr.add(DEQUE_CAP)).as_i64() } as usize;
                    let head = unsafe { (*header_ptr.add(DEQUE_HEAD)).as_i64() } as usize;
                    let buf_ptr = unsafe { (*header_ptr.add(DEQUE_DATA)).as_ptr::<u8>() };
                    let mut buf_data = unsafe {
                        buf_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };

                    // Grow if full
                    if len >= cap {
                        let new_cap = (cap * 2).max(16);
                        let new_buf = state.heap.alloc_array(TypeId::UNIT, new_cap)?;
                        state.record_allocation();
                        let new_buf_ptr = new_buf.as_ptr() as *mut u8;
                        let new_buf_data = unsafe {
                            new_buf_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                        };
                        for i in 0..len {
                            let src_idx = (head + i) % cap;
                            unsafe { *new_buf_data.add(i) = *buf_data.add(src_idx); }
                        }
                        for i in len..new_cap {
                            unsafe { *new_buf_data.add(i) = Value::unit(); }
                        }
                        buf_data = new_buf_data;
                        unsafe {
                            *header_ptr.add(DEQUE_CAP) = Value::from_i64(new_cap as i64);
                            *header_ptr.add(DEQUE_HEAD) = Value::from_i64(0);
                            *header_ptr.add(DEQUE_DATA) = Value::from_ptr(new_buf_ptr);
                        }
                        let tail = len;
                        unsafe { *buf_data.add(tail) = val; }
                    } else {
                        let tail = (head + len) % cap;
                        unsafe { *buf_data.add(tail) = val; }
                    }
                    len += 1;
                    unsafe { *header_ptr.add(DEQUE_LEN) = Value::from_i64(len as i64); }
                    return Ok(Some(Value::unit()));
                }
                "push_front" => {
                    let caller_base = state.reg_base();
                    let val = state.registers.get(caller_base, Reg(args.start.0));
                    let len = unsafe { (*header_ptr.add(DEQUE_LEN)).as_i64() } as usize;
                    let mut cap = unsafe { (*header_ptr.add(DEQUE_CAP)).as_i64() } as usize;
                    let mut head = unsafe { (*header_ptr.add(DEQUE_HEAD)).as_i64() } as usize;
                    let buf_ptr = unsafe { (*header_ptr.add(DEQUE_DATA)).as_ptr::<u8>() };
                    let mut buf_data = unsafe {
                        buf_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };

                    // Grow if full
                    if len >= cap {
                        let new_cap = (cap * 2).max(16);
                        let new_buf = state.heap.alloc_array(TypeId::UNIT, new_cap)?;
                        state.record_allocation();
                        let new_buf_ptr = new_buf.as_ptr() as *mut u8;
                        let new_buf_data = unsafe {
                            new_buf_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                        };
                        for i in 0..len {
                            let src_idx = (head + i) % cap;
                            unsafe { *new_buf_data.add(i) = *buf_data.add(src_idx); }
                        }
                        for i in len..new_cap {
                            unsafe { *new_buf_data.add(i) = Value::unit(); }
                        }
                        cap = new_cap;
                        buf_data = new_buf_data;
                        head = 0;
                        unsafe {
                            *header_ptr.add(DEQUE_CAP) = Value::from_i64(new_cap as i64);
                            *header_ptr.add(DEQUE_DATA) = Value::from_ptr(new_buf_ptr);
                        }
                    }
                    // Move head backward
                    head = if head == 0 { cap - 1 } else { head - 1 };
                    unsafe {
                        *buf_data.add(head) = val;
                        *header_ptr.add(DEQUE_LEN) = Value::from_i64((len + 1) as i64);
                        *header_ptr.add(DEQUE_HEAD) = Value::from_i64(head as i64);
                    }
                    return Ok(Some(Value::unit()));
                }
                "pop_back" => {
                    let len = unsafe { (*header_ptr.add(DEQUE_LEN)).as_i64() } as usize;
                    if len == 0 {
                        let result = make_none_value(state)?;
                        return Ok(Some(result));
                    }
                    let cap = unsafe { (*header_ptr.add(DEQUE_CAP)).as_i64() } as usize;
                    let head = unsafe { (*header_ptr.add(DEQUE_HEAD)).as_i64() } as usize;
                    let buf_ptr = unsafe { (*header_ptr.add(DEQUE_DATA)).as_ptr::<u8>() };
                    let buf_data = unsafe {
                        buf_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    let tail_idx = (head + len - 1) % cap;
                    let val = unsafe { *buf_data.add(tail_idx) };
                    unsafe {
                        *buf_data.add(tail_idx) = Value::unit();
                        *header_ptr.add(DEQUE_LEN) = Value::from_i64((len - 1) as i64);
                    }
                    let result = make_some_value(state, val)?;
                    return Ok(Some(result));
                }
                "pop_front" => {
                    let len = unsafe { (*header_ptr.add(DEQUE_LEN)).as_i64() } as usize;
                    if len == 0 {
                        let result = make_none_value(state)?;
                        return Ok(Some(result));
                    }
                    let cap = unsafe { (*header_ptr.add(DEQUE_CAP)).as_i64() } as usize;
                    let head = unsafe { (*header_ptr.add(DEQUE_HEAD)).as_i64() } as usize;
                    let buf_ptr = unsafe { (*header_ptr.add(DEQUE_DATA)).as_ptr::<u8>() };
                    let buf_data = unsafe {
                        buf_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    let val = unsafe { *buf_data.add(head) };
                    let new_head = (head + 1) % cap;
                    unsafe {
                        *buf_data.add(head) = Value::unit();
                        *header_ptr.add(DEQUE_LEN) = Value::from_i64((len - 1) as i64);
                        *header_ptr.add(DEQUE_HEAD) = Value::from_i64(new_head as i64);
                    }
                    let result = make_some_value(state, val)?;
                    return Ok(Some(result));
                }
                "front" | "first" => {
                    let len = unsafe { (*header_ptr.add(DEQUE_LEN)).as_i64() } as usize;
                    if len == 0 {
                        let result = make_none_value(state)?;
                        return Ok(Some(result));
                    }
                    let head = unsafe { (*header_ptr.add(DEQUE_HEAD)).as_i64() } as usize;
                    let buf_ptr = unsafe { (*header_ptr.add(DEQUE_DATA)).as_ptr::<u8>() };
                    let buf_data = unsafe {
                        buf_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let val = unsafe { *buf_data.add(head) };
                    let result = make_some_value(state, val)?;
                    return Ok(Some(result));
                }
                "back" | "last" => {
                    let len = unsafe { (*header_ptr.add(DEQUE_LEN)).as_i64() } as usize;
                    if len == 0 {
                        let result = make_none_value(state)?;
                        return Ok(Some(result));
                    }
                    let cap = unsafe { (*header_ptr.add(DEQUE_CAP)).as_i64() } as usize;
                    let head = unsafe { (*header_ptr.add(DEQUE_HEAD)).as_i64() } as usize;
                    let buf_ptr = unsafe { (*header_ptr.add(DEQUE_DATA)).as_ptr::<u8>() };
                    let buf_data = unsafe {
                        buf_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let tail_idx = (head + len - 1) % cap;
                    let val = unsafe { *buf_data.add(tail_idx) };
                    let result = make_some_value(state, val)?;
                    return Ok(Some(result));
                }
                "get" => {
                    let caller_base = state.reg_base();
                    let idx = state.registers.get(caller_base, Reg(args.start.0)).as_i64() as usize;
                    let len = unsafe { (*header_ptr.add(DEQUE_LEN)).as_i64() } as usize;
                    if idx >= len {
                        let result = make_none_value(state)?;
                        return Ok(Some(result));
                    }
                    let cap = unsafe { (*header_ptr.add(DEQUE_CAP)).as_i64() } as usize;
                    let head = unsafe { (*header_ptr.add(DEQUE_HEAD)).as_i64() } as usize;
                    let buf_ptr = unsafe { (*header_ptr.add(DEQUE_DATA)).as_ptr::<u8>() };
                    let buf_data = unsafe {
                        buf_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let actual_idx = (head + idx) % cap;
                    let val = unsafe { *buf_data.add(actual_idx) };
                    let result = make_some_value(state, val)?;
                    return Ok(Some(result));
                }
                "clear" => {
                    let cap = unsafe { (*header_ptr.add(DEQUE_CAP)).as_i64() } as usize;
                    let buf_ptr = unsafe { (*header_ptr.add(DEQUE_DATA)).as_ptr::<u8>() };
                    let buf_data = unsafe {
                        buf_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    for i in 0..cap {
                        unsafe { *buf_data.add(i) = Value::unit(); }
                    }
                    unsafe {
                        *header_ptr.add(DEQUE_LEN) = Value::from_i64(0);
                        *header_ptr.add(DEQUE_HEAD) = Value::from_i64(0);
                    }
                    return Ok(Some(Value::unit()));
                }
                "to_list" => {
                    let len = unsafe { (*header_ptr.add(DEQUE_LEN)).as_i64() } as usize;
                    let cap = unsafe { (*header_ptr.add(DEQUE_CAP)).as_i64() } as usize;
                    let head = unsafe { (*header_ptr.add(DEQUE_HEAD)).as_i64() } as usize;
                    let buf_ptr = unsafe { (*header_ptr.add(DEQUE_DATA)).as_ptr::<u8>() };
                    let buf_data = unsafe {
                        buf_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let mut elems = Vec::with_capacity(len);
                    for i in 0..len {
                        let actual_idx = (head + i) % cap;
                        elems.push(unsafe { *buf_data.add(actual_idx) });
                    }
                    let result = alloc_list_from_values(state, elems)?;
                    return Ok(Some(result));
                }
                _ => {} // fall through
            }
        }

        // ============================================================
        // Channel methods (bounded queue: [len, cap, head, buffer_ptr, closed])
        // ============================================================
        let is_channel = header.type_id == TypeId::CHANNEL;
        if is_channel {
            let header_ptr = unsafe {
                ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
            };
            match method {
                "send" => {
                    // Channel.send(val) -> Bool (true if sent, false if closed/full)
                    let caller_base = state.reg_base();
                    let val = state.registers.get(caller_base, Reg(args.start.0));
                    let closed = unsafe { (*header_ptr.add(4)).as_i64() };
                    if closed != 0 {
                        return Ok(Some(Value::from_bool(false)));
                    }
                    let len = unsafe { (*header_ptr).as_i64() } as usize;
                    let cap = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
                    if len >= cap {
                        return Ok(Some(Value::from_bool(false))); // full
                    }
                    let head = unsafe { (*header_ptr.add(2)).as_i64() } as usize;
                    let buf_ptr = unsafe { (*header_ptr.add(3)).as_ptr::<u8>() };
                    let buf_data = unsafe {
                        buf_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    let tail = (head + len) % cap;
                    unsafe {
                        *buf_data.add(tail) = val;
                        *header_ptr = Value::from_i64((len + 1) as i64);
                    }
                    return Ok(Some(Value::from_bool(true)));
                }
                "recv" | "receive" => {
                    // Channel.recv() -> Maybe<T>
                    let len = unsafe { (*header_ptr).as_i64() } as usize;
                    if len == 0 {
                        let result = make_none_value(state)?;
                        return Ok(Some(result));
                    }
                    let cap = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
                    let head = unsafe { (*header_ptr.add(2)).as_i64() } as usize;
                    let buf_ptr = unsafe { (*header_ptr.add(3)).as_ptr::<u8>() };
                    let buf_data = unsafe {
                        buf_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    let val = unsafe { *buf_data.add(head) };
                    let new_head = (head + 1) % cap;
                    unsafe {
                        *buf_data.add(head) = Value::unit();
                        *header_ptr = Value::from_i64((len - 1) as i64);
                        *header_ptr.add(2) = Value::from_i64(new_head as i64);
                    }
                    let result = make_some_value(state, val)?;
                    return Ok(Some(result));
                }
                "close" => {
                    // Channel.close() - mark channel as closed
                    unsafe { *header_ptr.add(4) = Value::from_i64(1); }
                    return Ok(Some(Value::unit()));
                }
                "is_closed" => {
                    let closed = unsafe { (*header_ptr.add(4)).as_i64() };
                    return Ok(Some(Value::from_bool(closed != 0)));
                }
                "len" | "count" => {
                    let len = unsafe { (*header_ptr).as_i64() };
                    return Ok(Some(Value::from_i64(len)));
                }
                "is_empty" => {
                    let len = unsafe { (*header_ptr).as_i64() };
                    return Ok(Some(Value::from_bool(len == 0)));
                }
                "is_full" => {
                    let len = unsafe { (*header_ptr).as_i64() } as usize;
                    let cap = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
                    return Ok(Some(Value::from_bool(len >= cap)));
                }
                "capacity" => {
                    let cap = unsafe { (*header_ptr.add(1)).as_i64() };
                    return Ok(Some(Value::from_i64(cap)));
                }
                _ => {} // fall through
            }
        }

        // Stopwatch methods (struct: field 0=start, field 1=running, field 2=accumulated)
        // DeadlineTimer methods (struct: field 0=deadline, field 1=has_deadline)
        let field_count = header.size as usize / std::mem::size_of::<Value>();
        match method {
            "elapsed" if field_count == 3 => {
                // Stopwatch.elapsed(): if running: accumulated + (now - start), else: accumulated
                let start_val = unsafe {
                    *(ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value)
                };
                let running_val = unsafe {
                    *(ptr.add(heap::OBJECT_HEADER_SIZE + std::mem::size_of::<Value>()) as *const Value)
                };
                let acc_val = unsafe {
                    *(ptr.add(heap::OBJECT_HEADER_SIZE + 2 * std::mem::size_of::<Value>()) as *const Value)
                };
                let acc = acc_val.as_i64();
                if running_val.as_bool() {
                    let start = start_val.as_i64();
                    let now = monotonic_nanos_shared();
                    return Ok(Some(Value::from_i64(acc + (now - start).max(0))));
                } else {
                    return Ok(Some(Value::from_i64(acc)));
                }
            }
            "stop" if field_count == 3 => {
                // Stopwatch.stop(): if running: accumulated += (now - start), running = false
                let start_val = unsafe {
                    *(ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value)
                };
                let running_val = unsafe {
                    *(ptr.add(heap::OBJECT_HEADER_SIZE + std::mem::size_of::<Value>()) as *const Value)
                };
                let acc_val = unsafe {
                    *(ptr.add(heap::OBJECT_HEADER_SIZE + 2 * std::mem::size_of::<Value>()) as *const Value)
                };
                if running_val.as_bool() {
                    let start = start_val.as_i64();
                    let now = monotonic_nanos_shared();
                    let new_acc = acc_val.as_i64() + (now - start).max(0);
                    // Update accumulated (field 2)
                    unsafe {
                        *(ptr.add(heap::OBJECT_HEADER_SIZE + 2 * std::mem::size_of::<Value>()) as *mut Value) = Value::from_i64(new_acc);
                    }
                    // Set running = false (field 1)
                    unsafe {
                        *(ptr.add(heap::OBJECT_HEADER_SIZE + std::mem::size_of::<Value>()) as *mut Value) = Value::from_bool(false);
                    }
                }
                return Ok(Some(Value::unit()));
            }
            "start" if field_count == 3 => {
                // Stopwatch.start(): if !running: start = now, running = true
                let running_val = unsafe {
                    *(ptr.add(heap::OBJECT_HEADER_SIZE + std::mem::size_of::<Value>()) as *const Value)
                };
                if !running_val.as_bool() {
                    let now = monotonic_nanos_shared();
                    // Set start = now (field 0)
                    unsafe {
                        *(ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value) = Value::from_i64(now);
                    }
                    // Set running = true (field 1)
                    unsafe {
                        *(ptr.add(heap::OBJECT_HEADER_SIZE + std::mem::size_of::<Value>()) as *mut Value) = Value::from_bool(true);
                    }
                }
                return Ok(Some(Value::unit()));
            }
            "reset" if field_count == 3 => {
                // Stopwatch.reset(): start = 0, running = false, accumulated = 0
                unsafe {
                    *(ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value) = Value::from_i64(0);
                    *(ptr.add(heap::OBJECT_HEADER_SIZE + std::mem::size_of::<Value>()) as *mut Value) = Value::from_bool(false);
                    *(ptr.add(heap::OBJECT_HEADER_SIZE + 2 * std::mem::size_of::<Value>()) as *mut Value) = Value::from_i64(0);
                }
                return Ok(Some(Value::unit()));
            }
            "is_expired" if field_count == 2 => {
                // DeadlineTimer.is_expired(): now >= deadline
                let deadline_val = unsafe {
                    *(ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value)
                };
                let now = monotonic_nanos_shared();
                return Ok(Some(Value::from_bool(now >= deadline_val.as_i64())));
            }
            "remaining" if field_count == 2 => {
                // DeadlineTimer.remaining(): max(deadline - now, 0)
                let deadline_val = unsafe {
                    *(ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value)
                };
                let now = monotonic_nanos_shared();
                return Ok(Some(Value::from_i64((deadline_val.as_i64() - now).max(0))));
            }
            _ => {} // fall through
        }
    }

    // Raw pointer methods (pointer arithmetic, CBGR generation)
    if receiver.is_ptr() && !receiver.is_nil() {
        let ptr_addr = receiver.as_ptr::<u8>() as usize;
        match method {
            "sub" | "byte_sub" => {
                let n = state.get_reg(Reg(args.start.0)).as_i64() as usize;
                return Ok(Some(Value::from_ptr(ptr_addr.wrapping_sub(n) as *mut u8)));
            }
            "add" | "byte_add" => {
                let n = state.get_reg(Reg(args.start.0)).as_i64() as usize;
                return Ok(Some(Value::from_ptr(ptr_addr.wrapping_add(n) as *mut u8)));
            }
            "offset" => {
                let n = state.get_reg(Reg(args.start.0)).as_i64();
                return Ok(Some(Value::from_ptr(
                    (ptr_addr as isize).wrapping_add(n as isize) as usize as *mut u8,
                )));
            }
            "is_null" => {
                return Ok(Some(Value::from_bool(ptr_addr == 0)));
            }
            "read" => {
                let val = unsafe { *(ptr_addr as *const Value) };
                return Ok(Some(val));
            }
            "write" => {
                let val = state.get_reg(Reg(args.start.0));
                unsafe { *(ptr_addr as *mut Value) = val; }
                return Ok(Some(Value::unit()));
            }
            "generation" | "stored_generation" => {
                // For CBGR data pointers: go back 32 bytes to the AllocationHeader,
                // read u32 at offset 8 (generation field)
                // Layout: [size:4][align:4][generation:4][epoch:2][caps:2][type_id:4][flags:4][reserved:8]
                let header_addr = ptr_addr.wrapping_sub(32);
                if state.cbgr_allocations.contains(&header_addr) {
                    let generation = unsafe { *((header_addr + 8) as *const u32) };
                    return Ok(Some(Value::from_i64(generation as i64)));
                }
                // Fall through for non-CBGR pointers
            }
            "is_valid" => {
                // Check if the CBGR allocation is still valid (not freed)
                // Layout: [size:4][align:4][generation:4][epoch:2][caps:2][type_id:4][flags:4][reserved:8]
                let header_addr = ptr_addr.wrapping_sub(32);
                if state.cbgr_allocations.contains(&header_addr) {
                    let flags = unsafe { *((header_addr + 20) as *const u32) };
                    return Ok(Some(Value::from_bool(flags & 1 == 0)));
                }
                // Non-CBGR pointer: always valid
                return Ok(Some(Value::from_bool(true)));
            }
            "epoch" => {
                // References capture epoch at creation time (current epoch)
                // For Heap pointers called via .epoch(), return allocation-time epoch
                // from AllocationHeader for the allocation itself, but current epoch
                // for reference-style access
                // Layout: [size:4][align:4][generation:4][epoch:2][caps:2][type_id:4][flags:4][reserved:8]
                let header_addr = ptr_addr.wrapping_sub(32);
                if state.cbgr_allocations.contains(&header_addr) {
                    let epoch = unsafe { *((header_addr + 12) as *const u16) };
                    return Ok(Some(Value::from_i64(epoch as i64)));
                }
                return Ok(Some(Value::from_i64(state.cbgr_epoch as i64)));
            }
            "epoch_caps" | "epoch_caps_raw" | "raw_epoch_caps" => {
                // Return packed epoch + capabilities (u32)
                // References capture epoch at CREATION time (current epoch), not allocation time
                // Encoding: ((epoch & 0x00FF_FFFF) << 8) | capabilities
                let epoch = state.cbgr_epoch as u32;
                let is_mut = state.cbgr_mutable_ptrs.contains(&ptr_addr);
                let cap_bits: u32 = if is_mut { 0x03 } else { 0x01 };
                let packed = ((epoch & 0x00FF_FFFF) << 8) | cap_bits;
                return Ok(Some(Value::from_i64(packed as i64)));
            }
            "raw_ptr" => {
                // Return the raw pointer address as Int
                return Ok(Some(Value::from_i64(ptr_addr as i64)));
            }
            "capabilities" => {
                let epoch = state.cbgr_epoch as u32;
                let is_mut = state.cbgr_mutable_ptrs.contains(&ptr_addr);
                let cap_bits: u32 = if is_mut { 0x03 } else { 0x01 };
                let packed = ((epoch & 0x00FF_FFFF) << 8) | cap_bits;
                return Ok(Some(Value::from_i64(packed as i64)));
            }
            "can_read" => {
                return Ok(Some(Value::from_bool(true)));
            }
            "can_write" => {
                let is_mut = state.cbgr_mutable_ptrs.contains(&ptr_addr);
                return Ok(Some(Value::from_bool(is_mut)));
            }
            _ => {} // Fall through to other pointer dispatch (arrays, etc.)
        }
    }

    // Text/String methods
    if receiver.is_small_string() || is_heap_string(receiver) {
        let text = extract_string(receiver, state);
        match method {
            "len" => {
                return Ok(Some(Value::from_i64(text.len() as i64)));
            }
            "char_len" => {
                return Ok(Some(Value::from_i64(text.chars().count() as i64)));
            }
            "is_empty" => {
                return Ok(Some(Value::from_bool(text.is_empty())));
            }
            "contains" => {
                let arg = state.get_reg(Reg(args.start.0));
                let needle = extract_string(&arg, state);
                return Ok(Some(Value::from_bool(text.contains(&*needle))));
            }
            "starts_with" => {
                let arg = state.get_reg(Reg(args.start.0));
                let prefix = extract_string(&arg, state);
                return Ok(Some(Value::from_bool(text.starts_with(&*prefix))));
            }
            "ends_with" => {
                let arg = state.get_reg(Reg(args.start.0));
                let suffix = extract_string(&arg, state);
                return Ok(Some(Value::from_bool(text.ends_with(&*suffix))));
            }
            "trim" => {
                let trimmed = text.trim();
                return Ok(Some(alloc_string_value(state, trimmed)?));
            }
            "trim_start" => {
                let trimmed = text.trim_start();
                return Ok(Some(alloc_string_value(state, trimmed)?));
            }
            "trim_end" => {
                let trimmed = text.trim_end();
                return Ok(Some(alloc_string_value(state, trimmed)?));
            }
            "to_uppercase" => {
                let upper = text.to_uppercase();
                return Ok(Some(alloc_string_value(state, &upper)?));
            }
            "to_lowercase" => {
                let lower = text.to_lowercase();
                return Ok(Some(alloc_string_value(state, &lower)?));
            }
            "replace" => {
                let from_arg = state.get_reg(Reg(args.start.0));
                let to_arg = state.get_reg(Reg(args.start.0 + 1));
                let from = extract_string(&from_arg, state);
                let to = extract_string(&to_arg, state);
                let replaced = text.replace(&*from, &to);
                return Ok(Some(alloc_string_value(state, &replaced)?));
            }
            "split" => {
                let sep_arg = state.get_reg(Reg(args.start.0));
                let sep = extract_string(&sep_arg, state);
                let parts: Vec<String> = text.split(&*sep).map(|s| s.to_string()).collect();
                let mut values = Vec::with_capacity(parts.len());
                for part in &parts {
                    values.push(alloc_string_value(state, part)?);
                }
                return Ok(Some(alloc_list_from_values(state, values)?));
            }
            "substring" | "slice" => {
                let start_idx = state.get_reg(Reg(args.start.0)).as_i64() as usize;
                let end_idx = if args.count > 1 {
                    state.get_reg(Reg(args.start.0 + 1)).as_i64() as usize
                } else {
                    text.len()
                };
                // Clamp indices to valid byte boundaries
                let start_clamped = start_idx.min(text.len());
                let end_clamped = end_idx.min(text.len());
                if start_clamped <= end_clamped {
                    // Find valid UTF-8 boundaries
                    let actual_start = if text.is_char_boundary(start_clamped) {
                        start_clamped
                    } else {
                        // Scan forward to next char boundary
                        (start_clamped..=end_clamped).find(|&i| text.is_char_boundary(i)).unwrap_or(end_clamped)
                    };
                    let actual_end = if text.is_char_boundary(end_clamped) {
                        end_clamped
                    } else {
                        // Scan backward to previous char boundary
                        (actual_start..=end_clamped).rev().find(|&i| text.is_char_boundary(i)).unwrap_or(actual_start)
                    };
                    let sub = &text[actual_start..actual_end];
                    return Ok(Some(alloc_string_value(state, sub)?));
                } else {
                    return Ok(Some(alloc_string_value(state, "")?));
                }
            }
            "find" => {
                let needle_arg = state.get_reg(Reg(args.start.0));
                let needle = extract_string(&needle_arg, state);
                match text.find(&*needle) {
                    Some(idx) => {
                        let val = Value::from_i64(idx as i64);
                        return Ok(Some(make_some_value(state, val)?));
                    }
                    None => {
                        return Ok(Some(make_none_value(state)?));
                    }
                }
            }
            "rfind" => {
                let needle_arg = state.get_reg(Reg(args.start.0));
                let needle = extract_string(&needle_arg, state);
                match text.rfind(&*needle) {
                    Some(idx) => {
                        let val = Value::from_i64(idx as i64);
                        return Ok(Some(make_some_value(state, val)?));
                    }
                    None => {
                        return Ok(Some(make_none_value(state)?));
                    }
                }
            }
            "repeat" => {
                let n = state.get_reg(Reg(args.start.0)).as_i64();
                let repeated = text.repeat(n.max(0) as usize);
                return Ok(Some(alloc_string_value(state, &repeated)?));
            }
            "chars" => {
                let mut values = Vec::with_capacity(text.len());
                for ch in text.chars() {
                    let mut buf = [0u8; 4];
                    let s = ch.encode_utf8(&mut buf);
                    values.push(alloc_string_value(state, s)?);
                }
                return Ok(Some(alloc_list_from_values(state, values)?));
            }
            "pad_left" | "pad_start" => {
                let width = state.get_reg(Reg(args.start.0)).as_i64() as usize;
                let pad_char = if args.count > 1 {
                    let pad_arg = state.get_reg(Reg(args.start.0 + 1));
                    let pad_str = extract_string(&pad_arg, state);
                    pad_str.chars().next().unwrap_or(' ')
                } else {
                    ' '
                };
                let char_count = text.chars().count();
                if char_count >= width {
                    return Ok(Some(alloc_string_value(state, &text)?));
                } else {
                    let padding: String = std::iter::repeat_n(pad_char, width - char_count).collect();
                    let padded = format!("{}{}", padding, text);
                    return Ok(Some(alloc_string_value(state, &padded)?));
                }
            }
            "pad_right" | "pad_end" => {
                let width = state.get_reg(Reg(args.start.0)).as_i64() as usize;
                let pad_char = if args.count > 1 {
                    let pad_arg = state.get_reg(Reg(args.start.0 + 1));
                    let pad_str = extract_string(&pad_arg, state);
                    pad_str.chars().next().unwrap_or(' ')
                } else {
                    ' '
                };
                let char_count = text.chars().count();
                if char_count >= width {
                    return Ok(Some(alloc_string_value(state, &text)?));
                } else {
                    let padding: String = std::iter::repeat_n(pad_char, width - char_count).collect();
                    let padded = format!("{}{}", text, padding);
                    return Ok(Some(alloc_string_value(state, &padded)?));
                }
            }
            "to_int" => {
                match text.trim().parse::<i64>() {
                    Ok(n) => {
                        return Ok(Some(make_some_value(state, Value::from_i64(n))?));
                    }
                    Err(_) => {
                        return Ok(Some(make_none_value(state)?));
                    }
                }
            }
            "to_float" => {
                match text.trim().parse::<f64>() {
                    Ok(f) => {
                        return Ok(Some(make_some_value(state, Value::from_f64(f))?));
                    }
                    Err(_) => {
                        return Ok(Some(make_none_value(state)?));
                    }
                }
            }
            "reverse" => {
                let reversed: String = text.chars().rev().collect();
                return Ok(Some(alloc_string_value(state, &reversed)?));
            }
            "eq" => {
                let arg = state.get_reg(Reg(args.start.0));
                let other = extract_string(&arg, state);
                return Ok(Some(Value::from_bool(text == other)));
            }
            "ne" => {
                let arg = state.get_reg(Reg(args.start.0));
                let other = extract_string(&arg, state);
                return Ok(Some(Value::from_bool(text != other)));
            }
            "lt" => {
                let arg = state.get_reg(Reg(args.start.0));
                let other = extract_string(&arg, state);
                return Ok(Some(Value::from_bool(text < other)));
            }
            "le" => {
                let arg = state.get_reg(Reg(args.start.0));
                let other = extract_string(&arg, state);
                return Ok(Some(Value::from_bool(text <= other)));
            }
            "gt" => {
                let arg = state.get_reg(Reg(args.start.0));
                let other = extract_string(&arg, state);
                return Ok(Some(Value::from_bool(text > other)));
            }
            "ge" => {
                let arg = state.get_reg(Reg(args.start.0));
                let other = extract_string(&arg, state);
                return Ok(Some(Value::from_bool(text >= other)));
            }
            "cmp" => {
                let arg = state.get_reg(Reg(args.start.0));
                let other = extract_string(&arg, state);
                return Ok(Some(make_ordering(state, text.cmp(&other))?));
            }
            "hash" => {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                text.hash(&mut hasher);
                let hash_val = hasher.finish() as i64;
                return Ok(Some(Value::from_i64(hash_val)));
            }
            "clone" => {
                return Ok(Some(alloc_string_value(state, &text)?));
            }
            "to_string" | "display" => {
                return Ok(Some(alloc_string_value(state, &text)?));
            }
            "debug" => {
                let debug_repr = format!("{:?}", text);
                return Ok(Some(alloc_string_value(state, &debug_repr)?));
            }
            _ => return Ok(None),
        }
    }

    // Bool methods
    if receiver.is_bool() {
        let v = receiver.as_bool();
        let result = match method {
            "and_then" => {
                // Eager AND: true.and_then(x) = x, false.and_then(x) = false
                if v {
                    state.get_reg(Reg(args.start.0))
                } else {
                    Value::from_bool(false)
                }
            }
            "or_else" => {
                // Eager OR: true.or_else(x) = true, false.or_else(x) = x
                if v {
                    Value::from_bool(true)
                } else {
                    state.get_reg(Reg(args.start.0))
                }
            }
            "select" => {
                // select(a, b): true => a, false => b
                if v {
                    state.get_reg(Reg(args.start.0))
                } else {
                    state.get_reg(Reg(args.start.0 + 1))
                }
            }
            "xor" => {
                let other = state.get_reg(Reg(args.start.0)).as_bool();
                Value::from_bool(v ^ other)
            }
            "to_int" => Value::from_i64(if v { 1 } else { 0 }),
            // Eq protocol methods - handle eq/ne directly for Bool primitives
            "eq" => {
                let other_val = state.get_reg(Reg(args.start.0));
                // Handle CBGR reference: the argument might be &Bool, need to deref
                let other = if is_cbgr_ref(&other_val) {
                    let (abs_index, _) = decode_cbgr_ref(other_val.as_i64());
                    state.registers.get_absolute(abs_index).as_bool()
                } else {
                    other_val.as_bool()
                };
                Value::from_bool(v == other)
            }
            "ne" => {
                let other_val = state.get_reg(Reg(args.start.0));
                // Handle CBGR reference: the argument might be &Bool, need to deref
                let other = if is_cbgr_ref(&other_val) {
                    let (abs_index, _) = decode_cbgr_ref(other_val.as_i64());
                    state.registers.get_absolute(abs_index).as_bool()
                } else {
                    other_val.as_bool()
                };
                Value::from_bool(v != other)
            }
            _ => return Ok(None),
        };
        return Ok(Some(result));
    }

    Ok(None)
}

/// Get the length of an array (Value array or List).
pub(super) fn get_array_length(ptr: *const u8, header: &heap::ObjectHeader) -> InterpreterResult<usize> {
    if header.type_id == TypeId::LIST {
        let data_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
        Ok(unsafe { (*data_ptr).as_i64() } as usize)
    } else {
        Ok(header.size as usize / std::mem::size_of::<Value>())
    }
}

/// Get element at index from an array (Value array or List).
pub(super) fn get_array_element(ptr: *const u8, header: &heap::ObjectHeader, index: usize) -> InterpreterResult<Value> {
    if header.type_id == TypeId::LIST {
        let data_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
        let backing = unsafe { (*data_ptr.add(2)).as_ptr::<u8>() };
        let elem_offset = index * std::mem::size_of::<Value>();
        let elem_ptr = unsafe { backing.add(heap::OBJECT_HEADER_SIZE + elem_offset) as *const Value };
        Ok(unsafe { *elem_ptr })
    } else {
        let elem_offset = index * std::mem::size_of::<Value>();
        let elem_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE + elem_offset) as *const Value };
        Ok(unsafe { *elem_ptr })
    }
}

/// Call a closure synchronously, returning its result.
/// Sets up a call frame for the closure and runs a nested dispatch loop.
pub(crate) fn call_closure_sync(
    state: &mut InterpreterState,
    closure_val: Value,
    args: &[Value],
) -> InterpreterResult<Value> {
    if !closure_val.is_ptr() || closure_val.is_nil() {
        return Err(InterpreterError::TypeMismatch {
            expected: "closure",
            got: "non-pointer",
            operation: "call_closure_sync",
        });
    }

    let base_ptr = closure_val.as_ptr::<u8>();
    let header_offset = heap::OBJECT_HEADER_SIZE;

    let (func_id, capture_count) = unsafe {
        let func_id = *(base_ptr.add(header_offset) as *const u32);
        let capture_count = *(base_ptr.add(header_offset + 4) as *const u32);
        (FunctionId(func_id), capture_count as usize)
    };

    let func = state
        .module
        .get_function(func_id)
        .ok_or({
            InterpreterError::FunctionNotFound(func_id)
        })?;

    let reg_count = func.register_count;
    let return_pc = state.pc();
    let entry_depth = state.call_stack.depth();

    // Use Reg(0) as a dummy dst — the return value comes from dispatch_loop_table_with_entry_depth
    let new_base = state.call_stack.push_frame(func_id, reg_count, return_pc, Reg(0))?;
    state.registers.push_frame(reg_count);

    // Copy captured values (registers [0..capture_count))
    unsafe {
        let captures_offset = header_offset + 8;
        for i in 0..capture_count {
            let cap_ptr = base_ptr.add(captures_offset + i * std::mem::size_of::<Value>()) as *const Value;
            state.registers.set(new_base, Reg(i as u16), std::ptr::read(cap_ptr));
        }
    }

    // Copy arguments (registers [capture_count..capture_count+args.len()))
    for (i, val) in args.iter().enumerate() {
        state.registers.set(new_base, Reg((capture_count + i) as u16), *val);
    }

    state.set_pc(0);

    // Run nested dispatch loop — returns when the closure returns
    dispatch_loop_table_with_entry_depth(state, entry_depth)
}

/// Execute a function by FunctionId synchronously, returning its result.
///
/// This is the core primitive for async task execution: Spawn uses this to
/// eagerly evaluate spawned functions. The function runs to completion in a
/// nested dispatch loop and the return value is captured.
///
/// # Arguments
/// * `state` - Interpreter state
/// * `func_id` - Function to call
/// * `args` - Argument values
///
/// # Returns
/// The function's return value.
pub(super) fn call_function_sync(
    state: &mut InterpreterState,
    func_id: FunctionId,
    args: &[Value],
) -> InterpreterResult<Value> {
    let func = state
        .module
        .get_function(func_id)
        .ok_or(InterpreterError::FunctionNotFound(func_id))?;

    let reg_count = func.register_count;
    let return_pc = state.pc();
    let entry_depth = state.call_stack.depth();

    // Push frame with Reg(0) as dummy dst
    let new_base = state.call_stack.push_frame(func_id, reg_count, return_pc, Reg(0))?;
    state.registers.push_frame(reg_count);

    // Copy arguments
    for (i, val) in args.iter().enumerate() {
        state.registers.set(new_base, Reg(i as u16), *val);
    }

    // Start at function entry
    state.set_pc(0);

    // Run nested dispatch loop — returns when the function returns
    dispatch_loop_table_with_entry_depth(state, entry_depth)
}

/// Allocate a new List from a Vec of Values, returning a pointer Value.
pub(crate) fn alloc_list_from_values(state: &mut InterpreterState, values: Vec<Value>) -> InterpreterResult<Value> {
    let len = values.len();
    let cap = len.max(1); // at least 1 to avoid zero-size backing

    // Allocate List header: [len, cap, backing_ptr]
    let obj = state.heap.alloc(TypeId::LIST, 3 * std::mem::size_of::<Value>())?;
    state.record_allocation();

    // Allocate backing array
    let backing = state.heap.alloc_array(TypeId::LIST, cap)?;
    state.record_allocation();

    // Write elements to backing
    let backing_data = unsafe {
        (backing.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
    };
    for (i, val) in values.into_iter().enumerate() {
        unsafe { *backing_data.add(i) = val };
    }

    // Initialize List header
    let data_ptr = unsafe {
        (obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
    };
    unsafe {
        *data_ptr = Value::from_i64(len as i64);        // len
        *data_ptr.add(1) = Value::from_i64(cap as i64); // cap
        *data_ptr.add(2) = Value::from_ptr(backing.as_ptr() as *mut u8); // backing_ptr
    }

    Ok(Value::from_ptr(obj.as_ptr() as *mut u8))
}

/// Build a new Set from a Vec of unique (or to-be-deduplicated) values.
/// Layout is the builtin Set shape `[count, capacity, entries_ptr]` with
/// `[key, unit]` slot pairs — identical to what `Set.new()` +
/// `insert` produces, so subsequent builtin Set methods work over it.
pub(super) fn build_set_from_values(
    state: &mut InterpreterState,
    elements: Vec<Value>,
) -> InterpreterResult<Value> {
    let initial_cap = (elements.len() * 2).max(16);
    let obj = state.heap.alloc(TypeId::SET, 3 * std::mem::size_of::<Value>())?;
    state.record_allocation();
    let backing = state.heap.alloc_array(TypeId::UNIT, initial_cap * 2)?;
    state.record_allocation();
    let backing_ptr = backing.as_ptr() as *mut u8;
    let data = unsafe {
        backing_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
    };
    // Initialise every slot to unit (empty-slot marker).
    for i in 0..(initial_cap * 2) {
        unsafe { *data.add(i) = Value::unit(); }
    }
    // Probe-insert each element, skipping duplicates.
    let mut live: usize = 0;
    for elem in elements.into_iter() {
        let hash = value_hash(elem);
        let mut idx = hash % initial_cap;
        let start = idx;
        loop {
            let slot_key = unsafe { *data.add(idx * 2) };
            if slot_key.is_unit() {
                unsafe {
                    *data.add(idx * 2) = elem;
                    *data.add(idx * 2 + 1) = Value::unit();
                }
                live += 1;
                break;
            }
            if value_eq(slot_key, elem) {
                break; // dedup
            }
            idx = (idx + 1) % initial_cap;
            if idx == start { break; }
        }
    }
    let header = unsafe {
        (obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
    };
    unsafe {
        *header = Value::from_i64(live as i64);
        *header.add(1) = Value::from_i64(initial_cap as i64);
        *header.add(2) = Value::from_ptr(backing_ptr);
    }
    Ok(Value::from_ptr(obj.as_ptr() as *mut u8))
}

/// Extract the payload of a `Maybe::Some(x)` variant; return `None` when
/// the value is `Maybe::None`. Variant layout (as produced by
/// `make_some_value` / `make_none_value`): `[ObjectHeader][tag:u32]
/// [field_count:u32][payload_0:Value…]` — tag 0 = None, tag 1 = Some.
pub(super) fn unwrap_maybe_some(value: Value) -> Option<Value> {
    if !value.is_ptr() || value.is_nil() {
        return None;
    }
    let ptr = value.as_ptr::<u8>();
    if ptr.is_null() { return None; }
    let tag = unsafe { *(ptr.add(heap::OBJECT_HEADER_SIZE) as *const u32) };
    if tag == 0 {
        return None;
    }
    let payload = unsafe {
        *(ptr.add(heap::OBJECT_HEADER_SIZE + 8) as *const Value)
    };
    Some(payload)
}

/// Push a value onto a List.
/// List layout: [len: Value(i64), cap: Value(i64), backing: Value(ptr)]
pub(super) fn list_push(state: &mut InterpreterState, list_val: Value, new_val: Value) -> InterpreterResult<()> {
    let list_ptr = list_val.as_ptr::<u8>();
    let data_ptr = unsafe {
        list_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
    };
    let len = unsafe { (*data_ptr).as_i64() } as usize;
    let cap = unsafe { (*data_ptr.add(1)).as_i64() } as usize;
    let backing_ptr = unsafe { (*data_ptr.add(2)).as_ptr::<u8>() };

    if len >= cap {
        // Grow: allocate new backing with 2x capacity
        let new_cap = (cap * 2).max(8);
        let new_backing = state.heap.alloc_array(TypeId::LIST, new_cap)?;
        state.record_allocation();

        // Copy old elements
        let old_data = unsafe {
            backing_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
        };
        let new_data = unsafe {
            (new_backing.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
        };
        for i in 0..len {
            unsafe { *new_data.add(i) = *old_data.add(i) };
        }

        // Write new element
        unsafe { *new_data.add(len) = new_val };

        // Update list header
        unsafe {
            *data_ptr = Value::from_i64((len + 1) as i64);
            *data_ptr.add(1) = Value::from_i64(new_cap as i64);
            *data_ptr.add(2) = Value::from_ptr(new_backing.as_ptr() as *mut u8);
        }
    } else {
        // Write directly
        let backing_data = unsafe {
            backing_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
        };
        unsafe { *backing_data.add(len) = new_val };
        // Update len
        unsafe { *data_ptr = Value::from_i64((len + 1) as i64) };
    }
    Ok(())
}

/// Dispatch built-in methods on variant types (Result, Maybe).
///
/// Variant layout in data section: [tag:u32][field_count:u32][payload:Value * field_count]
/// TypeId range 0x8000+ indicates variant objects.
///
/// Conventions:
///   Result<T, E> = Ok(T) | Err(E)  → Ok = tag 0, Err = tag 1
///   Maybe<T>     = None  | Some(T) → None = tag 0 (field_count=0), Some = tag 1
///
/// `method_full` is the qualified call-site method name (e.g.
/// `"Result.unwrap"` vs `"Maybe.unwrap"`). When it starts with
/// `Result.` the dispatcher applies Result semantics — `unwrap`
/// panics on `Err`. When it starts with `Maybe.` it applies Maybe
/// semantics. Without a qualified prefix (bare `"unwrap"`) it
/// preserves the historic "return payload if fc > 0" behaviour for
/// back-compatibility with generic-parameter dispatch.
pub(super) fn dispatch_variant_method(
    state: &mut InterpreterState,
    receiver: Value,
    method: &str,
    args: &RegRange,
    method_full: &str,
) -> InterpreterResult<Option<Value>> {
    let base_ptr = receiver.as_ptr::<u8>();
    if base_ptr.is_null() {
        return Ok(None);
    }

    // Check TypeId — variant objects use 0x8000+ range
    let header = unsafe { &*(base_ptr as *const heap::ObjectHeader) };
    let type_id_val = header.type_id.0;
    if !(0x8000..0xA000).contains(&type_id_val) {
        return Ok(None); // Not a variant object
    }

    // Read tag and field_count from the data section
    let data_start = unsafe { base_ptr.add(heap::OBJECT_HEADER_SIZE) };
    let tag = unsafe { *(data_start as *const u32) };
    let field_count = unsafe { *((data_start as *const u32).add(1)) };

    match method {
        // Variant layout emitted by `MakeVariant` — tag encodes variant index
        // within the type declaration (`handle_make_variant` @ pattern_matching.rs).
        //
        //   Maybe  is None | Some(T)  →  None: tag=0, fc=0   Some: tag=1, fc=1
        //   Result is Ok(T) | Err(E)  →  Ok:   tag=0, fc=1   Err:  tag=1, fc=1
        //
        // `is_ok`/`is_err` follow Result semantics; `is_some`/`is_none`
        // follow Maybe. The cases with fc>0 across both types are
        // indistinguishable purely from the data (type_id = 0x8000 + tag
        // for every `MakeVariant`), so `is_some` can't reject Result.Err
        // — callers that need disambiguation should pattern-match.
        "is_ok"   => Ok(Some(Value::from_bool(tag == 0 && field_count > 0))),
        "is_err"  => Ok(Some(Value::from_bool(tag != 0))),
        "is_some" => Ok(Some(Value::from_bool(field_count > 0))),
        "is_none" => Ok(Some(Value::from_bool(field_count == 0))),

        // Value extraction with type-aware semantics.
        //
        //   Maybe.None  (tag=0, fc=0) → panic
        //   Maybe.Some  (tag=1, fc=1) → extract payload
        //   Result.Ok   (tag=0, fc=1) → extract payload
        //   Result.Err  (tag=1, fc=1) → panic (when method_full says Result.*)
        //
        // Without `method_full`, `Maybe.Some` and `Result.Err` are
        // indistinguishable from runtime data alone (same `(tag, fc)`
        // after `MakeVariant`; `TypeId = 0x8000 + tag` collapses both
        // into the same id range). The qualified call-site name —
        // emitted by codegen as `Result.unwrap` vs `Maybe.unwrap` — is
        // load-bearing here: it lets the dispatcher distinguish the
        // two type semantics without needing the type tag in the heap
        // header. Bare `unwrap` (no type prefix) preserves the
        // historic "return payload if fc > 0" route.
        "unwrap" | "expect" => {
            let is_result = method_full.starts_with("Result.")
                || method_full == "Result::unwrap"
                || method_full == "Result::expect";
            let is_maybe = method_full.starts_with("Maybe.")
                || method_full.starts_with("Option.")
                || method_full == "Maybe::unwrap"
                || method_full == "Option::unwrap";
            if is_result {
                // Result semantics: tag 0 = Ok, tag != 0 = Err.
                if tag == 0 && field_count > 0 {
                    let payload_ptr = unsafe {
                        base_ptr.add(heap::OBJECT_HEADER_SIZE + 8) as *const Value
                    };
                    Ok(Some(unsafe { *payload_ptr }))
                } else {
                    Err(InterpreterError::Panic {
                        message: format!("called `{}` on an Err value", method),
                    })
                }
            } else if is_maybe {
                // Maybe semantics: payload extraction iff fc > 0.
                if field_count > 0 {
                    let payload_ptr = unsafe {
                        base_ptr.add(heap::OBJECT_HEADER_SIZE + 8) as *const Value
                    };
                    Ok(Some(unsafe { *payload_ptr }))
                } else {
                    Err(InterpreterError::Panic {
                        message: format!("called `{}` on a None value", method),
                    })
                }
            } else if field_count > 0 {
                // Bare-name dispatch (no type prefix). Historic behaviour.
                let payload_ptr = unsafe {
                    base_ptr.add(heap::OBJECT_HEADER_SIZE + 8) as *const Value
                };
                Ok(Some(unsafe { *payload_ptr }))
            } else {
                Err(InterpreterError::Panic {
                    message: format!("called `{}` on a None value", method),
                })
            }
        }
        // unwrap_err is the mirror of unwrap for Result: returns the Err
        // payload if present (tag != 0), panics on Ok.
        "unwrap_err" => {
            if tag != 0 && field_count > 0 {
                let payload_ptr = unsafe {
                    base_ptr.add(heap::OBJECT_HEADER_SIZE + 8) as *const Value
                };
                Ok(Some(unsafe { *payload_ptr }))
            } else {
                Err(InterpreterError::Panic {
                    message: "called `unwrap_err` on an Ok value".to_string(),
                })
            }
        }
        // unwrap_or: returns Some/Ok payload, else the default argument.
        // Mirrors `unwrap`'s `fc > 0` convention (see comment above).
        "unwrap_or" => {
            if field_count > 0 {
                let payload_ptr = unsafe {
                    base_ptr.add(heap::OBJECT_HEADER_SIZE + 8) as *const Value
                };
                Ok(Some(unsafe { *payload_ptr }))
            } else if args.count > 0 {
                let caller_base = state.reg_base();
                let default_val = state.registers.get(caller_base, Reg(args.start.0));
                Ok(Some(default_val))
            } else {
                Ok(Some(Value::nil()))
            }
        }
        // as_ref / as_mut: in Verum's value model, the value itself is
        // effectively a reference to the heap variant, so they are no-ops
        // that return the receiver.
        "as_ref" | "as_mut" => Ok(Some(receiver)),
        // take: replaces payload with None-shape (tag=0, fc=0) and
        // returns the original value. Mirrors Option::take() semantics.
        "take" => {
            let original = receiver;
            if field_count > 0 {
                // Mutate in place: clear tag and field_count to turn this
                // variant into the None/unit shape.
                unsafe {
                    let tag_ptr = base_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut u32;
                    *tag_ptr = 0;
                    *tag_ptr.add(1) = 0;
                }
            }
            Ok(Some(original))
        }
        // ok: Result<T, E>.ok() -> Maybe<T> — for Ok(v) returns Some(v),
        // for Err(_) returns None. Returning the receiver as-is works
        // because the variant representation is identical.
        "ok" => {
            if tag == 0 && field_count > 0 {
                Ok(Some(receiver))
            } else {
                // Allocate a fresh None variant (tag=0, fc=0).
                let none_val = make_none_value(state)?;
                Ok(Some(none_val))
            }
        }

        // Result / Maybe combinators that take a closure.
        //
        // These are defined generically in `core/base/result.vr` and
        // `core/base/maybe.vr` with `<T, E, F, U>`-quantified bodies.
        // VBC monomorphisation does not eagerly produce instances for
        // every concrete `Result<T, E>` reachable from runtime IO
        // paths — most prominently `core/net/tcp.vr:321` calling
        // `socket(...).map_err(IoError.from_os)?`, which led to
        // `Panic: method 'Result.map_err' not found on value` at the
        // catch-all below. Native dispatch is monomorphisation-free
        // because the variant layout is type-erased at runtime; the
        // closure handles the type-specific work.
        //
        // Semantics (Result):
        //   Ok(v).map(f)         = Ok(f(v))
        //   Err(e).map(_)        = Err(e)               (identity)
        //   Ok(v).map_err(_)     = Ok(v)                (identity)
        //   Err(e).map_err(f)    = Err(f(e))
        //   Ok(v).and_then(f)    = f(v)                 (f returns Result)
        //   Err(e).and_then(_)   = Err(e)               (identity)
        //   Ok(v).or_else(_)     = Ok(v)                (identity)
        //   Err(e).or_else(f)    = f(e)                 (f returns Result)
        //
        // Semantics (Maybe / Option) — same code path because the
        // variant tag layout is identical (None = tag 0, fc = 0;
        // Some(v) = tag 1, fc = 1; Ok / Err follow the same shape).
        // For `map_err` on Maybe the call is a no-op (Maybe has no
        // error track) — handled by the Ok-branch identity.
        //
        // Failure modes preserved: closure panics propagate; closure
        // captures live through `call_closure_sync` exactly as if the
        // user had written the match in source.
        "map" | "map_err" | "and_then" | "or_else" => {
            // Closure / function-value sits at the first arg slot.
            if args.count == 0 {
                return Ok(None); // shape mismatch — let user-compiled fallback try
            }
            let caller_base = state.reg_base();
            let closure_val = state.registers.get(caller_base, Reg(args.start.0));

            // Guard: receiver must be a `(tag, fc=1)`-shaped variant
            // for the closure-applying branches; otherwise pass-through
            // on identity. `tag == 0` is Ok / Some; `tag != 0` is Err / None.
            // (method, tag) → "apply closure" vs "identity pass-through":
            //   "map"      / 0  (Ok/Some): apply, wrap with same tag
            //   "map"      / != 0       : identity
            //   "map_err"  / 0          : identity
            //   "map_err"  / != 0 (Err) : apply, wrap with same tag
            //   "and_then" / 0          : closure already returns Result → return raw
            //   "and_then" / != 0       : identity
            //   "or_else"  / 0          : identity
            //   "or_else"  / != 0       : closure already returns Result → return raw
            let apply_branch = match (method, tag) {
                ("map", 0) => true,
                ("map", _) => false,
                ("map_err", 0) => false,
                ("map_err", _) => true,
                ("and_then", 0) => true,
                ("and_then", _) => false,
                ("or_else", 0) => false,
                ("or_else", _) => true,
                _ => return Ok(None), // unreachable for the outer match arm
            };

            if !apply_branch {
                // Identity branch — return receiver as-is.
                return Ok(Some(receiver));
            }

            // Pull payload (Value at base + header + 8). For tag=0 / fc=0
            // (None) and Maybe semantics, callers shouldn't reach this
            // branch via map/and_then on None — that's the identity
            // case handled above. For (tag=0, fc=0) Ordering variants
            // we never enter this match arm because the type is not Result/Maybe.
            if field_count == 0 {
                // Defensive fall-through: malformed variant data.
                return Ok(None);
            }
            let payload_ptr = unsafe {
                base_ptr.add(heap::OBJECT_HEADER_SIZE + 8) as *const Value
            };
            let payload = unsafe { *payload_ptr };

            // Invoke the closure synchronously with the payload.
            let result_val = call_closure_sync(state, closure_val, &[payload])?;

            // For `and_then` / `or_else` the closure already returns
            // a Result/Maybe-shaped variant — return it directly.
            // For `map` / `map_err` we wrap the closure output back
            // into a same-tag variant.
            match method {
                "and_then" | "or_else" => Ok(Some(result_val)),
                "map" | "map_err" => {
                    // Re-emit with the same tag the receiver carried.
                    let wrapped = make_result_variant(state, tag, result_val)?;
                    Ok(Some(wrapped))
                }
                _ => unreachable!(),
            }
        }

        _ => Ok(None), // Not a variant method we handle - fall through to user-defined methods
    }
}

/// Dispatch built-in methods on array/list types (map, filter, fold).
pub(super) fn dispatch_array_method(
    state: &mut InterpreterState,
    receiver: Value,
    method: &str,
    args: &RegRange,
) -> InterpreterResult<Option<Value>> {
    let ptr = receiver.as_ptr::<u8>();
    // Safety: validate pointer alignment before dereferencing as ObjectHeader
    if ptr.is_null() || ((ptr as usize) & (std::mem::align_of::<heap::ObjectHeader>() - 1)) != 0 {
        return Ok(None);
    }
    let header = unsafe { &*(ptr as *const heap::ObjectHeader) };

    // Handle pointer extraction methods for ALL array types (including byte arrays)
    if method == "as_mut_ptr" || method == "as_ptr" {
        // Return a pointer to the start of the data section
        let data_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) };
        return Ok(Some(Value::from_ptr(data_ptr)));
    }

    // `as_slice` / `as_mut_slice` are identity casts at runtime — a
    // Verum slice and a List share the same `{ObjectHeader, data...}`
    // memory layout, the distinction is purely type-system.  The
    // stdlib defines them in `core/collections/list.vr:725` as
    // `unsafe { slice_from_raw_parts(self.ptr, self.len) }`; the
    // VBC interpreter pre-fix had no entry for them and panicked
    // "method 'List.as_slice' not found on value" — broke every
    // call site passing a Verum-built buffer to a C-ABI syscall.
    // Closes RUNTIME-2 from `internal/diag/sqlite-real/FINDINGS.md`.
    if method == "as_slice" || method == "as_mut_slice" {
        return Ok(Some(receiver));
    }

    // Only handle Value arrays and Lists, not byte arrays or specialized collections
    if header.type_id == TypeId::U8 {
        return Ok(None);
    }

    // Skip Text — it's a primitive value type (type_id=4) whose stdlib
    // `push`/`pop`/`len` methods operate on the `{ptr, len, cap}` struct
    // layout, not on a List header. Routing them through the List
    // dispatcher below treats the Text struct as if its first two Values
    // were `(len, cap, backing_ptr)` — then `push` corrupts all three
    // fields with list-header writes (field0 becomes 1, field1 becomes 8,
    // field2 becomes an entirely new List allocation).
    if header.type_id == TypeId::TEXT {
        return Ok(None);
    }

    // Skip associative/channel builtins — they have their own dispatch in
    // dispatch_primitive_method (Map has explicit filter/map/fold arms;
    // Set/Deque/Channel are handled elsewhere or must fall through to the
    // user-defined stdlib impl). Treating their
    // `[count, capacity, entries_ptr]` header as a raw Value-array would
    // iterate the metadata fields as if they were elements, producing
    // nonsense like `filter` yielding `(count, capacity, entries_ptr)`
    // triplets.
    if header.type_id == TypeId::DEQUE
        || header.type_id == TypeId::CHANNEL
        || header.type_id == TypeId::MAP
        || header.type_id == TypeId::SET
    {
        return Ok(None);
    }

    // Skip variant types (0x8000+ range) - they have their own dispatch in dispatch_variant_method
    // and user-defined methods like Maybe.map should be called via function lookup, not here.
    let type_id_val = header.type_id.0;
    if (0x8000..0xA000).contains(&type_id_val) {
        return Ok(None);
    }

    // Skip user-defined record types. These are not arrays/lists,
    // and their methods (e.g. user-defined `swap`, `reverse`, `insert`,
    // or builder chain `min`/`max`) must be looked up via the user
    // function dispatch path, not treated as List ops.
    //
    // Historical note: the original guard was `(FIRST_USER..256)` — but
    // user type IDs can exceed 256 whenever the module defines >240
    // record types (stdlib easily does). Types with IDs in the gap
    // 256..TypeId::LIST.0 were then incorrectly routed through the
    // array dispatch: e.g. `FlexItem.min(5)` picked the array-min
    // built-in, which returned a truncated object and later crashed
    // with `field access out of bounds: field index 3 (offset 24+8 =
    // 32) exceeds object data size 16`.
    //
    // The correct bound is "anything below the first built-in
    // collection id" — LIST=512 today, so the range is `16..512`. If
    // a new built-in lands between FIRST_USER and LIST we must update
    // this alongside. Reproducer:
    //   vcs/specs/L0-critical/vbc/struct_layout/flex_item_builder.vr
    if (TypeId::FIRST_USER..TypeId::LIST.0).contains(&type_id_val) {
        return Ok(None);
    }

    let len = get_array_length(ptr, header)?;
    let caller_base = state.reg_base();

    match method {
        "collect" => {
            // collect() on a List/array returns it as-is. In this Tier 0 interpreter,
            // map/filter on iterators already return Lists eagerly, so this is a passthrough.
            Ok(Some(receiver))
        }
        "map" => {
            let closure_val = state.registers.get(caller_base, Reg(args.start.0));

            let mut results = Vec::with_capacity(len);
            for i in 0..len {
                let elem = get_array_element(ptr, header, i)?;
                let mapped = call_closure_sync(state, closure_val, &[elem])?;
                results.push(mapped);
            }

            let result_val = alloc_list_from_values(state, results)?;
            Ok(Some(result_val))
        }
        "filter" => {
            let closure_val = state.registers.get(caller_base, Reg(args.start.0));

            let mut results = Vec::new();
            for i in 0..len {
                let elem = get_array_element(ptr, header, i)?;
                let keep = call_closure_sync(state, closure_val, &[elem])?;
                if keep.as_bool() {
                    results.push(elem);
                }
            }

            let result_val = alloc_list_from_values(state, results)?;
            Ok(Some(result_val))
        }
        "fold" => {
            let mut acc = state.registers.get(caller_base, Reg(args.start.0));
            let closure_val = state.registers.get(caller_base, Reg(args.start.0 + 1));

            for i in 0..len {
                let elem = get_array_element(ptr, header, i)?;
                acc = call_closure_sync(state, closure_val, &[acc, elem])?;
            }

            Ok(Some(acc))
        }
        // ===== Length / emptiness =====
        "len" | "count" => {
            Ok(Some(Value::from_i64(len as i64)))
        }
        "is_empty" => {
            Ok(Some(Value::from_bool(len == 0)))
        }

        // ===== Element access =====
        "get" => {
            let idx = state.registers.get(caller_base, Reg(args.start.0)).as_i64() as usize;
            if idx < len {
                let elem = get_array_element(ptr, header, idx)?;
                let result = make_some_value(state, elem)?;
                Ok(Some(result))
            } else {
                let result = make_none_value(state)?;
                Ok(Some(result))
            }
        }
        "first" => {
            if len > 0 {
                let elem = get_array_element(ptr, header, 0)?;
                let result = make_some_value(state, elem)?;
                Ok(Some(result))
            } else {
                let result = make_none_value(state)?;
                Ok(Some(result))
            }
        }
        "last" => {
            if len > 0 {
                let elem = get_array_element(ptr, header, len - 1)?;
                let result = make_some_value(state, elem)?;
                Ok(Some(result))
            } else {
                let result = make_none_value(state)?;
                Ok(Some(result))
            }
        }
        "contains" => {
            let needle = state.registers.get(caller_base, Reg(args.start.0));
            let mut found = false;
            for i in 0..len {
                let elem = get_array_element(ptr, header, i)?;
                if value_eq(elem, needle) {
                    found = true;
                    break;
                }
            }
            Ok(Some(Value::from_bool(found)))
        }

        // ===== Mutating methods =====
        "push" => {
            let new_val = state.registers.get(caller_base, Reg(args.start.0));
            list_push(state, receiver, new_val)?;
            Ok(Some(Value::unit()))
        }
        "pop" => {
            if header.type_id != TypeId::LIST {
                return Ok(None); // pop only works on Lists
            }
            let data_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value };
            let current_len = unsafe { (*data_ptr).as_i64() } as usize;
            if current_len == 0 {
                let result = make_none_value(state)?;
                Ok(Some(result))
            } else {
                let backing_ptr = unsafe { (*data_ptr.add(2)).as_ptr::<u8>() };
                let backing_data = unsafe {
                    backing_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                };
                let last_elem = unsafe { *backing_data.add(current_len - 1) };
                // Decrement length
                unsafe { *data_ptr = Value::from_i64((current_len - 1) as i64); }
                let result = make_some_value(state, last_elem)?;
                Ok(Some(result))
            }
        }
        "insert" => {
            if header.type_id != TypeId::LIST {
                return Ok(None);
            }
            let idx = state.registers.get(caller_base, Reg(args.start.0)).as_i64() as usize;
            let new_val = state.registers.get(caller_base, Reg(args.start.0 + 1));
            // First push a dummy to ensure capacity (this also increments len)
            list_push(state, receiver, Value::unit())?;
            // Re-read pointers after potential reallocation
            let data_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value };
            let current_len = unsafe { (*data_ptr).as_i64() } as usize; // already incremented by push
            let backing_ptr = unsafe { (*data_ptr.add(2)).as_ptr::<u8>() };
            let backing_data = unsafe {
                backing_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
            };
            let idx = idx.min(current_len - 1);
            // Shift elements right from end down to idx
            for i in (idx + 1..current_len).rev() {
                unsafe { *backing_data.add(i) = *backing_data.add(i - 1); }
            }
            // Write the new element at idx
            unsafe { *backing_data.add(idx) = new_val; }
            Ok(Some(Value::unit()))
        }
        "remove" => {
            if header.type_id != TypeId::LIST {
                return Ok(None);
            }
            let idx = state.registers.get(caller_base, Reg(args.start.0)).as_i64() as usize;
            let data_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value };
            let current_len = unsafe { (*data_ptr).as_i64() } as usize;
            if idx >= current_len {
                return Err(InterpreterError::TypeMismatch {
                    expected: "valid index",
                    got: "out of bounds",
                    operation: "List.remove",
                });
            }
            let backing_ptr = unsafe { (*data_ptr.add(2)).as_ptr::<u8>() };
            let backing_data = unsafe {
                backing_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
            };
            let removed = unsafe { *backing_data.add(idx) };
            // Shift elements left
            for i in idx..current_len - 1 {
                unsafe { *backing_data.add(i) = *backing_data.add(i + 1); }
            }
            // Decrement length
            unsafe { *data_ptr = Value::from_i64((current_len - 1) as i64); }
            Ok(Some(removed))
        }
        "clear" => {
            if header.type_id != TypeId::LIST {
                return Ok(None);
            }
            let data_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value };
            unsafe { *data_ptr = Value::from_i64(0); }
            Ok(Some(Value::unit()))
        }
        "swap" => {
            let idx_a = state.registers.get(caller_base, Reg(args.start.0)).as_i64() as usize;
            let idx_b = state.registers.get(caller_base, Reg(args.start.0 + 1)).as_i64() as usize;
            if idx_a >= len || idx_b >= len {
                return Err(InterpreterError::TypeMismatch {
                    expected: "valid indices",
                    got: "out of bounds",
                    operation: "List.swap",
                });
            }
            if header.type_id == TypeId::LIST {
                let data_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
                let backing_ptr = unsafe { (*data_ptr.add(2)).as_ptr::<u8>() };
                let backing_data = unsafe {
                    backing_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                };
                unsafe {
                    let tmp = *backing_data.add(idx_a);
                    *backing_data.add(idx_a) = *backing_data.add(idx_b);
                    *backing_data.add(idx_b) = tmp;
                }
            } else {
                let _elem_size = std::mem::size_of::<Value>();
                let base = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value };
                unsafe {
                    let tmp = *base.add(idx_a);
                    *base.add(idx_a) = *base.add(idx_b);
                    *base.add(idx_b) = tmp;
                }
            }
            Ok(Some(Value::unit()))
        }
        "reverse" => {
            if header.type_id == TypeId::LIST {
                let data_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
                let backing_ptr = unsafe { (*data_ptr.add(2)).as_ptr::<u8>() };
                let backing_data = unsafe {
                    backing_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                };
                let mut lo = 0usize;
                let mut hi = if len > 0 { len - 1 } else { 0 };
                while lo < hi {
                    unsafe {
                        let tmp = *backing_data.add(lo);
                        *backing_data.add(lo) = *backing_data.add(hi);
                        *backing_data.add(hi) = tmp;
                    }
                    lo += 1;
                    hi -= 1;
                }
            } else {
                let base = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value };
                let mut lo = 0usize;
                let mut hi = if len > 0 { len - 1 } else { 0 };
                while lo < hi {
                    unsafe {
                        let tmp = *base.add(lo);
                        *base.add(lo) = *base.add(hi);
                        *base.add(hi) = tmp;
                    }
                    lo += 1;
                    hi -= 1;
                }
            }
            Ok(Some(Value::unit()))
        }
        "sort" => {
            // Collect all elements into a Vec, sort, write back
            let mut elems = Vec::with_capacity(len);
            for i in 0..len {
                elems.push(get_array_element(ptr, header, i)?);
            }
            elems.sort_by(|a, b| {
                // Try integer comparison first, then float, then bitwise
                if a.is_int() && !a.is_bool() && b.is_int() && !b.is_bool() {
                    a.as_i64().cmp(&b.as_i64())
                } else if a.is_float() && b.is_float() {
                    a.as_f64().partial_cmp(&b.as_f64()).unwrap_or(std::cmp::Ordering::Equal)
                } else {
                    a.to_bits().cmp(&b.to_bits())
                }
            });
            // Write back
            if header.type_id == TypeId::LIST {
                let data_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
                let backing_ptr = unsafe { (*data_ptr.add(2)).as_ptr::<u8>() };
                let backing_data = unsafe {
                    backing_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                };
                for (i, v) in elems.into_iter().enumerate() {
                    unsafe { *backing_data.add(i) = v; }
                }
            } else {
                let base = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value };
                for (i, v) in elems.into_iter().enumerate() {
                    unsafe { *base.add(i) = v; }
                }
            }
            Ok(Some(Value::unit()))
        }
        "sort_by" => {
            let closure_val = state.registers.get(caller_base, Reg(args.start.0));
            // Collect elements
            let mut elems = Vec::with_capacity(len);
            for i in 0..len {
                elems.push(get_array_element(ptr, header, i)?);
            }
            // Use a simple insertion sort so we can call the closure comparator
            // (cannot use sort_by with a closure that borrows mutable state)
            for i in 1..elems.len() {
                let key = elems[i];
                let mut j = i;
                while j > 0 {
                    let cmp_result = call_closure_sync(state, closure_val, &[elems[j - 1], key])?;
                    // Comparator returns: negative = less, 0 = equal, positive = greater
                    let cmp_val = cmp_result.as_i64();
                    if cmp_val > 0 {
                        elems[j] = elems[j - 1];
                        j -= 1;
                    } else {
                        break;
                    }
                }
                elems[j] = key;
            }
            // Re-read pointers since closure calls may have triggered GC
            let ptr = receiver.as_ptr::<u8>();
            let header = unsafe { &*(ptr as *const heap::ObjectHeader) };
            if header.type_id == TypeId::LIST {
                let data_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
                let backing_ptr = unsafe { (*data_ptr.add(2)).as_ptr::<u8>() };
                let backing_data = unsafe {
                    backing_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
                };
                for (i, v) in elems.into_iter().enumerate() {
                    unsafe { *backing_data.add(i) = v; }
                }
            } else {
                let base = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value };
                for (i, v) in elems.into_iter().enumerate() {
                    unsafe { *base.add(i) = v; }
                }
            }
            Ok(Some(Value::unit()))
        }

        // ===== Iteration / higher-order =====
        "for_each" => {
            let closure_val = state.registers.get(caller_base, Reg(args.start.0));
            // Collect elements first since closure calls may invalidate pointers
            let mut elems = Vec::with_capacity(len);
            for i in 0..len {
                elems.push(get_array_element(ptr, header, i)?);
            }
            for elem in elems {
                let _ = call_closure_sync(state, closure_val, &[elem])?;
            }
            Ok(Some(Value::unit()))
        }
        "any" => {
            let closure_val = state.registers.get(caller_base, Reg(args.start.0));
            let mut elems = Vec::with_capacity(len);
            for i in 0..len {
                elems.push(get_array_element(ptr, header, i)?);
            }
            for elem in elems {
                let result = call_closure_sync(state, closure_val, &[elem])?;
                if result.as_bool() {
                    return Ok(Some(Value::from_bool(true)));
                }
            }
            Ok(Some(Value::from_bool(false)))
        }
        "all" => {
            let closure_val = state.registers.get(caller_base, Reg(args.start.0));
            let mut elems = Vec::with_capacity(len);
            for i in 0..len {
                elems.push(get_array_element(ptr, header, i)?);
            }
            for elem in elems {
                let result = call_closure_sync(state, closure_val, &[elem])?;
                if !result.as_bool() {
                    return Ok(Some(Value::from_bool(false)));
                }
            }
            Ok(Some(Value::from_bool(true)))
        }
        "find" => {
            let closure_val = state.registers.get(caller_base, Reg(args.start.0));
            let mut elems = Vec::with_capacity(len);
            for i in 0..len {
                elems.push(get_array_element(ptr, header, i)?);
            }
            for elem in elems {
                let result = call_closure_sync(state, closure_val, &[elem])?;
                if result.as_bool() {
                    let some_val = make_some_value(state, elem)?;
                    return Ok(Some(some_val));
                }
            }
            let none_val = make_none_value(state)?;
            Ok(Some(none_val))
        }
        "position" => {
            let closure_val = state.registers.get(caller_base, Reg(args.start.0));
            let mut elems = Vec::with_capacity(len);
            for i in 0..len {
                elems.push(get_array_element(ptr, header, i)?);
            }
            for (i, elem) in elems.into_iter().enumerate() {
                let result = call_closure_sync(state, closure_val, &[elem])?;
                if result.as_bool() {
                    let some_val = make_some_value(state, Value::from_i64(i as i64))?;
                    return Ok(Some(some_val));
                }
            }
            let none_val = make_none_value(state)?;
            Ok(Some(none_val))
        }
        "flat_map" => {
            let closure_val = state.registers.get(caller_base, Reg(args.start.0));
            let mut elems = Vec::with_capacity(len);
            for i in 0..len {
                elems.push(get_array_element(ptr, header, i)?);
            }
            let mut results = Vec::new();
            for elem in elems {
                let inner_list = call_closure_sync(state, closure_val, &[elem])?;
                // Read the inner list's elements
                let inner_ptr = inner_list.as_ptr::<u8>();
                if !inner_ptr.is_null() {
                    let inner_header = unsafe { &*(inner_ptr as *const heap::ObjectHeader) };
                    let inner_len = get_array_length(inner_ptr, inner_header)?;
                    for j in 0..inner_len {
                        let inner_elem = get_array_element(inner_ptr, inner_header, j)?;
                        results.push(inner_elem);
                    }
                }
            }
            let result_val = alloc_list_from_values(state, results)?;
            Ok(Some(result_val))
        }
        "flatten" => {
            // Flatten List<List<T>> -> List<T>
            let mut results = Vec::new();
            for i in 0..len {
                let inner_list = get_array_element(ptr, header, i)?;
                if inner_list.is_ptr() && !inner_list.is_nil() {
                    let inner_ptr = inner_list.as_ptr::<u8>();
                    if !inner_ptr.is_null() {
                        let inner_header = unsafe { &*(inner_ptr as *const heap::ObjectHeader) };
                        let inner_len = get_array_length(inner_ptr, inner_header)?;
                        for j in 0..inner_len {
                            let inner_elem = get_array_element(inner_ptr, inner_header, j)?;
                            results.push(inner_elem);
                        }
                    }
                }
            }
            let result_val = alloc_list_from_values(state, results)?;
            Ok(Some(result_val))
        }

        // ===== Slicing / subsequences =====
        "skip" => {
            let n = state.registers.get(caller_base, Reg(args.start.0)).as_i64() as usize;
            let start = n.min(len);
            let mut results = Vec::with_capacity(len.saturating_sub(start));
            for i in start..len {
                results.push(get_array_element(ptr, header, i)?);
            }
            let result_val = alloc_list_from_values(state, results)?;
            Ok(Some(result_val))
        }
        "take" => {
            let n = state.registers.get(caller_base, Reg(args.start.0)).as_i64() as usize;
            let end = n.min(len);
            let mut results = Vec::with_capacity(end);
            for i in 0..end {
                results.push(get_array_element(ptr, header, i)?);
            }
            let result_val = alloc_list_from_values(state, results)?;
            Ok(Some(result_val))
        }
        "slice" => {
            let start = state.registers.get(caller_base, Reg(args.start.0)).as_i64() as usize;
            let end = state.registers.get(caller_base, Reg(args.start.0 + 1)).as_i64() as usize;
            let start = start.min(len);
            let end = end.min(len);
            let end = end.max(start); // ensure end >= start
            let mut results = Vec::with_capacity(end - start);
            for i in start..end {
                results.push(get_array_element(ptr, header, i)?);
            }
            let result_val = alloc_list_from_values(state, results)?;
            Ok(Some(result_val))
        }

        // ===== Aggregation =====
        "sum" => {
            let mut total: i64 = 0;
            for i in 0..len {
                let elem = get_array_element(ptr, header, i)?;
                if elem.is_float() {
                    // If we encounter a float, switch to float sum
                    let mut ftotal = total as f64 + elem.as_f64();
                    for j in (i + 1)..len {
                        let e = get_array_element(ptr, header, j)?;
                        if e.is_float() {
                            ftotal += e.as_f64();
                        } else {
                            ftotal += e.as_i64() as f64;
                        }
                    }
                    return Ok(Some(Value::from_f64(ftotal)));
                }
                total += elem.as_i64();
            }
            Ok(Some(Value::from_i64(total)))
        }
        "min" => {
            if len == 0 {
                let result = make_none_value(state)?;
                return Ok(Some(result));
            }
            let mut min_val = get_array_element(ptr, header, 0)?;
            for i in 1..len {
                let elem = get_array_element(ptr, header, i)?;
                let is_less = if elem.is_float() && min_val.is_float() {
                    elem.as_f64() < min_val.as_f64()
                } else if elem.is_int() && !elem.is_bool() && min_val.is_int() && !min_val.is_bool() {
                    elem.as_i64() < min_val.as_i64()
                } else {
                    elem.to_bits() < min_val.to_bits()
                };
                if is_less {
                    min_val = elem;
                }
            }
            let result = make_some_value(state, min_val)?;
            Ok(Some(result))
        }
        "max" => {
            if len == 0 {
                let result = make_none_value(state)?;
                return Ok(Some(result));
            }
            let mut max_val = get_array_element(ptr, header, 0)?;
            for i in 1..len {
                let elem = get_array_element(ptr, header, i)?;
                let is_greater = if elem.is_float() && max_val.is_float() {
                    elem.as_f64() > max_val.as_f64()
                } else if elem.is_int() && !elem.is_bool() && max_val.is_int() && !max_val.is_bool() {
                    elem.as_i64() > max_val.as_i64()
                } else {
                    elem.to_bits() > max_val.to_bits()
                };
                if is_greater {
                    max_val = elem;
                }
            }
            let result = make_some_value(state, max_val)?;
            Ok(Some(result))
        }

        // ===== In-place mutation with other list =====
        "extend" => {
            let other_val = state.registers.get(caller_base, Reg(args.start.0));
            if !other_val.is_ptr() || other_val.is_nil() {
                return Ok(Some(Value::unit()));
            }
            let other_ptr = other_val.as_ptr::<u8>();
            if other_ptr.is_null() {
                return Ok(Some(Value::unit()));
            }
            let other_header = unsafe { &*(other_ptr as *const heap::ObjectHeader) };
            let other_len = get_array_length(other_ptr, other_header)?;
            // Collect elements from the other list first
            let mut other_elems = Vec::with_capacity(other_len);
            for i in 0..other_len {
                other_elems.push(get_array_element(other_ptr, other_header, i)?);
            }
            // Push each element (list_push handles growth and pointer updates)
            for elem in other_elems {
                list_push(state, receiver, elem)?;
            }
            Ok(Some(Value::unit()))
        }
        "dedup" => {
            if header.type_id != TypeId::LIST {
                return Ok(None);
            }
            if len <= 1 {
                return Ok(Some(Value::unit()));
            }
            let data_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value };
            let backing_ptr = unsafe { (*data_ptr.add(2)).as_ptr::<u8>() };
            let backing_data = unsafe {
                backing_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
            };
            let mut write_idx = 1usize;
            let mut prev = unsafe { *backing_data };
            for read_idx in 1..len {
                let current = unsafe { *backing_data.add(read_idx) };
                if !value_eq(current, prev) {
                    unsafe { *backing_data.add(write_idx) = current; }
                    write_idx += 1;
                    prev = current;
                }
            }
            unsafe { *data_ptr = Value::from_i64(write_idx as i64); }
            Ok(Some(Value::unit()))
        }
        "retain" => {
            if header.type_id != TypeId::LIST {
                return Ok(None);
            }
            let closure_val = state.registers.get(caller_base, Reg(args.start.0));
            // Collect all elements first (closure calls may invalidate pointers)
            let mut elems = Vec::with_capacity(len);
            for i in 0..len {
                elems.push(get_array_element(ptr, header, i)?);
            }
            let mut kept = Vec::with_capacity(len);
            for elem in elems {
                let keep = call_closure_sync(state, closure_val, &[elem])?;
                if keep.as_bool() {
                    kept.push(elem);
                }
            }
            // Re-read pointers after closure calls
            let ptr = receiver.as_ptr::<u8>();
            let data_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value };
            let backing_ptr = unsafe { (*data_ptr.add(2)).as_ptr::<u8>() };
            let backing_data = unsafe {
                backing_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
            };
            for (i, v) in kept.iter().enumerate() {
                unsafe { *backing_data.add(i) = *v; }
            }
            unsafe { *data_ptr = Value::from_i64(kept.len() as i64); }
            Ok(Some(Value::unit()))
        }

        // ===== Tuple-producing methods =====
        "enumerate" => {
            let mut results = Vec::with_capacity(len);
            for i in 0..len {
                let elem = get_array_element(ptr, header, i)?;
                // Allocate a 2-element tuple: (index, element)
                let tuple_size = 2 * std::mem::size_of::<Value>();
                let tuple_obj = state.heap.alloc_with_init(
                    TypeId::TUPLE,
                    tuple_size,
                    |_data| {},
                )?;
                state.record_allocation();
                let tuple_data = tuple_obj.data_ptr() as *mut Value;
                unsafe {
                    *tuple_data = Value::from_i64(i as i64);
                    *tuple_data.add(1) = elem;
                }
                results.push(Value::from_ptr(tuple_obj.as_ptr()));
            }
            let result_val = alloc_list_from_values(state, results)?;
            Ok(Some(result_val))
        }
        "zip" => {
            let other_val = state.registers.get(caller_base, Reg(args.start.0));
            if !other_val.is_ptr() || other_val.is_nil() {
                let result_val = alloc_list_from_values(state, Vec::new())?;
                return Ok(Some(result_val));
            }
            let other_ptr = other_val.as_ptr::<u8>();
            if other_ptr.is_null() {
                let result_val = alloc_list_from_values(state, Vec::new())?;
                return Ok(Some(result_val));
            }
            let other_header = unsafe { &*(other_ptr as *const heap::ObjectHeader) };
            let other_len = get_array_length(other_ptr, other_header)?;
            let zip_len = len.min(other_len);
            // Collect elements from both lists first
            let mut self_elems = Vec::with_capacity(zip_len);
            let mut other_elems = Vec::with_capacity(zip_len);
            for i in 0..zip_len {
                self_elems.push(get_array_element(ptr, header, i)?);
                other_elems.push(get_array_element(other_ptr, other_header, i)?);
            }
            let mut results = Vec::with_capacity(zip_len);
            for i in 0..zip_len {
                let tuple_size = 2 * std::mem::size_of::<Value>();
                let tuple_obj = state.heap.alloc_with_init(
                    TypeId::TUPLE,
                    tuple_size,
                    |_data| {},
                )?;
                state.record_allocation();
                let tuple_data = tuple_obj.data_ptr() as *mut Value;
                unsafe {
                    *tuple_data = self_elems[i];
                    *tuple_data.add(1) = other_elems[i];
                }
                results.push(Value::from_ptr(tuple_obj.as_ptr()));
            }
            let result_val = alloc_list_from_values(state, results)?;
            Ok(Some(result_val))
        }

        // ===== String conversion =====
        "join" => {
            let sep_val = state.registers.get(caller_base, Reg(args.start.0));
            let sep = if sep_val.is_small_string() {
                sep_val.as_small_string().as_str().to_string()
            } else if sep_val.is_ptr() && !sep_val.is_nil() {
                // Try reading as heap string
                let sep_ptr = sep_val.as_ptr::<u8>();
                if !sep_ptr.is_null() {
                    unsafe {
                        let data_offset = heap::OBJECT_HEADER_SIZE;
                        let len_ptr = sep_ptr.add(data_offset) as *const u64;
                        let slen = *len_ptr as usize;
                        if slen <= 65536 {
                            let bytes_ptr = sep_ptr.add(data_offset + 8);
                            let bytes = std::slice::from_raw_parts(bytes_ptr, slen);
                            String::from_utf8_lossy(bytes).to_string()
                        } else {
                            String::new()
                        }
                    }
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            let mut parts = Vec::with_capacity(len);
            for i in 0..len {
                let elem = get_array_element(ptr, header, i)?;
                let s = format_value_for_print(state, elem);
                parts.push(s);
            }
            let joined = parts.join(&sep);
            let result = alloc_string_value(state, &joined)?;
            Ok(Some(result))
        }

        _ => Ok(None),
    }
}

/// Create a Maybe variant: Some(int_value) or None
/// Maybe variant tags follow declaration order: `type Maybe<T> is None | Some(T);`
/// so None=0 and Some=1. Must agree with register_type_constructors and the
/// hard-coded constant/variant tables in codegen/mod.rs.
pub(super) fn make_maybe_int(state: &mut InterpreterState, opt: Option<i64>) -> InterpreterResult<Value> {
    match opt {
        Some(v) => {
            // MakeVariant tag=1 (Some), field_count=1, then set field 0
            let data_size = 8 + std::mem::size_of::<Value>();
            let type_id = TypeId(0x8001); // tag 1
            let obj = state.heap.alloc_with_init(
                type_id,
                data_size,
                |data| {
                    let tag_ptr = data.as_mut_ptr() as *mut u32;
                    unsafe {
                        *tag_ptr = 1;          // Some tag
                        *tag_ptr.add(1) = 1;   // field_count = 1
                    }
                },
            )?;
            // Set payload
            unsafe {
                let base = obj.as_ptr() as *mut u8;
                let payload_ptr = base.add(heap::OBJECT_HEADER_SIZE + 8) as *mut Value;
                std::ptr::write(payload_ptr, Value::from_i64(v));
            }
            state.record_allocation();
            Ok(Value::from_ptr(obj.as_ptr() as *mut u8))
        }
        None => {
            // MakeVariant tag=0 (None), field_count=0
            let data_size = 8 + std::mem::size_of::<Value>(); // min 1 field
            let type_id = TypeId(0x8000); // tag 0
            let obj = state.heap.alloc_with_init(
                type_id,
                data_size,
                |data| {
                    let tag_ptr = data.as_mut_ptr() as *mut u32;
                    unsafe {
                        *tag_ptr = 0;          // None tag
                        *tag_ptr.add(1) = 0;   // field_count = 0
                    }
                },
            )?;
            state.record_allocation();
            Ok(Value::from_ptr(obj.as_ptr() as *mut u8))
        }
    }
}

/// Create a Some variant wrapping any value.
/// Maybe is declared `None | Some(T)`, so Some gets tag=1.
pub(super) fn make_some_value(state: &mut InterpreterState, value: Value) -> InterpreterResult<Value> {
    let data_size = 8 + std::mem::size_of::<Value>();
    let type_id = TypeId(0x8001); // tag 1 for Some
    let obj = state.heap.alloc_with_init(
        type_id,
        data_size,
        |data| {
            let tag_ptr = data.as_mut_ptr() as *mut u32;
            unsafe {
                *tag_ptr = 1;         // Some tag
                *tag_ptr.add(1) = 1;  // field_count = 1
            }
        },
    )?;
    // Set payload
    unsafe {
        let base = obj.as_ptr() as *mut u8;
        let payload_ptr = base.add(heap::OBJECT_HEADER_SIZE + 8) as *mut Value;
        std::ptr::write(payload_ptr, value);
    }
    state.record_allocation();
    Ok(Value::from_ptr(obj.as_ptr() as *mut u8))
}

/// Create a None variant.
/// Maybe is declared `None | Some(T)`, so None gets tag=0.
pub(super) fn make_none_value(state: &mut InterpreterState) -> InterpreterResult<Value> {
    let data_size = 8;
    let type_id = TypeId(0x8000); // tag 0 for None
    let obj = state.heap.alloc_with_init(
        type_id,
        data_size,
        |data| {
            let tag_ptr = data.as_mut_ptr() as *mut u32;
            unsafe {
                *tag_ptr = 0;       // None tag
                *tag_ptr.add(1) = 0; // field_count = 0
            }
        },
    )?;
    state.record_allocation();
    Ok(Value::from_ptr(obj.as_ptr() as *mut u8))
}

/// Create a Result variant carrying `payload` with the given tag
/// (0 = Ok, 1 = Err). Used by Result combinator handlers
/// (`map_err`, `map`, …) to wrap the closure result without going
/// through user-compiled code paths.
///
/// Layout matches `MakeVariant` (`pattern_matching::handle_make_variant`)
/// and `make_some_value` / `make_none_value` above:
///   `[ObjectHeader][tag: u32][field_count: u32][payload: Value]`.
fn make_result_variant(
    state: &mut InterpreterState,
    tag: u32,
    payload: Value,
) -> InterpreterResult<Value> {
    let data_size = 8 + std::mem::size_of::<Value>();
    let type_id = TypeId(0x8000 + tag);
    let obj = state.heap.alloc_with_init(
        type_id,
        data_size,
        |data| {
            let tag_ptr = data.as_mut_ptr() as *mut u32;
            unsafe {
                *tag_ptr = tag;
                *tag_ptr.add(1) = 1; // field_count = 1 (Ok(T) / Err(E))
            }
        },
    )?;
    unsafe {
        let base = obj.as_ptr() as *mut u8;
        let payload_ptr = base.add(heap::OBJECT_HEADER_SIZE + 8) as *mut Value;
        std::ptr::write(payload_ptr, payload);
    }
    state.record_allocation();
    Ok(Value::from_ptr(obj.as_ptr() as *mut u8))
}

/// Create an Ordering variant: Less (tag 0), Equal (tag 1), or Greater (tag 2)
pub(super) fn make_ordering(state: &mut InterpreterState, ord: std::cmp::Ordering) -> InterpreterResult<Value> {
    let tag = match ord {
        std::cmp::Ordering::Less => 0u32,
        std::cmp::Ordering::Equal => 1u32,
        std::cmp::Ordering::Greater => 2u32,
    };
    // Ordering variants are unit types (no payload), but allocate min 8 bytes for tag storage
    let data_size = 8;
    let type_id = TypeId(0x8000 + tag);
    let obj = state.heap.alloc_with_init(
        type_id,
        data_size,
        |data| {
            let tag_ptr = data.as_mut_ptr() as *mut u32;
            unsafe { *tag_ptr = tag; }
        },
    )?;
    state.record_allocation();
    Ok(Value::from_ptr(obj.as_ptr() as *mut u8))
}

