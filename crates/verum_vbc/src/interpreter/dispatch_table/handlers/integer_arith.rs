//! Integer arithmetic handlers for VBC interpreter dispatch.

use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::super::format_value_for_print;
use super::bytecode_io::*;
use crate::value::Value;

// ============================================================================
// Handler Implementations - Integer Arithmetic
// ============================================================================

/// Coerce an operand to `f64` for the generic-arithmetic float arms
/// (T0497).
///
/// Generic arithmetic erases `a: T, b: T` (with `T = Float`) to a
/// type-param `TypeKind`, so codegen emits the integer `AddI/SubI/…`
/// opcodes rather than their float twins. The handlers below add a
/// float arm that fires when either operand is a real NaN-box float, so
/// the value is computed in `f64` instead of being truncated through
/// `as_integer_compatible`. A real float decodes via `as_f64`; an
/// integer-tagged operand (a mixed `Float`/`Int` generic instantiation)
/// converts through its integer value — never `as_f64` on an int
/// bitcast, whose bit pattern reads back as `NaN`.
#[inline]
fn operand_as_f64(v: Value) -> f64 {
    if v.is_float() {
        v.as_f64()
    } else {
        v.as_integer_compatible() as f64
    }
}

/// T0272: true when either operand is a boxed 128-bit integer, so the op MUST
/// be evaluated at full 128-bit width instead of narrowing through `as_i64`
/// (which keeps only the low 64 bits, silently corrupting wide values). Shared
/// with the bitwise handlers (`super::bitwise`).
#[inline]
pub(super) fn is_i128_op(a: Value, b: Value) -> bool {
    a.is_boxed_i128() || b.is_boxed_i128()
}

/// Signedness to stamp on a 128-bit arithmetic *result*: the primary boxed
/// operand's flag (the typechecker guarantees both operands share a type, so
/// they agree), signed by default. Steers display only — the arithmetic bits
/// are two's-complement and identical for a given signed/unsigned opcode.
#[inline]
pub(super) fn i128_result_signed(a: Value, b: Value) -> bool {
    if a.is_boxed_i128() {
        a.boxed_i128_is_signed()
    } else if b.is_boxed_i128() {
        b.boxed_i128_is_signed()
    } else {
        true
    }
}

pub(in super::super) fn handle_addi(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let val_a = state.get_reg(a);
    let val_b = state.get_reg(b);

    // Fast path: both are inline integers (most common case)
    // Check tag bits directly via is_inline_int() to skip the string check entirely
    if val_a.is_inline_int() && val_b.is_inline_int() {
        let result = val_a
            .as_integer_compatible()
            .wrapping_add(val_b.as_integer_compatible());
        state.set_reg(dst, Value::from_i64(result));
        return Ok(DispatchResult::Continue);
    }

    // ADDI-RESOLVE-1: resolve reference shapes BEFORE classifying the
    // slow path.  `acc += &(*w).clone()` reaches AddI with a CBGR ref
    // to the clone's TEMP register on the RHS; the pre-fix
    // classification saw neither a small string nor an inline int and
    // fell into the integer-extract arm — summing NaN-box/pointer BITS
    // and rendering `acc` as a number ("26826347891").  Refs are never
    // inline ints, so the fast path above is unaffected.
    let val_a = super::cbgr_helpers::resolve_arg_value(state, val_a);
    let val_b = super::cbgr_helpers::resolve_arg_value(state, val_b);

    // FLOAT arm (T0497): a generic `a + b` on `Float` type-params reaches
    // AddI (codegen sees a non-`Float` type-param `TypeKind` and never
    // emits AddF), so without this arm the integer path below truncates
    // both operands (`add_them(2.5, 1.0)` → 3). Placed alongside the
    // string-concat arm, after the reference resolve. Real floats reach
    // AddI only via that generic erasure — the typechecker never feeds
    // AddI two concrete floats — so this is strictly-better, and the
    // both-inline-int fast path above is left untouched (zero cost).
    if val_a.is_float() || val_b.is_float() {
        let result = operand_as_f64(val_a) + operand_as_f64(val_b);
        state.set_reg(dst, Value::from_f64(result));
        return Ok(DispatchResult::Continue);
    }

    // 128-bit arm (T0272): when either operand is a boxed Int128/UInt128,
    // compute the sum at full width and re-box, so the result never collapses
    // through the i64 window. `wrapping_add` on the raw u128 bits is correct
    // for both signednesses (two's complement).
    if is_i128_op(val_a, val_b) {
        let result = val_a.as_i128_raw().wrapping_add(val_b.as_i128_raw());
        state.set_reg(
            dst,
            Value::from_i128_raw_signed(result, i128_result_signed(val_a, val_b)),
        );
        return Ok(DispatchResult::Continue);
    }

    // Slow path: string concatenation fallback.  Heap Texts count —
    // an accumulator that outgrew the 6-byte small-string form must
    // keep concatenating, not fall into the integer-extract arm.
    if val_a.is_small_string()
        || val_b.is_small_string()
        || super::string_helpers::is_heap_string(&val_a)
        || super::string_helpers::is_heap_string(&val_b)
    {
        let a_str = format_value_for_print(state, val_a);
        let b_str = format_value_for_print(state, val_b);
        let concat = format!("{}{}", a_str, b_str);
        let result = if let Some(small) = Value::from_small_string(&concat) {
            small
        } else {
            // Canonical heap Text record (ARCH-P5 final leg).
            let obj = state.heap.alloc_text(concat.as_bytes())?;
            state.record_allocation();
            Value::from_ptr(obj.as_ptr() as *mut u8)
        };
        state.set_reg(dst, result);
    } else {
        // Non-inline integers (boxed, pointer-tagged from compiled stdlib, etc.) — extract and add
        let result = val_a
            .as_integer_compatible()
            .wrapping_add(val_b.as_integer_compatible());
        state.set_reg(dst, Value::from_i64(result));
    }
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_subi(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let va = state.get_reg(a);
    let vb = state.get_reg(b);
    // FLOAT arm (T0497): generic `a - b` on `Float` type-params erases to
    // SubI; without this the integer path truncates the operands. See
    // `handle_addi` for the full rationale.
    if va.is_float() || vb.is_float() {
        let result = operand_as_f64(va) - operand_as_f64(vb);
        state.set_reg(dst, Value::from_f64(result));
        return Ok(DispatchResult::Continue);
    }
    // 128-bit arm (T0272): full-width subtraction, see `handle_addi`.
    if is_i128_op(va, vb) {
        let result = va.as_i128_raw().wrapping_sub(vb.as_i128_raw());
        state.set_reg(
            dst,
            Value::from_i128_raw_signed(result, i128_result_signed(va, vb)),
        );
        return Ok(DispatchResult::Continue);
    }
    // Use `as_integer_compatible` (matches `handle_addi`) so operands that
    // are not tagged Int — pointer-tagged values from compiled stdlib,
    // Unit/Nil holes, small-string residuals — do not panic. The CBGR
    // allocator's `Shared::new` path passes `SharedInner<T>.size` through
    // a codegen path that lands here on a value still wearing its
    // construction-time tag (observed in `Shared<Int>::new(42)`).
    let result = va
        .as_integer_compatible()
        .wrapping_sub(vb.as_integer_compatible());
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_muli(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let va = state.get_reg(a);
    let vb = state.get_reg(b);
    // FLOAT arm (T0497): generic `a * b` on `Float` type-params erases to
    // MulI; see `handle_addi`.
    if va.is_float() || vb.is_float() {
        let result = operand_as_f64(va) * operand_as_f64(vb);
        state.set_reg(dst, Value::from_f64(result));
        return Ok(DispatchResult::Continue);
    }
    // 128-bit arm (T0272): full-width multiply, see `handle_addi`. This is the
    // case that catches two moderate Int128 operands whose product overflows
    // i64 (e.g. 10^12 * 10^12) — the pre-fix i64 path silently wrapped it.
    if is_i128_op(va, vb) {
        let result = va.as_i128_raw().wrapping_mul(vb.as_i128_raw());
        state.set_reg(
            dst,
            Value::from_i128_raw_signed(result, i128_result_signed(va, vb)),
        );
        return Ok(DispatchResult::Continue);
    }
    // Same tag-robustness as handle_addi / handle_subi.
    let result = va
        .as_integer_compatible()
        .wrapping_mul(vb.as_integer_compatible());
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_divi(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let va = state.get_reg(a);
    let vb = state.get_reg(b);
    // FLOAT arm (T0497): generic `a / b` on `Float` type-params erases to
    // DivI; see `handle_addi`. Divide-by-zero is guarded consistently with
    // the integer path below.
    if va.is_float() || vb.is_float() {
        let divisor = operand_as_f64(vb);
        if divisor == 0.0 {
            return Err(InterpreterError::DivisionByZero);
        }
        state.set_reg(dst, Value::from_f64(operand_as_f64(va) / divisor));
        return Ok(DispatchResult::Continue);
    }
    // 128-bit arm (T0272): signed full-width division (DivI is the signed
    // opcode; codegen emits UDivI for unsigned operands, handled separately).
    if is_i128_op(va, vb) {
        let b = vb.as_i128_raw() as i128;
        if b == 0 {
            return Err(InterpreterError::DivisionByZero);
        }
        let result = (va.as_i128_raw() as i128).wrapping_div(b);
        state.set_reg(
            dst,
            Value::from_i128_raw_signed(result as u128, i128_result_signed(va, vb)),
        );
        return Ok(DispatchResult::Continue);
    }
    let divisor = vb.as_integer_compatible();
    if divisor == 0 {
        return Err(InterpreterError::DivisionByZero);
    }
    let result = va.as_integer_compatible().wrapping_div(divisor);
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_modi(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let va = state.get_reg(a);
    let vb = state.get_reg(b);
    // FLOAT arm (T0497): generic `a % b` on `Float` type-params erases to
    // ModI; see `handle_addi`. Divide-by-zero is guarded consistently with
    // the integer path below.
    if va.is_float() || vb.is_float() {
        let divisor = operand_as_f64(vb);
        if divisor == 0.0 {
            return Err(InterpreterError::DivisionByZero);
        }
        state.set_reg(dst, Value::from_f64(operand_as_f64(va) % divisor));
        return Ok(DispatchResult::Continue);
    }
    // 128-bit arm (T0272): signed full-width remainder, see `handle_divi`.
    if is_i128_op(va, vb) {
        let b = vb.as_i128_raw() as i128;
        if b == 0 {
            return Err(InterpreterError::DivisionByZero);
        }
        let result = (va.as_i128_raw() as i128).wrapping_rem(b);
        state.set_reg(
            dst,
            Value::from_i128_raw_signed(result as u128, i128_result_signed(va, vb)),
        );
        return Ok(DispatchResult::Continue);
    }
    let divisor = vb.as_integer_compatible();
    if divisor == 0 {
        return Err(InterpreterError::DivisionByZero);
    }
    let result = va.as_integer_compatible().wrapping_rem(divisor);
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
pub(in super::super) fn handle_udivi(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    // 128-bit arm (T0272): unsigned full-width division for UInt128 operands.
    let va = state.get_reg(a);
    let vb = state.get_reg(b);
    if is_i128_op(va, vb) {
        let divisor = vb.as_i128_raw();
        if divisor == 0 {
            return Err(InterpreterError::DivisionByZero);
        }
        let result = va.as_i128_raw().wrapping_div(divisor);
        state.set_reg(dst, Value::from_i128_raw_signed(result, false));
        return Ok(DispatchResult::Continue);
    }
    let divisor = vb.as_integer_compatible() as u64;
    if divisor == 0 {
        return Err(InterpreterError::DivisionByZero);
    }
    let dividend = va.as_integer_compatible() as u64;
    let result = dividend.wrapping_div(divisor) as i64;
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

/// Unsigned integer remainder: `dst = (a as u64) % (b as u64)`.
/// Sister handler to `handle_udivi` — same justification.
pub(in super::super) fn handle_umodi(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    // 128-bit arm (T0272): unsigned full-width remainder for UInt128 operands.
    let va = state.get_reg(a);
    let vb = state.get_reg(b);
    if is_i128_op(va, vb) {
        let divisor = vb.as_i128_raw();
        if divisor == 0 {
            return Err(InterpreterError::DivisionByZero);
        }
        let result = va.as_i128_raw().wrapping_rem(divisor);
        state.set_reg(dst, Value::from_i128_raw_signed(result, false));
        return Ok(DispatchResult::Continue);
    }
    let divisor = vb.as_integer_compatible() as u64;
    if divisor == 0 {
        return Err(InterpreterError::DivisionByZero);
    }
    let dividend = va.as_integer_compatible() as u64;
    let result = dividend.wrapping_rem(divisor) as i64;
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

// ============================================================================
// Handler Implementations - Unary Integer Operations
// ============================================================================

pub(in super::super) fn handle_negi(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let sv = state.get_reg(src);
    // FLOAT arm (T0497): generic unary `-x` on a `Float` type-param erases
    // to NegI; see `handle_addi`.
    if sv.is_float() {
        state.set_reg(dst, Value::from_f64(-sv.as_f64()));
        return Ok(DispatchResult::Continue);
    }
    // 128-bit arm (T0272): full-width negation (two's complement), preserving
    // the operand's signedness for display.
    if sv.is_boxed_i128() {
        let result = 0u128.wrapping_sub(sv.as_i128_raw());
        state.set_reg(
            dst,
            Value::from_i128_raw_signed(result, sv.boxed_i128_is_signed()),
        );
        return Ok(DispatchResult::Continue);
    }
    let result = sv.as_integer_compatible().wrapping_neg();
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

// ============================================================================
// Handler Implementations - More Arithmetic (0x28-0x2F)
// ============================================================================

/// Integer power: `dst = a ** b`
pub(in super::super) fn handle_powi(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let base = read_reg(state)?;
    let exp = read_reg(state)?;
    let base_v = state.get_reg(base);
    let exp_v = state.get_reg(exp);
    // FLOAT arm (T0497): generic `a ** b` on `Float` type-params erases to
    // PowI; compute via `f64::powf`. See `handle_addi`.
    if base_v.is_float() || exp_v.is_float() {
        let result = operand_as_f64(base_v).powf(operand_as_f64(exp_v));
        state.set_reg(dst, Value::from_f64(result));
        return Ok(DispatchResult::Continue);
    }
    // 128-bit arm (T0272): full-width integer power when the base is a boxed
    // Int128. Exponents are small non-negative counts (a negative exponent
    // truncates to 0, matching the i64 path).
    if base_v.is_boxed_i128() {
        let exp_i128 = exp_v.as_i128_raw() as i128;
        let result = if (0..=u32::MAX as i128).contains(&exp_i128) {
            base_v.as_i128_raw().wrapping_pow(exp_i128 as u32)
        } else {
            0
        };
        state.set_reg(
            dst,
            Value::from_i128_raw_signed(result, base_v.boxed_i128_is_signed()),
        );
        return Ok(DispatchResult::Continue);
    }
    let base_val = base_v.as_integer_compatible();
    let exp_val = exp_v.as_integer_compatible();
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
pub(in super::super) fn handle_absi(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let src_val = state.get_reg(src);
    // FLOAT arm (T0497): generic `|x|` on a `Float` type-param erases to
    // AbsI; see `handle_addi`.
    if src_val.is_float() {
        state.set_reg(dst, Value::from_f64(src_val.as_f64().abs()));
        return Ok(DispatchResult::Continue);
    }
    // 128-bit arm (T0272): full-width absolute value. An unsigned value is
    // already non-negative (identity).
    if src_val.is_boxed_i128() {
        let raw = src_val.as_i128_raw();
        let signed = src_val.boxed_i128_is_signed();
        let result = if signed {
            (raw as i128).wrapping_abs() as u128
        } else {
            raw
        };
        state.set_reg(dst, Value::from_i128_raw_signed(result, signed));
        return Ok(DispatchResult::Continue);
    }
    let result = src_val.as_integer_compatible().wrapping_abs();
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

/// Increment: `dst = src + 1`
pub(in super::super) fn handle_inc(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let sv = state.get_reg(src);
    // 128-bit arm (T0272): keep a boxed Int128 at full width across `+ 1`.
    if sv.is_boxed_i128() {
        let result = sv.as_i128_raw().wrapping_add(1);
        state.set_reg(
            dst,
            Value::from_i128_raw_signed(result, sv.boxed_i128_is_signed()),
        );
        return Ok(DispatchResult::Continue);
    }
    let result = sv.as_integer_compatible().wrapping_add(1);
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

/// Decrement: `dst = src - 1`
pub(in super::super) fn handle_dec(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let sv = state.get_reg(src);
    // 128-bit arm (T0272): keep a boxed Int128 at full width across `- 1`.
    if sv.is_boxed_i128() {
        let result = sv.as_i128_raw().wrapping_sub(1);
        state.set_reg(
            dst,
            Value::from_i128_raw_signed(result, sv.boxed_i128_is_signed()),
        );
        return Ok(DispatchResult::Continue);
    }
    let result = sv.as_integer_compatible().wrapping_sub(1);
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}
