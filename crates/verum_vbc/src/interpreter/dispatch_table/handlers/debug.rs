//! Debug, assertion, and verification/contract handlers for VBC interpreter dispatch.
//!
//! Handles: Assert (0xD6), Panic (0xD7), Unreachable (0xD8), DebugPrint (0xD9),
//! Spec (0xD4), Guard (0xD5), Requires (0xDA), Ensures (0xDB), Invariant (0xDC)

use crate::module::{ConstId, Constant};
use crate::value::Value;
use crate::interpreter::heap;
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::*;

// ============================================================================
// Handler Implementations - Debug and Assertions
// ============================================================================

/// Assert (0xC2) - Assert condition is true.
///
/// Encoding: opcode + cond + message_id (varint)
/// Effect: If `cond` is false, raises an assertion failure error.
pub(in super::super) fn handle_assert(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let cond_reg = read_reg(state)?;
    let message_id = read_varint(state)? as u32;

    let cond = state.get_reg(cond_reg);

    let is_true = cond.is_truthy();

    if !is_true {
        // Get message from string table
        let message = if let Some(msg) = state.module.get_string(crate::types::StringId(message_id)) {
            msg.to_string()
        } else {
            format!("assertion failed (message_id: {})", message_id)
        };

        return Err(InterpreterError::AssertionFailed {
            message,
            pc: state.pc() as usize,
        });
    }

    Ok(DispatchResult::Continue)
}

/// Panic (0xD7) - Terminate execution with error message.
///
/// Encoding: opcode + message_id (varint)
/// Effect: Raises a Panic error with the message from string pool.
pub(in super::super) fn handle_panic(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let message_id = read_varint(state)? as u32;

    // Get message from string table
    let message = if let Some(msg) = state.module.get_string(crate::types::StringId(message_id)) {
        msg.to_string()
    } else {
        format!("panic! (message_id: {})", message_id)
    };

    Err(InterpreterError::Panic { message })
}

/// Unreachable (0xD8) - Marker for unreachable code paths.
///
/// Encoding: opcode (no operands)
/// Effect: Raises an Unreachable error - indicates a code path that should never execute.
pub(in super::super) fn handle_unreachable(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    Err(InterpreterError::Unreachable { pc: state.pc() as usize })
}

/// DebugPrint (0xC5) - Print value to stdout for debugging.
///
/// Encoding: opcode + value_reg
/// Output: Prints the value to stdout with a newline.
pub(in super::super) fn handle_debug_print(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let value_reg = read_reg(state)?;
    let value = state.get_reg(value_reg);

    // Format the value based on its type tag
    let output = format_value_for_print(state, value);
    state.writeln_stdout(&output);

    Ok(DispatchResult::Continue)
}

/// Format a value for printing (handles strings, ints, floats, bools, variants, lists, etc.)
pub(crate) fn format_value_for_print(state: &InterpreterState, value: Value) -> String {
    format_value_for_print_depth(state, value, 0)
}

/// Maximum recursion depth for printing nested values.
/// Prevents infinite loops on recursive Heap<T> variant structures.
const MAX_PRINT_DEPTH: usize = 32;

fn format_value_for_print_depth(state: &InterpreterState, value: Value, depth: usize) -> String {
    if depth >= MAX_PRINT_DEPTH {
        return "<...>".to_string();
    }
    // Check for small string (inline string up to 7 bytes)
    if value.is_small_string() {
        let ss = value.as_small_string();
        return ss.as_str().to_string();
    }

    // ThinRef / FatRef — deref to the pointed-to value and recurse.
    //
    // `List.iter().next()` and related builtin iterators wrap each
    // element in a ThinRef (see `handlers/method_dispatch.rs` iterator
    // dispatch) so the caller can observe the slot without taking
    // ownership. When an f-string or `print` receives such a
    // reference without explicit deref, we must auto-deref for
    // rendering — otherwise the reference's raw address bits would
    // be formatted as a heap pointer and then re-interpreted as an
    // `ObjectHeader`, which SIGSEGVs on arbitrary memory.
    //
    // We deref into a `Value` (NaN-boxed) because that's how builtin
    // iterator slots are stored. This covers the common case of
    // `List<Int>` / `List<Float>` / `List<Text>` / `List<Bool>` etc.
    if value.is_thin_ref() {
        let tr = value.as_thin_ref();
        if !tr.ptr.is_null() {
            let derefed = unsafe { *(tr.ptr as *const Value) };
            return format_value_for_print_depth(state, derefed, depth + 1);
        }
        return "<null thin ref>".to_string();
    }
    if value.is_fat_ref() {
        let fr = value.as_fat_ref();
        if !fr.thin.ptr.is_null() {
            let derefed = unsafe { *(fr.thin.ptr as *const Value) };
            return format_value_for_print_depth(state, derefed, depth + 1);
        }
        return "<null fat ref>".to_string();
    }

    // Integer values - format as numbers.
    if value.is_int() && !value.is_bool() {
        return format!("{}", value.as_i64());
    }

    // Check various primitive types
    if value.is_nil() {
        return "nil".to_string();
    }

    if value.is_bool() {
        return value.as_bool().to_string();
    }

    if value.is_unit() {
        return "()".to_string();
    }

    // Check for float - always include decimal point for whole numbers
    if value.is_float() {
        let f = value.as_f64();
        let s = format!("{}", f);
        // Ensure whole-number floats display with ".0" (e.g., 5.0 not 5)
        if !s.contains('.') && !s.contains('e') && !s.contains('E') && !s.contains("inf") && !s.contains("NaN") {
            return format!("{}.0", s);
        }
        return s;
    }

    // Check for heap-allocated objects (strings, lists, variants, maps, etc.)
    if value.is_ptr() && !value.is_nil() && !value.is_boxed_int() {
        let base_ptr = value.as_ptr::<u8>();
        if !base_ptr.is_null() {
            // Read the ObjectHeader to determine the type
            let header = unsafe { &*(base_ptr as *const heap::ObjectHeader) };
            let data_offset = heap::OBJECT_HEADER_SIZE;

            // Heap-allocated string: type_id == TEXT (4) or 0x0001 (legacy).
            //
            // Two layouts can land under this type_id:
            //
            //   **Compact** — `[header | len:u64 | bytes[len]]`.
            //   Emitted by `load_constant` for text literals.
            //
            //   **Struct** — `[header | ptr:Value | len:Value | cap:Value]`.
            //   Produced by the stdlib's `Text { ptr, len, cap }` record
            //   literal (see `core/text/text.vr:170`). Each field is a
            //   NaN-boxed `Value` and `header.size == 24`. Same dual-layout
            //   dispatch is mirrored in `string_helpers::format_value_for_print`.
            if header.type_id == crate::types::TypeId::TEXT || header.type_id == crate::types::TypeId(0x0001) {
                unsafe {
                    // Prefer the struct layout when the header advertises
                    // exactly 24 bytes of payload and the first two fields
                    // look like well-formed Values (pointer-or-nil + integer).
                    if header.size as usize == 24 {
                        let field0 = *(base_ptr.add(data_offset) as *const crate::value::Value);
                        let field1 = *((base_ptr.add(data_offset) as *const crate::value::Value).add(1));
                        if (field0.is_ptr() || field0.is_nil()) && field1.is_int() {
                            let builder_ptr = if field0.is_nil() {
                                std::ptr::null::<u8>()
                            } else {
                                field0.as_ptr::<u8>() as *const u8
                            };
                            let builder_len = field1.as_i64() as usize;
                            if builder_ptr.is_null() || builder_len == 0 {
                                return String::new();
                            }
                            if builder_len <= 1 << 30 {
                                let bytes = std::slice::from_raw_parts(builder_ptr, builder_len);
                                if let Ok(s) = std::str::from_utf8(bytes) {
                                    return s.to_string();
                                }
                            }
                        }
                    }

                    // Compact layout: `[header | len:u64 | bytes[len]]`.
                    let len_ptr = base_ptr.add(data_offset) as *const u64;
                    let len = *len_ptr as usize;
                    if len <= 65536 {
                        let bytes_ptr = base_ptr.add(data_offset + 8);
                        let bytes = std::slice::from_raw_parts(bytes_ptr, len);
                        if let Ok(s) = std::str::from_utf8(bytes) {
                            return s.to_string();
                        }
                    }
                }
                return "<invalid string>".to_string();
            }

            // List: type_id == LIST (512)
            if header.type_id == crate::types::TypeId::LIST {
                return format_list_for_print_depth(state, base_ptr, depth + 1);
            }

            // Map: type_id == MAP (513)
            if header.type_id == crate::types::TypeId::MAP {
                return format_map_for_print_depth(state, base_ptr, depth + 1);
            }

            // Set: type_id == SET (514)
            if header.type_id == crate::types::TypeId::SET {
                return format_set_for_print_depth(state, base_ptr, depth + 1);
            }

            // Variant: type_id >= 0x8000
            if header.type_id.0 >= 0x8000 {
                return format_variant_for_print_depth(state, base_ptr, depth + 1);
            }

            // Tuple: type_id == TUPLE (521)
            if header.type_id == crate::types::TypeId::TUPLE {
                let elem_count = header.size as usize / std::mem::size_of::<Value>();
                let data_ptr = unsafe { base_ptr.add(data_offset) as *const Value };
                let mut parts = Vec::new();
                for i in 0..elem_count {
                    let elem = unsafe { *data_ptr.add(i) };
                    parts.push(format_value_for_print_depth(state, elem, depth + 1));
                }
                return format!("({})", parts.join(", "));
            }

            // Record/struct: for objects with fields stored as Values
            // Fall through to a generic object description
            let elem_count = header.size as usize / std::mem::size_of::<Value>();
            if elem_count > 0 && elem_count <= 64 {
                let data_ptr = unsafe { base_ptr.add(data_offset) as *const Value };
                let mut parts = Vec::new();
                for i in 0..elem_count {
                    let elem = unsafe { *data_ptr.add(i) };
                    parts.push(format_value_for_print_depth(state, elem, depth + 1));
                }
                return format!("{{{}}}", parts.join(", "));
            }

            return format!("<object type_id={}>", header.type_id.0);
        }
    }

    // Fallback for unknown types - avoid panicking
    format!("<value 0x{:016x}>", value.to_bits())
}

/// Format a list value for printing: [elem1, elem2, ...]
fn format_list_for_print_depth(state: &InterpreterState, base_ptr: *const u8, depth: usize) -> String {
    if depth >= MAX_PRINT_DEPTH {
        return "[...]".to_string();
    }
    let data_offset = heap::OBJECT_HEADER_SIZE;
    unsafe {
        let data_ptr = base_ptr.add(data_offset) as *const Value;
        let len = (*data_ptr).as_i64() as usize;
        // Field 2 is backing array pointer
        let backing_val = *data_ptr.add(2);
        if !backing_val.is_ptr() || backing_val.is_nil() {
            return "[]".to_string();
        }
        let backing_ptr = backing_val.as_ptr::<u8>();
        if backing_ptr.is_null() {
            return "[]".to_string();
        }
        let backing_data = backing_ptr.add(data_offset) as *const Value;
        let mut parts = Vec::with_capacity(len);
        for i in 0..len {
            let elem = *backing_data.add(i);
            parts.push(format_value_for_print_depth(state, elem, depth + 1));
        }
        format!("[{}]", parts.join(", "))
    }
}

/// Format a variant value for printing: Some(value) or None etc.
fn format_variant_for_print_depth(state: &InterpreterState, base_ptr: *const u8, depth: usize) -> String {
    if depth >= MAX_PRINT_DEPTH {
        return "<...>".to_string();
    }
    let data_offset = heap::OBJECT_HEADER_SIZE;
    unsafe {
        let tag_ptr = base_ptr.add(data_offset) as *const u32;
        let tag = *tag_ptr;
        let field_count = *tag_ptr.add(1) as usize;

        // Look up variant name from module type metadata.
        // Scan the module's type descriptors for a variant with this tag.
        let name_from_metadata = state.module.types.iter().find_map(|td| {
            td.variants.iter().find_map(|v| {
                if v.tag == tag {
                    state.module.get_string(v.name)
                } else {
                    None
                }
            })
        });
        let name = name_from_metadata.unwrap_or("");

        if field_count == 0 {
            if name.is_empty() {
                return format!("Variant({})", tag);
            }
            return name.to_string();
        }

        // Read payload fields
        let payload_ptr = base_ptr.add(data_offset + 8) as *const Value;
        let mut parts = Vec::with_capacity(field_count);
        for i in 0..field_count {
            let field = *payload_ptr.add(i);
            parts.push(format_value_for_print_depth(state, field, depth + 1));
        }

        if name.is_empty() {
            format!("Variant({}, {})", tag, parts.join(", "))
        } else if field_count == 1 {
            format!("{}({})", name, parts[0])
        } else {
            format!("{}({})", name, parts.join(", "))
        }
    }
}

/// Format a map value for printing: {key: value, ...}
fn format_map_for_print_depth(state: &InterpreterState, base_ptr: *const u8, depth: usize) -> String {
    if depth >= MAX_PRINT_DEPTH {
        return "{...}".to_string();
    }
    let data_offset = heap::OBJECT_HEADER_SIZE;
    unsafe {
        let data_ptr = base_ptr.add(data_offset) as *const Value;
        let count = (*data_ptr).as_i64() as usize;
        let cap = (*data_ptr.add(1)).as_i64() as usize;
        if count == 0 {
            return "{}".to_string();
        }
        let entries_val = *data_ptr.add(2);
        if !entries_val.is_ptr() || entries_val.is_nil() {
            return "{}".to_string();
        }
        let entries_ptr = entries_val.as_ptr::<u8>();
        if entries_ptr.is_null() {
            return "{}".to_string();
        }
        let entries_data = entries_ptr.add(data_offset) as *const Value;
        // Map entry layout: [hash, key, value, _reserved] = 4 Values per entry
        let mut parts = Vec::new();
        for i in 0..cap {
            let hash = (*entries_data.add(i * 4)).as_i64();
            if hash != 0 {
                let key = *entries_data.add(i * 4 + 1);
                let val = *entries_data.add(i * 4 + 2);
                parts.push(format!("{}: {}",
                    format_value_for_print_depth(state, key, depth + 1),
                    format_value_for_print_depth(state, val, depth + 1)));
            }
        }
        format!("{{{}}}", parts.join(", "))
    }
}

/// Format a set value for printing: {elem1, elem2, ...}
fn format_set_for_print_depth(state: &InterpreterState, base_ptr: *const u8, depth: usize) -> String {
    if depth >= MAX_PRINT_DEPTH {
        return "{...}".to_string();
    }
    let data_offset = heap::OBJECT_HEADER_SIZE;
    unsafe {
        let data_ptr = base_ptr.add(data_offset) as *const Value;
        let count = (*data_ptr).as_i64() as usize;
        let cap = (*data_ptr.add(1)).as_i64() as usize;
        if count == 0 {
            return "{}".to_string();
        }
        let entries_val = *data_ptr.add(2);
        if !entries_val.is_ptr() || entries_val.is_nil() {
            return "{}".to_string();
        }
        let entries_ptr = entries_val.as_ptr::<u8>();
        if entries_ptr.is_null() {
            return "{}".to_string();
        }
        let entries_data = entries_ptr.add(data_offset) as *const Value;
        // Set entry layout: [hash, key, _unused, _reserved] = 4 Values per entry
        let mut parts = Vec::new();
        for i in 0..cap {
            let hash = (*entries_data.add(i * 4)).as_i64();
            if hash != 0 {
                let key = *entries_data.add(i * 4 + 1);
                parts.push(format_value_for_print_depth(state, key, depth + 1));
            }
        }
        format!("{{{}}}", parts.join(", "))
    }
}

// ============================================================================
// JIT Hint Operations (0xD4-0xD5)
// ============================================================================

/// Spec (0xD4) - JIT specialization hint.
///
/// Encoding: opcode + reg + type_id (varint)
/// Effect: No-op in interpreter (JIT optimization hint).
pub(in super::super) fn handle_spec(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let _reg = read_reg(state)?;
    let _type_id = read_varint(state)?;
    // JIT optimization hint - no-op in interpreter
    Ok(DispatchResult::Continue)
}

/// Guard (0xD5) - Type guard (deopt if mismatch).
///
/// Encoding: opcode + reg + expected_type_id + deopt_offset
/// Effect: If value doesn't match expected type, deoptimize (in JIT) or continue (interpreter).
///
/// In interpreter, this validates type compatibility but doesn't deoptimize.
pub(in super::super) fn handle_guard(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let reg = read_reg(state)?;
    let _expected_type = read_varint(state)?;
    let _deopt_offset = read_signed_varint(state)?;
    // In interpreter, we just validate the value exists and continue
    let _value = state.get_reg(reg);
    Ok(DispatchResult::Continue)
}

// ============================================================================
// Verification/Contract Operations (0xDA-0xDC)
// ============================================================================

/// Requires (0xDA) - Contract precondition check.
///
/// Encoding: opcode + cond_reg + message_const_id
/// Effect: Panics with message if condition is false.
pub(in super::super) fn handle_requires(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let cond_reg = read_reg(state)?;
    let msg_const_id = read_varint(state)?;

    let cond = state.get_reg(cond_reg);
    if !cond.is_truthy() {
        let message = if let Some(Constant::String(string_id)) = state.module.get_constant(ConstId(msg_const_id as u32)).cloned() {
            let msg_str = state.module.get_string(string_id).unwrap_or("<unknown>");
            format!("precondition failed: {}", msg_str)
        } else {
            "precondition failed".to_string()
        };
        return Err(InterpreterError::Panic { message });
    }
    Ok(DispatchResult::Continue)
}

/// Ensures (0xDB) - Contract postcondition check.
///
/// Encoding: opcode + cond_reg + message_const_id
/// Effect: Panics with message if condition is false.
pub(in super::super) fn handle_ensures(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let cond_reg = read_reg(state)?;
    let msg_const_id = read_varint(state)?;

    let cond = state.get_reg(cond_reg);
    if !cond.is_truthy() {
        let message = if let Some(Constant::String(string_id)) = state.module.get_constant(ConstId(msg_const_id as u32)).cloned() {
            let msg_str = state.module.get_string(string_id).unwrap_or("<unknown>");
            format!("postcondition failed: {}", msg_str)
        } else {
            "postcondition failed".to_string()
        };
        return Err(InterpreterError::Panic { message });
    }
    Ok(DispatchResult::Continue)
}

/// Invariant (0xDC) - Loop invariant check.
///
/// Encoding: opcode + cond_reg + message_const_id
/// Effect: Panics with message if condition is false.
pub(in super::super) fn handle_invariant(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let cond_reg = read_reg(state)?;
    let msg_const_id = read_varint(state)?;

    let cond = state.get_reg(cond_reg);
    if !cond.is_truthy() {
        let message = if let Some(Constant::String(string_id)) = state.module.get_constant(ConstId(msg_const_id as u32)).cloned() {
            let msg_str = state.module.get_string(string_id).unwrap_or("<unknown>");
            format!("invariant violated: {}", msg_str)
        } else {
            "invariant violated".to_string()
        };
        return Err(InterpreterError::Panic { message });
    }
    Ok(DispatchResult::Continue)
}
