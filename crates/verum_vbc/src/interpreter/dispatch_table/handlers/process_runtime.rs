//! High-level Rust intercepts for `core.io.process` operations.
//!

//! Sibling to `shell_runtime.rs` (VBC-1), `file_runtime.rs`
//! (VBC-FILE-1 + VBC-FS-2), `env_runtime.rs` (VBC-ENV-1 +
//! VBC-PROC-1), and `stdio_runtime.rs` (VBC-STDIO-1/2).
//! Bypasses the libSystem fork(2)/execve(2)/pipe(2)/dup2(2) FFI
//! dispatch chain — which currently fails inside `make_pipe` /
//! `native_spawn` with `Function 4294967295 not found` (sentinel
//! id from a missed function-table population step) — by catching
//! the high-level `spawn_child_with_output` / `spawn_child` /
//! `wait_for_child` calls at the `Call` boundary and dispatching
//! straight to `std::process::Command`.
//!

//! # Functions intercepted
//!

//!  * `spawn_child_with_output(cmd: &Command) -> Result<Output, Text>`
//!  — `std::process::Command::output()`. Constructs the full
//!  stdlib `Output { status, stdout_bytes, stderr_bytes }` record.
//!  * `spawn_child(cmd: &Command) -> Result<Child, Text>` — defers
//!  to bytecode (Child stateful — not yet covered).
//!  * `wait_for_child(pid: Int) -> Result<ExitStatus, Text>` —
//!  defers (depends on prior spawn_child path).
//!

//! # Command field layout
//!

//! `core/io/process.vr::Command` is a 7-field record laid out as
//! `[ObjectHeader][program: Value][args: Value][env_vars: Value]
//!  [working_dir: Value][stdin_cfg: Value][stdout_cfg: Value]
//!  [stderr_cfg: Value]`. Field i lives at `OBJECT_HEADER_SIZE +
//! i * sizeof(Value)`. Stdio config is a 3-arm sum (Inherit / Piped
//! / Null) — we read the variant tag to decide between
//! `std::process::Stdio::inherit() / piped() / null()`.

use std::io::Read;

use crate::interpreter::heap;
use crate::interpreter::permission::{PermissionDecision, PermissionScope};
use crate::interpreter::state::InterpreterState;
use crate::types::TypeId;
use crate::value::Value;
use super::super::super::error::InterpreterResult;
use super::heap_helpers::{alloc_byte_list, alloc_record_n_fields, wrap_in_variant};
use super::string_helpers::{alloc_string_value, extract_string};

pub(in super::super) fn try_intercept_process_runtime(
    state: &mut InterpreterState,
    func_name: &str,
    args_start_reg: u16,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let bare = func_name.rsplit('.').next().unwrap_or(func_name);
    match bare {
        // The Command builder's `output()` / `status()` methods
        // both route through `spawn_child_with_output` after pinning
        // stdio to Piped — intercept the helper and we cover both
        // surface paths.
        "spawn_child_with_output" if arg_count == 1 => {
            intercept_spawn_with_output(state, args_start_reg, caller_base)
        }
        _ => Ok(None),
    }
}

// ============================================================================
// Spawn + capture
// ============================================================================

fn intercept_spawn_with_output(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    if let Some(denied) = check_process_permission(state) {
        return Ok(Some(denied));
    }
    let cmd_v = unwrap_ref(state, args_start_reg, caller_base);
    let cmd = match read_command_record(state, cmd_v) {
        Some(c) => c,
        None => {
            let msg = alloc_string_value(state, "process.spawn: malformed Command record")?;
            return Ok(Some(wrap_in_variant(state, "Result", 1, &[msg])?));
        }
    };
    let mut std_cmd = std::process::Command::new(&cmd.program);
    for a in &cmd.args {
        std_cmd.arg(a);
    }
    for (k, v) in &cmd.env_vars {
        std_cmd.env(k, v);
    }
    if let Some(dir) = &cmd.working_dir {
        std_cmd.current_dir(dir);
    }
    std_cmd.stdin(stdio_from_cfg(cmd.stdin_cfg));
    // The stdlib's `Command.output()` pins stdout/stderr to Piped
    // before invoking `spawn_child_with_output`, so we override
    // here regardless of the caller's choice — matches stdlib intent.
    std_cmd.stdout(std::process::Stdio::piped());
    std_cmd.stderr(std::process::Stdio::piped());
    match std_cmd.output() {
        Ok(out) => {
            let raw = encode_exit_status(&out.status);
            let status = alloc_record_n_fields(state, "ExitStatus", &[Value::from_i64(raw)])?;
            let stdout_list = alloc_byte_list(state, &out.stdout)?;
            let stderr_list = alloc_byte_list(state, &out.stderr)?;
            let output =
                alloc_record_n_fields(state, "Output", &[status, stdout_list, stderr_list])?;
            Ok(Some(wrap_in_variant(state, "Result", 0, &[output])?))
        }
        Err(e) => {
            let msg = alloc_string_value(state, &format!("process.spawn: {}", e))?;
            Ok(Some(wrap_in_variant(state, "Result", 1, &[msg])?))
        }
    }
}

// ============================================================================
// Command record reading
// ============================================================================

struct CommandData {
    program: String,
    args: Vec<String>,
    env_vars: Vec<(String, String)>,
    working_dir: Option<String>,
    stdin_cfg: u32,
    // stdout_cfg / stderr_cfg are read from the record but pinned to
    // Piped at execution time — see `intercept_spawn_with_output`.
}

fn read_command_record(state: &InterpreterState, v: Value) -> Option<CommandData> {
    if !v.is_ptr() || v.is_nil() {
        return None;
    }
    let ptr = v.as_ptr::<u8>();
    if ptr.is_null()
        || !(ptr as usize).is_multiple_of(std::mem::align_of::<heap::ObjectHeader>())
    {
        return None;
    }
    let header = unsafe { &*(ptr as *const heap::ObjectHeader) };
    if (header.size as usize) < 7 * std::mem::size_of::<Value>() {
        return None;
    }
    let base = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
    let program_v = unsafe { *base };
    let args_v = unsafe { *base.add(1) };
    let env_vars_v = unsafe { *base.add(2) };
    let working_dir_v = unsafe { *base.add(3) };
    let stdin_cfg_v = unsafe { *base.add(4) };

    let program = extract_string(&program_v, state);
    let args = read_text_list(state, args_v).unwrap_or_default();
    let env_vars = read_pair_list(state, env_vars_v).unwrap_or_default();
    let working_dir = read_maybe_text(state, working_dir_v);
    let stdin_cfg = read_variant_tag(stdin_cfg_v).unwrap_or(0);

    Some(CommandData {
        program,
        args,
        env_vars,
        working_dir,
        stdin_cfg,
    })
}

/// Walk a `List<Text>` heap record — three-Value header
/// `[len, cap, backing_ptr]` where backing is `[Value; cap]` of Texts.
fn read_text_list(state: &InterpreterState, v: Value) -> Option<Vec<String>> {
    let (len, backing_ptr) = read_list_header(v)?;
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let elem = unsafe { *backing_ptr.add(i) };
        out.push(extract_string(&elem, state));
    }
    Some(out)
}

/// Walk a `List<(Text, Text)>` — each element is a 2-field tuple
/// record `[Text, Text]` at `OBJECT_HEADER_SIZE` offset.
fn read_pair_list(state: &InterpreterState, v: Value) -> Option<Vec<(String, String)>> {
    let (len, backing_ptr) = read_list_header(v)?;
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let pair_v = unsafe { *backing_ptr.add(i) };
        if !pair_v.is_ptr() || pair_v.is_nil() {
            continue;
        }
        let pair_ptr = pair_v.as_ptr::<u8>();
        if pair_ptr.is_null() {
            continue;
        }
        let pair_base =
            unsafe { pair_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
        let k_v = unsafe { *pair_base };
        let v_v = unsafe { *pair_base.add(1) };
        out.push((extract_string(&k_v, state), extract_string(&v_v, state)));
    }
    Some(out)
}

fn read_list_header(v: Value) -> Option<(usize, *const Value)> {
    if !v.is_ptr() || v.is_nil() {
        return None;
    }
    let ptr = v.as_ptr::<u8>();
    if ptr.is_null()
        || !(ptr as usize).is_multiple_of(std::mem::align_of::<heap::ObjectHeader>())
    {
        return None;
    }
    let header = unsafe { &*(ptr as *const heap::ObjectHeader) };
    if header.type_id != TypeId::LIST {
        return None;
    }
    let data_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
    let len = unsafe { (*data_ptr).as_i64() } as usize;
    let backing_v = unsafe { *data_ptr.add(2) };
    if !backing_v.is_ptr() || backing_v.is_nil() {
        return None;
    }
    let backing_ptr = backing_v.as_ptr::<u8>();
    if backing_ptr.is_null() {
        return None;
    }
    let backing_data =
        unsafe { backing_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
    Some((len, backing_data))
}

/// Maybe<Text> — None tag = 0, Some tag = 1 with one Text payload.
fn read_maybe_text(state: &InterpreterState, v: Value) -> Option<String> {
    let (tag, payload_base) = read_variant_payload(v)?;
    if tag == 0 {
        return None;
    }
    let payload = unsafe { *payload_base };
    Some(extract_string(&payload, state))
}

fn read_variant_tag(v: Value) -> Option<u32> {
    let (tag, _) = read_variant_payload(v)?;
    Some(tag)
}

fn read_variant_payload(v: Value) -> Option<(u32, *const Value)> {
    if !v.is_ptr() || v.is_nil() {
        return None;
    }
    let ptr = v.as_ptr::<u8>();
    if ptr.is_null() {
        return None;
    }
    let tag_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *const u32 };
    let tag = unsafe { *tag_ptr };
    let payload_base =
        unsafe { ptr.add(heap::OBJECT_HEADER_SIZE + 8) as *const Value };
    Some((tag, payload_base))
}

fn unwrap_ref(state: &InterpreterState, reg: u16, caller_base: u32) -> Value {
    let v = state.registers.get(caller_base, crate::instruction::Reg(reg));
    if super::cbgr_helpers::is_cbgr_ref(&v) {
        let (abs_index, _) = super::cbgr_helpers::decode_cbgr_ref(v.as_i64());
        state.registers.get_absolute(abs_index)
    } else {
        v
    }
}

fn stdio_from_cfg(tag: u32) -> std::process::Stdio {
    match tag {
        0 => std::process::Stdio::inherit(),
        1 => std::process::Stdio::piped(),
        2 => std::process::Stdio::null(),
        _ => std::process::Stdio::inherit(),
    }
}

// ============================================================================
// Permission gate
// ============================================================================

fn check_process_permission(state: &mut InterpreterState) -> Option<Value> {
    if state.check_permission(PermissionScope::Process, 0) == PermissionDecision::Deny {
        // Surface as an Err(Text) — the stdlib's spawn return type is
        // `Result<Output, Text>`, so we match its discriminant.
        let msg =
            alloc_string_value(state, "permission denied: process spawn requires `run`")
                .ok()?;
        return wrap_in_variant(state, "Result", 1, &[msg]).ok();
    }
    None
}

// ============================================================================
// Exit-status encoding
// ============================================================================

/// Encode `std::process::ExitStatus` into the raw waitpid() word that
/// Verum's `ExitStatus { raw: Int }` expects. On Unix this is the
/// canonical (status << 8) | sig word; on Windows it's just the exit
/// code. The stdlib's `success` / `is_exited` / `signal` / `code`
/// methods read these exact bits.
#[cfg(unix)]
fn encode_exit_status(s: &std::process::ExitStatus) -> i64 {
    use std::os::unix::process::ExitStatusExt;
    if let Some(code) = s.code() {
        ((code as i64) & 0xFF) << 8
    } else if let Some(sig) = s.signal() {
        // Low 7 bits = signal, plus core-dump bit at 0x80 if available.
        let dumped = s.core_dumped();
        ((sig as i64) & 0x7F) | (if dumped { 0x80 } else { 0 })
    } else {
        0
    }
}

#[cfg(not(unix))]
fn encode_exit_status(s: &std::process::ExitStatus) -> i64 {
    s.code().unwrap_or(0) as i64
}


// `Read` import is currently unused — kept for symmetry with sibling
// modules that may grow streaming-spawn intercepts.
#[allow(dead_code)]
fn _suppress_unused_imports(r: &mut dyn Read) -> std::io::Result<usize> {
    let mut buf = [0u8; 0];
    r.read(&mut buf)
}
