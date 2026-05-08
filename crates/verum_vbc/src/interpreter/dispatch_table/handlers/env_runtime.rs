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
use super::string_helpers::alloc_string_value;
use crate::interpreter::permission::{PermissionDecision, PermissionScope};
use crate::interpreter::state::InterpreterState;
use crate::types::TypeId;
use crate::value::Value;

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
            //
            // Script-level vs system-level disambiguation:
            //   * `core.shell.script.args()` strips argv[0] (the
            //     program name) — script's user-supplied args only.
            //   * `std.env.args()` returns the full host argv
            //     (program name included).
            // Both share the bare name `args` so qualifier inspection
            // is the only signal we have.  Strips when the qualified
            // name is in the `script` namespace.
            if arg_count != 0 {
                return Ok(None);
            }
            let strip_argv0 = func_name.contains("script.args")
                || func_name.contains("shell.script");
            intercept_args(state, strip_argv0)
        }
        "args_count" => {
            if arg_count != 0 {
                return Ok(None);
            }
            let strip_argv0 = func_name.contains("script");
            let count = std::env::args().count();
            let adjusted = if strip_argv0 && count > 0 { count - 1 } else { count };
            Ok(Some(Value::from_i64(adjusted as i64)))
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
    let key = extract_text_arg(state, args_start_reg, caller_base);
    let value = extract_text_arg(state, args_start_reg + 1, caller_base);
    // VBC-PERM-1 — granular target_id: hash the env-var key so a
    // script frontmatter `permissions = ["env=PATH"]` grants only
    // the named variable.  Falls through to the WILDCARD check
    // for scripts that grant `"env"` without a target.
    use crate::interpreter::permission::{target_id_for, WILDCARD_TARGET_ID};
    let tid = target_id_for(&key);
    if state.check_permission(PermissionScope::Process, tid) != PermissionDecision::Allow
        && state.check_permission(PermissionScope::Process, WILDCARD_TARGET_ID)
            == PermissionDecision::Deny
    {
        return Ok(Some(Value::unit()));
    }
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
    let key = extract_text_arg(state, args_start_reg, caller_base);
    // VBC-PERM-1 — same granular env-key pattern as set_var.
    use crate::interpreter::permission::{target_id_for, WILDCARD_TARGET_ID};
    let tid = target_id_for(&key);
    if state.check_permission(PermissionScope::Process, tid) != PermissionDecision::Allow
        && state.check_permission(PermissionScope::Process, WILDCARD_TARGET_ID)
            == PermissionDecision::Deny
    {
        return Ok(Some(Value::unit()));
    }
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
    let path = extract_path_or_text(state, args_start_reg, caller_base);
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

fn intercept_args(
    state: &mut InterpreterState,
    strip_argv0: bool,
) -> InterpreterResult<Option<Value>> {
    let mut argv: Vec<String> = std::env::args().collect();
    if strip_argv0 && !argv.is_empty() {
        // Script-level args() strips the program name (argv[0]).
        //
        // Under `verum run script.vr -- a b c`, the verum CLI passes
        // ["a", "b", "c"] as the script's args via `pipeline.run_compiled_vbc`.
        // But std::env::args() reflects the verum process's own argv:
        // ["./target/release/verum", "run", "script.vr", "--", "a", "b", "c"].
        // Walk past the verum subcommand args so the script sees ITS args,
        // not the verum invocation chain.  The reliable separator is
        // either the `--` token or the script-path token (containing
        // `/` or ending in `.vr`); after that everything is the script's.
        if let Some(idx) = argv.iter().position(|a| a == "--") {
            argv.drain(0..=idx);
        } else if let Some(idx) = argv.iter().position(|a| a.ends_with(".vr")) {
            // Script path arg without `--` separator — keep everything
            // AFTER the script path; argv[0] (program name) plus any
            // verum-flags before the script path are dropped.
            argv.drain(0..=idx);
        } else {
            // Fallback: just strip argv[0] (program name).
            argv.remove(0);
        }
    }
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
fn alloc_text_list(state: &mut InterpreterState, items: &[Value]) -> InterpreterResult<Value> {
    use crate::interpreter::heap::OBJECT_HEADER_SIZE;
    let len = items.len();
    let cap = if len < 16 { 16 } else { len };

    let backing = state
        .heap
        .alloc(TypeId::LIST, cap * std::mem::size_of::<Value>())?;
    state.record_allocation();
    let backing_data =
        unsafe { (backing.as_ptr() as *mut u8).add(OBJECT_HEADER_SIZE) as *mut Value };
    for (i, v) in items.iter().enumerate() {
        unsafe {
            *backing_data.add(i) = *v;
        }
    }

    let list = state
        .heap
        .alloc(TypeId::LIST, 3 * std::mem::size_of::<Value>())?;
    state.record_allocation();
    let data_ptr = unsafe { (list.as_ptr() as *mut u8).add(OBJECT_HEADER_SIZE) as *mut Value };
    unsafe {
        *data_ptr = Value::from_i64(len as i64);
        *data_ptr.add(1) = Value::from_i64(cap as i64);
        *data_ptr.add(2) = Value::from_ptr(backing.as_ptr() as *mut u8);
    }
    Ok(Value::from_ptr(list.as_ptr() as *mut u8))
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
fn extract_path_or_text(state: &InterpreterState, reg: u16, caller_base: u32) -> String {
    use super::heap_helpers::unwrap_ref;
    use crate::interpreter::heap;
    use super::string_helpers::extract_string;
    let v = state
        .registers
        .get(caller_base, crate::instruction::Reg(reg));
    let unwrapped = unwrap_ref(state, v);
    if unwrapped.is_small_string() {
        return extract_string(&unwrapped, state);
    }
    if !unwrapped.is_ptr() || unwrapped.is_nil() {
        return extract_string(&unwrapped, state);
    }
    let ptr = unwrapped.as_ptr::<u8>();
    if ptr.is_null() {
        return String::new();
    }
    if !(ptr as usize).is_multiple_of(std::mem::align_of::<heap::ObjectHeader>()) {
        return extract_string(&unwrapped, state);
    }
    let header = unsafe { &*(ptr as *const heap::ObjectHeader) };
    if (header.size as usize) < std::mem::size_of::<Value>() {
        return extract_string(&unwrapped, state);
    }
    // Heap-string TEXT shape: route to extract_string directly.
    if header.type_id == TypeId::TEXT || header.type_id == TypeId(0x0001) {
        return extract_string(&unwrapped, state);
    }
    // Try Path { inner: Text } — read field 0.
    let field0 = unsafe {
        *(ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value)
    };
    if field0.is_small_string() {
        return extract_string(&field0, state);
    }
    if field0.is_ptr() && !field0.is_nil() {
        let inner_ptr = field0.as_ptr::<u8>();
        if !inner_ptr.is_null()
            && (inner_ptr as usize)
                .is_multiple_of(std::mem::align_of::<heap::ObjectHeader>())
        {
            let inner_header =
                unsafe { &*(inner_ptr as *const heap::ObjectHeader) };
            if inner_header.type_id == TypeId::TEXT
                || inner_header.type_id == TypeId(0x0001)
            {
                return extract_string(&field0, state);
            }
            // PathBuf { path: Path { inner: Text } } — drill once more.
            if (inner_header.size as usize) >= std::mem::size_of::<Value>() {
                let inner_field0 = unsafe {
                    *(inner_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value)
                };
                if inner_field0.is_small_string() {
                    return extract_string(&inner_field0, state);
                }
                if inner_field0.is_ptr() && !inner_field0.is_nil() {
                    let deeper_ptr = inner_field0.as_ptr::<u8>();
                    if !deeper_ptr.is_null() {
                        let deeper_header = unsafe {
                            &*(deeper_ptr as *const heap::ObjectHeader)
                        };
                        if deeper_header.type_id == TypeId::TEXT
                            || deeper_header.type_id == TypeId(0x0001)
                        {
                            return extract_string(&inner_field0, state);
                        }
                    }
                }
            }
        }
    }
    // Last-resort: render whatever the value looks like.
    extract_string(&unwrapped, state)
}
