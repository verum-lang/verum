//! High-level Rust intercepts for `core.base.env` operations.
//!

//! Sibling to `shell_runtime.rs` (VBC-1) and `file_runtime.rs`
//! (VBC-FILE-1). Bypasses the libSystem `getenv`/`setenv`/`unsetenv`
//! FFI chain and dispatches directly to `std::env` from the
//! interpreter host process.
//!

//! # Functions intercepted
//!

//!  * `var(key: &Text) -> Result<Text, VarError>` — `std::env::var`
//!  mapped to `Result.Ok(text)` / `Result.Err(VarError.NotPresent)`
//!  / `Result.Err(VarError.NotUnicode(bytes))`.
//!  * `var_opt(key: &Text) -> Maybe<Text>` — `std::env::var` mapped
//!  to `Maybe.Some(text)` on success, `Maybe.None` otherwise.
//!  * `set_var(key: &Text, value: &Text) -> Unit` — `std::env::set_var`.
//!  * `remove_var(key: &Text) -> Unit` — `std::env::remove_var`.
//!

//! # Permission gate
//!

//! Reading env vars is unrestricted (matches the libSystem
//! `getenv` permission policy at `ffi_extended.rs` — the symbol is
//! NOT in `ffi_symbol_permission_scope`'s table). Mutating env
//! vars (`set_var`, `remove_var`) consults `PermissionScope::Process`
//! (the same scope `setenv`/`unsetenv` are mapped to) so a
//! `permissions = ["time"]` script can't quietly mutate
//! environment that affects child-process behaviour.

use crate::interpreter::permission::{PermissionDecision, PermissionScope};
use crate::interpreter::state::InterpreterState;
use crate::types::TypeId;
use crate::value::Value;
use super::super::super::error::InterpreterResult;
use super::heap_helpers::{
    alloc_byte_list, alloc_record_n_fields, extract_text_arg, wrap_in_variant,
};
use super::string_helpers::alloc_string_value;

pub(in super::super) fn try_intercept_env_runtime(
    state: &mut InterpreterState,
    func_name: &str,
    args_start_reg: u16,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let bare = func_name.rsplit('.').next().unwrap_or(func_name);
    // Disambiguation: only catch the env-namespace versions. `var`
    // and `set_var` collide with too many other surfaces; gate
    // them on `base.env` qualifier. `var_opt` and `remove_var`
    // are unique enough.
    match bare {
        "var_opt" => {
            if arg_count != 1 {
                return Ok(None);
            }
            intercept_var_opt(state, args_start_reg, caller_base)
        }
        "var" => {
            if arg_count != 1 || !is_env_qualified(func_name) {
                return Ok(None);
            }
            intercept_var(state, args_start_reg, caller_base)
        }
        "set_var" => {
            if arg_count != 2 || !is_env_qualified(func_name) {
                return Ok(None);
            }
            intercept_set_var(state, args_start_reg, caller_base)
        }
        "remove_var" => {
            if arg_count != 1 {
                return Ok(None);
            }
            intercept_remove_var(state, args_start_reg, caller_base)
        }
        // Process-state intercepts (VBC-PROC-1). current_dir uses
        // sys_getcwd FFI + iterator chains that fail in interpreter;
        // args/args_count/arg() rely on the C-runtime argv pointer
        // table that's not populated for `verum run` invocations.
        "current_dir" => {
            if arg_count != 0 {
                return Ok(None);
            }
            intercept_current_dir(state)
        }
        "set_current_dir" => {
            if arg_count != 1 {
                return Ok(None);
            }
            intercept_set_current_dir(state, args_start_reg, caller_base)
        }
        "args" => {
            // `args()` is a 0-arg constructor — collisions with
            // method receivers (Command.args(...)) take ≥1 arg, so
            // gating on arg_count alone disambiguates without
            // needing a qualifier check (which fails for unqualified
            // call sites where the codegen registers the function
            // under just `args`).
            if arg_count != 0 {
                return Ok(None);
            }
            intercept_args(state)
        }
        "args_count" => {
            if arg_count != 0 {
                return Ok(None);
            }
            Ok(Some(Value::from_i64(std::env::args().count() as i64)))
        }
        "arg" => {
            // Same reasoning as `args` — 1-arg variant. Collisions
            // (Command.arg(text)) also take 1 arg but the receiver
            // would be passed as arg 0 making the actual user arg
            // index different; the env-namespace `arg(idx)` takes
            // exactly 1 arg (the index), so we accept this and
            // fall back to None on type mismatch (caller's bytecode
            // path then takes over). In practice this isn't
            // ambiguous because Command.arg goes through method
            // dispatch (CallM), not plain Call.
            if arg_count != 1 {
                return Ok(None);
            }
            intercept_arg(state, args_start_reg, caller_base)
        }
        _ => Ok(None),
    }
}

fn is_env_qualified(func_name: &str) -> bool {
    func_name.contains("base.env") || func_name.contains("base::env")
}

// ============================================================================
// Per-function intercepts
// ============================================================================

fn intercept_var_opt(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let key = extract_text_arg(state, args_start_reg, caller_base);
    match std::env::var(&key) {
        Ok(value) => {
            let text = alloc_string_value(state, &value)?;
            Ok(Some(wrap_in_variant(state, "Maybe", 1, &[text])?))
        }
        Err(_) => Ok(Some(wrap_in_variant(state, "Maybe", 0, &[])?)),
    }
}

fn intercept_var(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let key = extract_text_arg(state, args_start_reg, caller_base);
    match std::env::var(&key) {
        Ok(value) => {
            let text = alloc_string_value(state, &value)?;
            Ok(Some(wrap_in_variant(state, "Result", 0, &[text])?))
        }
        Err(std::env::VarError::NotPresent) => {
            let err = wrap_in_variant(state, "VarError", 0, &[])?;
            Ok(Some(wrap_in_variant(state, "Result", 1, &[err])?))
        }
        Err(std::env::VarError::NotUnicode(_)) => {
            // NotUnicode(List<Byte>) — payload is the raw bytes; we
            // don't have them in std::env::var (the OsString variant
            // would expose them via env::var_os, but we used var
            // here). Substitute an empty list so the variant
            // structure stays sound.
            let empty_list = alloc_byte_list(state, &[])?;
            let err = wrap_in_variant(state, "VarError", 1, &[empty_list])?;
            Ok(Some(wrap_in_variant(state, "Result", 1, &[err])?))
        }
    }
}

fn intercept_set_var(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    if state.check_permission(PermissionScope::Process, 0) == PermissionDecision::Deny {
        return Ok(Some(Value::unit()));
    }
    let key = extract_text_arg(state, args_start_reg, caller_base);
    let value = extract_text_arg(state, args_start_reg + 1, caller_base);
    // SAFETY: `set_var` is unsafe in newer Rust due to threading
    // concerns, but the interpreter is single-threaded at this point.
    // The safety contract is met by the surrounding interpreter
    // invariant.
    unsafe {
        std::env::set_var(&key, &value);
    }
    Ok(Some(Value::unit()))
}

fn intercept_remove_var(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    if state.check_permission(PermissionScope::Process, 0) == PermissionDecision::Deny {
        return Ok(Some(Value::unit()));
    }
    let key = extract_text_arg(state, args_start_reg, caller_base);
    // SAFETY: see set_var above.
    unsafe {
        std::env::remove_var(&key);
    }
    Ok(Some(Value::unit()))
}

// ----------------------------------------------------------------------------
// Process-state intercepts (current_dir, set_current_dir, args, arg)
// ----------------------------------------------------------------------------

fn intercept_current_dir(state: &mut InterpreterState) -> InterpreterResult<Option<Value>> {
    match std::env::current_dir() {
        Ok(p) => {
            let s = p.to_string_lossy().to_string();
            let text = alloc_string_value(state, &s)?;
            // PathBuf has shape `{ inner: Text }` — single-field record.
            let pathbuf = alloc_record_n_fields(state, "PathBuf", &[text])?;
            Ok(Some(wrap_in_variant(state, "Result", 0, &[pathbuf])?))
        }
        Err(_e) => {
            // Build Err(StreamError { kind: Other, message: None })
            let kind = wrap_in_variant(state, "IoErrorKind", 19, &[])?;
            let none = wrap_in_variant(state, "Maybe", 0, &[])?;
            let err = alloc_record_n_fields(state, "StreamError", &[kind, none])?;
            Ok(Some(wrap_in_variant(state, "Result", 1, &[err])?))
        }
    }
}

fn intercept_set_current_dir(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    if state.check_permission(PermissionScope::Process, 0) == PermissionDecision::Deny {
        let kind = wrap_in_variant(state, "IoErrorKind", 1, &[])?; // PermissionDenied
        let none = wrap_in_variant(state, "Maybe", 0, &[])?;
        let err = alloc_record_n_fields(state, "StreamError", &[kind, none])?;
        return Ok(Some(wrap_in_variant(state, "Result", 1, &[err])?));
    }
    let path = extract_text_arg(state, args_start_reg, caller_base);
    match std::env::set_current_dir(&path) {
        Ok(()) => Ok(Some(wrap_in_variant(state, "Result", 0, &[Value::unit()])?)),
        Err(_e) => {
            let kind = wrap_in_variant(state, "IoErrorKind", 19, &[])?;
            let none = wrap_in_variant(state, "Maybe", 0, &[])?;
            let err = alloc_record_n_fields(state, "StreamError", &[kind, none])?;
            Ok(Some(wrap_in_variant(state, "Result", 1, &[err])?))
        }
    }
}

fn intercept_args(state: &mut InterpreterState) -> InterpreterResult<Option<Value>> {
    let argv: Vec<String> = std::env::args().collect();
    let mut text_values: Vec<Value> = Vec::with_capacity(argv.len());
    for s in &argv {
        text_values.push(alloc_string_value(state, s)?);
    }
    Ok(Some(alloc_text_list(state, &text_values)?))
}

fn intercept_arg(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let idx_val = state
        .registers
        .get(caller_base, crate::instruction::Reg(args_start_reg));
    let idx = if super::cbgr_helpers::is_cbgr_ref(&idx_val) {
        let (abs, _) = super::cbgr_helpers::decode_cbgr_ref(idx_val.as_i64());
        state.registers.get_absolute(abs).as_i64()
    } else {
        idx_val.as_i64()
    };
    let argv: Vec<String> = std::env::args().collect();
    if idx < 0 || (idx as usize) >= argv.len() {
        return Ok(Some(alloc_string_value(state, "")?));
    }
    Ok(Some(alloc_string_value(state, &argv[idx as usize])?))
}

/// Allocate a `List<Text>` Verum value with the given Value entries
/// (each entry must already be a Text Value — i.e. small-string or
/// heap-string pointer). Layout matches the codegen's List
/// representation: `[len:Value(i64)] [cap:Value(i64)] [backing_ptr:Value]`
/// where backing is an array of Values.
fn alloc_text_list(
    state: &mut InterpreterState,
    items: &[Value],
) -> InterpreterResult<Value> {
    use crate::interpreter::heap::OBJECT_HEADER_SIZE;
    let len = items.len();
    let cap = if len < 16 { 16 } else { len };

    let backing = state
        .heap
        .alloc(TypeId::LIST, cap * std::mem::size_of::<Value>())?;
    state.record_allocation();
    let backing_data = unsafe { (backing.as_ptr() as *mut u8).add(OBJECT_HEADER_SIZE) as *mut Value };
    for (i, v) in items.iter().enumerate() {
        unsafe { *backing_data.add(i) = *v; }
    }

    let list = state.heap.alloc(TypeId::LIST, 3 * std::mem::size_of::<Value>())?;
    state.record_allocation();
    let data_ptr = unsafe { (list.as_ptr() as *mut u8).add(OBJECT_HEADER_SIZE) as *mut Value };
    unsafe {
        *data_ptr = Value::from_i64(len as i64);
        *data_ptr.add(1) = Value::from_i64(cap as i64);
        *data_ptr.add(2) = Value::from_ptr(backing.as_ptr() as *mut u8);
    }
    Ok(Value::from_ptr(list.as_ptr() as *mut u8))
}

