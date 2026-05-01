//! High-level Rust intercepts for `core.io.stdio` operations.
//!

//! Sibling to `shell_runtime.rs` (VBC-1), `file_runtime.rs`
//! (VBC-FILE-1), and `env_runtime.rs` (VBC-ENV-1 + VBC-PROC-1).
//! Bypasses the libSystem `write(2)`/`read(2)` FFI dispatch on
//! stdin/stdout/stderr and uses `std::io::stdin/stdout/stderr`
//! directly from the host process.
//!

//! # Functions intercepted
//!

//!  * `read_line() -> IoResult<Text>` — `std::io::stdin().read_line()`
//!  with trailing `\n` (and `\r` for CRLF) stripped to match the
//!  Verum stdlib's contract.
//!  * `read_int() -> IoResult<Int>` — `read_line()` + parse to i64;
//!  parse failure surfaces as `StreamError { kind: InvalidData, ... }`.
//!  * `read_float() -> IoResult<Float>` — same shape, parse to f64.
//!  * `read_to_end() -> IoResult<Text>` — drain stdin to EOF
//!  (uses `read_to_string`).
//!  * `print(s: &Text)` / `println(s: &Text)` — `std::io::stdout()`
//!  write + flush; `println` appends `\n`.
//!  * `eprint(s: &Text)` / `eprintln(s: &Text)` — `std::io::stderr()`
//!  write + flush; `eprintln` appends `\n`.
//!  * `println_empty()` — `std::io::stdout()` write `\n` + flush.
//!

//! No permission gate — stdin/stdout/stderr are the script's
//! foreground I/O channels, always available. (The Verum stdlib's
//! surface-layer permission gates apply at the higher `using [...]`
//! capability level, not at this intrinsic intercept.)

use super::super::super::error::InterpreterResult;
use super::heap_helpers::{alloc_record_n_fields, extract_text_arg, wrap_in_variant};
use super::string_helpers::alloc_string_value;
use crate::interpreter::state::InterpreterState;
use crate::value::Value;

pub(in super::super) fn try_intercept_stdio_runtime(
    state: &mut InterpreterState,
    func_name: &str,
    args_start_reg: u16,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let bare = func_name.rsplit('.').next().unwrap_or(func_name);
    // Disambiguation: stdio's free-function form takes 0 args; the
    // method-on-Stdin form (`stdin.read_line(&mut buf)`) has different
    // arg count and routes through CallM not Call. Gating on
    // `arg_count == 0` is enough to disambiguate from the method form.
    match bare {
        "read_line" if arg_count == 0 => intercept_read_line(state),
        "read_int" if arg_count == 0 => intercept_read_int(state),
        "read_float" if arg_count == 0 => intercept_read_float(state),
        "read_to_end" if arg_count == 0 && is_stdio_qualified(func_name) => {
            intercept_read_to_end(state)
        }
        // Output: bare-name `print`/`println` are compiled to
        // `DebugPrint` at codegen (with newline), so only mount-imported
        // stdlib forms (`core.io.stdio.{print,println}` /
        // `core.text.format.{print,println}`) reach here as a real
        // Call. Both stdlib definitions are stdout writers — the only
        // ambiguity would be an unrelated user-defined `print` or
        // `println` falling through to the same name, which is
        // vanishingly unlikely in scripts. No qualifier check needed.
        // `eprint`/`eprintln`/`println_empty` aren't builtins so every
        // call arrives here.
        "print" if arg_count == 1 => {
            intercept_write(state, args_start_reg, caller_base, Stream::Stdout, false)
        }
        "println" if arg_count == 1 => {
            intercept_write(state, args_start_reg, caller_base, Stream::Stdout, true)
        }
        "println_empty" if arg_count == 0 => intercept_println_empty(state),
        "eprint" if arg_count == 1 => {
            intercept_write(state, args_start_reg, caller_base, Stream::Stderr, false)
        }
        "eprintln" if arg_count == 1 => {
            intercept_write(state, args_start_reg, caller_base, Stream::Stderr, true)
        }
        _ => Ok(None),
    }
}

fn is_stdio_qualified(func_name: &str) -> bool {
    func_name.contains("io.stdio") || func_name.contains("io::stdio")
}

#[derive(Copy, Clone)]
enum Stream {
    Stdout,
    Stderr,
}

// ============================================================================
// Per-function intercepts
// ============================================================================

fn intercept_read_line(state: &mut InterpreterState) -> InterpreterResult<Option<Value>> {
    use std::io::BufRead;
    let stdin = std::io::stdin();
    let mut handle = stdin.lock();
    let mut line = String::new();
    match handle.read_line(&mut line) {
        Ok(_) => {
            // Strip trailing \n and optional \r (CRLF) — matches the
            // Verum stdlib `read_line` contract at stdio.vr:388-394.
            if line.ends_with('\n') {
                line.pop();
            }
            if line.ends_with('\r') {
                line.pop();
            }
            let text = alloc_string_value(state, &line)?;
            Ok(Some(wrap_in_variant(state, "Result", 0, &[text])?))
        }
        Err(e) => Ok(Some(build_io_err_text(state, &e)?)),
    }
}

fn intercept_read_int(state: &mut InterpreterState) -> InterpreterResult<Option<Value>> {
    let line_result = read_one_line()?;
    match line_result {
        Ok(line) => match line.trim().parse::<i64>() {
            Ok(n) => {
                let v = Value::from_i64(n);
                Ok(Some(wrap_in_variant(state, "Result", 0, &[v])?))
            }
            Err(_) => Ok(Some(build_io_err_kind(state, 12)?)), // InvalidData
        },
        Err(e) => Ok(Some(build_io_err_text(state, &e)?)),
    }
}

fn intercept_read_float(state: &mut InterpreterState) -> InterpreterResult<Option<Value>> {
    let line_result = read_one_line()?;
    match line_result {
        Ok(line) => match line.trim().parse::<f64>() {
            Ok(f) => {
                let v = Value::from_f64(f);
                Ok(Some(wrap_in_variant(state, "Result", 0, &[v])?))
            }
            Err(_) => Ok(Some(build_io_err_kind(state, 12)?)), // InvalidData
        },
        Err(e) => Ok(Some(build_io_err_text(state, &e)?)),
    }
}

fn intercept_read_to_end(state: &mut InterpreterState) -> InterpreterResult<Option<Value>> {
    use std::io::Read;
    let mut buf = String::new();
    match std::io::stdin().read_to_string(&mut buf) {
        Ok(_) => {
            let text = alloc_string_value(state, &buf)?;
            Ok(Some(wrap_in_variant(state, "Result", 0, &[text])?))
        }
        Err(e) => Ok(Some(build_io_err_text(state, &e)?)),
    }
}

/// Write `s` to stdout/stderr, optionally appending `\n`, then flush.
/// The stdlib `print`/`println`/`eprint`/`eprintln` returns `()` so we
/// store `Value::unit()` into the destination register; ignore the
/// `std::io::Write` result to match the stdlib's `let _ = sys_write`
/// contract.
fn intercept_write(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
    stream: Stream,
    with_newline: bool,
) -> InterpreterResult<Option<Value>> {
    use std::io::Write;
    let s = extract_text_arg(state, args_start_reg, caller_base);
    match stream {
        Stream::Stdout => {
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(s.as_bytes());
            if with_newline {
                let _ = out.write_all(b"\n");
            }
            let _ = out.flush();
        }
        Stream::Stderr => {
            let mut err = std::io::stderr().lock();
            let _ = err.write_all(s.as_bytes());
            if with_newline {
                let _ = err.write_all(b"\n");
            }
            let _ = err.flush();
        }
    }
    Ok(Some(Value::unit()))
}

fn intercept_println_empty(_state: &mut InterpreterState) -> InterpreterResult<Option<Value>> {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(b"\n");
    let _ = out.flush();
    Ok(Some(Value::unit()))
}

/// Read one trimmed line from stdin (host-side; reused by read_int /
/// read_float). Returns `Ok(line_no_newline)` or `Err(io_error)`.
fn read_one_line() -> InterpreterResult<Result<String, std::io::Error>> {
    use std::io::BufRead;
    let stdin = std::io::stdin();
    let mut handle = stdin.lock();
    let mut line = String::new();
    match handle.read_line(&mut line) {
        Ok(_) => {
            if line.ends_with('\n') {
                line.pop();
            }
            if line.ends_with('\r') {
                line.pop();
            }
            Ok(Ok(line))
        }
        Err(e) => Ok(Err(e)),
    }
}

// ============================================================================
// Result/StreamError construction (mirror file_runtime.rs)
// ============================================================================

fn build_io_err_text(state: &mut InterpreterState, e: &std::io::Error) -> InterpreterResult<Value> {
    use std::io::ErrorKind as K;
    let kind_tag = match e.kind() {
        K::NotFound => 0u32,
        K::PermissionDenied => 1,
        K::ConnectionRefused => 2,
        K::ConnectionReset => 3,
        K::ConnectionAborted => 4,
        K::NotConnected => 5,
        K::AddrInUse => 6,
        K::AddrNotAvailable => 7,
        K::BrokenPipe => 8,
        K::AlreadyExists => 9,
        K::WouldBlock => 10,
        K::InvalidInput => 11,
        K::InvalidData => 12,
        K::TimedOut => 13,
        K::WriteZero => 14,
        K::Interrupted => 15,
        K::UnexpectedEof => 16,
        K::OutOfMemory => 17,
        K::Unsupported => 18,
        _ => 19,
    };
    let kind_variant = wrap_in_variant(state, "IoErrorKind", kind_tag, &[])?;
    let msg_text = alloc_string_value(state, &format!("{}", e))?;
    let msg_some = wrap_in_variant(state, "Maybe", 1, &[msg_text])?;
    let stream_err = alloc_record_n_fields(state, "StreamError", &[kind_variant, msg_some])?;
    wrap_in_variant(state, "Result", 1, &[stream_err])
}

fn build_io_err_kind(state: &mut InterpreterState, kind_tag: u32) -> InterpreterResult<Value> {
    let kind_variant = wrap_in_variant(state, "IoErrorKind", kind_tag, &[])?;
    let none = wrap_in_variant(state, "Maybe", 0, &[])?;
    let stream_err = alloc_record_n_fields(state, "StreamError", &[kind_variant, none])?;
    wrap_in_variant(state, "Result", 1, &[stream_err])
}

// ============================================================================
