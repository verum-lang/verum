//! String and value comparison helpers for VBC interpreter dispatch.

use crate::types::TypeId;
use crate::value::Value;
use super::super::super::error::InterpreterResult;
use super::super::super::state::InterpreterState;
use super::super::super::heap;
use super::cbgr_helpers::{is_cbgr_ref, decode_cbgr_ref};

// ============================================================================
// Value Comparison Helpers
// ============================================================================

/// Extracts a string from a Value (small string or heap string).
pub(super) fn extract_string(value: &Value, _state: &InterpreterState) -> String {
    use heap::OBJECT_HEADER_SIZE;

    if value.is_small_string() {
        value.as_small_string().as_str().to_string()
    } else if value.is_ptr() {
        let ptr = value.as_ptr::<u8>();
        if ptr.is_null() {
            String::new()
        } else {
            // The pointer points TO the ObjectHeader.
            // Layout: [ObjectHeader (24 bytes)][len: u64 (8 bytes)][byte data...]
            let header = unsafe { &*(ptr as *const heap::ObjectHeader) };

            // Check for string types: TEXT (0x7004) or concat/heap-string (0x0001)
            if header.type_id == crate::types::TypeId::TEXT
                || header.type_id == crate::types::TypeId(0x0001)
            {
                // Read length from after the header
                let len_ptr = unsafe { ptr.add(OBJECT_HEADER_SIZE) as *const u64 };
                let len = unsafe { *len_ptr } as usize;
                if len > 0 && len <= 1_000_000 {
                    let bytes_ptr = unsafe { ptr.add(OBJECT_HEADER_SIZE + 8) };
                    let bytes = unsafe { std::slice::from_raw_parts(bytes_ptr, len) };
                    String::from_utf8_lossy(bytes).to_string()
                } else if len == 0 {
                    String::new()
                } else {
                    // Fallback: try header.size field for TEXT type
                    let sz = header.size as usize;
                    if sz > 0 && sz <= 1_000_000 {
                        let data_ptr = unsafe { ptr.add(OBJECT_HEADER_SIZE) };
                        let bytes = unsafe { std::slice::from_raw_parts(data_ptr, sz) };
                        String::from_utf8_lossy(bytes).to_string()
                    } else {
                        String::new()
                    }
                }
            } else {
                format!("<ptr@{:p}>", ptr)
            }
        }
    } else {
        format!("<value:{}>", value.to_bits())
    }
}

/// Allocates a string on the heap or as a small string, returning a Value.
/// Uses small-string optimization when the string fits in 6 bytes.
pub(super) fn alloc_string_value(state: &mut InterpreterState, s: &str) -> InterpreterResult<Value> {
    if let Some(sv) = Value::from_small_string(s) {
        return Ok(sv);
    }
    let bytes = s.as_bytes();
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
    Ok(Value::from_ptr(obj.as_ptr() as *mut u8))
}

/// Check if a Value is a heap-allocated string (pointer to object with TEXT type id or concat layout).
pub(super) fn is_heap_string(v: &Value) -> bool {
    // Small strings are NOT heap strings (they're inline NaN-boxed)
    if v.is_small_string() {
        return false;
    }
    if !v.is_ptr() || v.is_nil() || v.is_boxed_int() {
        return false;
    }
    let ptr = v.as_ptr::<u8>();
    if ptr.is_null() {
        return false;
    }
    // Verify pointer alignment before dereferencing (ObjectHeader requires 4-byte alignment)
    if !(ptr as usize).is_multiple_of(std::mem::align_of::<heap::ObjectHeader>()) {
        return false;
    }
    let header = unsafe { &*(ptr as *const heap::ObjectHeader) };
    // TEXT type or the concat type (0x0001)
    header.type_id == crate::types::TypeId::TEXT || header.type_id == crate::types::TypeId(0x0001)
}

/// Check if a Value is an integer that represents a string table ID.
pub(super) fn is_string_id(v: &Value, state: &InterpreterState) -> bool {
    if !v.is_int() || v.is_bool() {
        return false;
    }
    let idx = v.as_i64();
    (0..100000).contains(&idx) && state.module.get_string(crate::types::StringId(idx as u32)).is_some()
}

/// Resolve a Value to its string content, handling all three representations:
/// small string, heap string, or string table ID.
pub(super) fn resolve_string_value(v: &Value, state: &InterpreterState) -> String {
    if v.is_small_string() {
        return v.as_small_string().as_str().to_string();
    }
    if v.is_ptr() && !v.is_nil() && !v.is_boxed_int() {
        let ptr = v.as_ptr::<u8>();
        if !ptr.is_null() {
            unsafe {
                let data_offset = heap::OBJECT_HEADER_SIZE;
                let len_ptr = ptr.add(data_offset) as *const u64;
                let len = *len_ptr as usize;
                if len <= 65536 {
                    let bytes_ptr = ptr.add(data_offset + 8);
                    let bytes = std::slice::from_raw_parts(bytes_ptr, len);
                    if let Ok(s) = std::str::from_utf8(bytes) {
                        return s.to_string();
                    }
                }
            }
        }
    }
    if v.is_int() && !v.is_bool() {
        let idx = v.as_i64();
        if let Some(s) = state.module.get_string(crate::types::StringId(idx as u32)) {
            return s.to_string();
        }
    }
    format!("<value:{}>", v.to_bits())
}

/// Check if a type_id represents an array-like type (flat value array, List, or Array).
pub(super) fn is_array_type_id(type_id: u32) -> bool {
    type_id == 0  // TypeId::UNIT - flat value arrays
        || type_id == TypeId::LIST.0
        || type_id == TypeId::ARRAY.0
        || type_id == TypeId::TUPLE.0
}

/// Recursively compares two values for deep equality.
///
/// Handles primitives, strings, and nested variant types (Maybe<T>, Result<T,E>, etc.).
/// Variant layout: ObjectHeader + [tag:u32][padding:u32][payload:Value]
///
/// NOTE: This function calls `get_array_length` and `get_array_element` which remain
/// in dispatch_table.rs. When those are extracted, the imports here will need updating.
pub(super) fn deep_value_eq(va: &Value, vb: &Value, state: &InterpreterState) -> bool {
    deep_value_eq_depth(va, vb, state, 0)
}

/// Maximum recursion depth for deep equality comparison.
/// Prevents infinite loops on recursive Heap<T> variant structures.
const MAX_EQ_DEPTH: usize = 64;

fn deep_value_eq_depth(va: &Value, vb: &Value, state: &InterpreterState, depth: usize) -> bool {
    if depth >= MAX_EQ_DEPTH {
        // At max depth, fall back to bit-level comparison
        return va.to_bits() == vb.to_bits();
    }
    use heap::OBJECT_HEADER_SIZE;

    // IEEE 754: NaN is never equal to anything, including itself.
    // We must check this before the bit pattern optimization.
    // NaN values have TAG_NAN (0x7), so check for that tag.
    const TAG_NAN: u8 = 0x7;
    if va.tag() == Some(TAG_NAN) || vb.tag() == Some(TAG_NAN) {
        return false;
    }
    // Also check for unboxed NaN floats (values that are_float() but have NaN bit patterns)
    if va.is_float() && va.as_f64().is_nan() {
        return false;
    }
    if vb.is_float() && vb.as_f64().is_nan() {
        return false;
    }

    // Fast path: identical bit patterns (safe now that NaN is handled)
    if va.to_bits() == vb.to_bits() {
        return true;
    }

    // Handle ThinRef values - compare dereferenced values
    if va.is_thin_ref() && vb.is_thin_ref() {
        let thin_a = va.as_thin_ref();
        let thin_b = vb.as_thin_ref();
        if thin_a.is_null() && thin_b.is_null() {
            return true;
        }
        if thin_a.is_null() || thin_b.is_null() {
            return false;
        }
        // Dereference and compare the pointed-to values
        let deref_a = unsafe { *(thin_a.ptr as *const Value) };
        let deref_b = unsafe { *(thin_b.ptr as *const Value) };
        return deep_value_eq_depth(&deref_a, &deref_b, state, depth + 1);
    }

    // Handle FatRef values - compare dereferenced values
    if va.is_fat_ref() && vb.is_fat_ref() {
        let fat_a = va.as_fat_ref();
        let fat_b = vb.as_fat_ref();
        if fat_a.is_null() && fat_b.is_null() {
            return true;
        }
        if fat_a.is_null() || fat_b.is_null() {
            return false;
        }
        // Dereference and compare the pointed-to values
        let deref_a = unsafe { *(fat_a.ptr() as *const Value) };
        let deref_b = unsafe { *(fat_b.ptr() as *const Value) };
        return deep_value_eq_depth(&deref_a, &deref_b, state, depth + 1);
    }

    // Handle mixed reference types (ThinRef vs CBGR register ref)
    // ThinRef points to heap memory, CBGR register ref points to a register
    if va.is_thin_ref() && is_cbgr_ref(vb) {
        let thin_a = va.as_thin_ref();
        if thin_a.is_null() {
            return false;
        }
        let deref_a = unsafe { *(thin_a.ptr as *const Value) };
        let (abs_idx, _gen) = decode_cbgr_ref(vb.as_i64());
        let deref_b = state.registers.get_absolute(abs_idx);
        return deep_value_eq_depth(&deref_a, &deref_b, state, depth + 1);
    }

    if is_cbgr_ref(va) && vb.is_thin_ref() {
        let thin_b = vb.as_thin_ref();
        if thin_b.is_null() {
            return false;
        }
        let (abs_idx, _gen) = decode_cbgr_ref(va.as_i64());
        let deref_a = state.registers.get_absolute(abs_idx);
        let deref_b = unsafe { *(thin_b.ptr as *const Value) };
        return deep_value_eq_depth(&deref_a, &deref_b, state, depth + 1);
    }

    // Handle two CBGR register references - compare dereferenced values
    if is_cbgr_ref(va) && is_cbgr_ref(vb) {
        let (abs_idx_a, _gen_a) = decode_cbgr_ref(va.as_i64());
        let (abs_idx_b, _gen_b) = decode_cbgr_ref(vb.as_i64());
        let deref_a = state.registers.get_absolute(abs_idx_a);
        let deref_b = state.registers.get_absolute(abs_idx_b);
        return deep_value_eq_depth(&deref_a, &deref_b, state, depth + 1);
    }

    // Handle primitives first
    if va.is_float() && vb.is_float() {
        return va.as_f64() == vb.as_f64();
    }
    if va.is_bool() && vb.is_bool() {
        return va.as_bool() == vb.as_bool();
    }
    // Bool ↔ Int cross-type comparison: Bool is a 1-bit integer (false=0, true=1)
    if (va.is_bool() && vb.is_int()) || (va.is_int() && vb.is_bool()) {
        let ia = if va.is_bool() { va.as_bool() as i64 } else { va.as_i64() };
        let ib = if vb.is_bool() { vb.as_bool() as i64 } else { vb.as_i64() };
        return ia == ib;
    }
    if va.is_unit() && vb.is_unit() {
        return true;
    }
    if va.is_nil() && vb.is_nil() {
        return true;
    }

    // Check if either value could be a string representation.
    // Strings in VBC can be: small strings, heap string pointers, or integer string IDs.
    // We must handle cross-representation comparison.
    let a_is_string_like = va.is_small_string() || is_heap_string(va) || is_string_id(va, state);
    let b_is_string_like = vb.is_small_string() || is_heap_string(vb) || is_string_id(vb, state);

    if a_is_string_like && b_is_string_like {
        let str_a = resolve_string_value(va, state);
        let str_b = resolve_string_value(vb, state);
        return str_a == str_b;
    }

    // Pure integer comparison (only when neither is a string ID)
    if va.is_int() && vb.is_int() {
        return va.as_i64() == vb.as_i64();
    }

    // Handle small strings (one side is small_string, other is non-string)
    if va.is_small_string() || vb.is_small_string() {
        return false; // Type mismatch: one is string, other is not
    }

    // Handle heap pointers (strings, variants, objects)
    if va.is_ptr() && vb.is_ptr() {
        let ptr_a = va.as_ptr::<u8>();
        let ptr_b = vb.as_ptr::<u8>();

        if ptr_a.is_null() && ptr_b.is_null() {
            return true;
        }
        if ptr_a.is_null() || ptr_b.is_null() {
            return false;
        }

        // Read type_id from object header (first u32 field)
        let type_id_a = unsafe { *(ptr_a as *const u32) };
        let type_id_b = unsafe { *(ptr_b as *const u32) };

        // Variant types have type_id >= 0x8000 (0x8000 + tag)
        if type_id_a >= 0x8000 && type_id_b >= 0x8000 {
            // Both are variants - compare tags first
            let tag_a = unsafe { *(ptr_a.add(OBJECT_HEADER_SIZE) as *const u32) };
            let tag_b = unsafe { *(ptr_b.add(OBJECT_HEADER_SIZE) as *const u32) };

            if tag_a != tag_b {
                return false; // Different variant tags
            }

            // Same tag - compare all payload fields
            let header_a = unsafe { &*(ptr_a as *const heap::ObjectHeader) };
            let header_b = unsafe { &*(ptr_b as *const heap::ObjectHeader) };
            let size_a = header_a.size as usize;
            let size_b = header_b.size as usize;
            if size_a != size_b {
                return false;
            }
            let field_count = size_a.saturating_sub(8) / std::mem::size_of::<Value>();
            let payload_offset = OBJECT_HEADER_SIZE + 8;
            for i in 0..field_count {
                let fa = unsafe { &*(ptr_a.add(payload_offset + i * std::mem::size_of::<Value>()) as *const Value) };
                let fb = unsafe { &*(ptr_b.add(payload_offset + i * std::mem::size_of::<Value>()) as *const Value) };
                if !deep_value_eq_depth(fa, fb, state, depth + 1) {
                    return false;
                }
            }
            return true;
        } else if (type_id_a == 0 || type_id_a == TypeId::TUPLE.0) && (type_id_b == 0 || type_id_b == TypeId::TUPLE.0) {
            // Both are tuple/pack objects (TypeId 0 or TypeId::TUPLE)
            let header_a = unsafe { &*(ptr_a as *const heap::ObjectHeader) };
            let header_b = unsafe { &*(ptr_b as *const heap::ObjectHeader) };
            let size_a = header_a.size as usize;
            let size_b = header_b.size as usize;
            if size_a != size_b {
                return false;
            }
            let field_count = size_a / std::mem::size_of::<Value>();
            let data_offset = OBJECT_HEADER_SIZE;
            for i in 0..field_count {
                let fa = unsafe { &*(ptr_a.add(data_offset + i * std::mem::size_of::<Value>()) as *const Value) };
                let fb = unsafe { &*(ptr_b.add(data_offset + i * std::mem::size_of::<Value>()) as *const Value) };
                if !deep_value_eq_depth(fa, fb, state, depth + 1) {
                    return false;
                }
            }
            return true;
        } else if is_array_type_id(type_id_a) && is_array_type_id(type_id_b) {
            // Array/List structural comparison
            let header_a = unsafe { &*(ptr_a as *const heap::ObjectHeader) };
            let header_b = unsafe { &*(ptr_b as *const heap::ObjectHeader) };
            let len_a = super::super::get_array_length(ptr_a, header_a).unwrap_or(0);
            let len_b = super::super::get_array_length(ptr_b, header_b).unwrap_or(0);
            if len_a != len_b {
                return false;
            }
            for i in 0..len_a {
                let ea = super::super::get_array_element(ptr_a, header_a, i).unwrap_or(Value::nil());
                let eb = super::super::get_array_element(ptr_b, header_b, i).unwrap_or(Value::nil());
                if !deep_value_eq_depth(&ea, &eb, state, depth + 1) {
                    return false;
                }
            }
            return true;
        } else if type_id_a == type_id_b {
            // Same non-variant type - likely strings
            let str_a = extract_string(va, state);
            let str_b = extract_string(vb, state);
            return str_a == str_b;
        } else {
            // Different types -> not equal
            return false;
        }
    }

    // Type mismatch
    false
}
