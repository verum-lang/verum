//! High-level Rust intercepts for `core.text.text.Text` static
//! factory methods.
//!
//! Capacity constructors and ownership copies execute their stdlib bodies.

use super::super::super::error::InterpreterResult;
use super::super::super::heap;
use super::super::super::state::InterpreterState;
use super::cbgr_helpers::{resolve_arg_value, resolve_receiver_place};
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
            use super::string_helpers::extract_text_bytes;
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
            let receiver = resolve_receiver_place(state, self_raw)?;
            receiver.require_write()?;
            let (current_val, target) = (receiver.value, receiver.reference);
            // Byte-exact extraction (a reserve may land mid-code-point
            // during byte-wise construction).
            let bytes = extract_text_bytes(&current_val, state);
            let current_len = bytes.len() as i64;
            let (_, current_cap) = super::text_storage::extents(state, current_val)?;
            let current_cap = current_cap as i64;
            let needed_cap = current_len + additional;
            let new_cap = if needed_cap > current_cap { needed_cap } else { current_cap };
            // ONE canonical capacity-carrying TEXT record carrying the
            // existing bytes in place (ARCH-P5 final leg — the former
            // builder stored field0 = a pointer to a separate
            // heap-string OBJECT header, which record readers misread
            // as a bytes pointer).
            let new_val = alloc_text_capacity_record(state, &bytes, new_cap)?;
            if !super::text_storage::replace(state, current_val, target, new_val)? {
                state.registers.set(caller_base, Reg(args_start_reg), new_val);
            }
            Ok(Some(Value::unit()))
        }
        "from_str" if arg_count == 1 => {
            // `Result<Text, Utf8Error>` — Verum `Text` is
            // UTF-8-valid at every value boundary, so the
            // conversion is total — always Ok.
            let v = read_arg(state, args_start_reg, 0, caller_base)
                .unwrap_or_else(empty_text);
            let bytes = super::string_helpers::extract_text_bytes(&v, state);
            let value = super::string_helpers::alloc_text_bytes_value(state, &bytes)?;
            Ok(Some(wrap_in_variant(state, "Result", 0, &[value])?))
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
            let bytes = extract_byte_slice(state, args_start_reg, caller_base)?;
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

            let parts_val_raw = read_arg(state, args_start_reg, 0, caller_base)
                .unwrap_or_else(empty_text);
            let sep_val_raw = read_arg(state, args_start_reg, 1, caller_base)
                .unwrap_or_else(empty_text);

            // Auto-deref CBGR-ref / ThinRef (parts is `&[Text]` /
            // `&List<Text>`; sep is `&Text`).

            let parts_val = super::heap_helpers::unwrap_ref(state, parts_val_raw)?;
            let sep_val = super::heap_helpers::unwrap_ref(state, sep_val_raw)?;

            let sep_str = super::string_helpers::extract_string(&sep_val, state);

            // Recover the parts: List<Text> heap layout
            // `[ObjectHeader][len: Value(i64)][cap: Value(i64)][backing: Value(ptr)]`
            // where `backing` is a Value holding a pointer to an
            // ObjectHeader-prefixed array of Text-shaped Values.
            // Element data starts at `backing_ptr + OBJECT_HEADER_SIZE`.
            // See `method_dispatch.rs::list_push` for the canonical
            // layout reference.
            //
            // Pre-fix the intercept read field 0 as a raw `i64` (skipping
            // the NaN-box unwrap) and field 2 as a direct element-data
            // pointer (skipping the backing-array's ObjectHeader),
            // causing every non-empty join to return empty.
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
                    let data_ptr = unsafe {
                        base.add(heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    // Field 0: len (NaN-boxed Value, not raw i64).
                    let len_val = unsafe { *data_ptr };
                    if len_val.is_int() {
                        let len = len_val.as_i64();
                        if (0..=1_000_000).contains(&len) {
                            let len = len as usize;
                            // Field 2: backing-array pointer (Value).
                            let backing_val = unsafe { *data_ptr.add(2) };
                            if backing_val.is_ptr() && !backing_val.is_nil() && len > 0 {
                                let backing_ptr = backing_val.as_ptr::<u8>();
                                if !backing_ptr.is_null() {
                                    // Element data starts after the
                                    // backing-array's own ObjectHeader.
                                    let elem_data = unsafe {
                                        backing_ptr.add(heap::OBJECT_HEADER_SIZE)
                                            as *const Value
                                    };
                                    for i in 0..len {
                                        let elem = unsafe { *elem_data.add(i) };
                                        texts.push(
                                            super::string_helpers::extract_string(
                                                &elem, state,
                                            ),
                                        );
                                    }
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
            use super::string_helpers::{alloc_text_bytes_value, extract_string, extract_text_bytes};
            let self_raw = state
                .registers
                .get(caller_base, Reg(args_start_reg));
            let arg_raw = state
                .registers
                .get(caller_base, Reg(args_start_reg + 1));
            let receiver = resolve_receiver_place(state, self_raw)?;
            receiver.require_write()?;
            let (self_val, target) = (receiver.value, receiver.reference);
            let arg_val = resolve_arg_value(state, arg_raw)?;
            // BYTE-domain round trip (SSO-UTF8-SPLIT-1): `push_byte`
            // builds multi-byte UTF-8 sequences one byte at a time
            // (stdlib `to_lowercase` / `to_uppercase` / `slice` /
            // `concat` bodies all lower to per-byte `push_byte`
            // loops), so between two calls the receiver legitimately
            // holds a PARTIAL code point (e.g. just `0xCE`, the lead
            // byte of `'Ω'`). The previous String round trip
            // (`extract_string` → `as_mut_vec().push` →
            // `alloc_string_value`) SSO-packed that invalid
            // intermediate into the NaN box — `SmallString::as_str`
            // panics on it in debug and silently drops the pending
            // bytes in release (mojibake). Accumulate in `Vec<u8>` and
            // let `alloc_text_bytes_value` widen not-yet-valid
            // intermediates to the byte-transparent heap blob.
            let mut current_bytes = extract_text_bytes(&self_val, state);
            match method {
                "push_byte" => {
                    if !arg_val.is_int() {
                        return Ok(None);
                    }
                    let byte = (arg_val.as_i64() as u32 & 0xFF) as u8;
                    // Raw byte append — caller-responsible EVENTUAL
                    // UTF-8 validity (mirrors stdlib `Text.push_byte`
                    // contract); mid-sequence states stay byte-exact.
                    current_bytes.push(byte);
                }
                "push_str" => {
                    let s = extract_string(&arg_val, state);
                    current_bytes.extend_from_slice(s.as_bytes());
                }
                "push" | "push_char" => {
                    if arg_val.is_int() {
                        if let Some(ch) = char::from_u32(arg_val.as_i64() as u32) {
                            let mut buf = [0u8; 4];
                            current_bytes
                                .extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        } else {
                            return Ok(None);
                        }
                    } else if arg_val.is_small_string() {
                        // Single-char Text arg → append its bytes.
                        current_bytes
                            .extend_from_slice(arg_val.as_small_string().as_bytes());
                    } else {
                        return Ok(None);
                    }
                }
                _ => unreachable!(),
            }
            let new_value = alloc_text_bytes_value(state, &current_bytes)?;
            if !super::text_storage::replace(state, self_val, target, new_value)? {
                state.registers.set(caller_base, Reg(args_start_reg), new_value);
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

/// Allocate the canonical capacity-carrying TEXT record for the
/// `reserve` operation
/// (ARCH-P5 final leg): ONE self-contained
/// `[ObjectHeader(TEXT)]{ptr, len, cap}[storage…]` allocation via
/// `Heap::alloc_text_with_capacity`, with the bytes carried IN PLACE.
///
/// A non-positive capacity request with no bytes degrades to the
/// canonical immutable empty record (`cap == 0`), preserving the
/// observable `capacity() == 0` of the former nil-ptr builder.
fn alloc_text_capacity_record(
    state: &mut InterpreterState,
    bytes: &[u8],
    cap: i64,
) -> InterpreterResult<Value> {
    let obj = if cap <= 0 && bytes.is_empty() {
        state.heap.alloc_text(&[])?
    } else {
        state
            .heap
            .alloc_text_with_capacity(bytes, cap.max(0) as usize)?
    };
    state.record_allocation();
    Ok(Value::from_ptr(obj.as_ptr() as *mut u8))
}
