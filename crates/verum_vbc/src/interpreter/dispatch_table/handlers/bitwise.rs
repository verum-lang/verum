//! Bitwise operations and generic arithmetic handlers for VBC interpreter dispatch.

use crate::value::Value;
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::super::format_value_for_print;
use super::bytecode_io::*;

// ============================================================================
// Handler Implementations - Generic Arithmetic
// ============================================================================

pub(in super::super) fn handle_addg(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    // Generic add via protocol - dispatch to int, float, or string concat
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let val_a = state.get_reg(a);
    let val_b = state.get_reg(b);

    let result = if val_a.is_float() {
        Value::from_f64(val_a.as_f64() + val_b.as_f64())
    } else if val_a.is_small_string() || val_b.is_small_string()
        || (val_a.is_ptr() && !val_a.is_nil() && !val_a.is_int() && !val_a.is_float()
            && val_b.is_ptr() && !val_b.is_nil() && !val_b.is_int() && !val_b.is_float())
    {
        // String concatenation: convert both to strings and concat
        let a_str = format_value_for_print(state, val_a);
        let b_str = format_value_for_print(state, val_b);
        let concat = format!("{}{}", a_str, b_str);
        if let Some(small) = Value::from_small_string(&concat) {
            small
        } else {
            let bytes = concat.as_bytes();
            let len = bytes.len();
            let alloc_size = 8 + len;
            let obj = state.heap.alloc(crate::types::TypeId(0x0001), alloc_size)?;
            state.record_allocation();
            let base_ptr = obj.as_ptr() as *mut u8;
            unsafe {
                let data_offset = crate::interpreter::heap::OBJECT_HEADER_SIZE;
                let len_ptr = base_ptr.add(data_offset) as *mut u64;
                *len_ptr = len as u64;
                let bytes_ptr = base_ptr.add(data_offset + 8);
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), bytes_ptr, len);
            }
            Value::from_ptr(base_ptr)
        }
    } else {
        Value::from_i64(val_a.as_integer_compatible().wrapping_add(val_b.as_integer_compatible()))
    };
    state.set_reg(dst, result);
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_subg(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let val_a = state.get_reg(a);
    let val_b = state.get_reg(b);

    let result = if val_a.is_float() {
        Value::from_f64(val_a.as_f64() - val_b.as_f64())
    } else {
        Value::from_i64(val_a.as_integer_compatible().wrapping_sub(val_b.as_integer_compatible()))
    };
    state.set_reg(dst, result);
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_mulg(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let val_a = state.get_reg(a);
    let val_b = state.get_reg(b);

    let result = if val_a.is_float() {
        Value::from_f64(val_a.as_f64() * val_b.as_f64())
    } else {
        Value::from_i64(val_a.as_integer_compatible().wrapping_mul(val_b.as_integer_compatible()))
    };
    state.set_reg(dst, result);
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_divg(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let val_a = state.get_reg(a);
    let val_b = state.get_reg(b);

    let result = if val_a.is_float() {
        Value::from_f64(val_a.as_f64() / val_b.as_f64())
    } else {
        let divisor = val_b.as_integer_compatible();
        if divisor == 0 {
            return Err(InterpreterError::DivisionByZero);
        }
        Value::from_i64(val_a.as_integer_compatible().wrapping_div(divisor))
    };
    state.set_reg(dst, result);
    Ok(DispatchResult::Continue)
}

// ============================================================================
// Handler Implementations - Bitwise Operations
// ============================================================================

pub(in super::super) fn handle_band(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let result = state.get_reg(a).as_integer_compatible() & state.get_reg(b).as_integer_compatible();
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_bor(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let result = state.get_reg(a).as_integer_compatible() | state.get_reg(b).as_integer_compatible();
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_bxor(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let result = state.get_reg(a).as_integer_compatible() ^ state.get_reg(b).as_integer_compatible();
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_shl(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let shift = (state.get_reg(b).as_integer_compatible() & 63) as u32;
    let result = state.get_reg(a).as_integer_compatible().wrapping_shl(shift);
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

/// Arithmetic (signed) shift right: `dst = a >> b`
/// Sign bit is preserved (sign-extended)
pub(in super::super) fn handle_shr(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let shift = (state.get_reg(b).as_integer_compatible() & 63) as u32;
    // Arithmetic shift: shift i64 directly to preserve sign bit
    let result = state.get_reg(a).as_integer_compatible().wrapping_shr(shift);
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

/// Logical (unsigned) shift right: `dst = a >>> b`
pub(in super::super) fn handle_ushr(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let shift = (state.get_reg(b).as_integer_compatible() & 63) as u32;
    let result = (state.get_reg(a).as_integer_compatible() as u64).wrapping_shr(shift) as i64;
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_bnot(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    // Bnot uses 3-register encoding (dst, a, b) for consistency with other bitwise ops
    // but b is ignored for NOT operation
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let _ignored = read_reg(state)?; // b register is ignored for NOT
    let result = !state.get_reg(src).as_integer_compatible();
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}
