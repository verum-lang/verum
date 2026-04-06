//! String operation handlers for VBC interpreter.

use crate::value::Value;
use super::super::super::error::InterpreterResult;
use super::super::super::state::InterpreterState;
use super::super::super::heap;
use super::super::DispatchResult;
use super::super::format_value_for_print;
use super::bytecode_io::*;

// ============================================================================
// String Operations
// ============================================================================

/// ToString (0x7A) - Convert value to string.
///
/// Encoding: opcode + dst + src
/// Effect: Converts `src` value to a string representation and stores in `dst`.
pub(in super::super) fn handle_to_string(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let value = state.get_reg(src);

    // Convert value to string
    let string_repr = format_value_for_print(state, value);

    // Store as a string - try small string optimization first
    if let Some(small_str_value) = Value::from_small_string(&string_repr) {
        state.set_reg(dst, small_str_value);
    } else {
        // Need to allocate on heap
        let bytes = string_repr.as_bytes();
        let len = bytes.len();

        // Allocate string data on heap: [len: u64][bytes...]
        let alloc_size = 8 + len;
        let obj = state.heap.alloc(crate::types::TypeId(0x0001), alloc_size)?; // String type
        state.record_allocation();
        let base_ptr = obj.as_ptr() as *mut u8;

        unsafe {
            let data_offset = heap::OBJECT_HEADER_SIZE;
            // Store length
            let len_ptr = base_ptr.add(data_offset) as *mut u64;
            *len_ptr = len as u64;
            // Store bytes
            let bytes_ptr = base_ptr.add(data_offset + 8);
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), bytes_ptr, len);
        }

        state.set_reg(dst, Value::from_ptr(base_ptr));
    }

    Ok(DispatchResult::Continue)
}

/// Concat (0x7B) - Concatenate strings.
///
/// Encoding: opcode + dst + a + b
/// Effect: Concatenates strings `a` and `b` into `dst`.
pub(in super::super) fn handle_concat(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a_reg = read_reg(state)?;
    let b_reg = read_reg(state)?;

    let a_value = state.get_reg(a_reg);
    let b_value = state.get_reg(b_reg);

    // Convert both to strings
    let a_str = format_value_for_print(state, a_value);
    let b_str = format_value_for_print(state, b_value);

    let result = format!("{}{}", a_str, b_str);

    // Store result - try small string optimization first
    if let Some(small_str_value) = Value::from_small_string(&result) {
        state.set_reg(dst, small_str_value);
    } else {
        let bytes = result.as_bytes();
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

        state.set_reg(dst, Value::from_ptr(base_ptr));
    }

    Ok(DispatchResult::Continue)
}

/// CharToStr (0xCB) - Convert Char to string.
///
/// Encoding: opcode + dst + src
/// Effect: Converts a Char value (stored as Int codepoint) to a 1-character string.
pub(in super::super) fn handle_char_to_str(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let value = state.get_reg(src);

    // Get the character codepoint
    let codepoint = value.as_i64();

    // Convert to char (validate codepoint)
    let c = if (0..=0x10FFFF).contains(&codepoint) {
        char::from_u32(codepoint as u32).unwrap_or('\u{FFFD}')
    } else {
        '\u{FFFD}' // replacement character for invalid codepoints
    };

    // Create string from the character
    let string_repr = c.to_string();

    // Store as a string - try small string optimization first (1-4 bytes for a char is always small)
    if let Some(small_str_value) = Value::from_small_string(&string_repr) {
        state.set_reg(dst, small_str_value);
    } else {
        // Shouldn't happen for single char, but handle just in case
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

        state.set_reg(dst, Value::from_ptr(base_ptr));
    }

    Ok(DispatchResult::Continue)
}

