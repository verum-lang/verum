//! String operation handlers for VBC interpreter.

use super::super::super::error::InterpreterResult;
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::super::format_value_for_print;
use super::bytecode_io::*;
use crate::value::Value;

// ============================================================================
// String Operations
// ============================================================================

/// Store a Rust string as a Text Value: small-string optimized when it
/// fits the 6-byte NaN box, otherwise ONE canonical heap Text record
/// `[ObjectHeader(TEXT)]{ptr, len, cap=0}[bytes…]` (ARCH-P5 final leg —
/// the legacy `TypeId(0x0001)` `[len:u64][bytes…]` form is retired).
#[inline]
fn store_string_result(
    state: &mut InterpreterState,
    dst: crate::instruction::Reg,
    s: &str,
) -> InterpreterResult<DispatchResult> {
    if let Some(small) = Value::from_small_string(s) {
        state.set_reg(dst, small);
    } else {
        let obj = state.heap.alloc_text(s.as_bytes())?;
        state.record_allocation();
        state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
    }
    Ok(DispatchResult::Continue)
}

/// ToString (0x7A) - Convert value to string.
///

/// Encoding: opcode + dst + src
/// Effect: Converts `src` value to a string representation and stores in `dst`.
pub(in super::super) fn handle_to_string(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let value = state.get_reg(src);

    // Convert value to string
    let string_repr = format_value_for_print(state, value);

    store_string_result(state, dst, &string_repr)
}

/// Concat (0x7B) - Concatenate strings.
///

/// Encoding: opcode + dst + a + b
/// Effect: Concatenates strings `a` and `b` into `dst`.
pub(in super::super) fn handle_concat(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a_reg = read_reg(state)?;
    let b_reg = read_reg(state)?;

    let a_value = state.get_reg(a_reg);
    let b_value = state.get_reg(b_reg);

    // Convert both to strings
    let a_str = format_value_for_print(state, a_value);
    let b_str = format_value_for_print(state, b_value);

    let result = format!("{}{}", a_str, b_str);

    store_string_result(state, dst, &result)
}

/// CharToStr (0xCB) - Convert Char to string.
///

/// Encoding: opcode + dst + src
/// Effect: Converts a Char value (stored as Int codepoint) to a 1-character string.
pub(in super::super) fn handle_char_to_str(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
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

    // Create string from the character (1-4 bytes — always fits the
    // small-string box; the heap path is defensive only).
    let string_repr = c.to_string();

    store_string_result(state, dst, &string_repr)
}
