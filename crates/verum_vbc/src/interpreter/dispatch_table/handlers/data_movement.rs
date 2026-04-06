//! Data movement and type conversion handlers for VBC interpreter dispatch.

use crate::value::Value;
use crate::types::TypeId;
use crate::module::ConstId;
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::super::load_constant;
use super::bytecode_io::*;

// ============================================================================
// Handler Implementations - Data Movement
// ============================================================================

pub(in super::super) fn handle_mov(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let value = state.get_reg(src);
    state.set_reg(dst, value);
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_loadk(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let const_id = read_varint(state)? as u32;
    let value = load_constant(state, ConstId(const_id))?;
    state.set_reg(dst, value);
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_loadi(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let value = read_signed_varint(state)?;
    state.set_reg(dst, Value::from_i64(value));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_loadf(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let value = read_f64(state)?;
    state.set_reg(dst, Value::from_f64(value));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_load_true(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    state.set_reg(dst, Value::from_bool(true));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_load_false(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    state.set_reg(dst, Value::from_bool(false));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_load_unit(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    state.set_reg(dst, Value::unit());
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_loadt(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let type_id = read_varint(state)? as u32;
    state.set_reg(dst, Value::from_type(TypeId(type_id)));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_load_smalli(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let value = read_i8(state)? as i64;
    state.set_reg(dst, Value::from_i64(value));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_load_nil(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    state.set_reg(dst, Value::nil());
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_nop(_state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    Ok(DispatchResult::Continue)
}

// ============================================================================
// Handler Implementations - Type Conversions
// ============================================================================

pub(in super::super) fn handle_cvt_if(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let int_val = state.get_reg(src).as_i64();
    state.set_reg(dst, Value::from_f64(int_val as f64));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_cvt_fi(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let mode = read_u8(state)?;
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let float_val = state.get_reg(src).as_f64();

    let int_val = match mode {
        0 => float_val.trunc() as i64,
        1 => float_val.floor() as i64,
        2 => float_val.ceil() as i64,
        3 => float_val.round() as i64,
        _ => float_val.trunc() as i64,
    };
    state.set_reg(dst, Value::from_i64(int_val));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_cvt_ic(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let int_val = state.get_reg(src).as_i64();

    if !(0..=0x10FFFF).contains(&int_val) || (0xD800..=0xDFFF).contains(&int_val) {
        return Err(InterpreterError::InvalidCharConversion {
            value: int_val,
            reason: "out of Unicode range or surrogate".to_string(),
        });
    }
    // SAFETY: range check above guarantees valid Unicode scalar value
    let ch = char::from_u32(int_val as u32).ok_or(InterpreterError::InvalidCharConversion {
        value: int_val,
        reason: "char::from_u32 returned None despite range check".to_string(),
    })?;
    state.set_reg(dst, Value::from_char(ch));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_cvt_ci(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let char_val = state.get_reg(src).as_char();
    state.set_reg(dst, Value::from_i64(char_val as i64));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_cvt_bi(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let bool_val = state.get_reg(src).as_bool();
    state.set_reg(dst, Value::from_i64(if bool_val { 1 } else { 0 }));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_cvt_toi(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let val = state.get_reg(src);

    // Note: chars are stored as integers, so is_int() covers them too
    let int_val = if val.is_int() {
        val.as_i64()
    } else if val.is_float() {
        val.as_f64().trunc() as i64
    } else if val.is_bool() {
        if val.as_bool() { 1 } else { 0 }
    } else if val.is_ptr() {
        // Pointer to Int: extract the memory address as an integer.
        // This supports `ptr as Int` for pointer arithmetic and ordering.
        val.as_ptr::<u8>() as usize as i64
    } else {
        0
    };
    state.set_reg(dst, Value::from_i64(int_val));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_cvt_tof(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let val = state.get_reg(src);

    let float_val = if val.is_float() {
        val.as_f64()
    } else if val.is_int() {
        val.as_i64() as f64
    } else {
        0.0
    };
    state.set_reg(dst, Value::from_f64(float_val));
    Ok(DispatchResult::Continue)
}
