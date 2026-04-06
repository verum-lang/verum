//! Iterator and range instruction handlers for VBC interpreter.

use crate::instruction::Reg;
use crate::types::TypeId;
use crate::value::Value;
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::{InterpreterState, GeneratorId, GeneratorStatus};
use super::super::super::heap;
use super::super::DispatchResult;
use super::super::dispatch_loop_table_with_entry_depth;
use super::bytecode_io::*;
use super::cbgr_helpers::{is_cbgr_ref, decode_cbgr_ref};

// ── Iterator type constants ──
const ITER_TYPE_LIST: i64 = 0;
const ITER_TYPE_MAP: i64 = 1;
const ITER_TYPE_ARRAY: i64 = 2;
const ITER_TYPE_RANGE: i64 = 3;
const ITER_TYPE_GENERATOR: i64 = 4;

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
pub(in super::super) fn handle_iter_new(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;

    let source = state.get_reg(src);

    // If source is a CBGR register reference (e.g., &List<Int> parameter),
    // deref it to get the actual collection pointer
    let source = if is_cbgr_ref(&source) {
        let (abs_index, _generation) = decode_cbgr_ref(source.as_i64());
        state.registers.get_absolute(abs_index)
    } else {
        source
    };

    // Check for generator values first (NaN-boxed generator tag, not pointer)
    if source.is_generator() {
        // Generator iterator: store generator value directly in iterator object.
        // IterNext will detect ITER_TYPE_GENERATOR and resume the generator.
        let iter_obj = state.heap.alloc(TypeId::UNIT, 3 * std::mem::size_of::<Value>())?;
        state.record_allocation();
        let iter_ptr = unsafe {
            (iter_obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
        };
        unsafe {
            *iter_ptr = source;                                    // generator value
            *iter_ptr.add(1) = Value::from_i64(0);                // unused for generators
            *iter_ptr.add(2) = Value::from_i64(ITER_TYPE_GENERATOR); // iter_type
        }
        state.set_reg(dst, Value::from_ptr(iter_obj.as_ptr() as *mut u8));
        return Ok(DispatchResult::Continue);
    }

    // Determine collection type by examining the object header
    let iter_type = if source.is_ptr() {
        let source_ptr = source.as_ptr::<u8>();
        if !source_ptr.is_null() {
            // Read object header to get type_id
            let header = unsafe { &*(source_ptr as *const heap::ObjectHeader) };
            match header.type_id {
                TypeId::MAP | TypeId::SET => ITER_TYPE_MAP,
                TypeId::ARRAY => ITER_TYPE_ARRAY,
                TypeId::RANGE => ITER_TYPE_RANGE,
                // LIST and all other types default to list iteration
                _ => ITER_TYPE_LIST,
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
    let iter_obj = state.heap.alloc(TypeId::UNIT, 3 * std::mem::size_of::<Value>())?;
    state.record_allocation();

    let iter_ptr = unsafe {
        (iter_obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
    };

    // Initialize iterator
    unsafe {
        *iter_ptr = source;                              // source_ptr
        *iter_ptr.add(1) = Value::from_i64(0);           // current_idx = 0
        *iter_ptr.add(2) = Value::from_i64(iter_type);   // iter_type
    }

    state.set_reg(dst, Value::from_ptr(iter_obj.as_ptr() as *mut u8));
    Ok(DispatchResult::Continue)
}

/// IterNext (0xC1) - Get next element from iterator.
///
/// Format: `IterNext dst, has_next_dst, iter`
/// Advances iterator, sets dst to next value (or unit if exhausted),
/// and sets has_next_dst to bool indicating if there was a value.
pub(in super::super) fn handle_iter_next(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let has_next_dst = read_reg(state)?;
    let iter_reg = read_reg(state)?;

    let iter_ptr = state.get_reg(iter_reg).as_ptr::<u8>();
    if iter_ptr.is_null() {
        return Err(InterpreterError::NullPointer);
    }

    // Read iterator state
    let iter_data = unsafe {
        iter_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
    };
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

        if !state.generators.get(gen_id).map(|g| g.can_resume()).unwrap_or(false) {
            state.set_reg(dst, Value::unit());
            state.set_reg(has_next_dst, Value::from_bool(false));
            return Ok(DispatchResult::Continue);
        }

        let (func_id, status, reg_count) = {
            let generator = state.generators.get(gen_id)
                .ok_or(InterpreterError::InvalidGeneratorId { generator_id: gen_id })?;
            (generator.func_id, generator.status, generator.reg_count)
        };

        let _func = state.module.get_function(func_id)
            .ok_or(InterpreterError::FunctionNotFound(func_id))?;

        // PC is relative to function start (matching handle_call which sets pc=0)
        let (resume_pc, restore_registers, restore_contexts) = match status {
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
        state.call_stack.push_frame(func_id, reg_count, return_pc, dst)?;
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

        match result {
            Ok(value) => {
                if state.generators.get(gen_id)
                    .map(|g| g.status == GeneratorStatus::Yielded)
                    .unwrap_or(false)
                {
                    let yielded = state.generators.get(gen_id)
                        .and_then(|g| g.yielded_value)
                        .unwrap_or(value);
                    state.set_reg(dst, yielded);
                    state.set_reg(has_next_dst, Value::from_bool(true));
                } else {
                    state.set_reg(dst, Value::unit());
                    state.set_reg(has_next_dst, Value::from_bool(false));
                }
            }
            Err(e) => return Err(e),
        }

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
            let list_header = unsafe {
                source_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
            };
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
                backing_ptr.add(heap::OBJECT_HEADER_SIZE + current_idx * std::mem::size_of::<Value>()) as *const Value
            };
            let element = unsafe { *elem_ptr };

            // Advance iterator
            unsafe { *iter_data.add(1) = Value::from_i64((current_idx + 1) as i64); }

            state.set_reg(dst, element);
            state.set_reg(has_next_dst, Value::from_bool(true));
        }
        ITER_TYPE_MAP => {
            // Read map header: [count, capacity, entries_ptr]
            let map_header = unsafe {
                source_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
            };
            let capacity = unsafe { (*map_header.add(1)).as_i64() } as usize;
            let entries_ptr = unsafe { (*map_header.add(2)).as_ptr::<u8>() };

            let entries_data = unsafe {
                entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
            };

            // Find next non-empty entry starting from current_idx
            let mut idx = current_idx;
            while idx < capacity {
                let entry_key = unsafe { *entries_data.add(idx * 2) };
                if !entry_key.is_unit() {
                    // Found an entry - return the key (or could return key-value pair)
                    // For simplicity, we return the key
                    let entry_val = unsafe { *entries_data.add(idx * 2 + 1) };

                    // Advance iterator to next slot
                    unsafe { *iter_data.add(1) = Value::from_i64((idx + 1) as i64); }

                    // Return the value (key is implicit in the iteration order)
                    // In a full implementation, we might return a (key, value) tuple
                    state.set_reg(dst, entry_val);
                    state.set_reg(has_next_dst, Value::from_bool(true));
                    return Ok(DispatchResult::Continue);
                }
                idx += 1;
            }

            // Exhausted
            unsafe { *iter_data.add(1) = Value::from_i64(capacity as i64); }
            state.set_reg(dst, Value::unit());
            state.set_reg(has_next_dst, Value::from_bool(false));
        }
        ITER_TYPE_ARRAY => {
            // Read array length from header (arrays store len in first slot after object header)
            let array_header = unsafe {
                source_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
            };
            // For arrays, we use a simpler layout: elements directly after header
            // The length is stored separately or passed as metadata
            // For now, treat similarly to list
            let len = unsafe { (*array_header).as_i64() } as usize;

            if current_idx >= len {
                state.set_reg(dst, Value::unit());
                state.set_reg(has_next_dst, Value::from_bool(false));
                return Ok(DispatchResult::Continue);
            }

            let elem_ptr = unsafe {
                array_header.add(1 + current_idx)
            };
            let element = unsafe { *elem_ptr };

            unsafe { *iter_data.add(1) = Value::from_i64((current_idx + 1) as i64); }

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
            let range_header = unsafe {
                source_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
            };

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
            unsafe { *iter_data.add(1) = Value::from_i64(current_val + 1); }
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
///   [0] start:      Starting value (Int)
///   [1] end:        Ending value (Int)
///   [2] inclusive:  Whether end is included (Bool: 0 or 1)
pub(in super::super) fn handle_new_range(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
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
    let obj = state.heap.alloc(crate::types::TypeId::RANGE, 3 * std::mem::size_of::<Value>())?;
    state.record_allocation();

    let base_ptr = obj.as_ptr() as *mut u8;
    let data_ptr = unsafe {
        base_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
    };

    // Write range data - must match IterNext's expected layout
    unsafe {
        *data_ptr = Value::from_i64(start_int);         // [0] start
        *data_ptr.add(1) = Value::from_i64(end_int);    // [1] end
        *data_ptr.add(2) = Value::from_bool(inclusive); // [2] inclusive flag
    }

    state.set_reg(dst, Value::from_ptr(base_ptr));
    Ok(DispatchResult::Continue)
}

