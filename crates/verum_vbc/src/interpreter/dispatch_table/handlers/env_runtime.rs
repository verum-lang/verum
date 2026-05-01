//! High-level Rust intercepts for `core.base.env` operations.
//!
//! Sibling to `shell_runtime.rs` (VBC-1) and `file_runtime.rs`
//! (VBC-FILE-1).  Bypasses the libSystem `getenv`/`setenv`/`unsetenv`
//! FFI chain and dispatches directly to `std::env` from the
//! interpreter host process.
//!
//! # Functions intercepted
//!
//!   * `var(key: &Text) -> Result<Text, VarError>` — `std::env::var`
//!     mapped to `Result.Ok(text)` / `Result.Err(VarError.NotPresent)`
//!     / `Result.Err(VarError.NotUnicode(bytes))`.
//!   * `var_opt(key: &Text) -> Maybe<Text>` — `std::env::var` mapped
//!     to `Maybe.Some(text)` on success, `Maybe.None` otherwise.
//!   * `set_var(key: &Text, value: &Text) -> Unit` — `std::env::set_var`.
//!   * `remove_var(key: &Text) -> Unit` — `std::env::remove_var`.
//!
//! # Permission gate
//!
//! Reading env vars is unrestricted (matches the libSystem
//! `getenv` permission policy at `ffi_extended.rs` — the symbol is
//! NOT in `ffi_symbol_permission_scope`'s table).  Mutating env
//! vars (`set_var`, `remove_var`) consults `PermissionScope::Process`
//! (the same scope `setenv`/`unsetenv` are mapped to) so a
//! `permissions = ["time"]` script can't quietly mutate
//! environment that affects child-process behaviour.

use crate::interpreter::heap;
use crate::interpreter::permission::{PermissionDecision, PermissionScope};
use crate::interpreter::state::InterpreterState;
use crate::types::TypeId;
use crate::value::Value;
use super::super::super::error::InterpreterResult;
use super::string_helpers::{alloc_string_value, extract_string};

pub(in super::super) fn try_intercept_env_runtime(
    state: &mut InterpreterState,
    func_name: &str,
    args_start_reg: u16,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let bare = func_name.rsplit('.').next().unwrap_or(func_name);
    // Disambiguation: only catch the env-namespace versions.  `var`
    // and `set_var` collide with too many other surfaces; gate
    // them on `base.env` qualifier.  `var_opt` and `remove_var`
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
            // here).  Substitute an empty list so the variant
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

// ============================================================================
// Helpers (mirror file_runtime.rs)
// ============================================================================

fn extract_text_arg(state: &InterpreterState, reg: u16, caller_base: u32) -> String {
    let v = state.registers.get(caller_base, crate::instruction::Reg(reg));
    let unwrapped = if super::cbgr_helpers::is_cbgr_ref(&v) {
        let (abs_index, _) = super::cbgr_helpers::decode_cbgr_ref(v.as_i64());
        state.registers.get_absolute(abs_index)
    } else {
        v
    };
    extract_string(&unwrapped, state)
}

fn alloc_byte_list(state: &mut InterpreterState, bytes: &[u8]) -> InterpreterResult<Value> {
    use crate::interpreter::heap::OBJECT_HEADER_SIZE;
    let len = bytes.len();
    let cap = if len < 16 { 16 } else { len };
    let backing = state.heap.alloc(TypeId::LIST, cap)?;
    state.record_allocation();
    if !bytes.is_empty() {
        let backing_data = unsafe { (backing.as_ptr() as *mut u8).add(OBJECT_HEADER_SIZE) };
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), backing_data, len); }
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

fn wrap_in_variant(
    state: &mut InterpreterState,
    type_name: &str,
    tag: u32,
    fields: &[Value],
) -> InterpreterResult<Value> {
    use crate::interpreter::heap::OBJECT_HEADER_SIZE;
    let type_id = lookup_type_id_by_name(state, type_name).unwrap_or(TypeId(0x8000 + tag));
    let field_count = fields.len() as u32;
    let data_size = 8 + (fields.len() * std::mem::size_of::<Value>());
    let obj = state.heap.alloc(type_id, data_size)?;
    state.record_allocation();
    let base = obj.as_ptr() as *mut u8;
    unsafe {
        let tag_ptr = base.add(OBJECT_HEADER_SIZE) as *mut u32;
        *tag_ptr = tag;
        *tag_ptr.add(1) = field_count;
        let payload_ptr = base.add(OBJECT_HEADER_SIZE + 8) as *mut Value;
        for (i, v) in fields.iter().enumerate() {
            *payload_ptr.add(i) = *v;
        }
    }
    Ok(Value::from_ptr(base))
}

fn lookup_type_id_by_name(state: &InterpreterState, name: &str) -> Option<TypeId> {
    state
        .module
        .types
        .iter()
        .find(|td| {
            state.module.strings.get(td.name) == Some(name)
                && !matches!(td.kind, crate::types::TypeKind::Protocol)
        })
        .map(|td| td.id)
}
