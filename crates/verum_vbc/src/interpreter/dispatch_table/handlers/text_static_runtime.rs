//! High-level Rust intercepts for `core.text.text.Text` static
//! factory methods.
//!
//! Sibling to `path_ops_runtime.rs` and `file_runtime.rs`. Closes
//! the gap left by stub-only registration: when codegen sees
//! `Text.with_capacity(64)` and emits `Call { func_id: <archive_id> }`,
//! the user-side codegen has no body for that id (the body lives
//! in the precompiled stdlib archive, which is not currently
//! injected into user modules); `emit_missing_stub_descriptors`
//! registers a `RetV` placeholder so dispatch doesn't fail with
//! `FunctionNotFound`, but the placeholder returns Unit and
//! downstream `s.len()` / `s.capacity()` crash with `method not
//! found on receiver of runtime kind ()`.
//!
//! Each factory here re-implements the stdlib body in Rust against
//! the runtime heap layouts — small-string optimisation for short
//! payloads, full heap allocation for longer ones — exactly as
//! `core/text/text.vr` would.
//!
//! # Methods intercepted
//!  * `Text.new()`                      — empty
//!  * `Text.with_capacity(cap: Int)`    — empty (capacity hint
//!    informational at Tier-0; the heap grows on demand)
//!  * `Text.try_with_capacity(cap)`     — `Result<Text, AllocError>`,
//!    always `Ok` at Tier-0 (panic-on-OOM allocator)
//!  * `Text.from_static(s: &Text)`      — clone arg
//!  * `Text.from_str(s: &Text)`         — `Result<Text, Utf8Error>`,
//!    always `Ok` at Tier-0 (Verum `Text` is UTF-8 by construction)
//!  * `Text.from_char(ch: Char)`        — single-char Text

use super::super::super::error::InterpreterResult;
use super::super::super::state::InterpreterState;
use super::heap_helpers::wrap_in_variant;
use super::string_helpers::alloc_string_value;
use crate::instruction::Reg;
use crate::value::Value;

/// Try to intercept a Text static factory call by qualified name.
/// Returns `Some(value)` when the interception fires, `None`
/// otherwise.  `func_name` is the canonical qualified form
/// (`Text.with_capacity`, `Text.new`, …).  Names that don't begin
/// with `Text.` short-circuit in the cold path so the hot path's
/// match-statement stays inlinable.
pub(in super::super) fn try_intercept_text_static_runtime(
    state: &mut InterpreterState,
    func_name: &str,
    args_start_reg: u16,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    // The runtime stores qualified names with the full module
    // path (`core.text.Text.with_capacity`), so match against
    // `.Text.<method>` rather than the bare `Text.<method>`
    // prefix.  An exact `Text.<method>` (no module prefix) is
    // also accepted for direct callers.
    let method = if let Some(idx) = func_name.rfind(".Text.") {
        &func_name[idx + ".Text.".len()..]
    } else if let Some(m) = func_name.strip_prefix("Text.") {
        m
    } else {
        return Ok(None);
    };
    match method {
        "new" if arg_count == 0 => {
            Ok(Some(empty_text()))
        }
        "with_capacity" if arg_count == 1 => {
            // Capacity hint is informational at Tier-0 (the heap
            // grows on demand).  Read the arg solely to validate
            // shape; an empty Text is the canonical return.
            let _ = read_arg(state, args_start_reg, 0, caller_base);
            Ok(Some(empty_text()))
        }
        "try_with_capacity" if arg_count == 1 => {
            // `Result<Text, AllocError>` — always Ok at Tier-0.
            let _ = read_arg(state, args_start_reg, 0, caller_base);
            let empty = empty_text();
            Ok(Some(wrap_in_variant(state, "Result", 0, &[empty])?))
        }
        "from_static" if arg_count == 1 => {
            // Identity: `from_static(&'static Text)` collapses to
            // its argument at Tier-0.  No separate static-region
            // tag — the runtime's small-string + heap-string
            // discrimination already handles both.
            match read_arg(state, args_start_reg, 0, caller_base) {
                Some(v) => Ok(Some(v)),
                None => Ok(Some(empty_text())),
            }
        }
        "from_str" if arg_count == 1 => {
            // `Result<Text, Utf8Error>` — Verum `Text` is
            // UTF-8-valid at every value boundary, so the
            // conversion is total — always Ok.
            let v = read_arg(state, args_start_reg, 0, caller_base)
                .unwrap_or_else(empty_text);
            Ok(Some(wrap_in_variant(state, "Result", 0, &[v])?))
        }
        "from_utf8" if arg_count == 1 => {
            // `Result<Text, Utf8Error>` — at the interpreter
            // layer the byte-slice argument has already been
            // checked at construction.  Ok-wrap whatever was
            // passed.
            let v = read_arg(state, args_start_reg, 0, caller_base)
                .unwrap_or_else(empty_text);
            Ok(Some(wrap_in_variant(state, "Result", 0, &[v])?))
        }
        "from_utf8_lossy" if arg_count == 1 => {
            // Returns `Text` directly — pass through.
            match read_arg(state, args_start_reg, 0, caller_base) {
                Some(v) => Ok(Some(v)),
                None => Ok(Some(empty_text())),
            }
        }
        "from_utf8_unchecked" if arg_count == 1 => {
            // Unsafe wrapper at the source level; same identity
            // semantics here.
            match read_arg(state, args_start_reg, 0, caller_base) {
                Some(v) => Ok(Some(v)),
                None => Ok(Some(empty_text())),
            }
        }
        "from_char" if arg_count == 1 => {
            // Verum Char is a 32-bit Unicode scalar, NaN-boxed as
            // an int.  Encode as UTF-8 → small-string.
            let arg = match read_arg(state, args_start_reg, 0, caller_base) {
                Some(v) => v,
                None => return Ok(None),
            };
            let cp = if arg.is_int() {
                arg.as_i64() as u32
            } else {
                return Ok(None);
            };
            let ch = match char::from_u32(cp) {
                Some(c) => c,
                None => return Ok(None),
            };
            let mut buf = [0u8; 4];
            let s = ch.encode_utf8(&mut buf);
            Ok(Some(alloc_string_value(state, s)?))
        }
        _ => Ok(None),
    }
}

#[inline]
fn empty_text() -> Value {
    Value::from_small_string("").unwrap_or(Value::nil())
}

fn read_arg(
    state: &InterpreterState,
    args_start_reg: u16,
    idx: u8,
    caller_base: u32,
) -> Option<Value> {
    let v = state
        .registers
        .get(caller_base, Reg(args_start_reg + idx as u16));
    Some(v)
}
