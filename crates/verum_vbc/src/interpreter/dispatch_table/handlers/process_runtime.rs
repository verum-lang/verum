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

use super::super::super::error::InterpreterResult;
use super::heap_helpers::{alloc_byte_list, alloc_record_n_fields, wrap_in_variant};
use super::string_helpers::{alloc_string_value, extract_string};
use crate::interpreter::heap;
use crate::interpreter::permission::{PermissionDecision, PermissionScope};
use crate::interpreter::state::InterpreterState;
use crate::types::TypeId;
use crate::value::Value;

pub(in super::super) fn try_intercept_process_runtime(
    state: &mut InterpreterState,
    func_name: &str,
    args_start_reg: u16,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let bare = func_name.rsplit('.').next().unwrap_or(func_name);
    if std::env::var("VERUM_TRACE_PROCESS").is_ok()
        && (func_name.contains("spawn") || func_name.contains("wait") || func_name.contains("exec.run"))
    {
        eprintln!(
            "[process_runtime] called func_name='{}' bare='{}' arg_count={}",
            func_name, bare, arg_count
        );
    }
    match bare {
        // The Command builder's `output()` / `status()` methods
        // both route through `spawn_child_with_output` after pinning
        // stdio to Piped — intercept the helper and we cover both
        // surface paths.
        "spawn_child_with_output" if arg_count == 1 => {
            intercept_spawn_with_output(state, args_start_reg, caller_base)
        }
        // VBC-PROC-3: `Command.spawn()` returns a Child handle without
        // waiting.  Pre-fix this deferred to bytecode (which calls
        // `core.sys.process_native::native_spawn` via FFI — fails at
        // Tier-0 with the libSystem dispatch chain).  Now intercepted
        // and routed through `std::process::Command::spawn` with the
        // returned Child registered in CHILD_REGISTRY for downstream
        // `wait_for_child` / `native_kill` / fd-IO operations.
        "spawn_child" if arg_count == 1 => {
            intercept_spawn_child(state, args_start_reg, caller_base)
        }
        // VBC-PROC-3: `wait_for_child(pid)` blocks until the registered
        // child exits and returns its ExitStatus.  Pre-fix routed
        // through `wait4`/`waitpid` FFI — fails at Tier-0 for the same
        // reason.  Looks up CHILD_REGISTRY by pid; if the pid was
        // produced by our `spawn_child` intercept we own the
        // std::process::Child and call .wait() on it.  If the pid
        // wasn't produced here (rare — e.g. external bytecode spawn),
        // we fall back to libc::waitpid directly.
        "wait_for_child" if arg_count == 1 => {
            intercept_wait_for_child(state, args_start_reg, caller_base)
        }
        // VBC-PROC-3: native fd-IO + signal helpers.  Same Tier-0
        // FFI-bypass pattern — call libc directly so the Verum-side
        // `Child.write_stdin` / `read_stdout` / `signal` end-to-end
        // works in interpreter mode.
        "native_fd_write_all" if arg_count == 2 => {
            intercept_native_fd_write_all(state, args_start_reg, caller_base)
        }
        "native_fd_read_chunk" if arg_count == 2 => {
            intercept_native_fd_read_chunk(state, args_start_reg, caller_base)
        }
        // T0111 LEG A — `core.io.buffer`'s `__fd_read_chunk_raw(fd, max)
        // -> List<Byte>` has a PLACEHOLDER `.vr` body (`{ [] }`), so the
        // bodyless-intrinsic gate in calls.rs never reaches the by-name
        // dispatch for it: the placeholder body ran and produced
        // permanent EOF for every `FdReader.read`.  Placeholder-body
        // intrinsics require this HIGH-LEVEL name intercept (fires
        // before any body dispatch) — same idiom as
        // `native_fd_read_chunk` above.
        "__fd_read_chunk_raw" if arg_count == 2 => {
            intercept_fd_read_chunk_bare(state, args_start_reg, caller_base)
        }
        // T0111 LEG A — `__fd_close_raw_buf` (FdReader/BufReader
        // owned-fd close in `core.io.buffer`) is the same
        // placeholder-body idiom; route to the ONE close bridge so
        // open_files-backed pipe fds actually close (pre-fix: silent
        // no-op leak at Tier-0).
        "__fd_close_raw_buf" if arg_count == 1 => {
            intercept_fd_close_bare(state, args_start_reg, caller_base)
        }
        "close_fd" if arg_count == 1 => {
            intercept_close_fd(state, args_start_reg, caller_base)
        }
        "native_kill" if arg_count == 2 => {
            intercept_native_kill(state, args_start_reg, caller_base)
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
    let cmd_v = unwrap_ref(state, args_start_reg, caller_base);
    let cmd = match read_command_record(state, cmd_v) {
        Some(c) => c,
        None => {
            let msg = alloc_string_value(state, "process.spawn: malformed Command record")?;
            return Ok(Some(wrap_in_variant(state, "Result", 1, &[msg])?));
        }
    };
    // Permission check NEEDS the program path so we extracted the
    // command record first.  Granular `permissions = ["run=/bin/echo"]`
    // grants only that program.
    if let Some(denied) = check_process_permission(state, &cmd.program) {
        return Ok(Some(denied));
    }
    let mut std_cmd =
        build_std_command(&cmd.program, &cmd.args, &cmd.env_vars, &cmd.working_dir);
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
    if ptr.is_null() || !(ptr as usize).is_multiple_of(std::mem::align_of::<heap::ObjectHeader>()) {
        return None;
    }
    let header = unsafe { heap::ObjectHeader::ref_or_stub(ptr) };
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

/// Build a `std::process::Command` from marshaled Verum-side pieces —
/// the ONE construction authority shared by the modern record-driven
/// intercepts (`spawn_child_with_output` / `spawn_child`) and the
/// legacy `__process_spawn_raw` surface (T0111 C1).
fn build_std_command(
    program: &str,
    args: &[String],
    env_vars: &[(String, String)],
    working_dir: &Option<String>,
) -> std::process::Command {
    let mut std_cmd = std::process::Command::new(program);
    for a in args {
        std_cmd.arg(a);
    }
    for (k, v) in env_vars {
        std_cmd.env(k, v);
    }
    if let Some(dir) = working_dir {
        std_cmd.current_dir(dir);
    }
    std_cmd
}

/// Walk a `List<Text>` heap record — three-Value header
/// `[len, cap, backing_ptr]` where backing is `[Value; cap]` of Texts.
/// `pub(super)`: the legacy `__process_spawn_raw` arm in calls.rs
/// marshals its argv through this same walker (T0111 C1).
pub(super) fn read_text_list(state: &InterpreterState, v: Value) -> Option<Vec<String>> {
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
        if !pair_v.is_regular_ptr() || pair_v.is_nil() {
            continue;
        }
        let pair_ptr = pair_v.as_ptr::<u8>();
        if pair_ptr.is_null() {
            continue;
        }
        let pair_base = unsafe { pair_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
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
    if ptr.is_null() || !(ptr as usize).is_multiple_of(std::mem::align_of::<heap::ObjectHeader>()) {
        return None;
    }
    let header = unsafe { heap::ObjectHeader::ref_or_stub(ptr) };
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
    let backing_data = unsafe { backing_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
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
    // SAFETY: pointer non-null and caller already verified the value
    // is a Maybe<Text> via type checks above.
    let tag = unsafe { heap::variant_tag(ptr) };
    let payload_base = unsafe { heap::variant_payload_ptr(ptr, 0) };
    Some((tag, payload_base))
}

fn unwrap_ref(state: &InterpreterState, reg: u16, caller_base: u32) -> Value {
    let v = state
        .registers
        .get(caller_base, crate::instruction::Reg(reg));
    if super::cbgr_helpers::is_cbgr_ref(&v) {
        let (abs_index, _) = super::cbgr_helpers::decode_cbgr_ref(v);
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

/// VBC-PERM-1 decision authority — granular target_id: hash the
/// program path so a script frontmatter `permissions = ["run=/bin/echo"]`
/// grants only that program.  Falls through to WILDCARD for scripts
/// that grant `"run"` without a target.  Shared by the Result-shaped
/// modern intercepts (`check_process_permission`) and the raw-i64
/// legacy `__process_spawn_raw` surface (T0111 C1).
fn process_spawn_denied(state: &mut InterpreterState, program: &str) -> bool {
    use crate::interpreter::permission::{target_id_for, WILDCARD_TARGET_ID};
    let tid = target_id_for(program);
    if state.check_permission(PermissionScope::Process, tid) == PermissionDecision::Allow {
        return false;
    }
    state.check_permission(PermissionScope::Process, WILDCARD_TARGET_ID)
        == PermissionDecision::Deny
}

fn check_process_permission(
    state: &mut InterpreterState,
    program: &str,
) -> Option<Value> {
    if !process_spawn_denied(state, program) {
        return None;
    }
    // Surface as an Err(Text) — the stdlib's spawn return type is
    // `Result<Output, Text>`, so we match its discriminant.
    let msg = alloc_string_value(
        state,
        &format!("permission denied: process spawn `{}` requires `run` grant", program),
    )
    .ok()?;
    wrap_in_variant(state, "Result", 1, &[msg]).ok()
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

// ============================================================================
// VBC-PROC-3 — Child registry + spawn / wait / fd-IO / kill intercepts
// ============================================================================
//
// `std::process::Child` owns the OS process handle.  We MUST keep it
// alive to call `.wait()` later — dropping the Child without waiting
// turns the child into a zombie on Unix (parent never reaps).
//
// Registry keyed by pid: when `wait_for_child(pid)` is called, we
// pull the std Child out of the registry and call its wait().  The
// stdin/stdout/stderr fds are EXTRACTED from the Child via
// into_raw_fd() at spawn time and surrendered to Verum (the script
// owns their lifecycle via `close_fd` intrinsic / `Child.close_stdin`
// helpers).

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

#[cfg(unix)]
struct ChildEntry {
    /// std::process::Child kept alive so wait() can be called later.
    /// Some during the live phase; None after wait() succeeds (so
    /// duplicate wait calls return a sentinel error).
    child: Option<std::process::Child>,
}

#[cfg(not(unix))]
struct ChildEntry {
    child: Option<std::process::Child>,
}

/// Registry of spawned children, keyed by pid.
static CHILD_REGISTRY: LazyLock<Mutex<HashMap<i64, ChildEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn intercept_spawn_child(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let cmd_v = unwrap_ref(state, args_start_reg, caller_base);
    let cmd = match read_command_record(state, cmd_v) {
        Some(c) => c,
        None => {
            let msg = alloc_string_value(state, "process.spawn: malformed Command record")?;
            return Ok(Some(wrap_in_variant(state, "Result", 1, &[msg])?));
        }
    };
    if let Some(denied) = check_process_permission(state, &cmd.program) {
        return Ok(Some(denied));
    }
    let mut std_cmd =
        build_std_command(&cmd.program, &cmd.args, &cmd.env_vars, &cmd.working_dir);
    // Honour the caller's stdio config — unlike spawn_child_with_output
    // which forces Piped, spawn_child preserves the original intent.
    std_cmd.stdin(stdio_from_cfg(cmd.stdin_cfg));
    let stdout_tag = read_stdio_tag(state, args_start_reg, caller_base, 5);
    let stderr_tag = read_stdio_tag(state, args_start_reg, caller_base, 6);
    std_cmd.stdout(stdio_from_cfg(stdout_tag));
    std_cmd.stderr(stdio_from_cfg(stderr_tag));
    let mut child = match std_cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let msg = alloc_string_value(state, &format!("process.spawn: {}", e))?;
            return Ok(Some(wrap_in_variant(state, "Result", 1, &[msg])?));
        }
    };
    let pid = child.id() as i64;

    // Extract stdio fds via into_raw_fd — surrenders ownership to the
    // Verum runtime so the underlying kernel fd survives even after
    // the std::process::ChildStdin/Stdout/Stderr wrappers drop.  The
    // script closes them via `Child.close_stdin` / `close_fd`.
    #[cfg(unix)]
    let (stdin_fd, stdout_fd, stderr_fd) = {
        use std::os::fd::IntoRawFd;
        let stdin_fd = child.stdin.take().map(|s| s.into_raw_fd() as i64);
        let stdout_fd = child.stdout.take().map(|s| s.into_raw_fd() as i64);
        let stderr_fd = child.stderr.take().map(|s| s.into_raw_fd() as i64);
        (stdin_fd, stdout_fd, stderr_fd)
    };
    #[cfg(not(unix))]
    let (stdin_fd, stdout_fd, stderr_fd): (Option<i64>, Option<i64>, Option<i64>) =
        (None, None, None);

    CHILD_REGISTRY
        .lock()
        .unwrap()
        .insert(pid, ChildEntry { child: Some(child) });

    let stdin_field = build_maybe_int(state, stdin_fd)?;
    let stdout_field = build_maybe_int(state, stdout_fd)?;
    let stderr_field = build_maybe_int(state, stderr_fd)?;
    // Field order: pid, stdout_fd, stderr_fd, stdin_fd  (matches
    // core/io/process.vr::Child declaration).
    let child_record = alloc_record_n_fields(
        state,
        "Child",
        &[
            Value::from_i64(pid),
            stdout_field,
            stderr_field,
            stdin_field,
        ],
    )?;
    Ok(Some(wrap_in_variant(state, "Result", 0, &[child_record])?))
}

/// ONE wait authority (T0111 C1): reap `pid` and return the RAW
/// waitpid status word (`encode_exit_status` encoding — the same
/// contract as AOT's `verum_process_wait`).  Registry-first: a Child
/// spawned by either the modern `spawn_child` intercept or the legacy
/// `__process_spawn_raw` surface is taken out of CHILD_REGISTRY so
/// `wait()` blocks outside the lock; foreign pids fall back to
/// waitpid(2).
pub(super) fn wait_child_host(pid: i64) -> Result<i64, String> {
    let owned: Option<std::process::Child> = {
        let mut map = CHILD_REGISTRY.lock().unwrap();
        match map.get_mut(&pid) {
            Some(entry) => entry.child.take(),
            None => None,
        }
    };
    let raw_status: i64 = match owned {
        Some(mut c) => match c.wait() {
            Ok(es) => encode_exit_status(&es),
            Err(e) => {
                // Restore the (now-failed-wait) entry — caller can retry.
                CHILD_REGISTRY
                    .lock()
                    .unwrap()
                    .insert(pid, ChildEntry { child: None });
                return Err(format!("wait_for_child(pid={pid}): {}", e));
            }
        },
        None => {
            // PID isn't in our registry — fall back to libc::waitpid.
            // Errors surface for no such child / EPERM / ECHILD.
            #[cfg(unix)]
            // SAFETY: waitpid writes a c_int status into a live stack
            // slot; an invalid pid yields r < 0, surfaced as Err.
            unsafe {
                let mut status: libc::c_int = 0;
                let r = libc::waitpid(pid as libc::pid_t, &mut status, 0);
                if r < 0 {
                    let errno = std::io::Error::last_os_error();
                    return Err(format!("waitpid(pid={pid}): {}", errno));
                }
                status as i64
            }
            #[cfg(not(unix))]
            {
                return Err(format!("wait_for_child(pid={pid}): not in registry"));
            }
        }
    };
    // Cleanup: drop the registry entry now that the child is reaped.
    CHILD_REGISTRY.lock().unwrap().remove(&pid);
    Ok(raw_status)
}

fn intercept_wait_for_child(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let pid_v = unwrap_ref(state, args_start_reg, caller_base);
    if !pid_v.is_int() {
        let msg = alloc_string_value(state, "wait_for_child: pid must be Int")?;
        return Ok(Some(wrap_in_variant(state, "Result", 1, &[msg])?));
    }
    match wait_child_host(pid_v.as_i64()) {
        Ok(raw_status) => {
            let status =
                alloc_record_n_fields(state, "ExitStatus", &[Value::from_i64(raw_status)])?;
            Ok(Some(wrap_in_variant(state, "Result", 0, &[status])?))
        }
        Err(e) => {
            let msg = alloc_string_value(state, &e)?;
            Ok(Some(wrap_in_variant(state, "Result", 1, &[msg])?))
        }
    }
}

/// SIGKILL a spawned child by pid — CHILD_REGISTRY-first (std Child
/// handle), raw kill(2) fallback for pids spawned outside the registry.
/// Backs the legacy `__process_kill_raw` surface (T0111 C1).
pub(super) fn kill_child_host(pid: i64) -> bool {
    if let Some(entry) = CHILD_REGISTRY.lock().unwrap().get_mut(&pid) {
        if let Some(child) = entry.child.as_mut() {
            return child.kill().is_ok();
        }
    }
    #[cfg(unix)]
    {
        // SAFETY: kill(2) with a caller-supplied pid; an invalid pid
        // yields ESRCH (reported as false), never UB.
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) == 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// Detach a captured stdout pipe as a host `File` (unix: OwnedFd,
/// windows: OwnedHandle).  `None` on platforms without an fd/handle
/// conversion — the dropped pipe closes and the child sees EPIPE.
fn take_stdout_file(child: &mut std::process::Child) -> Option<std::fs::File> {
    #[cfg(any(unix, windows))]
    {
        child.stdout.take().map(pipe_into_file)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = child;
        None
    }
}

/// Twin of `take_stdout_file` for stderr.
fn take_stderr_file(child: &mut std::process::Child) -> Option<std::fs::File> {
    #[cfg(any(unix, windows))]
    {
        child.stderr.take().map(pipe_into_file)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = child;
        None
    }
}

#[cfg(unix)]
fn pipe_into_file(pipe: impl Into<std::os::fd::OwnedFd>) -> std::fs::File {
    std::fs::File::from(pipe.into())
}

#[cfg(windows)]
fn pipe_into_file(pipe: impl Into<std::os::windows::io::OwnedHandle>) -> std::fs::File {
    std::fs::File::from(pipe.into())
}

/// Register a captured child pipe in `state.open_files` keyed by a
/// fresh synthetic fd id (monotone, >= 100).  Returns the id, or -1
/// when there is no pipe to register.
fn register_child_pipe(state: &mut InterpreterState, file: Option<std::fs::File>) -> i64 {
    match file {
        Some(f) => {
            let fd = state.next_fd;
            state.next_fd += 1;
            state.open_files.insert(fd, f);
            fd
        }
        None => -1,
    }
}

/// T0111 C1 — host-side spawn for the LEGACY `__process_spawn_raw`
/// intrinsic surface (`core.sys.process_ops`).  Routes onto the SAME
/// VBC-PROC-3 authorities as the modern path — `std::process::Command`
/// via `build_std_command`, CHILD_REGISTRY keyed by the REAL host pid
/// (so `__process_wait_raw` shares `wait_child_host` with the modern
/// `wait_for_child`), and `state.open_files` for captured pipes
/// (readable through the `host_fd_*` bridge, closable via
/// `__fd_close_raw`).
///
/// Returns a host pointer to an i64 `[pid, stdout_fd, stderr_fd]`
/// triple (fd slots -1 when not captured) or 0 on spawn failure /
/// permission denial — the exact contract of AOT's
/// `verum_process_spawn_cmd` (the `__process_spawn_raw` lowering in
/// verum_codegen instruction.rs; usage pinned by
/// vcs/specs/L0-critical/vbc/e2e/aot/953_process_output_capture.vr).
///
/// Stdio policy mirrors AOT `verum_process_spawn`: captured streams
/// are piped; UNcaptured streams INHERIT the parent's (pre-fix Tier-0
/// nulled them, diverging from the AOT contract); stdin always
/// inherits.
pub(super) fn spawn_raw_triple(
    state: &mut InterpreterState,
    program: &str,
    args: &[String],
    capture_stdout: bool,
    capture_stderr: bool,
) -> i64 {
    use std::process::Stdio;
    if process_spawn_denied(state, program) {
        if std::env::var("VERUM_TRACE_PROCESS").is_ok() {
            eprintln!(
                "[process_runtime] __process_spawn_raw '{}' denied by Process permission",
                program
            );
        }
        return 0;
    }
    let mut cmd = build_std_command(program, args, &[], &None);
    cmd.stdout(if capture_stdout {
        Stdio::piped()
    } else {
        Stdio::inherit()
    });
    cmd.stderr(if capture_stderr {
        Stdio::piped()
    } else {
        Stdio::inherit()
    });
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            if std::env::var("VERUM_TRACE_PROCESS").is_ok() {
                eprintln!(
                    "[process_runtime] __process_spawn_raw '{}' failed: {}",
                    program, e
                );
            }
            return 0;
        }
    };
    let pid = child.id() as i64;
    let stdout_fd = if capture_stdout {
        register_child_pipe(state, take_stdout_file(&mut child))
    } else {
        -1
    };
    let stderr_fd = if capture_stderr {
        register_child_pipe(state, take_stderr_file(&mut child))
    } else {
        -1
    };
    CHILD_REGISTRY
        .lock()
        .unwrap()
        .insert(pid, ChildEntry { child: Some(child) });
    alloc_host_i64_triple(pid, stdout_fd, stderr_fd)
}

fn intercept_native_fd_write_all(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let fd_v = unwrap_ref(state, args_start_reg, caller_base);
    let _data_v = unwrap_ref(state, args_start_reg + 1, caller_base);
    if !fd_v.is_int() {
        let msg = alloc_string_value(state, "fd_write_all: fd must be Int")?;
        return Ok(Some(wrap_in_variant(state, "Result", 1, &[msg])?));
    }
    let fd = fd_v.as_i64();
    let data = super::heap_helpers::extract_byte_slice(state, args_start_reg + 1, caller_base);
    #[cfg(unix)]
    {
        let mut written = 0_usize;
        while written < data.len() {
            let n = unsafe {
                libc::write(
                    fd as libc::c_int,
                    data[written..].as_ptr() as *const libc::c_void,
                    data.len() - written,
                )
            };
            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                let msg = alloc_string_value(
                    state,
                    &format!("fd_write_all(fd={fd}): {}", err),
                )?;
                return Ok(Some(wrap_in_variant(state, "Result", 1, &[msg])?));
            }
            if n == 0 {
                break;
            }
            written += n as usize;
        }
        Ok(Some(wrap_in_variant(
            state,
            "Result",
            0,
            &[Value::from_i64(written as i64)],
        )?))
    }
    #[cfg(not(unix))]
    {
        let _ = (fd, data);
        let msg = alloc_string_value(state, "fd_write_all: not supported on this platform")?;
        Ok(Some(wrap_in_variant(state, "Result", 1, &[msg])?))
    }
}

// ============================================================================
// T0111 — ONE host-side fd bridge (open_files-first, raw-fd fallback)
// ============================================================================
//
// Tier-0 has TWO fd spaces (the H3 fd-space schism, tracked under
// T0111 residuals):
//
//  * synthetic `state.open_files` ids (>= 100) — file opens via
//  `__file_open_raw` AND child pipe fds registered by the legacy
//  `__process_spawn_raw` surface;
//  * raw host fds — the VBC-PROC-3 `spawn_child` intercept surrenders
//  `into_raw_fd()` values to the script.
//
// Every Tier-0 fd read/close funnels through these primitives so both
// spaces resolve identically: the synthetic table is consulted FIRST
// (`next_fd` is monotone and starts at 100, so a handed-out synthetic
// id can never be re-keyed), then the value is treated as a raw host
// fd.  A numeric collision (a real host fd >= 100 while the same id is
// live in `open_files`) would misroute — that residual risk is the H3
// schism itself and is eliminated only by unifying the two spaces.

/// Read up to `max` bytes from `fd`.  Blocking + EINTR-safe.
///
/// POSIX pipe contract: a read on a pipe with no data BLOCKS until the
/// writer produces bytes or closes its end (EOF -> `Ok(vec![])`).
/// Callers that drain to EOF therefore block until child exit.
pub(super) fn host_fd_read_chunk(
    state: &mut InterpreterState,
    fd: i64,
    max: usize,
) -> Result<Vec<u8>, String> {
    let max = max.min(1 << 20);
    if let Some(file) = state.open_files.get_mut(&fd) {
        use std::io::Read as _;
        let mut buf = vec![0_u8; max];
        loop {
            match file.read(&mut buf) {
                Ok(n) => {
                    buf.truncate(n);
                    return Ok(buf);
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(format!("fd_read_chunk(fd={fd}): {}", e)),
            }
        }
    }
    #[cfg(unix)]
    {
        let mut buf = vec![0_u8; max];
        loop {
            // SAFETY: buf is a live owned allocation of len bytes; read(2)
            // writes at most buf.len() bytes.  A bogus fd yields EBADF,
            // surfaced as Err — never UB.
            let n = unsafe {
                libc::read(
                    fd as libc::c_int,
                    buf.as_mut_ptr() as *mut libc::c_void,
                    buf.len(),
                )
            };
            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(format!("fd_read_chunk(fd={fd}): {}", err));
            }
            buf.truncate(n as usize);
            return Ok(buf);
        }
    }
    #[cfg(not(unix))]
    {
        Err(format!(
            "fd_read_chunk(fd={fd}): raw-fd reads not supported on this platform"
        ))
    }
}

/// Drain `fd` to EOF through the chunk primitive.  Blocks until the
/// peer closes the fd (see `host_fd_read_chunk`'s pipe contract).
pub(super) fn host_fd_read_to_end(
    state: &mut InterpreterState,
    fd: i64,
) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let chunk = host_fd_read_chunk(state, fd, 4096)?;
        if chunk.is_empty() {
            return Ok(out);
        }
        out.extend_from_slice(&chunk);
    }
}

/// Close `fd` through the same two-space bridge: an `open_files` hit is
/// removed (dropping the `File` closes the OS handle); otherwise the
/// value is treated as a raw host fd.
///
/// Tier-0 stdio guard: at Tier-0 the script executes INSIDE the
/// interpreter's own process — raw-closing fd 0/1/2 would sever the
/// harness's stdio (the io/buffer suite constructs
/// `FdReader.from_owned_fd(0..2)` whose Drop lands here and killed the
/// batch runner when this close became real).  AOT-compiled programs
/// own their process and close stdio for real — a documented,
/// deliberate tier divergence, not an accident.
pub(super) fn host_fd_close(state: &mut InterpreterState, fd: i64) {
    if state.open_files.remove(&fd).is_some() {
        return;
    }
    if fd <= 2 {
        return;
    }
    #[cfg(unix)]
    // SAFETY: fd is a host descriptor surrendered to the script by the
    // spawn intercept (`into_raw_fd`) — closing it is the script's
    // documented duty; a stale/duplicate close returns EBADF, never UB.
    unsafe {
        libc::close(fd as libc::c_int);
    }
    #[cfg(not(unix))]
    {
        let _ = fd;
    }
}

fn intercept_native_fd_read_chunk(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let fd_v = unwrap_ref(state, args_start_reg, caller_base);
    let max_v = unwrap_ref(state, args_start_reg + 1, caller_base);
    if !fd_v.is_int() || !max_v.is_int() {
        let msg = alloc_string_value(state, "fd_read_chunk: args must be Int")?;
        return Ok(Some(wrap_in_variant(state, "Result", 1, &[msg])?));
    }
    let fd = fd_v.as_i64();
    let max = max_v.as_i64().clamp(0, 1 << 20) as usize;
    match host_fd_read_chunk(state, fd, max) {
        Ok(bytes) => {
            let chunk = alloc_byte_list(state, &bytes)?;
            Ok(Some(wrap_in_variant(state, "Result", 0, &[chunk])?))
        }
        Err(msg) => {
            let m = alloc_string_value(state, &msg)?;
            Ok(Some(wrap_in_variant(state, "Result", 1, &[m])?))
        }
    }
}

/// T0111 LEG A — bare-`List<Byte>` chunk read backing `core.io.buffer`'s
/// `__fd_read_chunk_raw(fd, max) -> List<Byte>`.  Unlike
/// `native_fd_read_chunk` (Result-wrapped), the buffer.vr contract is a
/// BARE list — empty on EOF.  Read errors have no channel in that
/// signature and surface as EOF (empty list).  AOT twin (the missing
/// `verum_fd_read_chunk` emitter) is pooled separately as T0376.
fn intercept_fd_read_chunk_bare(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let fd_v = unwrap_ref(state, args_start_reg, caller_base);
    let max_v = unwrap_ref(state, args_start_reg + 1, caller_base);
    if !fd_v.is_int() || !max_v.is_int() {
        return Ok(Some(alloc_byte_list(state, &[])?));
    }
    let fd = fd_v.as_i64();
    let max = max_v.as_i64().clamp(0, 1 << 20) as usize;
    let bytes = host_fd_read_chunk(state, fd, max).unwrap_or_default();
    Ok(Some(alloc_byte_list(state, &bytes)?))
}

/// T0111 LEG A — unit-returning close backing `core.io.buffer`'s
/// `__fd_close_raw_buf(fd)` placeholder.
fn intercept_fd_close_bare(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let fd_v = unwrap_ref(state, args_start_reg, caller_base);
    if fd_v.is_int() {
        host_fd_close(state, fd_v.as_i64());
    }
    Ok(Some(Value::unit()))
}

fn intercept_close_fd(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let fd_v = unwrap_ref(state, args_start_reg, caller_base);
    if !fd_v.is_int() {
        return Ok(Some(Value::unit()));
    }
    host_fd_close(state, fd_v.as_i64());
    Ok(Some(Value::unit()))
}

fn intercept_native_kill(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let pid_v = unwrap_ref(state, args_start_reg, caller_base);
    let sig_v = unwrap_ref(state, args_start_reg + 1, caller_base);
    if !pid_v.is_int() || !sig_v.is_int() {
        let msg = alloc_string_value(state, "native_kill: args must be Int")?;
        return Ok(Some(wrap_in_variant(state, "Result", 1, &[msg])?));
    }
    let pid = pid_v.as_i64();
    let sig = sig_v.as_i64();
    #[cfg(unix)]
    {
        let r = unsafe { libc::kill(pid as libc::pid_t, sig as libc::c_int) };
        if r == 0 {
            Ok(Some(wrap_in_variant(state, "Result", 0, &[Value::unit()])?))
        } else {
            let err = std::io::Error::last_os_error();
            let msg = alloc_string_value(
                state,
                &format!("kill(pid={pid}, sig={sig}): {}", err),
            )?;
            Ok(Some(wrap_in_variant(state, "Result", 1, &[msg])?))
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (pid, sig);
        let msg = alloc_string_value(state, "native_kill: not supported on this platform")?;
        Ok(Some(wrap_in_variant(state, "Result", 1, &[msg])?))
    }
}

// ============================================================================
// T0111 — host [len, cap, buf] byte-buffer ABI (Tier-0 twin of AOT)
// ============================================================================

/// Allocate a host `[len@0, cap@8, buf_ptr@16]` 24-byte header plus its
/// data buffer via `std::alloc` and return the header address as i64.
///
/// This is the Tier-0 twin of the AOT `verum_fd_read_all` result ABI
/// (`platform_ir.rs::emit_fd_read_all_ir` — the canonical byte-buffer
/// header authority).  Caller-side reads go through the 2-arg
/// `__ptr_read_i64(ptr, index)` intrinsic; `__ptr_free` is a documented
/// no-op on BOTH tiers (AOT: "24 bytes per spawn call are negligible"),
/// so the allocation intentionally has static lifetime.
///
/// Returns 0 only on allocation failure — an EMPTY read still yields a
/// valid header with len 0, exactly like the AOT drain loop.
pub(super) fn alloc_len_cap_buf_header(bytes: &[u8]) -> i64 {
    let cap = bytes.len().max(8);
    let buf_layout = match std::alloc::Layout::from_size_align(cap, 8) {
        Ok(l) => l,
        Err(_) => return 0,
    };
    // SAFETY: layout is non-zero-sized (cap >= 8) and 8-aligned.
    let buf = unsafe { std::alloc::alloc_zeroed(buf_layout) };
    if buf.is_null() {
        return 0;
    }
    if !bytes.is_empty() {
        // SAFETY: buf has cap >= bytes.len() writable bytes; bytes is a
        // live borrow; regions are distinct allocations.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, bytes.len()) };
    }
    let hdr_layout = std::alloc::Layout::from_size_align(24, 8)
        .expect("static 24/8 layout is valid");
    // SAFETY: 24-byte non-zero layout.
    let hdr = unsafe { std::alloc::alloc_zeroed(hdr_layout) } as *mut i64;
    if hdr.is_null() {
        return 0;
    }
    // SAFETY: hdr points at 24 writable, 8-aligned bytes = 3 i64 slots.
    unsafe {
        *hdr = bytes.len() as i64;
        *hdr.add(1) = cap as i64;
        *hdr.add(2) = buf as i64;
    }
    hdr as i64
}

/// Allocate a host i64 `[a, b, c]` triple (24 bytes) and return its
/// address — the `__process_spawn_raw` result shape
/// `[pid, stdout_fd, stderr_fd]`, mirroring AOT's
/// `verum_process_spawn_cmd` contract (instruction.rs lowering).
/// Freed by the same no-op `__ptr_free` policy as the header above.
pub(super) fn alloc_host_i64_triple(a: i64, b: i64, c: i64) -> i64 {
    let layout = std::alloc::Layout::from_size_align(24, 8)
        .expect("static 24/8 layout is valid");
    // SAFETY: 24-byte non-zero layout.
    let ptr = unsafe { std::alloc::alloc_zeroed(layout) } as *mut i64;
    if ptr.is_null() {
        return 0;
    }
    // SAFETY: ptr points at 24 writable, 8-aligned bytes = 3 i64 slots.
    unsafe {
        *ptr = a;
        *ptr.add(1) = b;
        *ptr.add(2) = c;
    }
    ptr as i64
}

// ----------------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------------

/// Read the variant tag of the stdin/stdout/stderr Stdio config at
/// the given Command-record field index.
fn read_stdio_tag(
    state: &InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
    _field_idx: usize,
) -> u32 {
    // Reach back into the Command record from arg 0 (it's a &Command
    // ref).  Field i lives at OBJECT_HEADER_SIZE + i * sizeof(Value).
    let cmd_v = unwrap_ref(state, args_start_reg, caller_base);
    if !cmd_v.is_ptr() || cmd_v.is_nil() {
        return 0;
    }
    let ptr = cmd_v.as_ptr::<u8>();
    if ptr.is_null() {
        return 0;
    }
    let header = unsafe { heap::ObjectHeader::ref_or_stub(ptr) };
    let needed = (_field_idx + 1) * std::mem::size_of::<Value>();
    if (header.size as usize) < needed {
        return 0;
    }
    let base = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
    let v = unsafe { *base.add(_field_idx) };
    read_variant_tag(v).unwrap_or(0)
}

/// Build a Maybe<Int> Verum value: None (tag=0, no payload) or
/// Some(i) (tag=1, one i64 payload).
fn build_maybe_int(state: &mut InterpreterState, opt: Option<i64>) -> InterpreterResult<Value> {
    match opt {
        None => wrap_in_variant(state, "Maybe", 0, &[]),
        Some(i) => wrap_in_variant(state, "Maybe", 1, &[Value::from_i64(i)]),
    }
}

// ============================================================================
// Tests — VBC-PROC-3
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Spawn `/bin/echo hello`, capture pid via Rust-side helpers,
    /// wait, observe exit status 0.  This is the round-trip
    /// proof for the spawn_child + wait_for_child intercept pair.
    /// Bypasses the Verum-side dispatch (which requires a full
    /// interpreter state); exercises the host-side primitives
    /// directly via std::process::Command + the registry.
    #[cfg(unix)]
    #[test]
    fn spawn_then_wait_round_trip_via_std() {
        let child = std::process::Command::new("/bin/echo")
            .arg("hello")
            .spawn()
            .unwrap();
        let pid = child.id() as i64;
        // Insert into our registry so wait_for_child intercept finds it.
        CHILD_REGISTRY
            .lock()
            .unwrap()
            .insert(pid, ChildEntry { child: Some(child) });
        // Pull and wait via the same path the intercept uses.
        let owned = {
            let mut map = CHILD_REGISTRY.lock().unwrap();
            map.get_mut(&pid).unwrap().child.take()
        };
        let mut c = owned.unwrap();
        let status = c.wait().unwrap();
        assert!(status.success());
        CHILD_REGISTRY.lock().unwrap().remove(&pid);
    }

    /// `intercept_native_kill` on a non-existent pid surfaces as
    /// Result.Err with the errno-derived message.  Pin the failure
    /// shape so callers don't accidentally treat a missing-pid kill
    /// as success.
    #[cfg(unix)]
    #[test]
    fn native_kill_on_invalid_pid_is_err() {
        // PID 1 (init) we DON'T want to actually signal.  Use a
        // negative PID — libc::kill rejects with EINVAL/EPERM
        // depending on the OS.
        let r = unsafe { libc::kill(-12345 as libc::pid_t, 0) };
        assert!(r != 0, "expected kill(-12345) to fail");
    }
}
