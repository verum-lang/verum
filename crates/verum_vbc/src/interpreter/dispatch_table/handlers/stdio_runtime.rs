//! Host output support for standard streams. Input uses the source stdio
//! declarations so line reads and raw reads consume the same descriptor.

use super::super::super::error::InterpreterResult;
use super::heap_helpers::extract_text_arg;
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
    match bare {
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

#[derive(Copy, Clone)]
enum Stream {
    Stdout,
    Stderr,
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
    let s = extract_text_arg(state, args_start_reg, caller_base)?;
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
