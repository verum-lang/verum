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

use super::super::super::error::InterpreterResult;
use super::heap_helpers::{
    alloc_byte_list, alloc_record_n_fields, extract_text_arg, wrap_in_variant,
};
use super::file_runtime::extract_byte_list_arg;
use super::string_helpers::alloc_string_value;
use crate::interpreter::permission::{PermissionDecision, PermissionScope};
use crate::interpreter::state::InterpreterState;
use crate::types::TypeId;
use crate::value::Value;
use verum_common::well_known_types::{varerror_not_present_tag, varerror_not_unicode_tag};

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
        // ENV-IMPL-TRIO-1 (#55): the LOW-level `@intrinsic` trio the
        // stdlib bodies bottom out in. The high-level intercepts above
        // are gated on a `base.env` QUALIFIER — but bare-resolved call
        // sites (every core-tests/base/env test mounts the prelude and
        // calls `set_var(...)` unqualified) never match the gate, fall
        // through to the `*_impl` stubs, and those had NO registration
        // in either tier: `set_var` was a silent no-op, `var()` always
        // Err (93/97 of base/env red). These three names are globally
        // unique — no qualifier needed — and catching them closes EVERY
        // path (`var`/`var_opt`/`set_var`/`remove_var` all funnel here).
        "get_env_impl" if arg_count == 1 => {
            let name_bytes = extract_byte_list_arg(state, args_start_reg, caller_base)?;
            let key = String::from_utf8_lossy(&name_bytes).into_owned();
            env_get_maybe(state, &key)
        }
        "set_env_impl" if arg_count == 2 => {
            let name_bytes = extract_byte_list_arg(state, args_start_reg, caller_base)?;
            let value_bytes = extract_byte_list_arg(state, args_start_reg + 1, caller_base)?;
            let key = String::from_utf8_lossy(&name_bytes).into_owned();
            let value = String::from_utf8_lossy(&value_bytes).into_owned();
            env_set_raw(state, &key, &value);
            Ok(Some(env_unit_ok(state)?))
        }
        "unset_env_impl" if arg_count == 1 => {
            let name_bytes = extract_byte_list_arg(state, args_start_reg, caller_base)?;
            let key = String::from_utf8_lossy(&name_bytes).into_owned();
            env_unset_raw(state, &key);
            Ok(Some(env_unit_ok(state)?))
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
        _ => Ok(None),
    }
}

fn is_env_qualified(func_name: &str) -> bool {
    func_name.contains("base.env") || func_name.contains("base::env")
}

// ============================================================================
// Shared environment authority
//
// The environment has TWO entry points: the Call-level name intercepts below,
// and the `FfiExtended` EnvGet/EnvSet/EnvUnset arms that `core/base/env.vr`'s
// `@intrinsic` bodies lower to when they are inlined. Both funnel through
// these functions so the `std::env` call and the VBC-PERM-1 permission gate
// exist in exactly one place.
//
// Callers own only the Verum-level return SHAPE, which genuinely differs
// between them: `env.set_var` returns unit, while `set_env_impl` is declared
// `Result<(), Text>`.
// ============================================================================

/// `get_env_impl(name) -> Maybe<Text>` result shape, shared by the Call-level
/// intercept and the `FfiExtended::EnvGet` arm.
pub(super) fn env_get_maybe(
    state: &mut InterpreterState,
    key: &str,
) -> InterpreterResult<Option<Value>> {
    match env_get_raw(key) {
        Ok(value) => {
            let text = alloc_string_value(state, &value)?;
            Ok(Some(wrap_in_variant(state, "Maybe", 1, &[text])?))
        }
        Err(_) => Ok(Some(wrap_in_variant(state, "Maybe", 0, &[])?)),
    }
}

/// `Result.Ok(())` — the shape `set_env_impl` / `unset_env_impl` are declared
/// to return (`Result<(), Text>`). A permission denial is reported as `Ok`
/// here, preserving the pre-existing behaviour of the Call-level intercepts:
/// the stdlib discards this value (`let _ = set_env_impl(...)`), so surfacing
/// a denial as `Err` would be a user-visible semantic change that belongs to
/// the permission-policy surface, not to this wiring.
pub(super) fn env_unit_ok(state: &mut InterpreterState) -> InterpreterResult<Value> {
    let unit = Value::unit();
    wrap_in_variant(state, "Result", 0, &[unit])
}

/// Read an environment variable. Reads are ungated — the permission boundary
/// is on mutation. The full `VarError` is preserved so callers that
/// distinguish `NotPresent` from `NotUnicode` still can.
pub(super) fn env_get_raw(key: &str) -> Result<String, std::env::VarError> {
    std::env::var(key)
}

/// VBC-PERM-1 gate for environment mutation. A script frontmatter
/// `permissions = ["env=PATH"]` grants only the named variable via the hashed
/// target id; a bare `"env"` grant falls through to the wildcard check.
fn env_mutation_allowed(state: &mut InterpreterState, key: &str) -> bool {
    use crate::interpreter::permission::{WILDCARD_TARGET_ID, target_id_for};
    state.check_permission(PermissionScope::Process, target_id_for(key)) == PermissionDecision::Allow
        || state.check_permission(PermissionScope::Process, WILDCARD_TARGET_ID)
            != PermissionDecision::Deny
}

/// Set an environment variable. Returns `false` when the permission router
/// denied the write, in which case the variable is left untouched.
pub(super) fn env_set_raw(state: &mut InterpreterState, key: &str, value: &str) -> bool {
    if !env_mutation_allowed(state, key) {
        return false;
    }
    // A VERUM PROGRAMME MUST NOT BE ABLE TO ABORT THE INTERPRETER, and
    // `std::env::set_var` PANICS — it does not return an error — when
    // the key is empty, when either side carries a NUL, or when the key
    // carries `=`. `core/base/env.vr` declares `set_var` as returning
    // unit and its body says "ignore errors (best-effort)", so the
    // Verum surface has no way to express the failure and no reason to
    // expect a process death.
    //
    // Measured 2026-09-12: `core-tests/base/env/property_test.vr`'s
    // round-trip law generates arbitrary `Text`, drew a value with an
    // embedded NUL, and took the whole test RUNNER down with
    // SIGABRT — `failed to set environment variable … file name
    // contained an unexpected NUL byte`. Every remaining test in the
    // process was lost, and the report blamed the law rather than the
    // call.
    if !env_name_is_settable(key) || value.contains('\0') {
        return false;
    }
    // SAFETY: `set_var` is unsafe in newer Rust because concurrent readers in
    // other threads would race; the interpreter is single-threaded here.
    unsafe {
        std::env::set_var(key, value);
    }
    true
}

/// The names `std::env::{set_var, remove_var}` accept without panicking.
///
/// Both refuse an empty key, a key containing `=`, and a NUL anywhere —
/// by PANIC, which is why this is a precondition rather than a match on
/// an error. Stated once because two callers need it.
fn env_name_is_settable(key: &str) -> bool {
    !key.is_empty() && !key.contains('=') && !key.contains('\0')
}

/// Remove an environment variable. Returns `false` when denied.
pub(super) fn env_unset_raw(state: &mut InterpreterState, key: &str) -> bool {
    if !env_mutation_allowed(state, key) {
        return false;
    }
    // `remove_var` panics on the same three shapes `set_var` does; see
    // `env_name_is_settable`. A key that could never have been set
    // cannot be set now, so refusing is also the right answer.
    if !env_name_is_settable(key) {
        return false;
    }
    // SAFETY: see `env_set_raw`.
    unsafe {
        std::env::remove_var(key);
    }
    true
}

// ============================================================================
// Per-function intercepts
// ============================================================================

fn intercept_var_opt(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let key = extract_text_arg(state, args_start_reg, caller_base)?;
    match env_get_raw(&key) {
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
    let key = extract_text_arg(state, args_start_reg, caller_base)?;
    match env_get_raw(&key) {
        Ok(value) => {
            let text = alloc_string_value(state, &value)?;
            Ok(Some(wrap_in_variant(state, "Result", 0, &[text])?))
        }
        Err(std::env::VarError::NotPresent) => {
            let err = wrap_in_variant(state, "VarError", varerror_not_present_tag(), &[])?;
            Ok(Some(wrap_in_variant(state, "Result", 1, &[err])?))
        }
        Err(std::env::VarError::NotUnicode(_)) => {
            // NotUnicode(List<Byte>) — payload is the raw bytes; we
            // don't have them in std::env::var (the OsString variant
            // would expose them via env::var_os, but we used var
            // here). Substitute an empty list so the variant
            // structure stays sound.
            let empty_list = alloc_byte_list(state, &[])?;
            let err = wrap_in_variant(state, "VarError", varerror_not_unicode_tag(), &[empty_list])?;
            Ok(Some(wrap_in_variant(state, "Result", 1, &[err])?))
        }
    }
}

fn intercept_set_var(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let key = extract_text_arg(state, args_start_reg, caller_base)?;
    let value = extract_text_arg(state, args_start_reg + 1, caller_base)?;
    // `env.set_var` is declared to return unit, so a permission denial is
    // silent here — the gate itself lives in `env_set_raw`.
    env_set_raw(state, &key, &value);
    Ok(Some(Value::unit()))
}

fn intercept_remove_var(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let key = extract_text_arg(state, args_start_reg, caller_base)?;
    // `env.remove_var` returns unit; the gate lives in `env_unset_raw`.
    env_unset_raw(state, &key);
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
            // PathBuf has shape `{ path: Path { inner: Text } }` —
            // a 1-field record wrapping a `Path` 1-field record.
            // Pre-fix this allocated `{ inner: Text }` directly,
            // collapsing the PathBuf and Path shapes into one;
            // method dispatch on the result then mis-resolved
            // against the wrong field layout (`as_path` etc.).
            let inner_path = alloc_record_n_fields(state, "Path", &[text])?;
            let pathbuf = alloc_record_n_fields(state, "PathBuf", &[inner_path])?;
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
    // Path-aware extraction: `set_current_dir(&Path)` is the
    // canonical user-side call, and `&Path` arrives as a CBGR/
    // ThinRef pointing at a 1-field `Path { inner: Text }` record.
    // The plain `extract_text_arg` only unwraps refs and reads
    // small/heap strings — it returns "" for record pointers,
    // making chdir a silent no-op. Drill through Path/PathBuf
    // shapes the same way file_runtime does.
    let path = extract_path_or_text(state, args_start_reg, caller_base)?;
    // VBC-PERM-1 — granular target_id: hash the directory path so a
    // script frontmatter `permissions = ["run=/var/jobs"]` grants
    // chdir only into that directory.  Falls through to WILDCARD
    // for scripts that grant `"run"` without a target.
    use crate::interpreter::permission::{target_id_for, WILDCARD_TARGET_ID};
    let tid = target_id_for(&path);
    if state.check_permission(PermissionScope::Process, tid) != PermissionDecision::Allow
        && state.check_permission(PermissionScope::Process, WILDCARD_TARGET_ID)
            == PermissionDecision::Deny
    {
        let kind = wrap_in_variant(state, "IoErrorKind", 1, &[])?; // PermissionDenied
        let none = wrap_in_variant(state, "Maybe", 0, &[])?;
        let err = alloc_record_n_fields(state, "StreamError", &[kind, none])?;
        return Ok(Some(wrap_in_variant(state, "Result", 1, &[err])?));
    }
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

/// Extract a path argument that may be either:
///   * a Text (small or heap string) — returned verbatim;
///   * a `&Path` / `&PathBuf` reference — drill through CBGR/ThinRef
///     wrap, then through the 1-field record's `inner` Text (Path)
///     or 2-field PathBuf chain (`PathBuf { path: Path { inner } }`).
///
/// Sibling to `file_runtime::extract_path_arg`; replicated here to
/// keep the env-runtime intercept layer self-contained without
/// taking a cross-module dependency on file_runtime internals.
/// Used by `set_current_dir(&Path)` so chdir actually targets the
/// caller's intended directory.
fn extract_path_or_text(state: &InterpreterState, reg: u16, caller_base: u32) -> InterpreterResult<String> {
    use super::heap_helpers::unwrap_ref;
    use crate::interpreter::heap;
    use super::string_helpers::extract_string;
    let v = state
        .registers
        .get(caller_base, crate::instruction::Reg(reg));
    let unwrapped = unwrap_ref(state, v)?;
    if unwrapped.is_small_string() {
        return Ok(extract_string(&unwrapped, state));
    }
    if !unwrapped.is_ptr() || unwrapped.is_nil() {
        return Ok(extract_string(&unwrapped, state));
    }
    let ptr = unwrapped.as_ptr::<u8>();
    if ptr.is_null() {
        return Ok(String::new());
    }
    if !(ptr as usize).is_multiple_of(std::mem::align_of::<heap::ObjectHeader>()) {
        return Ok(extract_string(&unwrapped, state));
    }
    let header = unsafe { heap::ObjectHeader::ref_or_stub(ptr) };
    if (header.size as usize) < std::mem::size_of::<Value>() {
        return Ok(extract_string(&unwrapped, state));
    }
    // Heap-string TEXT shape: route to extract_string directly.
    if header.type_id == TypeId::TEXT {
        return Ok(extract_string(&unwrapped, state));
    }
    // Try Path { inner: Text } — read field 0.
    let field0 = unsafe {
        *(ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value)
    };
    if field0.is_small_string() {
        return Ok(extract_string(&field0, state));
    }
    if field0.is_ptr() && !field0.is_nil() {
        let inner_ptr = field0.as_ptr::<u8>();
        if !inner_ptr.is_null()
            && (inner_ptr as usize)
                .is_multiple_of(std::mem::align_of::<heap::ObjectHeader>())
        {
            let inner_header = unsafe { heap::ObjectHeader::ref_or_stub(inner_ptr) };
            if inner_header.type_id == TypeId::TEXT {
                return Ok(extract_string(&field0, state));
            }
            // PathBuf { path: Path { inner: Text } } — drill once more.
            if (inner_header.size as usize) >= std::mem::size_of::<Value>() {
                let inner_field0 = unsafe {
                    *(inner_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value)
                };
                if inner_field0.is_small_string() {
                    return Ok(extract_string(&inner_field0, state));
                }
                if inner_field0.is_ptr() && !inner_field0.is_nil() {
                    let deeper_ptr = inner_field0.as_ptr::<u8>();
                    let deeper_header = unsafe { heap::ObjectHeader::ref_or_stub(deeper_ptr) };
                    if deeper_header.type_id == TypeId::TEXT {
                        return Ok(extract_string(&inner_field0, state));
                    }
                }
            }
        }
    }
    // Last-resort: render whatever the value looks like.
    Ok(extract_string(&unwrapped, state))
}
