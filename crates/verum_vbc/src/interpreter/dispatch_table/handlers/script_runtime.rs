//! Interpreter handlers for the embedded scripting engine (`core.script`).
//!
//! These back the `Script*` sub-ops of [`ExtendedSubOpcode`] (0x20–0x5C),
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
//! Engines use interpreter-owned, monotonic opaque IDs and checked leases.
//! Their IDs are integer Values, so legacy pointer-based Session/Outcome/World
//! handlers reject an engine handle before dereferencing it. The other handle
//! kinds retain their existing boxed-pointer representation.

use std::sync::{Arc, Mutex, MutexGuard, atomic::{AtomicU64, Ordering}};

use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::read_reg;
use super::path_ops_runtime::extract_string_if_text;
use super::string_helpers::alloc_string_value;
use crate::interpreter::script_engine::{
    HandleMagic, ScriptEngine, ScriptError, ScriptOutcome, ScriptSession, ScriptValueOwned,
    ScriptWorld,
};
use crate::value::Value;

static NEXT_ENGINE_ID: AtomicU64 = AtomicU64::new(1);

fn engine_error(message: &str) -> InterpreterError {
    InterpreterError::Panic { message: message.into() }
}

fn next_engine_id(counter: &AtomicU64) -> InterpreterResult<u64> {
    counter.try_update(Ordering::Relaxed, Ordering::Relaxed, |id|
        (id <= crate::value::MAX_SMALL_INT as u64).then(|| id + 1))
        .map_err(|_| engine_error("script engine handle space exhausted"))
}

fn alloc_engine(state: &mut InterpreterState, engine: ScriptEngine) -> InterpreterResult<Value> {
    let id = next_engine_id(&NEXT_ENGINE_ID)?;
    state.script_engines.insert(id, Arc::new(Mutex::new(engine)));
    Ok(Value::from_i64(id as i64))
}

fn engine_id(value: Value) -> InterpreterResult<u64> {
    if !value.is_inline_int() || value.as_i64() <= 0 {
        return Err(engine_error("invalid script engine handle"));
    }
    Ok(value.as_i64() as u64)
}

fn engine_owner(state: &InterpreterState, value: Value) -> InterpreterResult<Arc<Mutex<ScriptEngine>>> {
    state.script_engines.get(&engine_id(value)?).cloned()
        .ok_or_else(|| engine_error("script engine handle is foreign or expired"))
}

fn lease_engine(owner: &Arc<Mutex<ScriptEngine>>) -> InterpreterResult<MutexGuard<'_, ScriptEngine>> {
    owner.try_lock().map_err(|error| match error {
        std::sync::TryLockError::WouldBlock => engine_error("script engine is active"),
        std::sync::TryLockError::Poisoned(_) => engine_error("script engine lease is poisoned"),
    })
}

/// Read an opaque outcome handle from a register, erroring on null.
fn read_outcome_ptr(
    state: &mut InterpreterState,
) -> InterpreterResult<*const ScriptOutcome> {
    let reg = read_reg(state)?;
    checked_handle_ptr::<ScriptOutcome>(state.get_reg(reg)).map(|p| p as *const _)
}

/// Check the legacy pointer carrier before interpreting a Session/Outcome/World
/// handle. This rejects integers, including checked Engine IDs. A pointer tag
/// does not establish ownership or allocation liveness; those legacy handle
/// kinds still rely on their caller's lifetime contract.
fn checked_handle_ptr<T>(value: crate::value::Value) -> InterpreterResult<*mut T> {
    if !value.is_ptr() || value.is_nil() {
        return Err(InterpreterError::NullPointer);
    }
    let ptr = value.as_ptr::<T>();
    if ptr.is_null() {
        return Err(InterpreterError::NullPointer);
    }
    Ok(ptr)
}

/// Check the kind marker of a legacy handle whose allocation must still be
/// live. The marker distinguishes Session/World kinds, not expired pointers.
fn checked_typed_handle_ptr<T: HandleMagic>(
    value: crate::value::Value,
) -> InterpreterResult<*mut T> {
    let ptr = checked_handle_ptr::<T>(value)?;
    // SAFETY: the legacy handle lifetime contract requires a live allocation
    // of at least eight bytes; its #[repr(C)] marker occupies the first word.
    let magic = unsafe { *(ptr as *const u64) };
    if magic != T::MAGIC {
        return Err(InterpreterError::NullPointer);
    }
    Ok(ptr)
}

/// `script_engine_new() -> RawScriptEngine`. Allocates a fresh engine.
pub(in super::super) fn handle_script_engine_new(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let engine = alloc_engine(state, ScriptEngine::new())?;
    state.set_reg(dst, engine);
    Ok(DispatchResult::Continue)
}

/// `script_engine_free(engine)`. Releases an idle engine owner exactly once.
pub(in super::super) fn handle_script_engine_free(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let engine_reg = read_reg(state)?;
    let value = state.get_reg(engine_reg);
    let owner = engine_owner(state, value)?;
    let lease = lease_engine(&owner)?;
    state.script_engines.remove(&engine_id(value)?);
    drop(lease);
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

    let owner = engine_owner(state, state.get_reg(engine_reg))?;
    let mut engine = lease_engine(&owner)?;
    let src_val = state.get_reg(src_reg);
    let source =
        super::path_ops_runtime::extract_string_if_text(state, &src_val).unwrap_or_default();

    // The lease remains outside the borrowed Engine across host re-entry.
    // Reentrant free/eval/mutation refuses without touching the active engine.
    let outcome = engine.eval_to_outcome_with_host(&source, Some(state));
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

    let owner = engine_owner(state, state.get_reg(engine_reg))?;
    let mut engine = lease_engine(&owner)?;
    let src_val = state.get_reg(src_reg);
    let source =
        super::path_ops_runtime::extract_string_if_text(state, &src_val).unwrap_or_default();
    let fn_val = state.get_reg(fn_reg);
    let fn_name =
        super::path_ops_runtime::extract_string_if_text(state, &fn_val).unwrap_or_default();

    let outcome = engine.call_named_with_host(&source, &fn_name, Some(state));
    let outcome_ptr = Box::into_raw(Box::new(outcome));
    state.set_reg(dst, Value::from_ptr(outcome_ptr as *mut u8));
    Ok(DispatchResult::Continue)
}

/// `script_engine_call_args(engine, source, fn_name, args) -> RawScriptOutcome`
/// — like `script_engine_call` but passing the `args` list's elements as the
/// entry's positional parameters (scalars cross to the fresh script interpreter
/// directly). The host builds `args` as a `List`.
pub(in super::super) fn handle_script_engine_call_args(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let engine_reg = read_reg(state)?;
    let src_reg = read_reg(state)?;
    let fn_reg = read_reg(state)?;
    let args_reg = read_reg(state)?;

    let owner = engine_owner(state, state.get_reg(engine_reg))?;
    let mut engine = lease_engine(&owner)?;
    let src_val = state.get_reg(src_reg);
    let source =
        super::path_ops_runtime::extract_string_if_text(state, &src_val).unwrap_or_default();
    let fn_val = state.get_reg(fn_reg);
    let fn_name =
        super::path_ops_runtime::extract_string_if_text(state, &fn_val).unwrap_or_default();
    let args_val = state.get_reg(args_reg);
    let arg_values = state.list_elements(args_val).unwrap_or_default();

    let outcome = engine.call_named_args_with_host(&source, &fn_name, &arg_values, Some(state));
    let outcome_ptr = Box::into_raw(Box::new(outcome));
    state.set_reg(dst, Value::from_ptr(outcome_ptr as *mut u8));
    Ok(DispatchResult::Continue)
}

// --- Zero-copy interop tier: a persistent linked ScriptSession ---

/// `script_engine_link2(engine, source_a, source_b) -> RawScriptSession` —
/// merge two scripts into one module and open a persistent session over them
/// (shared heap). Returns a null handle on compile/link failure.
pub(in super::super) fn handle_script_engine_link2(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let engine_reg = read_reg(state)?;
    let a_reg = read_reg(state)?;
    let b_reg = read_reg(state)?;

    let owner = engine_owner(state, state.get_reg(engine_reg))?;
    let mut engine = lease_engine(&owner)?;
    let a_val = state.get_reg(a_reg);
    let src_a =
        super::path_ops_runtime::extract_string_if_text(state, &a_val).unwrap_or_default();
    let b_val = state.get_reg(b_reg);
    let src_b =
        super::path_ops_runtime::extract_string_if_text(state, &b_val).unwrap_or_default();

    let handle = match engine.link2(&src_a, &src_b) {
        Ok(session) => Box::into_raw(Box::new(session)) as *mut u8,
        Err(_) => std::ptr::null_mut(),
    };
    state.set_reg(dst, Value::from_ptr(handle));
    Ok(DispatchResult::Continue)
}

/// `script_engine_link(engine, sources: List<Text>) -> RawScriptSession` — merge
/// N scripts into a persistent session (the zero-copy tier for a whole set).
pub(in super::super) fn handle_script_engine_link(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let engine_reg = read_reg(state)?;
    let sources_reg = read_reg(state)?;

    let owner = engine_owner(state, state.get_reg(engine_reg))?;
    let mut engine = lease_engine(&owner)?;
    let sources_val = state.get_reg(sources_reg);
    let source_values = state.list_elements(sources_val).unwrap_or_default();
    let sources: Vec<String> = source_values
        .iter()
        .filter_map(|v| state.read_text(*v))
        .collect();
    let source_refs: Vec<&str> = sources.iter().map(|s| s.as_str()).collect();

    let handle = match engine.link_n(&source_refs) {
        Ok(session) => Box::into_raw(Box::new(session)) as *mut u8,
        Err(_) => std::ptr::null_mut(),
    };
    state.set_reg(dst, Value::from_ptr(handle));
    Ok(DispatchResult::Continue)
}

/// `script_engine_stdout(engine) -> Text` — the stdout captured during the
/// engine's most recent `eval`/`call` (empty before any run). Surfaces the otherwise-unreachable captured output — the `.vr`
/// `eval`/`call` free the outcome (which carries stdout) without exposing it.
pub(in super::super) fn handle_script_engine_stdout(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let engine_reg = read_reg(state)?;
    let owner = engine_owner(state, state.get_reg(engine_reg))?;
    let out = lease_engine(&owner)?.last_stdout().to_string();
    let value = alloc_string_value(state, &out)?;
    state.set_reg(dst, value);
    Ok(DispatchResult::Continue)
}

/// `script_engine_last_error(engine) -> Text` — the message of the engine's
/// most recent `link`/`link2` failure (empty otherwise). Recovers the real
/// compile/link diagnostic the null-session path would otherwise hide.
pub(in super::super) fn handle_script_engine_last_error(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let engine_reg = read_reg(state)?;
    let owner = engine_owner(state, state.get_reg(engine_reg))?;
    let msg = lease_engine(&owner)?.last_error().to_string();
    let value = alloc_string_value(state, &msg)?;
    state.set_reg(dst, value);
    Ok(DispatchResult::Continue)
}

/// `script_engine_last_error_kind(engine) -> Int` — the numeric kind of the
/// engine's most recent failure (0 none / 1 compile / 2 runtime / 3 not-found /
/// 4 unavailable / 5 internal), so a host can branch without parsing the message.
pub(in super::super) fn handle_script_engine_last_error_kind(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let engine_reg = read_reg(state)?;
    let owner = engine_owner(state, state.get_reg(engine_reg))?;
    let code = lease_engine(&owner)?.last_error_kind();
    state.set_reg(dst, Value::from_i64(code));
    Ok(DispatchResult::Continue)
}

/// `script_session_call(session, fn_name) -> RawScriptOutcome` — run `fn_name`
/// on the persistent session (state written by earlier calls is visible).
pub(in super::super) fn handle_script_session_call(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let session_reg = read_reg(state)?;
    let fn_reg = read_reg(state)?;

    let session_ptr = checked_typed_handle_ptr::<ScriptSession>(state.get_reg(session_reg))
        .unwrap_or(std::ptr::null_mut());
    let fn_val = state.get_reg(fn_reg);
    let fn_name =
        super::path_ops_runtime::extract_string_if_text(state, &fn_val).unwrap_or_default();

    let outcome = if session_ptr.is_null() {
        ScriptOutcome {
            value: ScriptValueOwned::Nil,
            error: Some(ScriptError::Runtime("null session (link failed)".to_string())),
            stdout: String::new(),
        }
    } else {
        // SAFETY: non-null `session_ptr` is a live `Box<ScriptSession>` handle.
        unsafe { (*session_ptr).call(&fn_name) }
    };
    let outcome_ptr = Box::into_raw(Box::new(outcome));
    state.set_reg(dst, Value::from_ptr(outcome_ptr as *mut u8));
    Ok(DispatchResult::Continue)
}

/// `script_session_free(session)` — drop the session box.
pub(in super::super) fn handle_script_session_free(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let session_reg = read_reg(state)?;
    if let Ok(ptr) = checked_typed_handle_ptr::<ScriptSession>(state.get_reg(session_reg)) {
        // SAFETY: `ptr` originates from `Box::into_raw` in handle_script_engine_link2.
        // Zero the magic first so a double-free's second deref is rejected.
        unsafe {
            (*ptr).invalidate();
            drop(Box::from_raw(ptr));
        }
    }
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
    if let Ok(ptr) = checked_handle_ptr::<ScriptOutcome>(state.get_reg(reg)) {
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
    let engine = alloc_engine(state, ScriptEngine::sandboxed(mem, instr, time))?;
    state.set_reg(dst, engine);
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
    let owner = engine_owner(state, state.get_reg(engine_reg))?;
    lease_engine(&owner)?.set_global(name, ScriptValueOwned::Int(value));
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
    let owner = engine_owner(state, state.get_reg(engine_reg))?;
    lease_engine(&owner)?.set_global(name, ScriptValueOwned::Text(text));
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
    let owner = engine_owner(state, state.get_reg(engine_reg))?;
    lease_engine(&owner)?.set_global(name, ScriptValueOwned::Bool(value));
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
    let owner = engine_owner(state, state.get_reg(engine_reg))?;
    lease_engine(&owner)?.set_global(name, ScriptValueOwned::Float(value));
    Ok(DispatchResult::Continue)
}

/// `script_engine_set_global_value(engine, name, value)` — set ANY value (a
/// `List`, `Map`, nested collection, or scalar) as a host global, marshaled via
/// `extract_owned`. The host's channel for passing structured data in; the
/// script reads it via `script_global_list` / `script_global_map`.
pub(in super::super) fn handle_script_engine_set_global_value(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let engine_reg = read_reg(state)?;
    let name = read_name_arg(state)?;
    let value_reg = read_reg(state)?;
    let value = state.get_reg(value_reg);
    // Marshal the host value into an owned form (its borrow of `state` ends here,
    // before the engine deref below) — reuses the same deep marshaler as results.
    let owned = crate::interpreter::script_engine::extract_owned(state, value);
    let owner = engine_owner(state, state.get_reg(engine_reg))?;
    lease_engine(&owner)?.set_global(name, owned);
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
// host interpreter's state (call_closure_sync), so it sees the host's module,
// heap and globals. Int -> Int signature for now.
// =============================================================================

/// `script_engine_register(engine, name, fn)` — host registers a callback.
pub(in super::super) fn handle_script_engine_register(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let engine_reg = read_reg(state)?;
    let name = read_name_arg(state)?;
    let fn_reg = read_reg(state)?;
    let callable = state.get_reg(fn_reg);
    let owner = engine_owner(state, state.get_reg(engine_reg))?;
    lease_engine(&owner)?.register_callable(name, callable, state)?;
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
    let arg = state.get_reg(arg_reg);
    if !arg.is_int() {
        return Err(InterpreterError::TypeMismatch {
            expected: "Int", got: "non-Int", operation: "script_host_call_int argument",
        });
    }
    let (addr, callable) = state.host_call_ctx.as_ref()
        .and_then(|ctx| ctx.host_fns.get(&name).map(|f| (ctx.host_state_addr, f.clone())))
        .ok_or_else(|| InterpreterError::Panic {
            message: format!("script host callback '{name}' is unavailable"),
        })?;
    if addr == 0 || addr == state as *mut InterpreterState as usize {
        return Err(InterpreterError::Panic { message: "invalid script host re-entry context".into() });
    }
    // SAFETY: only ScriptEngine::run_with_host installs this private context,
    // borrowing a distinct, live host exclusively for the synchronous run.
    // The callback's origin is checked before any host heap dereference.
    let host = unsafe { &mut *(addr as *mut InterpreterState) };
    let callable = callable.resolve(host)?;
    let result = super::super::call_closure_sync(host, callable, &[Value::from_i64(arg.as_i64())])?;
    if !result.is_int() {
        return Err(InterpreterError::TypeMismatch {
            expected: "Int", got: "non-Int", operation: "script_host_call_int result",
        });
    }
    // Only the integer result crosses the boundary; the environment stays on the host.
    state.set_reg(dst, Value::from_i64(result.as_i64()));
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
    let world_ptr = checked_typed_handle_ptr::<ScriptWorld>(state.get_reg(world_reg))?;
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
    if let Ok(ptr) = checked_typed_handle_ptr::<ScriptWorld>(state.get_reg(reg)) {
        // SAFETY: from `Box::into_raw` in `handle_script_world_new`, freed once.
        // Zero the magic first so a double-free's second deref is rejected.
        unsafe {
            (*ptr).invalidate();
            drop(Box::from_raw(ptr));
        }
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

#[cfg(test)]
mod engine_handle_tests {
    use super::*;

    #[test]
    fn engine_ids_roundtrip_and_exhaust_without_wrap() {
        let max = crate::value::MAX_SMALL_INT as u64;
        let counter = AtomicU64::new(max - 1);
        for expected in [max - 1, max] {
            let id = next_engine_id(&counter).unwrap();
            assert_eq!(id, expected);
            let value = Value::from_i64(id as i64);
            assert!(value.is_inline_int());
            assert_eq!(engine_id(value).unwrap(), id);
        }
        for _ in 0..2 { assert!(next_engine_id(&counter).is_err()); }
        assert_eq!(counter.load(Ordering::Relaxed), max + 1);
    }
}
