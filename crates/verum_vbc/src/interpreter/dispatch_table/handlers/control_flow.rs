//! Control flow and logic operation handlers for VBC interpreter dispatch.

use crate::value::Value;
use super::super::super::error::InterpreterResult;
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::super::do_return;
use super::bytecode_io::*;
use super::string_helpers::deep_value_eq;

// ============================================================================
// Handler Implementations - Logic Operations
// ============================================================================

pub(in super::super) fn handle_land(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let result = state.get_reg(a).as_bool() && state.get_reg(b).as_bool();
    state.set_reg(dst, Value::from_bool(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_lor(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let result = state.get_reg(a).as_bool() || state.get_reg(b).as_bool();
    state.set_reg(dst, Value::from_bool(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_lnot(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let val = state.get_reg(src);
    // **Logical NOT** — always returns a Bool, never a bitwise complement.
    // Pre-fix the non-Bool branch fell through to integer bitwise NOT
    // (`!1i64 = -2`), which an `if` then evaluated as truthy and took
    // the wrong branch. Surface symptom: `if !is_valid_page_size(4096)`
    // when `is_valid_page_size` returns Bool true that wasn't NaN-boxed
    // as `is_bool()=true` (cross-module Call return path stores 1 as
    // Int) — pager_open_memory_with_size(4096) fails on a perfectly-
    // valid 4 KiB page.
    //

    // The bitwise complement is a separate operator (`UnOp::BitNot`)
    // that emits a different opcode (Bitwise::Not). `Opcode::Not`
    // (handle_lnot) is unconditionally logical: truthy → false,
    // falsy → true. This matches Verum's source-level semantics
    // ("`!` is logical, `~` is bitwise") and AOT codegen — which
    // already emits a `select` against the truthiness of the value,
    // not a bitwise xor.
    let result = !val.is_truthy();
    state.set_reg(dst, Value::from_bool(result));
    Ok(DispatchResult::Continue)
}

/// Logical XOR: dst = a ^^ b
pub(in super::super) fn handle_lxor(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let result = state.get_reg(a).as_bool() ^ state.get_reg(b).as_bool();
    state.set_reg(dst, Value::from_bool(result));
    Ok(DispatchResult::Continue)
}

// ============================================================================
// Handler Implementations - Jump/Branch Instructions
// ============================================================================

pub(in super::super) fn handle_jump(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let offset = read_signed_varint(state)? as i32;
    let new_pc = (state.pc() as i64 + offset as i64) as u32;
    state.set_pc(new_pc);
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_jump_if(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let cond = read_reg(state)?;
    let offset = read_signed_varint(state)? as i32;
    if state.get_reg(cond).is_truthy() {
        let new_pc = (state.pc() as i64 + offset as i64) as u32;
        state.set_pc(new_pc);
    }
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_jump_if_not(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let cond = read_reg(state)?;
    let offset = read_signed_varint(state)? as i32;
    let cond_val = state.get_reg(cond);
    if !cond_val.is_truthy() {
        let new_pc = (state.pc() as i64 + offset as i64) as u32;
        state.set_pc(new_pc);
    }
    Ok(DispatchResult::Continue)
}

/// Fused compare-and-jump: if a == b then jump
pub(in super::super) fn handle_jump_eq(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let offset = read_signed_varint(state)? as i32;
    let va = state.get_reg(a);
    let vb = state.get_reg(b);
    // Use deep equality for proper type handling
    if deep_value_eq(&va, &vb, state) {
        let new_pc = (state.pc() as i64 + offset as i64) as u32;
        state.set_pc(new_pc);
    }
    Ok(DispatchResult::Continue)
}

/// Fused compare-and-jump: if a != b then jump
pub(in super::super) fn handle_jump_ne(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let offset = read_signed_varint(state)? as i32;
    let va = state.get_reg(a);
    let vb = state.get_reg(b);
    // Use deep equality for proper type handling
    if !deep_value_eq(&va, &vb, state) {
        let new_pc = (state.pc() as i64 + offset as i64) as u32;
        state.set_pc(new_pc);
    }
    Ok(DispatchResult::Continue)
}

/// Fused compare-and-jump: if a < b then jump
pub(in super::super) fn handle_jump_lt(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let offset = read_signed_varint(state)? as i32;
    let va = state.get_reg(a);
    let vb = state.get_reg(b);
    if va.as_i64() < vb.as_i64() {
        let new_pc = (state.pc() as i64 + offset as i64) as u32;
        state.set_pc(new_pc);
    }
    Ok(DispatchResult::Continue)
}

/// Fused compare-and-jump: if a <= b then jump
pub(in super::super) fn handle_jump_le(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let offset = read_signed_varint(state)? as i32;
    let va = state.get_reg(a);
    let vb = state.get_reg(b);
    if va.as_i64() <= vb.as_i64() {
        let new_pc = (state.pc() as i64 + offset as i64) as u32;
        state.set_pc(new_pc);
    }
    Ok(DispatchResult::Continue)
}

/// Fused compare-and-jump: if a > b then jump
pub(in super::super) fn handle_jump_gt(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let offset = read_signed_varint(state)? as i32;
    let va = state.get_reg(a);
    let vb = state.get_reg(b);
    if va.as_i64() > vb.as_i64() {
        let new_pc = (state.pc() as i64 + offset as i64) as u32;
        state.set_pc(new_pc);
    }
    Ok(DispatchResult::Continue)
}

/// Fused compare-and-jump: if a >= b then jump
pub(in super::super) fn handle_jump_ge(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let offset = read_signed_varint(state)? as i32;
    let va = state.get_reg(a);
    let vb = state.get_reg(b);
    if va.as_i64() >= vb.as_i64() {
        let new_pc = (state.pc() as i64 + offset as i64) as u32;
        state.set_pc(new_pc);
    }
    Ok(DispatchResult::Continue)
}

// ============================================================================
// Handler Implementations - Return
// ============================================================================

pub(in super::super) fn handle_return(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let src = read_reg(state)?;
    let value = state.get_reg(src);
    do_return(state, value)
}

pub(in super::super) fn handle_return_unit(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    do_return(state, Value::unit())
}
