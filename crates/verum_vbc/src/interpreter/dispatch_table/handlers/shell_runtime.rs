//! High-level Rust intercepts for `core.shell.exec.{sh, sh_check}`.
//!

//! Architecture (Tier-0 / interpreter):
//!

//! Verum's shell-scripting surface (`sh#"..."` literals, `ShellCommand.run`,
//! `Command.output` etc.) ultimately bottoms out at `sh_check(cmd_text) ->
//! Result<ShellResult, ShellError>` in `core/shell/exec.vr`. The Verum
//! implementation routes through `Command::output()` →
//! `spawn_child_with_output()` → `native_spawn()` → `make_pipe()` → the
//! libSystem `pipe(2)` syscall via FFI.
//!

//! At Tier-0 the FFI dispatch chain for libSystem.B.dylib syscalls is
//! brittle (libloading + libffi pointer marshaling for `int*` out-params
//! has subtle ABI gotchas that bite on kqueue/inotify-style fd-array
//! sentinels). Rather than chase reliability through the FFI stack,
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
//!  * `sh_check(cmd_text: &Text) -> Result<ShellResult, ShellError>` —
//!  arg 0 is the command text (passed by reference; we extract via
//!  `string_helpers::extract_string`).
//!

//! Returns: `Result<ShellResult, ShellError>`. We construct the
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

use super::super::super::error::InterpreterResult;
use super::heap_helpers::{alloc_byte_list, alloc_record_n_fields, wrap_in_variant};
use super::string_helpers::{alloc_string_value, extract_string};
use crate::interpreter::permission::{PermissionDecision, PermissionScope};
use crate::interpreter::state::InterpreterState;
use crate::types::VariantKind;
use crate::value::Value;

/// Try to intercept a high-level shell-runtime call. Returns `Some(value)`
/// when the interception fires (caller must store the value into the
/// destination register and short-circuit normal call dispatch);
/// returns `None` for any function name that doesn't match.
///

/// Hot-path invariant: this function does ONE string equality check
/// (`func_name == "sh_check"`) and returns `None` for every other
/// function call in the program. No allocation on the miss path.
pub(in super::super) fn try_intercept_shell_runtime(
    state: &mut InterpreterState,
    func_name: &str,
    args_start_reg: u16,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    // Match the FUNCTION-NAME suffix against the canonical entry
    // points. Codegen registers functions under fully-qualified
    // names (`core.shell.exec.sh_check`), so we strip the prefix
    // before comparing — module-relative resolution and direct
    // bare-name calls both reach this intercept identically.
    let bare = func_name.rsplit('.').next().unwrap_or(func_name);
    // Accept any qualified registration that ends with one of the
    // canonical entry-points and lives under the `core.shell.*`
    // umbrella.  The re-export at `core.shell.mod.vr` produces a
    // second registration `core.shell.run` (in addition to the
    // canonical `core.shell.exec.run`); both must reach the same
    // intercept body.  Bare-name matches are accepted when the
    // function isn't qualified at all (direct user-side
    // registration shadowing the simple-name slot).
    let is_canonical_entry = matches!(bare, "sh_check" | "sh" | "run");
    if !is_canonical_entry {
        return Ok(None);
    }
    let qualified_under_shell = func_name.starts_with("core.shell.")
        && (func_name.starts_with("core.shell.exec.")
            || func_name.starts_with("core.shell.run")
            || func_name.starts_with("core.shell.sh")
            || // Bare re-export forms `core.shell.run` /
               // `core.shell.sh_check` (no `.exec.` segment).
               matches!(
                   func_name,
                   "core.shell.run" | "core.shell.sh" | "core.shell.sh_check"
               ));
    let unqualified = !func_name.contains('.');
    if !qualified_under_shell && !unqualified {
        return Ok(None);
    }

    // Argv-style `run(program, args)` — bypasses the entire
    // `Command::new(program).args(args)...output()` chain via
    // direct `std::process::Command` dispatch.  Closes the
    // build-paper.vr `run_step("pdflatex", args, ...)` failure
    // mode that previously surfaced as `Err(nil)` because the
    // Verum-side Command construction across the FFI boundary
    // returned nil mid-chain.
    if bare == "run" {
        if arg_count != 2 {
            return Ok(None);
        }
        return intercept_run_argv(state, args_start_reg, caller_base);
    }

    if arg_count == 0 {
        return Ok(None);
    }
    let cmd_val = state
        .registers
        .get(caller_base, crate::instruction::Reg(args_start_reg));
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
    //
    // VBC-PERM-1 — granular target_id: hash the full command line so
    // a script frontmatter `permissions = ["run=git status"]` grants
    // only that exact invocation.  Falls through to WILDCARD for
    // scripts that grant `"run"` without a target.
    {
        use crate::interpreter::permission::{target_id_for, WILDCARD_TARGET_ID};
        let tid = target_id_for(&cmd_text);
        if state.check_permission(PermissionScope::Process, tid) != PermissionDecision::Allow
            && state.check_permission(PermissionScope::Process, WILDCARD_TARGET_ID)
                == PermissionDecision::Deny
        {
            // Build Err(ShellError.SpawnFailed { command, reason }) directly.
            return Ok(Some(build_err_spawn_failed(
                state,
                &cmd_text,
                "permission denied: shell-spawn requires Process grant",
            )?));
        }
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
    ) = match StdCommand::new("/bin/sh").arg("-c").arg(&cmd_text).output() {
        Ok(out) => {
            // Pack ExitStatus into the Verum `raw: Int` shape: low
            // 8 bits = exit code (when normal exit), bit 7 set for
            // signal termination (waitpid encoding the C runtime
            // already produces; we synthesise a compatible value
            // from `Output.status.code()` since `std` doesn't
            // expose the raw waitpid status portably).
            let raw = match out.status.code() {
                Some(code) => (code as i64) << 8, // shift to mimic WIFEXITED layout
                None => 1,                        // signalled — non-zero raw
            };
            (raw, out.stdout, out.stderr, None)
        }
        Err(e) => (
            127, // POSIX "command not found" sentinel
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
// Argv-style `core.shell.exec.run(program: &Text, args: List<Text>)`
// ============================================================================

/// Intercept `core.shell.exec.run(program, args)` — the canonical
/// argv-style spawn entry-point.  Bypasses the entire Verum-side
/// `Command::new(program).args(&args)...output()` chain and the
/// FFI-bound `native_spawn` / `wait_for_child` / `read_all_from_fd`
/// stack, dispatching straight to `std::process::Command` in the
/// host process.
fn intercept_run_argv(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    use std::process::Command as StdCommand;

    let prog_raw = state
        .registers
        .get(caller_base, crate::instruction::Reg(args_start_reg));
    let prog_unwrapped = unwrap_ref_value(state, prog_raw);
    let program = extract_string(&prog_unwrapped, state);

    let args_raw = state
        .registers
        .get(caller_base, crate::instruction::Reg(args_start_reg + 1));
    let args_unwrapped = unwrap_ref_value(state, args_raw);
    let argv = read_text_list(state, args_unwrapped);

    let display = render_argv(&program, &argv);

    {
        use crate::interpreter::permission::{target_id_for, WILDCARD_TARGET_ID};
        let tid = target_id_for(&program);
        if state.check_permission(PermissionScope::Process, tid) != PermissionDecision::Allow
            && state.check_permission(PermissionScope::Process, WILDCARD_TARGET_ID)
                == PermissionDecision::Deny
        {
            return Ok(Some(build_err_spawn_failed(
                state,
                &display,
                "permission denied: process spawn requires Process grant",
            )?));
        }
    }

    let start = std::time::Instant::now();
    let mut cmd = StdCommand::new(&program);
    cmd.args(&argv);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    let (status_raw, stdout_bytes, stderr_bytes) = match cmd.output() {
        Ok(out) => {
            let raw = match out.status.code() {
                Some(code) => (code as i64) << 8,
                None => 1,
            };
            (raw, out.stdout, out.stderr)
        }
        Err(e) => {
            return Ok(Some(build_err_spawn_failed(
                state,
                &display,
                &format!("spawn failed: {}", e),
            )?));
        }
    };
    let elapsed_nanos = start.elapsed().as_nanos() as i64;
    Ok(Some(build_ok_shell_result(
        state,
        status_raw,
        &stdout_bytes,
        &stderr_bytes,
        &display,
        elapsed_nanos,
    )?))
}

/// Unwrap CBGR-ref / ThinRef encodings down to the underlying Value.
fn unwrap_ref_value(state: &InterpreterState, v: Value) -> Value {
    if super::cbgr_helpers::is_cbgr_ref(&v) {
        let (abs_index, _) = super::cbgr_helpers::decode_cbgr_ref(v.as_i64());
        return state.registers.get_absolute(abs_index);
    }
    if v.is_thin_ref() {
        let tr = v.as_thin_ref();
        if !tr.ptr.is_null() {
            return unsafe { *(tr.ptr as *const Value) };
        }
    }
    v
}

/// Walk a `List<Text>` heap record (`[len, cap, backing_ptr]`
/// header followed by `[Value; cap]` of Texts) into an owned
/// `Vec<String>`.
fn read_text_list(state: &InterpreterState, v: Value) -> Vec<String> {
    use crate::interpreter::heap;
    if !v.is_ptr() || v.is_nil() {
        return Vec::new();
    }
    let ptr = v.as_ptr::<u8>();
    if ptr.is_null() {
        return Vec::new();
    }
    let header = unsafe { heap::ObjectHeader::ref_or_stub(ptr) };
    if (header.size as usize) < 3 * std::mem::size_of::<Value>() {
        return Vec::new();
    }
    let base = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
    let len_v = unsafe { *base };
    let backing_v = unsafe { *base.add(2) };
    if !len_v.is_inline_int() || !backing_v.is_ptr() || backing_v.is_nil() {
        return Vec::new();
    }
    let len = len_v.as_i64() as usize;
    if len == 0 || len > 1_000_000 {
        return Vec::new();
    }
    let backing_ptr = backing_v.as_ptr::<u8>();
    if backing_ptr.is_null() {
        return Vec::new();
    }
    let backing_data = unsafe {
        backing_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
    };
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let elem = unsafe { *backing_data.add(i) };
        out.push(extract_string(&elem, state));
    }
    out
}

/// Render `program + args` as a display string for the
/// `ShellResult.command` / `ShellError.command` fields.  Quotes
/// args containing whitespace/special chars; mirrors the stdlib
/// `core.shell.exec::render_argv` semantics so the interpreter
/// intercept produces a string indistinguishable from the
/// bytecode-compiled path.
fn render_argv(program: &str, args: &[String]) -> String {
    let mut out = String::with_capacity(
        program.len() + args.iter().map(|a| a.len() + 3).sum::<usize>(),
    );
    out.push_str(program);
    for arg in args {
        out.push(' ');
        let needs_quote = arg.is_empty()
            || arg.as_bytes().iter().any(|&b| {
                b == b' ' || b == b'\t' || b == b'\'' || b == b'"' || b == b'$' || b == b'`'
            });
        if needs_quote {
            out.push('\'');
            for ch in arg.chars() {
                if ch == '\'' {
                    out.push_str("'\\''");
                } else {
                    out.push(ch);
                }
            }
            out.push('\'');
        } else {
            out.push_str(arg);
        }
    }
    out
}

// ============================================================================
// ShellResult shape probe — used by method_dispatch CallM path
// ============================================================================

/// Cheap shape check: does this Value point at a heap record that
/// matches our intercept-built `ShellResult` layout?  Fires the
/// `try_intercept_shell_result_inherent_call` shortcut from the
/// CallM dispatch path without paying per-method intercept cost
/// for unrelated receivers.
///
/// Layout signature (5 fields, ≥ 5 × sizeof(Value)):
///   field 0: List<Byte> (stdout_bytes)  — pointer
///   field 1: List<Byte> (stderr_bytes)  — pointer
///   field 2: ExitStatus (1-field record) — pointer
///   field 3: Text (command)              — pointer / small_string
///   field 4: Duration (1-field record)   — pointer
pub(in super::super) fn receiver_looks_like_shell_result(v: &Value) -> bool {
    use crate::interpreter::heap;
    if !v.is_ptr() || v.is_nil() {
        return false;
    }
    let ptr = v.as_ptr::<u8>();
    if ptr.is_null() {
        return false;
    }
    if !(ptr as usize).is_multiple_of(std::mem::align_of::<heap::ObjectHeader>()) {
        return false;
    }
    let header = unsafe { heap::ObjectHeader::ref_or_stub(ptr) };
    // Our shell_runtime intercept allocates exactly 5 fields (40 bytes).
    // Any record of EXACTLY this size + the field-shape check is a
    // safe signal — collisions with other 5-field heap records are
    // possible but the inherent_call intercept's qualified-name
    // check (`ShellResult.<method>`) screens them out at the next
    // step.
    let expected = 5 * std::mem::size_of::<Value>();
    (header.size as usize) == expected
}

// ============================================================================
// ShellResult inherent-method intercept
// ============================================================================

/// Intercept inherent-method calls on `ShellResult` that are
/// statically dispatched as `Call(func_id)` after type resolution.
/// Mirrors `path_ops_runtime::try_intercept_path_inherent_call` for
/// the `core.shell.result.ShellResult` shape.
///
/// Receiver-shape contract (keep in sync with
/// `build_ok_shell_result` field order):
///   field 0: stdout_bytes (List<Byte>)
///   field 1: stderr_bytes (List<Byte>)
///   field 2: status       (ExitStatus { raw: Int })
///   field 3: command      (Text)
///   field 4: duration     (Duration { nanos: Int })
pub(in super::super) fn try_intercept_shell_result_inherent_call(
    state: &mut InterpreterState,
    func_name: &str,
    args_start_reg: u16,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let bare = func_name.rsplit('.').next().unwrap_or(func_name);
    if !matches!(
        bare,
        "stdout" | "stderr" | "bytes" | "stderr_bytes" | "success" | "exit_code" | "code"
    ) {
        return Ok(None);
    }
    if arg_count == 0 {
        return Ok(None);
    }
    let recv_raw = state
        .registers
        .get(caller_base, crate::instruction::Reg(args_start_reg));
    let receiver = unwrap_ref_value(state, recv_raw);
    if !receiver.is_ptr() || receiver.is_nil() {
        return Ok(None);
    }
    // Receiver-shape gate: fire only when `arg[0]` actually points
    // at a ShellResult-shaped record.  This both closes the bare-
    // name dispatch path (`Call(func_id)` registered under
    // unqualified `success` / `stderr` / etc.) and prevents the
    // intercept from shadowing same-named methods on unrelated
    // types (e.g. `Iterator.success` if such a type ever exists).
    if !receiver_looks_like_shell_result(&receiver) {
        // Fall back to the qualified-name gate for callers that
        // routed a non-ShellResult receiver through here (an
        // emergency safety net — should never fire under the
        // shape check above passing, but let the qualified form
        // through for forward compatibility).
        let qualified = func_name.rsplit_terminator('.').take(2).collect::<Vec<_>>();
        if qualified.len() != 2 || qualified[1] != "ShellResult" {
            return Ok(None);
        }
    }
    let ptr = receiver.as_ptr::<u8>();
    if ptr.is_null() {
        return Ok(None);
    }
    use crate::interpreter::heap;
    let header = unsafe { heap::ObjectHeader::ref_or_stub(ptr) };
    if (header.size as usize) < 5 * std::mem::size_of::<Value>() {
        return Ok(None);
    }
    let base = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
    // Method-return-type contract per `core/shell/result.vr::ShellResult`:
    //   stdout()        -> Text  (UTF-8 lossy of stdout_bytes)
    //   stderr()        -> Text  (UTF-8 lossy of stderr_bytes)
    //   bytes()         -> List<Byte>  (zero-copy alias for stdout_bytes)
    //   stderr_bytes()  -> List<Byte>
    //   success()       -> Bool
    //   exit_code()/code() -> Int
    match bare {
        "stdout" => {
            let stdout_v = unsafe { *base };
            let bytes = read_byte_list(stdout_v);
            let s = String::from_utf8_lossy(&bytes).to_string();
            Ok(Some(alloc_string_value(state, &s)?))
        }
        "stderr" => {
            let stderr_v = unsafe { *base.add(1) };
            let bytes = read_byte_list(stderr_v);
            let s = String::from_utf8_lossy(&bytes).to_string();
            Ok(Some(alloc_string_value(state, &s)?))
        }
        "bytes" => {
            // Zero-copy in the stdlib body; we have to re-allocate
            // since intercepts return owned values across the
            // dispatch boundary.  Cheap clone of the byte content.
            let stdout_v = unsafe { *base };
            let bytes = read_byte_list(stdout_v);
            Ok(Some(alloc_byte_list(state, &bytes)?))
        }
        "stderr_bytes" => {
            let stderr_v = unsafe { *base.add(1) };
            let bytes = read_byte_list(stderr_v);
            Ok(Some(alloc_byte_list(state, &bytes)?))
        }
        "success" => {
            let status_v = unsafe { *base.add(2) };
            let raw = read_exit_status_raw(status_v);
            // success() ⇔ process exited normally with code 0.
            // Our raw layout puts code in bits 8..15 (WIFEXITED-style);
            // bits 0..6 are the signal byte.  Code 0 + signal 0 ⇒ success.
            Ok(Some(Value::from_bool(raw == 0)))
        }
        "exit_code" | "code" => {
            let status_v = unsafe { *base.add(2) };
            let raw = read_exit_status_raw(status_v);
            let code = (raw >> 8) & 0xFF;
            Ok(Some(Value::from_i64(code)))
        }
        _ => Ok(None),
    }
}

/// Read the `raw: Int` field of a 1-field `ExitStatus` record.
fn read_exit_status_raw(v: Value) -> i64 {
    use crate::interpreter::heap;
    if !v.is_ptr() || v.is_nil() {
        return 0;
    }
    let ptr = v.as_ptr::<u8>();
    if ptr.is_null() {
        return 0;
    }
    let header = unsafe { heap::ObjectHeader::ref_or_stub(ptr) };
    if (header.size as usize) < std::mem::size_of::<Value>() {
        return 0;
    }
    let raw_v = unsafe { *(ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value) };
    if raw_v.is_inline_int() {
        raw_v.as_i64()
    } else {
        0
    }
}

/// Walk a `List<Byte>` heap record into a `Vec<u8>`.
fn read_byte_list(v: Value) -> Vec<u8> {
    use crate::interpreter::heap;
    if !v.is_ptr() || v.is_nil() {
        return Vec::new();
    }
    let ptr = v.as_ptr::<u8>();
    if ptr.is_null() {
        return Vec::new();
    }
    let header = unsafe { heap::ObjectHeader::ref_or_stub(ptr) };
    if (header.size as usize) < 3 * std::mem::size_of::<Value>() {
        return Vec::new();
    }
    let base = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
    let len_v = unsafe { *base };
    let backing_v = unsafe { *base.add(2) };
    if !len_v.is_inline_int() || !backing_v.is_ptr() || backing_v.is_nil() {
        return Vec::new();
    }
    let len = len_v.as_i64() as usize;
    if len == 0 || len > 100_000_000 {
        return Vec::new();
    }
    let backing_ptr = backing_v.as_ptr::<u8>();
    if backing_ptr.is_null() {
        return Vec::new();
    }
    let backing_data = unsafe {
        backing_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
    };
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        out.push(unsafe { (*backing_data.add(i)).as_i64() as u8 });
    }
    out
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
