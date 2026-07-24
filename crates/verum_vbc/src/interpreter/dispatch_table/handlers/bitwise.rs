//! Bitwise operations and generic arithmetic handlers for VBC interpreter dispatch.

use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::super::format_value_for_print;
use super::bytecode_io::*;
use super::integer_arith::{i128_result_signed, is_i128_op};
use crate::value::Value;

// ============================================================================
// Handler Implementations - Generic Arithmetic
// ============================================================================

pub(in super::super) fn handle_addg(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    // Generic add via protocol - dispatch to int, float, or string concat
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let val_a = state.get_reg(a);
    let val_b = state.get_reg(b);

    let result = if val_a.is_float() {
        Value::from_f64(val_a.as_f64() + val_b.as_f64())
    } else if val_a.is_small_string()
        || val_b.is_small_string()
        || (val_a.is_ptr()
            && !val_a.is_nil()
            && !val_a.is_int()
            && !val_a.is_float()
            && val_b.is_ptr()
            && !val_b.is_nil()
            && !val_b.is_int()
            && !val_b.is_float())
    {
        // String concatenation: convert both to strings and concat
        let a_str = format_value_for_print(state, val_a);
        let b_str = format_value_for_print(state, val_b);
        let concat = format!("{}{}", a_str, b_str);
        if let Some(small) = Value::from_small_string(&concat) {
            small
        } else {
            // Canonical heap Text record (ARCH-P5 final leg).
            let obj = state.heap.alloc_text(concat.as_bytes())?;
            state.record_allocation();
            Value::from_ptr(obj.as_ptr() as *mut u8)
        }
    } else if is_i128_op(val_a, val_b) {
        // 128-bit arm (T0272): full-width generic add.
        Value::from_i128_raw_signed(
            val_a.as_i128_raw().wrapping_add(val_b.as_i128_raw()),
            i128_result_signed(val_a, val_b),
        )
    } else {
        Value::from_i64(
            val_a
                .as_integer_compatible()
                .wrapping_add(val_b.as_integer_compatible()),
        )
    };
    state.set_reg(dst, result);
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_subg(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let val_a = state.get_reg(a);
    let val_b = state.get_reg(b);

    let result = if val_a.is_float() {
        Value::from_f64(val_a.as_f64() - val_b.as_f64())
    } else if is_i128_op(val_a, val_b) {
        Value::from_i128_raw_signed(
            val_a.as_i128_raw().wrapping_sub(val_b.as_i128_raw()),
            i128_result_signed(val_a, val_b),
        )
    } else {
        Value::from_i64(
            val_a
                .as_integer_compatible()
                .wrapping_sub(val_b.as_integer_compatible()),
        )
    };
    state.set_reg(dst, result);
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_mulg(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let val_a = state.get_reg(a);
    let val_b = state.get_reg(b);

    let result = if val_a.is_float() {
        Value::from_f64(val_a.as_f64() * val_b.as_f64())
    } else if is_i128_op(val_a, val_b) {
        Value::from_i128_raw_signed(
            val_a.as_i128_raw().wrapping_mul(val_b.as_i128_raw()),
            i128_result_signed(val_a, val_b),
        )
    } else {
        Value::from_i64(
            val_a
                .as_integer_compatible()
                .wrapping_mul(val_b.as_integer_compatible()),
        )
    };
    state.set_reg(dst, result);
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_divg(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let val_a = state.get_reg(a);
    let val_b = state.get_reg(b);

    let result = if val_a.is_float() {
        Value::from_f64(val_a.as_f64() / val_b.as_f64())
    } else if is_i128_op(val_a, val_b) {
        // Generic divide is signed (matches DivI / handle_divg's i64 arm).
        let divisor = val_b.as_i128_raw() as i128;
        if divisor == 0 {
            return Err(InterpreterError::DivisionByZero);
        }
        Value::from_i128_raw_signed(
            (val_a.as_i128_raw() as i128).wrapping_div(divisor) as u128,
            i128_result_signed(val_a, val_b),
        )
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

pub(in super::super) fn handle_band(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let va = state.get_reg(a);
    let vb = state.get_reg(b);
    if is_i128_op(va, vb) {
        let result = va.as_i128_raw() & vb.as_i128_raw();
        state.set_reg(
            dst,
            Value::from_i128_raw_signed(result, i128_result_signed(va, vb)),
        );
        return Ok(DispatchResult::Continue);
    }
    let result = va.as_integer_compatible() & vb.as_integer_compatible();
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_bor(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let va = state.get_reg(a);
    let vb = state.get_reg(b);
    if is_i128_op(va, vb) {
        let result = va.as_i128_raw() | vb.as_i128_raw();
        state.set_reg(
            dst,
            Value::from_i128_raw_signed(result, i128_result_signed(va, vb)),
        );
        return Ok(DispatchResult::Continue);
    }
    let result = va.as_integer_compatible() | vb.as_integer_compatible();
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_bxor(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let va = state.get_reg(a);
    let vb = state.get_reg(b);
    if is_i128_op(va, vb) {
        let result = va.as_i128_raw() ^ vb.as_i128_raw();
        state.set_reg(
            dst,
            Value::from_i128_raw_signed(result, i128_result_signed(va, vb)),
        );
        return Ok(DispatchResult::Continue);
    }
    let result = va.as_integer_compatible() ^ vb.as_integer_compatible();
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_shl(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let va = state.get_reg(a);
    let vb = state.get_reg(b);
    // 128-bit arm (T0272): shift the full 128-bit value; the shift amount is
    // masked mod 128 (vs mod 64 for the i64 path).
    if va.is_boxed_i128() {
        let shift = (vb.as_integer_compatible() & 127) as u32;
        let result = va.as_i128_raw().wrapping_shl(shift);
        state.set_reg(
            dst,
            Value::from_i128_raw_signed(result, va.boxed_i128_is_signed()),
        );
        return Ok(DispatchResult::Continue);
    }
    let shift = (vb.as_integer_compatible() & 63) as u32;
    let result = va.as_integer_compatible().wrapping_shl(shift);
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

/// Arithmetic (signed) shift right: `dst = a >> b`
/// Sign bit is preserved (sign-extended)
pub(in super::super) fn handle_shr(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let va = state.get_reg(a);
    let vb = state.get_reg(b);
    // 128-bit arm (T0272): arithmetic (sign-preserving) shift of the full
    // 128-bit value, amount masked mod 128.
    if va.is_boxed_i128() {
        let shift = (vb.as_integer_compatible() & 127) as u32;
        let result = (va.as_i128_raw() as i128).wrapping_shr(shift) as u128;
        state.set_reg(
            dst,
            Value::from_i128_raw_signed(result, va.boxed_i128_is_signed()),
        );
        return Ok(DispatchResult::Continue);
    }
    let shift = (vb.as_integer_compatible() & 63) as u32;
    // Arithmetic shift: shift i64 directly to preserve sign bit
    let result = va.as_integer_compatible().wrapping_shr(shift);
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

/// Logical (unsigned) shift right: `dst = a >>> b`
pub(in super::super) fn handle_ushr(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let va = state.get_reg(a);
    let vb = state.get_reg(b);
    // 128-bit arm (T0272): logical (zero-fill) shift of the full 128-bit
    // value, amount masked mod 128. Result is unsigned.
    if va.is_boxed_i128() {
        let shift = (vb.as_integer_compatible() & 127) as u32;
        let result = va.as_i128_raw().wrapping_shr(shift);
        state.set_reg(dst, Value::from_i128_raw_signed(result, false));
        return Ok(DispatchResult::Continue);
    }
    let shift = (vb.as_integer_compatible() & 63) as u32;
    let result = (va.as_integer_compatible() as u64).wrapping_shr(shift) as i64;
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_bnot(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    // Bnot uses 3-register encoding (dst, a, b) for consistency with other bitwise ops
    // but b is ignored for NOT operation
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let _ignored = read_reg(state)?; // b register is ignored for NOT
    let sv = state.get_reg(src);
    // 128-bit arm (T0272): complement the full 128 bits.
    if sv.is_boxed_i128() {
        let result = !sv.as_i128_raw();
        state.set_reg(
            dst,
            Value::from_i128_raw_signed(result, sv.boxed_i128_is_signed()),
        );
        return Ok(DispatchResult::Continue);
    }
    let result = !sv.as_integer_compatible();
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}
