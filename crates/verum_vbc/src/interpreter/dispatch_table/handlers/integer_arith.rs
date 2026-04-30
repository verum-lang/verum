//! Integer arithmetic handlers for VBC interpreter dispatch.

use crate::value::Value;
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::super::format_value_for_print;
use super::bytecode_io::*;

// ============================================================================
// Handler Implementations - Integer Arithmetic
// ============================================================================

pub(in super::super) fn handle_addi(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let val_a = state.get_reg(a);
    let val_b = state.get_reg(b);

    // Fast path: both are inline integers (most common case)
    // Check tag bits directly via is_inline_int() to skip the string check entirely
    if val_a.is_inline_int() && val_b.is_inline_int() {
        let result = val_a.as_integer_compatible().wrapping_add(val_b.as_integer_compatible());
        state.set_reg(dst, Value::from_i64(result));
        return Ok(DispatchResult::Continue);
    }

    // Slow path: string concatenation fallback
    if val_a.is_small_string() || val_b.is_small_string() {
        let a_str = format_value_for_print(state, val_a);
        let b_str = format_value_for_print(state, val_b);
        let concat = format!("{}{}", a_str, b_str);
        let result = if let Some(small) = Value::from_small_string(&concat) {
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
        };
        state.set_reg(dst, result);
    } else {
        // Non-inline integers (boxed, pointer-tagged from compiled stdlib, etc.) — extract and add
        let result = val_a.as_integer_compatible().wrapping_add(val_b.as_integer_compatible());
        state.set_reg(dst, Value::from_i64(result));
    }
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_subi(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let va = state.get_reg(a);
    let vb = state.get_reg(b);
    // Use `as_integer_compatible` (matches `handle_addi`) so operands that
    // are not tagged Int — pointer-tagged values from compiled stdlib,
    // Unit/Nil holes, small-string residuals — do not panic. The CBGR
    // allocator's `Shared::new` path passes `SharedInner<T>.size` through
    // a codegen path that lands here on a value still wearing its
    // construction-time tag (observed in `Shared<Int>::new(42)`).
    let result = va.as_integer_compatible().wrapping_sub(vb.as_integer_compatible());
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_muli(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let va = state.get_reg(a);
    let vb = state.get_reg(b);
    // Same tag-robustness as handle_addi / handle_subi.
    let result = va.as_integer_compatible().wrapping_mul(vb.as_integer_compatible());
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_divi(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let divisor = state.get_reg(b).as_integer_compatible();
    if divisor == 0 {
        return Err(InterpreterError::DivisionByZero);
    }
    let result = state.get_reg(a).as_integer_compatible().wrapping_div(divisor);
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_modi(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let divisor = state.get_reg(b).as_integer_compatible();
    if divisor == 0 {
        return Err(InterpreterError::DivisionByZero);
    }
    let result = state.get_reg(a).as_integer_compatible().wrapping_rem(divisor);
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

/// Unsigned integer division: `dst = (a as u64) / (b as u64)`.
///
/// Reinterprets the i64 register payloads as `u64` for the division,
/// then stores the u64 result back as the same bit pattern. Required
/// because `(u64::MAX) / 10 = 1844674407370955161` whereas
/// `(i64)(-1) / 10 = 0` — same bit pattern, different operations.
/// `Text.parse_int` and any other stdlib path that operates on
/// `UInt64` magnitudes ≥ 2^63 depends on this.
pub(in super::super) fn handle_udivi(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let divisor = state.get_reg(b).as_integer_compatible() as u64;
    if divisor == 0 {
        return Err(InterpreterError::DivisionByZero);
    }
    let dividend = state.get_reg(a).as_integer_compatible() as u64;
    let result = dividend.wrapping_div(divisor) as i64;
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

/// Unsigned integer remainder: `dst = (a as u64) % (b as u64)`.
/// Sister handler to `handle_udivi` — same justification.
pub(in super::super) fn handle_umodi(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let divisor = state.get_reg(b).as_integer_compatible() as u64;
    if divisor == 0 {
        return Err(InterpreterError::DivisionByZero);
    }
    let dividend = state.get_reg(a).as_integer_compatible() as u64;
    let result = dividend.wrapping_rem(divisor) as i64;
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

// ============================================================================
// Handler Implementations - Unary Integer Operations
// ============================================================================

pub(in super::super) fn handle_negi(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let result = state.get_reg(src).as_integer_compatible().wrapping_neg();
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

// ============================================================================
// Handler Implementations - More Arithmetic (0x28-0x2F)
// ============================================================================

/// Integer power: `dst = a ** b`
pub(in super::super) fn handle_powi(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let base = read_reg(state)?;
    let exp = read_reg(state)?;
    let base_val = state.get_reg(base).as_integer_compatible();
    let exp_val = state.get_reg(exp).as_integer_compatible();
    // Use checked power to handle overflow
    let result = if exp_val >= 0 && exp_val <= u32::MAX as i64 {
        base_val.wrapping_pow(exp_val as u32)
    } else {
        0 // Negative exponent for int returns 0 (integer truncation)
    };
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

/// Integer absolute value: `dst = |src|`
pub(in super::super) fn handle_absi(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let src_val = state.get_reg(src);
    let result = src_val.as_integer_compatible().wrapping_abs();
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

/// Increment: `dst = src + 1`
pub(in super::super) fn handle_inc(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let result = state.get_reg(src).as_integer_compatible().wrapping_add(1);
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

/// Decrement: `dst = src - 1`
pub(in super::super) fn handle_dec(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let result = state.get_reg(src).as_integer_compatible().wrapping_sub(1);
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}
