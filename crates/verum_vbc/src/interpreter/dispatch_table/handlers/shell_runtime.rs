//! High-level Rust intercepts for `core.shell.exec.{sh, sh_check}`.
//!
//! Architecture (Tier-0 / interpreter):
//!
//! Verum's shell-scripting surface (`sh#"..."` literals, `ShellCommand.run`,
//! `Command.output` etc.) ultimately bottoms out at `sh_check(cmd_text) ->
//! Result<ShellResult, ShellError>` in `core/shell/exec.vr`.  The Verum
//! implementation routes through `Command::output()` →
//! `spawn_child_with_output()` → `native_spawn()` → `make_pipe()` → the
//! libSystem `pipe(2)` syscall via FFI.
//!
//! At Tier-0 the FFI dispatch chain for libSystem.B.dylib syscalls is
//! brittle (libloading + libffi pointer marshaling for `int*` out-params
//! has subtle ABI gotchas that bite on kqueue/inotify-style fd-array
//! sentinels).  Rather than chase reliability through the FFI stack,
//! this module bypasses the entire chain by intercepting `sh_check` at
//! the call dispatch boundary and running the command directly via
//! `std::process::Command` — the exact same primitive a Tier-1 AOT
//! lowering would emit, just executed in the interpreter host process.
//!
//! This is the canonical Tier-0 architecture for "complex syscall
//! sequence" surfaces: interpret high-level intrinsics in Rust;
//! reserve FFI dispatch for genuinely-foreign cases that don't have a
//! Rust equivalent.
//!
//! # Marshaling
//!
//! Args:
//!   * `sh_check(cmd_text: &Text) -> Result<ShellResult, ShellError>` —
//!     arg 0 is the command text (passed by reference; we extract via
//!     `string_helpers::extract_string`).
//!
//! Returns: `Result<ShellResult, ShellError>`.  We construct the
//! variant value with a real type_id resolved from the module's type
//! table (`state.module.types.iter().find(|td|
//! state.module.get_string(td.name) == Some("Result"))`), so
//! `format_variant_for_print_depth` and pattern-match dispatch see the
//! correct constructor name.
//!
//! On success: `Result.Ok(ShellResult { stdout_bytes, stderr_bytes,
//! status: ExitStatus { raw }, command, duration: Duration { nanos } })`.
//! On failure (spawn error): `Result.Err(ShellError.SpawnFailed
//! { command, reason })`.
//!
//! # Permission gate
//!
//! The interpreter's PermissionRouter is consulted before spawning —
//! a script declaring `permissions = ["time"]` (no `run`) is denied
//! with the same `Process` scope check that `ffi_extended.rs::
//! check_ffi_permission` applies to the libSystem fork/execve path.

use crate::interpreter::permission::{PermissionDecision, PermissionScope};
use crate::interpreter::state::InterpreterState;
use crate::types::VariantKind;
use crate::value::Value;
use super::super::super::error::InterpreterResult;
use super::heap_helpers::{alloc_byte_list, alloc_record_n_fields, wrap_in_variant};
use super::string_helpers::{alloc_string_value, extract_string};

/// Try to intercept a high-level shell-runtime call.  Returns `Some(value)`
/// when the interception fires (caller must store the value into the
/// destination register and short-circuit normal call dispatch);
/// returns `None` for any function name that doesn't match.
///
/// Hot-path invariant: this function does ONE string equality check
/// (`func_name == "sh_check"`) and returns `None` for every other
/// function call in the program.  No allocation on the miss path.
pub(in super::super) fn try_intercept_shell_runtime(
    state: &mut InterpreterState,
    func_name: &str,
    args_start_reg: u16,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    // Match the FUNCTION-NAME suffix against the canonical entry
    // points.  Codegen registers functions under fully-qualified
    // names (`core.shell.exec.sh_check`), so we strip the prefix
    // before comparing — module-relative resolution and direct
    // bare-name calls both reach this intercept identically.
    let bare = func_name.rsplit('.').next().unwrap_or(func_name);
    match bare {
        "sh_check" | "sh" => {}
        _ => return Ok(None),
    }

    // Extract command text from arg 0 (passed by &Text reference;
    // `extract_string` handles both small-string and heap-string
    // representations).
    if arg_count == 0 {
        return Ok(None);
    }
    let cmd_val = state
        .registers
        .get(caller_base, crate::instruction::Reg(args_start_reg));
    // Unwrap a CBGR-style register reference (`&cmd` lowers to a
    // negative-int encoding pointing back into the caller's frame).
    let unwrapped_val = if super::cbgr_helpers::is_cbgr_ref(&cmd_val) {
        let (abs_index, _) = super::cbgr_helpers::decode_cbgr_ref(cmd_val.as_i64());
        state.registers.get_absolute(abs_index)
    } else {
        cmd_val
    };
    let cmd_text = extract_string(&unwrapped_val, state);

    // Permission gate — process termination AND spawn live under
    // the same `Process` scope (matches `ffi_symbol_permission_scope`
    // mapping for `fork`/`execve`/`posix_spawn` etc.).
    if state.check_permission(PermissionScope::Process, 0)
        == PermissionDecision::Deny
    {
        // Build Err(ShellError.SpawnFailed { command, reason }) directly.
        return Ok(Some(build_err_spawn_failed(
            state,
            &cmd_text,
            "permission denied: shell-spawn requires Process grant",
        )?));
    }

    // Dispatch via std::process::Command — the canonical Tier-0
    // primitive for "run /bin/sh -c <text> and capture output".
    use std::process::Command as StdCommand;
    let start = std::time::Instant::now();
    let (status_raw, stdout_bytes, stderr_bytes, spawn_err): (
        i64,
        Vec<u8>,
        Vec<u8>,
        Option<String>,
    ) = match StdCommand::new("/bin/sh")
        .arg("-c")
        .arg(&cmd_text)
        .output()
    {
        Ok(out) => {
            // Pack ExitStatus into the Verum `raw: Int` shape: low
            // 8 bits = exit code (when normal exit), bit 7 set for
            // signal termination (waitpid encoding the C runtime
            // already produces; we synthesise a compatible value
            // from `Output.status.code()` since `std` doesn't
            // expose the raw waitpid status portably).
            let raw = match out.status.code() {
                Some(code) => (code as i64) << 8,    // shift to mimic WIFEXITED layout
                None => 1,                            // signalled — non-zero raw
            };
            (raw, out.stdout, out.stderr, None)
        }
        Err(e) => (
            127,                                       // POSIX "command not found" sentinel
            Vec::new(),
            Vec::new(),
            Some(format!("spawn failed: {}", e)),
        ),
    };
    let elapsed_nanos = start.elapsed().as_nanos() as i64;

    if let Some(reason) = spawn_err {
        return Ok(Some(build_err_spawn_failed(state, &cmd_text, &reason)?));
    }

    Ok(Some(build_ok_shell_result(
        state,
        status_raw,
        &stdout_bytes,
        &stderr_bytes,
        &cmd_text,
        elapsed_nanos,
    )?))
}

// ============================================================================
// Result / variant constructors
// ============================================================================

/// Construct `Result.Ok(ShellResult { ... })` on the heap.
fn build_ok_shell_result(
    state: &mut InterpreterState,
    status_raw: i64,
    stdout_bytes: &[u8],
    stderr_bytes: &[u8],
    command: &str,
    duration_nanos: i64,
) -> InterpreterResult<Value> {
    let stdout_val = alloc_byte_list(state, stdout_bytes)?;
    let stderr_val = alloc_byte_list(state, stderr_bytes)?;
    let status_val = alloc_record_one_field(state, "ExitStatus", Value::from_i64(status_raw))?;
    let cmd_val = alloc_string_value(state, command)?;
    let duration_val = alloc_record_one_field(state, "Duration", Value::from_i64(duration_nanos))?;

    let shell_result = alloc_record_n_fields(
        state,
        "ShellResult",
        &[stdout_val, stderr_val, status_val, cmd_val, duration_val],
    )?;

    // Wrap in Result.Ok (tag 0, single field).
    wrap_in_variant(state, "Result", 0, &[shell_result])
}

/// Construct `Result.Err(ShellError.SpawnFailed { command, reason })`.
fn build_err_spawn_failed(
    state: &mut InterpreterState,
    command: &str,
    reason: &str,
) -> InterpreterResult<Value> {
    let cmd_val = alloc_string_value(state, command)?;
    let reason_val = alloc_string_value(state, reason)?;
    // SpawnFailed has tag=1 in ShellError (declaration order:
    // NonZeroExit=0, SpawnFailed=1, ...).
    let spawn_failed = wrap_in_variant(state, "ShellError", 1, &[cmd_val, reason_val])?;
    // Wrap in Result.Err (tag 1, single field).
    wrap_in_variant(state, "Result", 1, &[spawn_failed])
}

// ============================================================================
// Heap allocation helpers
// ============================================================================

/// Allocate a `List<Byte>` Verum value with the given content.

/// Allocate a record type with exactly one field.
fn alloc_record_one_field(
    state: &mut InterpreterState,
    type_name: &str,
    field_value: Value,
) -> InterpreterResult<Value> {
    alloc_record_n_fields(state, type_name, &[field_value])
}

// Suppress unused-import warning when feature combos drop the variant kind.
#[allow(dead_code)]
const _USE_VARIANT_KIND: VariantKind = VariantKind::Unit;
