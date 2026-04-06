//! Comparison operation handlers for VBC interpreter dispatch.

use crate::instruction::Reg;
use crate::module::FunctionId;
use crate::types::StringId;
use crate::value::Value;
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::*;
use super::string_helpers::{extract_string, deep_value_eq};

// ============================================================================
// Handler Implementations - Comparison Operations
// ============================================================================

pub(in super::super) fn handle_eqi(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let va = state.get_reg(a);
    let vb = state.get_reg(b);
    let result = deep_value_eq(&va, &vb, state);
    state.set_reg(dst, Value::from_bool(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_nei(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let va = state.get_reg(a);
    let vb = state.get_reg(b);
    let result = !deep_value_eq(&va, &vb, state);
    state.set_reg(dst, Value::from_bool(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_lti(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let result = state.get_reg(a).as_integer_compatible() < state.get_reg(b).as_integer_compatible();
    state.set_reg(dst, Value::from_bool(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_lei(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let result = state.get_reg(a).as_integer_compatible() <= state.get_reg(b).as_integer_compatible();
    state.set_reg(dst, Value::from_bool(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_gti(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let result = state.get_reg(a).as_integer_compatible() > state.get_reg(b).as_integer_compatible();
    state.set_reg(dst, Value::from_bool(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_gei(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let result = state.get_reg(a).as_integer_compatible() >= state.get_reg(b).as_integer_compatible();
    state.set_reg(dst, Value::from_bool(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_eqf(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let result = state.get_reg(a).as_f64() == state.get_reg(b).as_f64();
    state.set_reg(dst, Value::from_bool(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_nef(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let result = state.get_reg(a).as_f64() != state.get_reg(b).as_f64();
    state.set_reg(dst, Value::from_bool(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_ltf(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let result = state.get_reg(a).as_f64() < state.get_reg(b).as_f64();
    state.set_reg(dst, Value::from_bool(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_lef(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let result = state.get_reg(a).as_f64() <= state.get_reg(b).as_f64();
    state.set_reg(dst, Value::from_bool(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_gtf(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let result = state.get_reg(a).as_f64() > state.get_reg(b).as_f64();
    state.set_reg(dst, Value::from_bool(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_gef(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let result = state.get_reg(a).as_f64() >= state.get_reg(b).as_f64();
    state.set_reg(dst, Value::from_bool(result));
    Ok(DispatchResult::Continue)
}

/// EqG (0x3C) - Generic equality via Eq protocol.
///
/// Encoding: opcode + dst:reg + a:reg + b:reg + protocol_id:varint
/// Effect: Sets `dst` to true if `a` equals `b` using deep structural comparison.
///
/// Supports:
/// - Primitive types (int, float, bool, unit, nil)
/// - Strings (both small strings and heap strings)
/// - Variants (Maybe<T>, Result<T,E>, etc.) with recursive payload comparison
pub(in super::super) fn handle_eqg(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let protocol_id = read_varint(state)? as u32;

    let va = state.get_reg(a);
    let vb = state.get_reg(b);

    // If protocol_id is non-zero, dispatch to a custom Eq implementation.
    // protocol_id encodes (string_table_index + 1) of the type name.
    if protocol_id > 0 {
        let type_name_idx = protocol_id - 1;
        if let Some(type_name) = state.module.strings.get(StringId(type_name_idx)) {
            // Use dot separator to match how impl methods are registered (e.g., "Point.eq")
            let eq_func_name = format!("{}.eq", type_name);
            // Search for the eq function in the module
            let mut found_func_id: Option<FunctionId> = None;
            for func in &state.module.functions {
                let func_name = state.module.strings.get(func.name).unwrap_or("");
                if func_name == eq_func_name {
                    found_func_id = Some(func.id);
                    break;
                }
            }

            if let Some(func_id) = found_func_id {
                if let Some(func) = state.module.get_function(func_id) {
                    let reg_count = func.register_count;
                    let return_pc = state.pc();

                    let new_base = state.call_stack.push_frame(func_id, reg_count, return_pc, dst)?;
                    state.registers.push_frame(reg_count);

                    // arg0 = self (va), arg1 = other (vb)
                    state.registers.set(new_base, Reg(0), va);
                    state.registers.set(new_base, Reg(1), vb);

                    state.set_pc(0);
                    state.record_call();
                    return Ok(DispatchResult::Continue);
                }
            }
        }
        // Fall through to structural comparison if function not found
    }

    // Default: use recursive deep structural equality comparison
    let result = deep_value_eq(&va, &vb, state);
    state.set_reg(dst, Value::from_bool(result));
    Ok(DispatchResult::Continue)
}

/// CmpG (0x3D) - Generic comparison via Ord protocol.
///
/// Encoding: opcode + dst:reg + a:reg + b:reg + protocol_id:varint
/// Effect: Sets `dst` to ordering value (-1 for Less, 0 for Equal, 1 for Greater).
///
/// Supports:
/// - Primitive types (int, float)
/// - Strings (lexicographic comparison)
/// - Complex types via deep comparison
pub(in super::super) fn handle_cmpg(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    // Read protocol_id (future: call protocol method)
    let _protocol_id = read_varint(state)? as u32;

    let va = state.get_reg(a);
    let vb = state.get_reg(b);

    // Determine ordering based on value types
    // Bool values are treated as integers (false=0, true=1) for comparison
    let va_is_int_like = va.is_int() || va.is_bool();
    let vb_is_int_like = vb.is_int() || vb.is_bool();
    let ordering: i64 = if va_is_int_like && vb_is_int_like {
        let ia = if va.is_bool() { va.as_bool() as i64 } else { va.as_i64() };
        let ib = if vb.is_bool() { vb.as_bool() as i64 } else { vb.as_i64() };
        if ia < ib { -1 } else if ia > ib { 1 } else { 0 }
    } else if va.is_float() && vb.is_float() {
        let fa = va.as_f64();
        let fb = vb.as_f64();
        // Handle NaN: NaN is not equal to anything, not less or greater
        if fa.is_nan() || fb.is_nan() {
            // NaN comparisons return 0 (undefined ordering)
            0
        } else if fa < fb {
            -1
        } else if fa > fb {
            1
        } else {
            0
        }
    } else if va.is_small_string() || vb.is_small_string() || (va.is_ptr() && vb.is_ptr()) {
        // String comparison
        let str_a = extract_string(&va, state);
        let str_b = extract_string(&vb, state);
        match str_a.cmp(&str_b) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Greater => 1,
            std::cmp::Ordering::Equal => 0,
        }
    } else {
        // Fallback: compare raw bits
        let bits_a = va.to_bits();
        let bits_b = vb.to_bits();
        if bits_a < bits_b { -1 } else if bits_a > bits_b { 1 } else { 0 }
    };

    state.set_reg(dst, Value::from_i64(ordering));
    Ok(DispatchResult::Continue)
}

/// EqRef (0x3E) - Reference equality (pointer comparison).
///
/// Encoding: opcode + dst:reg + a:reg + b:reg
/// Effect: Sets `dst` to true if `a` and `b` point to the same memory location.
///
/// This compares raw pointer values, not the content they point to.
/// For content comparison, use EqG instead.
pub(in super::super) fn handle_eqref(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;

    let va = state.get_reg(a);
    let vb = state.get_reg(b);

    // Compare raw bit representations (pointer equality)
    let result = va.to_bits() == vb.to_bits();
    state.set_reg(dst, Value::from_bool(result));
    Ok(DispatchResult::Continue)
}

/// CmpExtended (0x4F) - Extended comparison operations (unsigned comparisons).
///
/// Encoding: opcode(0x4F) + sub_op:u8 + dst:reg + a:reg + b:reg
/// Sub-opcodes:
/// - 0x00: LtU (unsigned less-than)
/// - 0x01: LeU (unsigned less-or-equal)
/// - 0x02: GtU (unsigned greater-than)
/// - 0x03: GeU (unsigned greater-or-equal)
///
/// Operands are interpreted as u64 for comparison purposes.
pub(in super::super) fn handle_cmp_extended(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let sub_op_byte = read_u8(state)?;
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;

    let va = state.get_reg(a).as_i64() as u64;
    let vb = state.get_reg(b).as_i64() as u64;

    let result = match sub_op_byte {
        0x00 => va < vb,  // LtU
        0x01 => va <= vb, // LeU
        0x02 => va > vb,  // GtU
        0x03 => va >= vb, // GeU
        _ => {
            return Err(InterpreterError::NotImplemented {
                feature: "CmpExtended unknown sub-opcode",
                opcode: Some(crate::instruction::Opcode::CmpExtended),
            });
        }
    };

    state.set_reg(dst, Value::from_bool(result));
    Ok(DispatchResult::Continue)
}
