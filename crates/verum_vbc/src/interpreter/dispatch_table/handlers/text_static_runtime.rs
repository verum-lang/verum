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
use super::heap_helpers::{extract_byte_slice, wrap_in_variant};
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
            // `Text.from_utf8(bytes: &[Byte]) -> Result<Text, Utf8Error>`.
            //
            // **Pre-fix**: this intercept Ok-wrapped the bytes value AS-IS,
            // returning `Result<List<Byte>>` — NOT `Result<Text>`. Tests
            // that asserted `t == "Hi"` failed because the unwrapped value
            // was a List<Byte>, not a Text. UTF-8 validation never ran.
            //
            // **Fix**: extract bytes via the canonical `extract_byte_slice`
            // helper (handles FatRef + LIST + BYTE_LIST shapes), validate
            // UTF-8 with Rust's std::str::from_utf8, allocate a real Text
            // from the validated bytes, and wrap in `Result.Ok`. Invalid
            // UTF-8 → `Result.Err(Utf8Error { valid_up_to })`.
            let bytes = extract_byte_slice(state, args_start_reg, caller_base);
            match std::str::from_utf8(&bytes) {
                Ok(s) => {
                    let text = alloc_string_value(state, s)?;
                    Ok(Some(wrap_in_variant(state, "Result", 0, &[text])?))
                }
                Err(e) => {
                    let valid_up_to = Value::from_i64(e.valid_up_to() as i64);
                    let utf8_err = wrap_in_variant(state, "Utf8Error", 0, &[valid_up_to])?;
                    Ok(Some(wrap_in_variant(state, "Result", 1, &[utf8_err])?))
                }
            }
        }
        "from_utf8_lossy" if arg_count == 1 => {
            // `Text.from_utf8_lossy(bytes: &[Byte]) -> Text`.
            //
            // Returns a Text where invalid UTF-8 sequences are replaced
            // with U+FFFD. Pre-fix this returned the raw bytes value; now
            // properly converts via `String::from_utf8_lossy`.
            let bytes = extract_byte_slice(state, args_start_reg, caller_base);
            let lossy = String::from_utf8_lossy(&bytes);
            Ok(Some(alloc_string_value(state, &lossy)?))
        }
        "from_utf8_unchecked" if arg_count == 1 => {
            // `unsafe Text.from_utf8_unchecked(bytes: &[Byte]) -> Text`.
            //
            // Caller asserts the bytes are already valid UTF-8. Convert
            // without validation — but DO convert the bytes to an actual
            // Text rather than passing through the List value (the
            // pre-fix behaviour). Use lossy decoding at runtime to
            // maintain memory-safety even when the caller contract is
            // violated.
            let bytes = extract_byte_slice(state, args_start_reg, caller_base);
            let s = String::from_utf8_lossy(&bytes);
            Ok(Some(alloc_string_value(state, &s)?))
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
        "from_int" if arg_count == 1 => {
            // `Text.from_int(n: Int) -> Text`. Decimal-render an Int.
            // Bypasses the `int_to_text` intrinsic / user-side body
            // (which can suffer function-id collision under archive
            // remap). Idempotent under round-trip with `parse_int`.
            let arg = match read_arg(state, args_start_reg, 0, caller_base) {
                Some(v) => v,
                None => return Ok(None),
            };
            if !arg.is_int() {
                return Ok(None);
            }
            let n = arg.as_i64();
            Ok(Some(alloc_string_value(state, &n.to_string())?))
        }
        "from_float" if arg_count == 1 => {
            // `Text.from_float(f: Float) -> Text`. Render a Float in
            // its canonical form (matches Rust's `f64::to_string` —
            // shortest round-trippable). Pre-fix the user-side body
            // depended on the `float_to_text` intrinsic which had its
            // own dispatch issues.
            let arg = match read_arg(state, args_start_reg, 0, caller_base) {
                Some(v) => v,
                None => return Ok(None),
            };
            let f = if arg.is_float() {
                arg.as_f64()
            } else if arg.is_int() {
                arg.as_i64() as f64
            } else {
                return Ok(None);
            };
            Ok(Some(alloc_string_value(state, &f.to_string())?))
        }
        "join" if arg_count == 2 => {
            // `Text.join(parts: &[Text], sep: &Text) -> Text`.
            //
            // Static method that concatenates `parts` separated by `sep`.
            // The user-side body iterates parts with for-loop + indexing
            // and chains push_str — works in principle now that the
            // push_str / iterator fixes have landed, but susceptible to
            // List<Text> ↔ &[Text] dispatch quirks. The Tier-0 intercept
            // bypasses every intermediate step: extract each Text element,
            // collect into a Rust Vec<String>, run `Vec.join(&sep)`,
            // alloc the result.
            use super::super::super::heap;
            use super::cbgr_helpers::{decode_cbgr_ref, is_cbgr_ref};

            let parts_val_raw = read_arg(state, args_start_reg, 0, caller_base)
                .unwrap_or_else(empty_text);
            let sep_val_raw = read_arg(state, args_start_reg, 1, caller_base)
                .unwrap_or_else(empty_text);

            // Auto-deref CBGR-ref / ThinRef (parts is `&[Text]` /
            // `&List<Text>`; sep is `&Text`).
            let unwrap = |mut v: Value| -> Value {
                if is_cbgr_ref(&v) {
                    let (abs_index, _) = decode_cbgr_ref(v.as_i64());
                    v = state.registers.get_absolute(abs_index);
                }
                if v.is_thin_ref() {
                    let tr = v.as_thin_ref();
                    if !tr.ptr.is_null() {
                        v = unsafe { *(tr.ptr as *const Value) };
                    }
                }
                v
            };
            let parts_val = unwrap(parts_val_raw);
            let sep_val = unwrap(sep_val_raw);

            let sep_str = super::string_helpers::extract_string(&sep_val, state);

            // Recover the parts: List<Text> heap layout
            // `[ObjectHeader][len:i64][cap:i64][data_ptr]` where
            // data_ptr points to backing array of Text-shaped Values.
            let mut texts: Vec<String> = Vec::new();
            if parts_val.is_fat_ref() {
                let fr = parts_val.as_fat_ref();
                let p = fr.ptr();
                let len = fr.len() as usize;
                if !p.is_null() && len > 0 && len <= 1_000_000 {
                    for i in 0..len {
                        let elem = unsafe { *(p as *const Value).add(i) };
                        texts.push(super::string_helpers::extract_string(&elem, state));
                    }
                }
            } else if parts_val.is_ptr() && !parts_val.is_nil() {
                let base = parts_val.as_ptr::<u8>();
                if !base.is_null()
                    && (base as usize)
                        .is_multiple_of(std::mem::align_of::<heap::ObjectHeader>())
                {
                    let after_header = unsafe {
                        base.add(std::mem::size_of::<heap::ObjectHeader>())
                    };
                    let len = unsafe { *(after_header as *const i64) };
                    if (0..=1_000_000).contains(&len) {
                        let len = len as usize;
                        if len > 0 {
                            let data_ptr = unsafe {
                                *(after_header.add(16) as *const *const Value)
                            };
                            if !data_ptr.is_null() {
                                for i in 0..len {
                                    let elem = unsafe { *data_ptr.add(i) };
                                    texts.push(super::string_helpers::extract_string(
                                        &elem, state,
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            let joined = texts.join(&sep_str);
            Ok(Some(alloc_string_value(state, &joined)?))
        }
        "from_bool" if arg_count == 1 => {
            // `Text.from_bool(b: Bool) -> Text`. Returns "true" /
            // "false". Pure data-shape conversion; no allocation
            // beyond the small-string for either literal.
            let arg = match read_arg(state, args_start_reg, 0, caller_base) {
                Some(v) => v,
                None => return Ok(None),
            };
            let b = if arg.is_bool() {
                arg.as_bool()
            } else if arg.is_int() {
                arg.as_i64() != 0
            } else {
                return Ok(None);
            };
            Ok(Some(alloc_string_value(state, if b { "true" } else { "false" })?))
        }
        _ => Ok(None),
    }
}

#[inline]
fn empty_text() -> Value {
    Value::from_small_string("").unwrap_or(Value::nil())
}

// `extract_byte_slice` lives in `heap_helpers` — it's the canonical
// helper used by every `&[Byte]` consumer (regex / file / shell /
// network / Text intercepts). Reused here rather than duplicated.

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
