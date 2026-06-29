//! Interpreter handlers for the embedded scripting engine (`core.script`).
//!
//! These back the `Script*` sub-ops of [`ExtendedSubOpcode`] (0x20–0x2F),
//! dispatched from [`super::extended::handle_extended`].  They are a thin
//! marshaling layer over [`ScriptEngine`] / [`ScriptOutcome`]: they read
//! operand registers, (un)box opaque host-owned handles, and write results
//! back.  All the real work — compile-via-hook, run-on-a-fresh-interpreter,
//! value classification — lives in [`crate::interpreter::script_engine`].
//!
//! ## Operand layout
//!
//! Value-returning ops carry their destination register FIRST, then their
//! argument registers (this matches the encoder in
//! `codegen::expressions::emit_intrinsic_instructions` for the
//! `ExtendedSubOp` strategy).  No-return ops carry only their argument
//! registers.
//!
//! ## Handle lifetime
//!
//! `script_engine_new` / `script_engine_eval` allocate a `Box` and return its
//! raw pointer as an opaque `*const Byte` Value.  Ownership transfers to the
//! script; the `core.script` `Engine` / `Outcome` wrappers free it via the
//! matching `*_free` op (typically from their `Drop`).  Each pointer is freed
//! exactly once.

use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::read_reg;
use super::path_ops_runtime::extract_string_if_text;
use super::string_helpers::alloc_string_value;
use crate::interpreter::script_engine::{ScriptEngine, ScriptOutcome, ScriptValueOwned};
use crate::value::Value;

/// Read an opaque outcome handle from a register, erroring on null.
fn read_outcome_ptr(
    state: &mut InterpreterState,
) -> InterpreterResult<*const ScriptOutcome> {
    let reg = read_reg(state)?;
    let ptr = state.get_reg(reg).as_ptr::<ScriptOutcome>();
    if ptr.is_null() {
        return Err(InterpreterError::NullPointer);
    }
    Ok(ptr)
}

/// `script_engine_new() -> RawScriptEngine`. Allocates a fresh engine.
pub(in super::super) fn handle_script_engine_new(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let engine = Box::into_raw(Box::new(ScriptEngine::new()));
    state.set_reg(dst, Value::from_ptr(engine as *mut u8));
    Ok(DispatchResult::Continue)
}

/// `script_engine_free(engine)`. Drops the engine box.
pub(in super::super) fn handle_script_engine_free(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let engine_reg = read_reg(state)?;
    let ptr = state.get_reg(engine_reg).as_ptr::<ScriptEngine>() as *mut ScriptEngine;
    if !ptr.is_null() {
        // SAFETY: `ptr` originates from `Box::into_raw` in
        // `handle_script_engine_new` and is freed exactly once (the
        // `core.script` `Engine` wrapper's Drop).
        unsafe { drop(Box::from_raw(ptr)) };
    }
    Ok(DispatchResult::Continue)
}

/// `script_engine_eval(engine, src: Text) -> RawScriptOutcome`.
/// Compiles + runs `src` on `engine`, boxing the result.
pub(in super::super) fn handle_script_engine_eval(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let engine_reg = read_reg(state)?;
    let src_reg = read_reg(state)?;

    let engine_ptr = state.get_reg(engine_reg).as_ptr::<ScriptEngine>() as *mut ScriptEngine;
    if engine_ptr.is_null() {
        return Err(InterpreterError::NullPointer);
    }
    let src_val = state.get_reg(src_reg);
    let source =
        super::path_ops_runtime::extract_string_if_text(state, &src_val).unwrap_or_default();

    // SAFETY: `engine_ptr` is a live `Box<ScriptEngine>` handle. The nested
    // evaluation runs on its own interpreter state, distinct from `state`, so
    // there is no aliasing of the host frame.
    let outcome = unsafe { (*engine_ptr).eval_to_outcome(&source) };
    let outcome_ptr = Box::into_raw(Box::new(outcome));
    state.set_reg(dst, Value::from_ptr(outcome_ptr as *mut u8));
    Ok(DispatchResult::Continue)
}

/// `script_outcome_is_ok(outcome) -> Bool`.
pub(in super::super) fn handle_script_outcome_is_ok(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let ptr = read_outcome_ptr(state)?;
    // SAFETY: `ptr` is a non-null live `Box<ScriptOutcome>` handle.
    let ok = unsafe { (*ptr).is_ok() };
    state.set_reg(dst, Value::from_bool(ok));
    Ok(DispatchResult::Continue)
}

/// `script_outcome_kind(outcome) -> Int`.
pub(in super::super) fn handle_script_outcome_kind(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let ptr = read_outcome_ptr(state)?;
    // SAFETY: see `handle_script_outcome_is_ok`.
    let kind = unsafe { (*ptr).kind() };
    state.set_reg(dst, Value::from_i64(kind));
    Ok(DispatchResult::Continue)
}

/// `script_outcome_as_int(outcome) -> Int`.
pub(in super::super) fn handle_script_outcome_as_int(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let ptr = read_outcome_ptr(state)?;
    // SAFETY: see `handle_script_outcome_is_ok`.
    let v = unsafe { (*ptr).as_int() };
    state.set_reg(dst, Value::from_i64(v));
    Ok(DispatchResult::Continue)
}

/// `script_outcome_as_float(outcome) -> Float`.
pub(in super::super) fn handle_script_outcome_as_float(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let ptr = read_outcome_ptr(state)?;
    // SAFETY: see `handle_script_outcome_is_ok`.
    let v = unsafe { (*ptr).as_float() };
    state.set_reg(dst, Value::from_f64(v));
    Ok(DispatchResult::Continue)
}

/// `script_outcome_as_bool(outcome) -> Bool`.
pub(in super::super) fn handle_script_outcome_as_bool(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let ptr = read_outcome_ptr(state)?;
    // SAFETY: see `handle_script_outcome_is_ok`.
    let v = unsafe { (*ptr).as_bool() };
    state.set_reg(dst, Value::from_bool(v));
    Ok(DispatchResult::Continue)
}

/// `script_outcome_free(outcome)`. Drops the outcome box.
pub(in super::super) fn handle_script_outcome_free(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let reg = read_reg(state)?;
    let ptr = state.get_reg(reg).as_ptr::<ScriptOutcome>() as *mut ScriptOutcome;
    if !ptr.is_null() {
        // SAFETY: `ptr` originates from `Box::into_raw` in
        // `handle_script_engine_eval` and is freed exactly once.
        unsafe { drop(Box::from_raw(ptr)) };
    }
    Ok(DispatchResult::Continue)
}

/// `script_outcome_as_text(outcome) -> Text`. Builds a host-heap Text from the
/// outcome's marshaled text value (empty when the value isn't text).
pub(in super::super) fn handle_script_outcome_as_text(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let ptr = read_outcome_ptr(state)?;
    // SAFETY: `ptr` is a non-null live `Box<ScriptOutcome>` handle.
    let text = unsafe { (*ptr).as_text().to_string() };
    let value = alloc_string_value(state, &text)?;
    state.set_reg(dst, value);
    Ok(DispatchResult::Continue)
}

/// `script_outcome_error(outcome) -> Text`. The error message, or empty.
pub(in super::super) fn handle_script_outcome_error(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let ptr = read_outcome_ptr(state)?;
    // SAFETY: see `handle_script_outcome_as_text`.
    let msg = unsafe { (*ptr).error_message().unwrap_or_default() };
    let value = alloc_string_value(state, &msg)?;
    state.set_reg(dst, value);
    Ok(DispatchResult::Continue)
}

/// `script_outcome_stdout(outcome) -> Text`. The script's captured stdout.
pub(in super::super) fn handle_script_outcome_stdout(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let ptr = read_outcome_ptr(state)?;
    // SAFETY: see `handle_script_outcome_as_text`.
    let out = unsafe { (*ptr).stdout().to_string() };
    let value = alloc_string_value(state, &out)?;
    state.set_reg(dst, value);
    Ok(DispatchResult::Continue)
}

/// `script_engine_new_sandboxed(mem, instr, time) -> RawScriptEngine`.
/// Allocates an engine with the given resource limits (0 = unlimited).
pub(in super::super) fn handle_script_engine_new_sandboxed(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let mem_reg = read_reg(state)?;
    let instr_reg = read_reg(state)?;
    let time_reg = read_reg(state)?;
    let mem = state.get_reg(mem_reg).as_i64().max(0) as usize;
    let instr = state.get_reg(instr_reg).as_i64().max(0) as u64;
    let time = state.get_reg(time_reg).as_i64().max(0) as u64;
    let engine = Box::into_raw(Box::new(ScriptEngine::sandboxed(mem, instr, time)));
    state.set_reg(dst, Value::from_ptr(engine as *mut u8));
    Ok(DispatchResult::Continue)
}

// =============================================================================
// Host <-> script data exchange (Phase 1)
//
// Host side (runs in the host interpreter, operates on the engine handle):
//   script_engine_set_global_{int,text}(engine, name, value)
// Script side (runs in the nested script interpreter, operates on the seeded
// `state.host_globals`):
//   script_global_{kind,int,text}(name)
// =============================================================================

/// Read a `Text` argument register into an owned `String` (empty if not text).
fn read_name_arg(state: &mut InterpreterState) -> InterpreterResult<String> {
    let reg = read_reg(state)?;
    let v = state.get_reg(reg);
    Ok(extract_string_if_text(state, &v).unwrap_or_default())
}

/// `script_engine_set_global_int(engine, name, value)`.
pub(in super::super) fn handle_script_engine_set_global_int(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let engine_reg = read_reg(state)?;
    let name = read_name_arg(state)?;
    let value_reg = read_reg(state)?;
    let value = state.get_reg(value_reg).as_i64();
    let ptr = state.get_reg(engine_reg).as_ptr::<ScriptEngine>() as *mut ScriptEngine;
    if !ptr.is_null() {
        // SAFETY: `ptr` is a live `Box<ScriptEngine>` handle.
        unsafe { (*ptr).set_global(name, ScriptValueOwned::Int(value)) };
    }
    Ok(DispatchResult::Continue)
}

/// `script_engine_set_global_text(engine, name, value)`.
pub(in super::super) fn handle_script_engine_set_global_text(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let engine_reg = read_reg(state)?;
    let name = read_name_arg(state)?;
    let value_reg = read_reg(state)?;
    let value_val = state.get_reg(value_reg);
    let text = extract_string_if_text(state, &value_val).unwrap_or_default();
    let ptr = state.get_reg(engine_reg).as_ptr::<ScriptEngine>() as *mut ScriptEngine;
    if !ptr.is_null() {
        // SAFETY: see `handle_script_engine_set_global_int`.
        unsafe { (*ptr).set_global(name, ScriptValueOwned::Text(text)) };
    }
    Ok(DispatchResult::Continue)
}

/// `script_global_kind(name) -> Int` — the dynamic-kind tag of a host global
/// (`0` if absent): 0=Nil,1=Bool,2=Int,3=Float,4=Text/other.
pub(in super::super) fn handle_script_global_kind(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let name = read_name_arg(state)?;
    let kind: i64 = match state.host_globals.get(&name) {
        None => 0,
        Some(v) if v.is_unit() || v.is_nil() => 0,
        Some(v) if v.is_bool() => 1,
        Some(v) if v.is_int() => 2,
        Some(v) if v.is_float() => 3,
        Some(_) => 4,
    };
    state.set_reg(dst, Value::from_i64(kind));
    Ok(DispatchResult::Continue)
}

/// `script_global_int(name) -> Int` — a host global as Int (`0` if absent).
pub(in super::super) fn handle_script_global_int(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let name = read_name_arg(state)?;
    let v = state
        .host_globals
        .get(&name)
        .map(|v| v.as_i64())
        .unwrap_or(0);
    state.set_reg(dst, Value::from_i64(v));
    Ok(DispatchResult::Continue)
}

/// `script_global_text(name) -> Text` — a host global as Text (empty if absent).
pub(in super::super) fn handle_script_global_text(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let name = read_name_arg(state)?;
    match state.host_globals.get(&name).copied() {
        Some(v) => state.set_reg(dst, v),
        None => {
            let empty = alloc_string_value(state, "")?;
            state.set_reg(dst, empty);
        }
    }
    Ok(DispatchResult::Continue)
}
