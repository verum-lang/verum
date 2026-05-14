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
use super::super::super::heap;
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
            // Allocate a builder-layout Text `{ptr, len, cap}` so that
            // the `cap` field survives to subsequent `capacity()` /
            // `push_*` calls.  Pre-fix this returned an empty small-
            // string, silently discarding the capacity argument — see
            // `core-tests/text/text/regression_test.vr::regression_with_capacity_*`.
            let cap = match read_arg(state, args_start_reg, 0, caller_base) {
                Some(v) if v.is_int() => v.as_i64(),
                _ => 0,
            };
            Ok(Some(alloc_text_builder(state, cap)?))
        }
        "try_with_capacity" if arg_count == 1 => {
            // `Result<Text, AllocError>` — always Ok at Tier-0 (the
            // host allocator panics on OOM rather than returning Err).
            // Same builder-layout allocation as `with_capacity`.
            let cap = match read_arg(state, args_start_reg, 0, caller_base) {
                Some(v) if v.is_int() => v.as_i64(),
                _ => 0,
            };
            let val = alloc_text_builder(state, cap)?;
            Ok(Some(wrap_in_variant(state, "Result", 0, &[val])?))
        }
        // Instance-method intercept: `Text.capacity(&self) -> Int`.
        //
        // The user-side body calls `text_byte_len(self)` which loses
        // the `cap` field on builder-layout Text values (the Tier-0
        // `TextExtended::ByteLen` handler only recognises small-string
        // values; non-small returns 0).  This intercept dispatches
        // directly on the runtime representation:
        //
        //   * small-string (NaN-boxed inline):  capacity == byte_len
        //   * FatRef Text (immutable byte view): capacity == byte_len
        //   * heap-string `[hdr][len:u64][bytes…]` (flat, immutable):
        //                                        capacity == byte_len
        //   * builder layout `[hdr]{ptr,len,cap}` (24-byte payload):
        //                                        capacity == field2 (cap)
        //
        // Pinned by `core-tests/text/text/regression_test.vr::
        // regression_with_capacity_reports_capacity` and siblings.
        "capacity" if arg_count == 1 => {
            use super::cbgr_helpers::{decode_cbgr_ref, is_cbgr_ref};
            let raw = state
                .registers
                .get(caller_base, Reg(args_start_reg));
            let v = if is_cbgr_ref(&raw) {
                let (abs_index, _) = decode_cbgr_ref(raw.as_i64());
                state.registers.get_absolute(abs_index)
            } else if raw.is_thin_ref() {
                let tr = raw.as_thin_ref();
                if !tr.ptr.is_null() {
                    unsafe { *(tr.ptr as *const Value) }
                } else {
                    raw
                }
            } else {
                raw
            };
            Ok(Some(Value::from_i64(text_capacity_value(&v))))
        }
        // Instance-method intercept: `Text.reserve(&mut self, additional: Int)`.
        //
        // The stdlib body grows the underlying buffer in-place via
        // `Text.grow`, but `Text.new()` returns a small-string that has
        // no heap object to mutate.  Migrating from small → builder
        // layout requires writing back through the receiver's CBGR ref
        // (same writeback discipline as `push_str` / `push` / `push_byte`).
        //
        // After the intercept, `self` holds a builder-layout heap object
        // with `len = previous_len`, `cap = max(old_cap, len + additional)`.
        "reserve" if arg_count == 2 => {
            use super::cbgr_helpers::{decode_cbgr_ref, is_cbgr_ref};
            use super::string_helpers::extract_string;
            let self_raw = state
                .registers
                .get(caller_base, Reg(args_start_reg));
            let add_raw = state
                .registers
                .get(caller_base, Reg(args_start_reg + 1));
            let additional = if add_raw.is_int() { add_raw.as_i64() } else { 0 };
            if additional <= 0 {
                return Ok(Some(Value::unit()));
            }
            // Decode receiver — may be a CBGR ref pointing into the
            // caller's register window, OR a ThinRef pointing into
            // some other register slot.  We need the absolute register
            // to write the new value back into.
            let (writeback_abs, current_val) = if is_cbgr_ref(&self_raw) {
                let (abs, _) = decode_cbgr_ref(self_raw.as_i64());
                (Some(abs), state.registers.get_absolute(abs))
            } else if self_raw.is_thin_ref() {
                let tr = self_raw.as_thin_ref();
                let v = if !tr.ptr.is_null() {
                    unsafe { *(tr.ptr as *const Value) }
                } else {
                    self_raw
                };
                (None, v)
            } else {
                (None, self_raw)
            };
            let bytes = extract_string(&current_val, state);
            let current_len = bytes.len() as i64;
            let current_cap = text_capacity_value(&current_val);
            let needed_cap = current_len + additional;
            let new_cap = if needed_cap > current_cap { needed_cap } else { current_cap };
            // Allocate a new builder layout and carry the existing
            // bytes across.  Field0 holds a fresh heap-string Value
            // carrying the bytes; field1 = len; field2 = cap.
            let bytes_val = if bytes.is_empty() {
                Value::nil()
            } else {
                alloc_string_value(state, &bytes)?
            };
            let new_val = alloc_text_builder_with_bytes(
                state,
                bytes_val,
                current_len,
                new_cap,
            )?;
            // Write back through the CBGR ref so the caller sees the
            // new representation.  Without writeback the receiver
            // register still points at the old small-string value.
            if let Some(abs) = writeback_abs {
                state.registers.set_absolute(abs, new_val);
            }
            Ok(Some(Value::unit()))
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
        // Instance-method intercept: `Text.push_byte(&mut self, b: Byte)`.
        //
        // **Why this can't go through the stdlib body**: at Tier-0,
        // `Text.new()` returns a small-string Value (NaN-boxed inline,
        // 6-byte payload). The stdlib `Text.push_byte` body assumes
        // self is a heap-allocated 3-field record `{ ptr, len, cap }`,
        // calls `self.grow()` which does `self.ptr = alloc(...)` via
        // SetF on field 0. SetF on a small-string Value null-derefs.
        //
        // Codegen lowers `s.push_byte(b)` to `Call(push_byte_fid)` with
        // args [self_cbgr_ref, byte]. We intercept here BEFORE the
        // body executes: extract the current text (small or heap),
        // append the byte, alloc a new value, write back to the
        // receiver register via the CBGR ref. Same writeback discipline
        // as the `push_str` / `push` / `push_char` intercept in
        // `method_dispatch.rs::handle_call_method` (line ~531) — except
        // those fire from CallM dispatch while push_byte arrives via
        // Call dispatch with self as arg[0].
        "push_byte" | "push_str" | "push" | "push_char" if arg_count == 2 => {
            use super::cbgr_helpers::{decode_cbgr_ref, is_cbgr_ref};
            use super::string_helpers::extract_string;
            let self_raw = state
                .registers
                .get(caller_base, Reg(args_start_reg));
            let arg_raw = state
                .registers
                .get(caller_base, Reg(args_start_reg + 1));
            // Auto-deref CBGR ref / ThinRef for both receiver and arg.
            let deref = |v: Value, st: &InterpreterState| -> Value {
                if is_cbgr_ref(&v) {
                    let (abs_index, _) = decode_cbgr_ref(v.as_i64());
                    st.registers.get_absolute(abs_index)
                } else if v.is_thin_ref() {
                    let tr = v.as_thin_ref();
                    if !tr.ptr.is_null() {
                        unsafe { *(tr.ptr as *const Value) }
                    } else {
                        v
                    }
                } else {
                    v
                }
            };
            let self_val = deref(self_raw, state);
            let arg_val = deref(arg_raw, state);
            let mut current_text = extract_string(&self_val, state);
            match method {
                "push_byte" => {
                    if !arg_val.is_int() {
                        return Ok(None);
                    }
                    let byte = (arg_val.as_i64() as u32 & 0xFF) as u8;
                    // SAFETY: caller-responsible UTF-8 validity (mirrors
                    // stdlib `Text.push_byte` contract).
                    unsafe { current_text.as_mut_vec().push(byte); }
                }
                "push_str" => {
                    let s = extract_string(&arg_val, state);
                    current_text.push_str(&s);
                }
                "push" | "push_char" => {
                    if arg_val.is_int() {
                        if let Some(ch) = char::from_u32(arg_val.as_i64() as u32) {
                            current_text.push(ch);
                        } else {
                            return Ok(None);
                        }
                    } else if arg_val.is_small_string() {
                        // Single-char Text arg → append as substring.
                        let s = extract_string(&arg_val, state);
                        current_text.push_str(&s);
                    } else {
                        return Ok(None);
                    }
                }
                _ => unreachable!(),
            }
            let new_value = alloc_string_value(state, &current_text)?;
            // Writeback via CBGR ref to the caller-frame slot — same
            // discipline as `handle_call_method`'s push_str intercept.
            // Without this, the mutation is local to the intercept and
            // the caller's variable retains the pre-call value.
            if is_cbgr_ref(&self_raw) {
                let (abs_index, _) = decode_cbgr_ref(self_raw.as_i64());
                state.registers.set_absolute(abs_index, new_value);
            } else {
                state
                    .registers
                    .set(caller_base, Reg(args_start_reg), new_value);
            }
            Ok(Some(Value::unit()))
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

/// Allocate a builder-layout Text `[ObjectHeader]{ptr,len,cap}` with
/// the given capacity hint, zero bytes written so far, and a null
/// `ptr` slot.  This is the canonical empty-with-capacity shape that
/// `Text.with_capacity` / `try_with_capacity` must produce so the
/// `cap` field survives to subsequent method dispatches.
///
/// Layout disambiguation: field1 (`len`) is stored as
/// `Value::from_i64(0)` so the `AsBytes` / `extract_string` handlers'
/// "field1 is Int → builder layout" branch fires; field2 (`cap`)
/// holds the requested capacity.  Field0 is `Value::nil()` (no
/// allocated byte buffer yet — the buffer is allocated lazily on the
/// first `push_*` that exceeds the small-string inline budget).
#[inline]
fn alloc_text_builder(
    state: &mut InterpreterState,
    cap: i64,
) -> InterpreterResult<Value> {
    alloc_text_builder_with_bytes(state, Value::nil(), 0, cap)
}

/// Allocate a builder-layout Text with an existing byte payload.  Used
/// by `reserve` to migrate a small-string value to the builder
/// representation while preserving the existing bytes.  `bytes_val`
/// is the Value carrying the bytes (typically a heap-string Value
/// produced by `alloc_string_value`), `len` is the byte count, and
/// `cap` is the new capacity (>= len).
fn alloc_text_builder_with_bytes(
    state: &mut InterpreterState,
    bytes_val: Value,
    len: i64,
    cap: i64,
) -> InterpreterResult<Value> {
    // 24-byte payload: three Value-sized slots.  Use `TypeId::TEXT` so
    // every downstream Text-classifier (`is_heap_string`, the AsBytes
    // handler's builder-layout branch, `extract_string`, …) recognises
    // the allocation as Text-shaped.
    let obj = state.heap.alloc(crate::types::TypeId::TEXT, 24)?;
    state.record_allocation();
    let base = obj.as_ptr() as *mut u8;
    unsafe {
        let data_ptr = base.add(heap::OBJECT_HEADER_SIZE) as *mut Value;
        *data_ptr = bytes_val;
        *data_ptr.add(1) = Value::from_i64(len);
        *data_ptr.add(2) = Value::from_i64(cap);
    }
    Ok(Value::from_ptr(base))
}

/// Read the capacity of a Text Value, dispatching by representation.
/// Returns the byte budget the buffer can hold without reallocating.
///
///   * small-string (NaN-boxed inline): byte_len
///   * FatRef Text (immutable byte view): byte_len
///   * heap-string flat layout `[hdr][len:u64][bytes…]`: byte_len
///   * builder layout `[hdr]{ptr,len,cap}` (24-byte payload): cap (field2)
fn text_capacity_value(v: &Value) -> i64 {
    if v.is_small_string() {
        return v.as_small_string().len() as i64;
    }
    if v.is_fat_ref() {
        return v.as_fat_ref().len() as i64;
    }
    if !v.is_ptr() || v.is_nil() {
        return 0;
    }
    let base = v.as_ptr::<u8>();
    if base.is_null()
        || !(base as usize).is_multiple_of(std::mem::align_of::<heap::ObjectHeader>())
    {
        return 0;
    }
    let header = unsafe { heap::ObjectHeader::ref_or_stub(base) };
    if header.type_id != crate::types::TypeId::TEXT
        && header.type_id != crate::types::TypeId(0x0001)
    {
        return 0;
    }
    let data_ptr = unsafe { base.add(heap::OBJECT_HEADER_SIZE) };
    let header_size = header.size as usize;
    if header_size == 24 {
        // Builder layout: disambiguate via field1 (Int → builder, else
        // → 16-byte heap-string).  Builder's field2 holds cap.
        let field1 = unsafe { *(data_ptr as *const Value).add(1) };
        if field1.is_int() {
            let field2 = unsafe { *(data_ptr as *const Value).add(2) };
            if field2.is_int() {
                return field2.as_i64();
            }
            return unsafe { *(data_ptr as *const u64).add(2) } as i64;
        }
        // 16-byte heap-string layout: `[len:u64][bytes…]`, no cap to
        // report — capacity == byte_len.
        let len_ptr = data_ptr as *const u64;
        return unsafe { *len_ptr } as i64;
    }
    // Flat heap-string layout: `[len:u64][bytes…]`, no cap, immutable.
    let len_ptr = data_ptr as *const u64;
    unsafe { *len_ptr as i64 }
}
