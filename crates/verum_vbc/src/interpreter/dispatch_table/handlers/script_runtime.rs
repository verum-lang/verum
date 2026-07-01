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
use crate::interpreter::script_engine::{ScriptEngine, ScriptOutcome, ScriptValueOwned, ScriptWorld};
use crate::module::FunctionId;
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

    // Pass the host interpreter's state address so the script can call back
    // into host-registered functions (it re-enters this same state). The host
    // is paused here for the whole nested run, so the address stays valid.
    let host_addr = state as *mut InterpreterState as usize;

    // SAFETY: `engine_ptr` is a live `Box<ScriptEngine>` handle. The nested
    // evaluation runs on its own interpreter state, distinct from `state`;
    // host-function callbacks re-enter `state` transactionally via
    // `call_function_sync` (see `handle_script_host_call_int`).
    let outcome = unsafe { (*engine_ptr).eval_to_outcome_with_host(&source, host_addr) };
    let outcome_ptr = Box::into_raw(Box::new(outcome));
    state.set_reg(dst, Value::from_ptr(outcome_ptr as *mut u8));
    Ok(DispatchResult::Continue)
}

/// `script_engine_call(engine, source, fn_name) -> RawScriptOutcome` — compile
/// `source` and run its named `fn_name` entry (rather than `main`), so a host
/// can invoke a script's individual functions. Args flow through the shared
/// globals (`set_*` before, `script_global_*` inside).
pub(in super::super) fn handle_script_engine_call(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let engine_reg = read_reg(state)?;
    let src_reg = read_reg(state)?;
    let fn_reg = read_reg(state)?;

    let engine_ptr = state.get_reg(engine_reg).as_ptr::<ScriptEngine>() as *mut ScriptEngine;
    if engine_ptr.is_null() {
        return Err(InterpreterError::NullPointer);
    }
    let src_val = state.get_reg(src_reg);
    let source =
        super::path_ops_runtime::extract_string_if_text(state, &src_val).unwrap_or_default();
    let fn_val = state.get_reg(fn_reg);
    let fn_name =
        super::path_ops_runtime::extract_string_if_text(state, &fn_val).unwrap_or_default();

    let host_addr = state as *mut InterpreterState as usize;

    // SAFETY: `engine_ptr` is a live `Box<ScriptEngine>` handle; see
    // `handle_script_engine_eval` for the host-re-entry safety contract.
    let outcome =
        unsafe { (*engine_ptr).call_named_with_host(&source, &fn_name, host_addr) };
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

/// `script_outcome_list_len(outcome) -> Int`.
pub(in super::super) fn handle_script_outcome_list_len(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let ptr = read_outcome_ptr(state)?;
    // SAFETY: `ptr` is a non-null live `Box<ScriptOutcome>` handle.
    let len = unsafe { (*ptr).list_len() };
    state.set_reg(dst, Value::from_i64(len));
    Ok(DispatchResult::Continue)
}

/// `script_outcome_list_elem_kind(outcome, idx) -> Int`.
pub(in super::super) fn handle_script_outcome_list_elem_kind(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let ptr = read_outcome_ptr(state)?;
    let idx = read_reg(state)?;
    let i = state.get_reg(idx).as_i64();
    // SAFETY: see `handle_script_outcome_list_len`.
    let k = unsafe { (*ptr).list_elem_kind(i) };
    state.set_reg(dst, Value::from_i64(k));
    Ok(DispatchResult::Continue)
}

/// `script_outcome_list_elem_int(outcome, idx) -> Int`.
pub(in super::super) fn handle_script_outcome_list_elem_int(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let ptr = read_outcome_ptr(state)?;
    let idx = read_reg(state)?;
    let i = state.get_reg(idx).as_i64();
    // SAFETY: see `handle_script_outcome_list_len`.
    let v = unsafe { (*ptr).list_elem_int(i) };
    state.set_reg(dst, Value::from_i64(v));
    Ok(DispatchResult::Continue)
}

/// `script_outcome_list_elem_float(outcome, idx) -> Float`.
pub(in super::super) fn handle_script_outcome_list_elem_float(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let ptr = read_outcome_ptr(state)?;
    let idx = read_reg(state)?;
    let i = state.get_reg(idx).as_i64();
    // SAFETY: see `handle_script_outcome_list_len`.
    let v = unsafe { (*ptr).list_elem_float(i) };
    state.set_reg(dst, Value::from_f64(v));
    Ok(DispatchResult::Continue)
}

/// `script_outcome_list_elem_bool(outcome, idx) -> Bool`.
pub(in super::super) fn handle_script_outcome_list_elem_bool(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let ptr = read_outcome_ptr(state)?;
    let idx = read_reg(state)?;
    let i = state.get_reg(idx).as_i64();
    // SAFETY: see `handle_script_outcome_list_len`.
    let v = unsafe { (*ptr).list_elem_bool(i) };
    state.set_reg(dst, Value::from_bool(v));
    Ok(DispatchResult::Continue)
}

/// `script_outcome_list_elem_text(outcome, idx) -> Text`.
pub(in super::super) fn handle_script_outcome_list_elem_text(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let ptr = read_outcome_ptr(state)?;
    let idx = read_reg(state)?;
    let i = state.get_reg(idx).as_i64();
    // SAFETY: see `handle_script_outcome_list_len`.
    let text = unsafe { (*ptr).list_elem_text(i).to_string() };
    let value = alloc_string_value(state, &text)?;
    state.set_reg(dst, value);
    Ok(DispatchResult::Continue)
}

// --- Map outcome accessors (entries as key/value pairs) ---

/// `script_outcome_map_len(outcome) -> Int`.
pub(in super::super) fn handle_script_outcome_map_len(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let ptr = read_outcome_ptr(state)?;
    // SAFETY: `ptr` is a non-null live `Box<ScriptOutcome>` handle.
    let len = unsafe { (*ptr).map_len() };
    state.set_reg(dst, Value::from_i64(len));
    Ok(DispatchResult::Continue)
}

/// Shared body for the 3-reg `(outcome, idx)` map key/value accessors: reads the
/// outcome ptr + index, calls `f`, stores its `Value` in `dst`.
fn map_entry_accessor(
    state: &mut InterpreterState,
    f: impl FnOnce(&ScriptOutcome, i64) -> Value,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let ptr = read_outcome_ptr(state)?;
    let idx = read_reg(state)?;
    let i = state.get_reg(idx).as_i64();
    // SAFETY: `ptr` is a non-null live `Box<ScriptOutcome>` handle.
    let v = unsafe { f(&*ptr, i) };
    state.set_reg(dst, v);
    Ok(DispatchResult::Continue)
}

/// Shared body for the 3-reg map key/value Text accessors.
fn map_entry_text_accessor(
    state: &mut InterpreterState,
    f: impl FnOnce(&ScriptOutcome, i64) -> String,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let ptr = read_outcome_ptr(state)?;
    let idx = read_reg(state)?;
    let i = state.get_reg(idx).as_i64();
    // SAFETY: `ptr` is a non-null live `Box<ScriptOutcome>` handle.
    let text = unsafe { f(&*ptr, i) };
    let value = alloc_string_value(state, &text)?;
    state.set_reg(dst, value);
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_script_outcome_map_key_kind(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    map_entry_accessor(state, |o, i| Value::from_i64(o.map_key_kind(i)))
}
pub(in super::super) fn handle_script_outcome_map_value_kind(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    map_entry_accessor(state, |o, i| Value::from_i64(o.map_value_kind(i)))
}
pub(in super::super) fn handle_script_outcome_map_key_int(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    map_entry_accessor(state, |o, i| Value::from_i64(o.map_key_int(i)))
}
pub(in super::super) fn handle_script_outcome_map_value_int(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    map_entry_accessor(state, |o, i| Value::from_i64(o.map_value_int(i)))
}
pub(in super::super) fn handle_script_outcome_map_key_float(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    map_entry_accessor(state, |o, i| Value::from_f64(o.map_key_float(i)))
}
pub(in super::super) fn handle_script_outcome_map_value_float(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    map_entry_accessor(state, |o, i| Value::from_f64(o.map_value_float(i)))
}
pub(in super::super) fn handle_script_outcome_map_key_bool(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    map_entry_accessor(state, |o, i| Value::from_bool(o.map_key_bool(i)))
}
pub(in super::super) fn handle_script_outcome_map_value_bool(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    map_entry_accessor(state, |o, i| Value::from_bool(o.map_value_bool(i)))
}
pub(in super::super) fn handle_script_outcome_map_key_text(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    map_entry_text_accessor(state, |o, i| o.map_key_text(i).to_string())
}
pub(in super::super) fn handle_script_outcome_map_value_text(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    map_entry_text_accessor(state, |o, i| o.map_value_text(i).to_string())
}

// --- Nested-collection marshaling: sub-handle accessors ---
//
// When an element of a List/Map is itself a List (kind 5) or Map (kind 6), the
// flat typed accessors can't read it. These wrap the owned nested value in a
// fresh `ScriptOutcome` box and hand back its opaque handle, so the host's
// `.vr` marshaler recurses (`marshal_outcome` on the sub-handle) to arbitrary
// depth and frees it with `script_outcome_free`.

/// Shared body: read (outcome, idx), clone the selected nested value via `f`,
/// box it as a new outcome handle, and store the handle in `dst`.
fn sub_outcome_accessor(
    state: &mut InterpreterState,
    f: impl FnOnce(&ScriptOutcome, i64) -> ScriptValueOwned,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let ptr = read_outcome_ptr(state)?;
    let idx = read_reg(state)?;
    let i = state.get_reg(idx).as_i64();
    // SAFETY: `ptr` is a non-null live `Box<ScriptOutcome>` handle.
    let sub_value = unsafe { f(&*ptr, i) };
    let sub = Box::into_raw(Box::new(ScriptOutcome::from_value(sub_value)));
    state.set_reg(dst, Value::from_ptr(sub as *mut u8));
    Ok(DispatchResult::Continue)
}

/// `script_outcome_list_elem_sub(outcome, i) -> RawScriptOutcome`.
pub(in super::super) fn handle_script_outcome_list_elem_sub(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    sub_outcome_accessor(state, |o, i| o.list_elem_owned(i))
}
/// `script_outcome_map_key_sub(outcome, i) -> RawScriptOutcome`.
pub(in super::super) fn handle_script_outcome_map_key_sub(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    sub_outcome_accessor(state, |o, i| o.map_key_owned(i))
}
/// `script_outcome_map_value_sub(outcome, i) -> RawScriptOutcome`.
pub(in super::super) fn handle_script_outcome_map_value_sub(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    sub_outcome_accessor(state, |o, i| o.map_value_owned(i))
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

/// `script_engine_set_global_bool(engine, name, value)`.
pub(in super::super) fn handle_script_engine_set_global_bool(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let engine_reg = read_reg(state)?;
    let name = read_name_arg(state)?;
    let value_reg = read_reg(state)?;
    let value = state.get_reg(value_reg).as_bool();
    let ptr = state.get_reg(engine_reg).as_ptr::<ScriptEngine>() as *mut ScriptEngine;
    if !ptr.is_null() {
        // SAFETY: see `handle_script_engine_set_global_int`.
        unsafe { (*ptr).set_global(name, ScriptValueOwned::Bool(value)) };
    }
    Ok(DispatchResult::Continue)
}

/// `script_engine_set_global_float(engine, name, value)`.
pub(in super::super) fn handle_script_engine_set_global_float(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let engine_reg = read_reg(state)?;
    let name = read_name_arg(state)?;
    let value_reg = read_reg(state)?;
    let value = state.get_reg(value_reg).as_f64();
    let ptr = state.get_reg(engine_reg).as_ptr::<ScriptEngine>() as *mut ScriptEngine;
    if !ptr.is_null() {
        // SAFETY: see `handle_script_engine_set_global_int`.
        unsafe { (*ptr).set_global(name, ScriptValueOwned::Float(value)) };
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
    // Canonical ScriptValue kind tag (see ScriptValueOwned::kind).
    let value = state.host_globals.get(&name).copied();
    let kind: i64 = match value {
        None => 0,
        Some(v) if v.is_unit() || v.is_nil() => 0,
        Some(v) if v.is_bool() => 1,
        Some(v) if v.is_int() => 2,
        Some(v) if v.is_float() => 3,
        Some(v) if state.read_text(v).is_some() => 4,
        Some(v) if state.list_elements(v).is_some() => 5,
        Some(v) if state.map_entries(v).is_some() => 6,
        Some(_) => 7,
    };
    state.set_reg(dst, Value::from_i64(kind));
    Ok(DispatchResult::Continue)
}

/// `script_global_list(name) -> List` / `script_global_map(name) -> Map` — a
/// host global as a heap collection (reconstructed into this script's heap by
/// `build_value` during seeding). Returns the value as-is; absent → Unit (the
/// caller should gate on `script_global_kind == 5/6`). Shared handler because a
/// reconstructed List/Map is already the right heap object — only the `.vr`
/// return type differs.
pub(in super::super) fn handle_script_global_value(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let name = read_name_arg(state)?;
    let v = state
        .host_globals
        .get(&name)
        .copied()
        .unwrap_or_else(Value::unit);
    state.set_reg(dst, v);
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

/// `script_global_bool(name) -> Bool` — a host global as Bool (`false` if absent).
pub(in super::super) fn handle_script_global_bool(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let name = read_name_arg(state)?;
    let v = state
        .host_globals
        .get(&name)
        .map(|v| v.as_bool())
        .unwrap_or(false);
    state.set_reg(dst, Value::from_bool(v));
    Ok(DispatchResult::Continue)
}

/// `script_global_float(name) -> Float` — a host global as Float (`0.0` if absent).
pub(in super::super) fn handle_script_global_float(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let name = read_name_arg(state)?;
    let v = state
        .host_globals
        .get(&name)
        .map(|v| v.as_f64())
        .unwrap_or(0.0);
    state.set_reg(dst, Value::from_f64(v));
    Ok(DispatchResult::Continue)
}

// =============================================================================
// Host-function callbacks (Phase 1): a script calls back into functions the
// host registered on the engine. The host function runs RE-ENTRANTLY on the
// host interpreter's state (call_function_sync), so it sees the host's module,
// heap and globals. Int -> Int signature for now.
// =============================================================================

/// Extract the underlying `FunctionId` from a function value — either a
/// zero-capture closure (the form a bare `fn` argument compiles to) or a bare
/// function reference.
fn function_id_of(v: &Value) -> Option<FunctionId> {
    if v.is_ptr() && !v.is_nil() {
        let ptr = v.as_ptr::<u8>();
        if ptr.is_null() {
            return None;
        }
        // SAFETY: a pointer-tagged non-null function value is a closure object
        // whose header carries (func_id, capture_count) at the canonical offset.
        let (raw_fid, _captures) = unsafe { crate::interpreter::heap::closure_header(ptr) };
        return Some(FunctionId(raw_fid));
    }
    None
}

/// `script_engine_register(engine, name, fn)` — host registers a callback.
pub(in super::super) fn handle_script_engine_register(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let engine_reg = read_reg(state)?;
    let name = read_name_arg(state)?;
    let fn_reg = read_reg(state)?;
    let func_id = function_id_of(&state.get_reg(fn_reg));
    let ptr = state.get_reg(engine_reg).as_ptr::<ScriptEngine>() as *mut ScriptEngine;
    if !ptr.is_null() {
        if let Some(fid) = func_id {
            // SAFETY: `ptr` is a live `Box<ScriptEngine>` handle.
            unsafe { (*ptr).register(name, fid) };
        }
    }
    Ok(DispatchResult::Continue)
}

/// `script_host_call_int(name, arg) -> Int` — call a host-registered Int->Int
/// function, re-entering the host interpreter.
pub(in super::super) fn handle_script_host_call_int(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let name = read_name_arg(state)?;
    let arg_reg = read_reg(state)?;
    let arg = state.get_reg(arg_reg).as_i64();

    // Resolve the host function + host state from the script's bridge.
    let resolved = state.host_call_ctx.as_ref().and_then(|ctx| {
        if ctx.host_state_addr == 0 {
            return None;
        }
        ctx.host_fns.get(&name).map(|fid| (ctx.host_state_addr, *fid))
    });

    let result = match resolved {
        Some((addr, fid)) => {
            // SAFETY: `addr` points to the live host `InterpreterState` — the
            // host is paused inside `script_engine_eval` for the whole nested
            // run — and is a DISTINCT object from `state` (the script interp),
            // so this does not alias the script's `&mut`. The host function
            // runs transactionally (push frame → run → pop), leaving the host
            // state consistent on return.
            let host_state =
                unsafe { &mut *(addr as *mut InterpreterState) };
            let host_arg = Value::from_i64(arg);
            match super::super::call_function_sync(host_state, fid, &[host_arg]) {
                Ok(v) => v.as_i64(),
                Err(_) => 0,
            }
        }
        None => 0,
    };
    state.set_reg(dst, Value::from_i64(result));
    Ok(DispatchResult::Continue)
}

// =============================================================================
// Shared-world (P2): zero-copy interop. A persistent world interpreter whose
// heap + shared-global table outlive each eval, so scripts share data by
// reference.
// =============================================================================

/// `script_world_new() -> RawScriptWorld`.
pub(in super::super) fn handle_script_world_new(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let world = Box::into_raw(Box::new(ScriptWorld::new()));
    state.set_reg(dst, Value::from_ptr(world as *mut u8));
    Ok(DispatchResult::Continue)
}

/// `script_world_eval(world, src: Text) -> RawScriptOutcome`. Runs `src` on the
/// world's PERSISTENT interpreter (shared heap + shared-global table).
pub(in super::super) fn handle_script_world_eval(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let world_reg = read_reg(state)?;
    let src_reg = read_reg(state)?;
    let world_ptr = state.get_reg(world_reg).as_ptr::<ScriptWorld>() as *mut ScriptWorld;
    if world_ptr.is_null() {
        return Err(InterpreterError::NullPointer);
    }
    let src_val = state.get_reg(src_reg);
    let source = extract_string_if_text(state, &src_val).unwrap_or_default();
    // SAFETY: `world_ptr` is a live `Box<ScriptWorld>` handle. Its interpreter
    // is distinct from `state`, so no aliasing.
    let outcome = unsafe { (*world_ptr).eval(&source) };
    let outcome_ptr = Box::into_raw(Box::new(outcome));
    state.set_reg(dst, Value::from_ptr(outcome_ptr as *mut u8));
    Ok(DispatchResult::Continue)
}

/// `script_world_free(world)`. Drops the world box.
pub(in super::super) fn handle_script_world_free(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let reg = read_reg(state)?;
    let ptr = state.get_reg(reg).as_ptr::<ScriptWorld>() as *mut ScriptWorld;
    if !ptr.is_null() {
        // SAFETY: from `Box::into_raw` in `handle_script_world_new`, freed once.
        unsafe { drop(Box::from_raw(ptr)) };
    }
    Ok(DispatchResult::Continue)
}

/// `script_set_int(name, value)` / `script_set_text(name, value)` — a script
/// writes a value into the current interpreter's shared-global table. The RAW
/// `Value` is stored (its tag carries the type), so a `Text`/heap value is
/// shared BY REFERENCE with other scripts in the same world (zero-copy).
pub(in super::super) fn handle_script_set_value(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let name = read_name_arg(state)?;
    let value_reg = read_reg(state)?;
    let value = state.get_reg(value_reg);
    // Raw store for reads within the SAME eval.
    state.host_globals.insert(name.clone(), value);
    // Owned snapshot for cross-eval persistence (a `ScriptWorld` reads these
    // back). Captured NOW, while the heap value is valid — it does not survive
    // the eval's frame teardown. `extract_owned` captures scalars, Text, List,
    // and Map (to arbitrary depth), so a script can share a structured value,
    // not only a scalar.
    let owned = crate::interpreter::script_engine::extract_owned(state, value);
    state.shared_writes.insert(name, owned);
    Ok(DispatchResult::Continue)
}
