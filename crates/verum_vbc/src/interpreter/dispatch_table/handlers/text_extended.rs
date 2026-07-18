//! Text extended opcode handler for VBC interpreter dispatch.

use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::*;
use crate::value::Value;

/// TextExtended (0x79) - Text parsing and conversion operations.
///

/// Format: `[0x79] [sub_opcode:u8] [operands...]`
///

/// Sub-opcodes:
/// - 0x00: FromStatic - Create Text from static string data
/// - 0x10: ParseInt - Parse integer from Text
/// - 0x11: ParseFloat - Parse float from Text
/// - 0x20: IntToText - Convert integer to Text
/// - 0x21: FloatToText - Convert float to Text
/// - 0x30: ByteLen - Get Text length in bytes
/// - 0x31: CharLen - Get Text length in characters
/// - 0x32: IsEmpty - Check if Text is empty
/// - 0x33: IsUtf8 - Check if Text is valid UTF-8
///

/// Performance: ~2ns dispatch via Rust match (vs ~15ns for LibraryCall)
pub(in super::super) fn handle_text_extended(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    use crate::instruction::TextSubOpcode;

    let sub_op_byte = read_u8(state)?;
    // Skip operand-length varint (see encode_instruction's
    // `Instruction::TextExtended` arm).
    let _operand_len = read_varint(state)?;
    let sub_op = TextSubOpcode::from_byte(sub_op_byte);
    let dst = read_reg(state)?;

    match sub_op {
        Some(TextSubOpcode::FromStatic) => {
            // Create Text from static string data
            // Args: ptr:reg, len:reg
            let ptr_reg = read_reg(state)?;
            let len_reg = read_reg(state)?;
            let ptr = state.get_reg(ptr_reg).as_i64() as *const u8;
            let len = state.get_reg(len_reg).as_i64() as usize;

            // Create Text from the static data
            // For small strings (up to 6 chars), use small string optimization
            if len <= 6 {
                // SAFETY: We trust the static string data to be valid UTF-8
                let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
                if let Ok(s) = std::str::from_utf8(slice) {
                    if let Some(small) = Value::from_small_string(s) {
                        state.set_reg(dst, small);
                    } else {
                        // Fallback to empty string
                        state.set_reg(dst, Value::from_small_string("").unwrap_or(Value::nil()));
                    }
                } else {
                    state.set_reg(dst, Value::from_small_string("").unwrap_or(Value::nil()));
                }
            } else {
                // For longer strings, we'd need heap allocation
                // For now, truncate to small string
                let slice = unsafe { std::slice::from_raw_parts(ptr, len.min(6)) };
                if let Ok(s) = std::str::from_utf8(slice) {
                    state.set_reg(dst, Value::from_small_string(s).unwrap_or(Value::nil()));
                } else {
                    state.set_reg(dst, Value::nil());
                }
            }
        }
        // The parse / render / byte-len handlers below were small-string-only
        // STUBS until 2026-07-03: any heap/builder Text — and any `&Text`
        // argument, which is what the declared signatures take — fell into
        // the `else` branch and answered 0/nil; IntToText/FloatToText
        // TRUNCATED every result to 6 characters to fit the small-string
        // box; and the parsers returned a raw Int/Float where the stdlib
        // signatures promise `Maybe<…>`.  They now use the canonical
        // machinery: `string_helpers::extract_string` (CBGR-deref + all
        // three Text representations), `alloc_string_value` (small OR heap
        // result, no truncation), and the `make_maybe`/`make_some`/
        // `make_none` variant builders.
        Some(TextSubOpcode::ParseInt) => {
            let text_reg = read_reg(state)?;
            let text = state.get_reg(text_reg);
            let s = super::string_helpers::extract_string(&text, state);
            // Full-string parse: surrounding whitespace tolerated, any
            // other trailing garbage is None.
            let parsed = s.trim().parse::<i64>().ok();
            let result = super::method_dispatch::make_maybe_int(state, parsed)?;
            state.set_reg(dst, result);
        }
        Some(TextSubOpcode::ParseFloat) => {
            let text_reg = read_reg(state)?;
            let text = state.get_reg(text_reg);
            let s = super::string_helpers::extract_string(&text, state);
            let result = match s.trim().parse::<f64>() {
                Ok(f) => super::method_dispatch::make_some_value(state, Value::from_f64(f))?,
                Err(_) => super::method_dispatch::make_none_value(state)?,
            };
            state.set_reg(dst, result);
        }
        Some(TextSubOpcode::IntToText) => {
            let value_reg = read_reg(state)?;
            let n = state.get_reg(value_reg).as_i64();
            let s = format!("{}", n);
            let text_val = super::string_helpers::alloc_string_value(state, &s)?;
            state.set_reg(dst, text_val);
        }
        Some(TextSubOpcode::FloatToText) => {
            let value_reg = read_reg(state)?;
            let f = state.get_reg(value_reg).as_f64();
            // Rust's shortest-round-trip rendering ("1.5", "-0.25") — the
            // canonical form text_parse_float accepts back verbatim.
            let s = format!("{}", f);
            let text_val = super::string_helpers::alloc_string_value(state, &s)?;
            state.set_reg(dst, text_val);
        }
        Some(TextSubOpcode::ByteLen) => {
            let text_reg = read_reg(state)?;
            let text = state.get_reg(text_reg);
            let s = super::string_helpers::extract_string(&text, state);
            state.set_reg(dst, Value::from_i64(s.len() as i64));
        }
        Some(TextSubOpcode::CharLen) => {
            // Get Text length in characters — canonical extraction, same
            // small-string-only-stub history as ByteLen above.
            let text_reg = read_reg(state)?;
            let text = state.get_reg(text_reg);
            let s = super::string_helpers::extract_string(&text, state);
            state.set_reg(dst, Value::from_i64(s.chars().count() as i64));
        }
        Some(TextSubOpcode::IsEmpty) => {
            // Check if Text is empty
            let text_reg = read_reg(state)?;
            let text = state.get_reg(text_reg);

            let is_empty = if text.is_small_string() {
                text.as_small_string().is_empty()
            } else {
                true
            };
            state.set_reg(dst, Value::from_bool(is_empty));
        }
        Some(TextSubOpcode::IsUtf8) => {
            // Text type is always valid UTF-8
            let _text_reg = read_reg(state)?;
            state.set_reg(dst, Value::from_bool(true));
        }
        Some(TextSubOpcode::AsBytes) => {
            // Borrow a Text as a byte slice — a BYTE_SLICE (528) heap
            // object `[ObjectHeader][ptr: i64][len: i64]` (ARCH-P5) —
            // handling small-string, heap-string, and reference forms.
            //

            // The runtime representation of Text:
            //  small string → 6 bytes packed into the NaN-boxed Value itself
            //  heap string → pointer to the canonical TEXT record
            //  `[ObjectHeader]{ptr, len, cap}[bytes…]` (ARCH-P5 final leg)
            // Reading `self.ptr` via GetF is wrong for small strings, so we
            // materialise the byte view here. References (`&Text`) first
            // deref to reach the underlying Text value.
            let text_reg = read_reg(state)?;
            let mut text = state.get_reg(text_reg);
            use super::super::super::heap;
            use super::super::handlers::cbgr_helpers::{decode_cbgr_ref, is_cbgr_ref};
            use crate::types::TypeId;

            // Auto-deref: CBGR register-ref → absolute register, ThinRef → pointee.
            if is_cbgr_ref(&text) {
                let (abs_index, _gen) = decode_cbgr_ref(text);
                text = state.registers.get_absolute(abs_index);
            }
            if text.is_thin_ref() {
                let tr = text.as_thin_ref();
                if !tr.ptr.is_null() {
                    text = unsafe { *(tr.ptr as *const Value) };
                }
            }
            // A `cbgr_mutable_ptr` interior reference — what `&list[i]`
            // produces for a `List<Text>` element passed as a `&Text` fn
            // argument — is a POINTER to the SLOT holding the Text. Without
            // dereferencing it the handler read the slot address as a Text
            // header and returned a zero-length byte view (`fn f(s:&Text){
            // s.as_bytes() }` called `f(&offers[i])` gave len 0, breaking
            // text_eq / split_media_type / select_best_media). Deref it —
            // but ONLY when the result is itself a Text: a `cbgr_mutable_ptr`
            // that already points AT the Text (some `&local` forms, cidr
            // `set.add_text(&s)`) must NOT be over-dereferenced into its
            // buffer pointer.
            if text.is_ptr() && !text.is_nil() {
                let addr = text.as_ptr::<u8>() as usize;
                if state.cbgr_mutable_ptrs.contains(&addr) {
                    let derefed = unsafe { *(addr as *const Value) };
                    let yields_text = derefed.is_small_string()
                        || derefed.is_fat_ref()
                        || (derefed.is_ptr()
                            && !derefed.is_nil()
                            && matches!(
                                unsafe {
                                    heap::ObjectHeader::try_type_id(derefed.as_ptr::<u8>())
                                },
                                Some(TypeId::TEXT)
                            ));
                    if yields_text {
                        text = derefed;
                    }
                }
            }

            // Defensive input arms for values that are ALREADY byte
            // views: a BYTE_SLICE object (a view of a view shares the
            // same `(ptr, len)`) and the legacy raw-slice FatRef shape
            // (generic `slice_from_raw_parts` output).  Since the
            // BYTE_SLICE migration (ARCH-P5) no Text VALUE is
            // FatRef-encoded — `Text.from_utf8_unchecked` and the
            // struct-literal builders all produce Text-shaped heap
            // records — so the FatRef arm only normalizes stray byte
            // slices, it no longer carries the Text representation.
            let (ptr, len): (*mut u8, u64) = if let Some((p, l)) =
                heap::value_as_byte_slice(&text)
            {
                (p, l)
            } else if text.is_fat_ref() {
                let fr = text.as_fat_ref();
                (fr.ptr(), fr.len())
            } else if text.is_small_string() {
                // Small string: copy the inline bytes into a fresh heap
                // buffer so the returned FatRef has a stable address for
                // the full lifetime of the Value.
                let ss = text.as_small_string();
                let bytes = ss.as_bytes();
                let n = bytes.len();
                if n == 0 {
                    (std::ptr::null_mut(), 0)
                } else {
                    let obj = state.heap.alloc(TypeId::U8, n)?;
                    let data_ptr = obj.data_ptr();
                    unsafe {
                        std::ptr::copy_nonoverlapping(bytes.as_ptr(), data_ptr, n);
                    }
                    (data_ptr, n as u64)
                }
            } else if text.is_ptr() && !text.is_nil() {
                let base = text.as_ptr::<u8>();
                let header = match unsafe { heap::ObjectHeader::try_from_ptr(base) } {
                    Some(h) => h,
                    // Misaligned / null base means we can't read the
                    // Text shape — fall through to the as_bytes_arg
                    // empty-byte-slice failure path.
                    None => return Ok(DispatchResult::Continue),
                };
                if header.type_id == TypeId::TEXT {
                    // Canonical heap Text record `{ptr, len, cap}` —
                    // ARCH-P5 final leg: every producer (runtime AND
                    // struct-literal codegen) emits the record, so the
                    // former `[len:u64][bytes…]` dual-layout branches
                    // are retired.  `text_record_payload` carries the
                    // field-0 encoding tolerance (NaN-boxed ptr / Nil /
                    // Int-boxed address / raw pointer bits — the
                    // `Text.from_utf8_unchecked` struct-literal path).
                    //
                    // SAFETY: TEXT header established via try_from_ptr;
                    // producers guarantee the record payload contract.
                    match unsafe { heap::text_record_payload(base) } {
                        Some((p, l)) => (p as *mut u8, l as u64),
                        None => (std::ptr::null_mut(), 0),
                    }
                } else {
                    // Unknown pointer type — return empty slice rather than
                    // corrupt memory.
                    (std::ptr::null_mut(), 0)
                }
            } else {
                (std::ptr::null_mut(), 0)
            };

            // ARCH-P5: materialize the byte view as a BYTE_SLICE (528)
            // heap object — `[ObjectHeader][ptr: i64][len: i64]`, raw
            // slots, bit-identical to the Tier-1 AsBytes Pack — instead
            // of a bare `FatRef { reserved = 1 }`.  This is the typed
            // producer that retires the `len <= 1_000_000`
            // FatRef-as-Text heuristic at every consumer.  Null/empty
            // ptr is normalized to a static empty buffer inside
            // `alloc_byte_slice` (never-null contract).
            let obj = state.heap.alloc_byte_slice(ptr, len)?;
            state.record_allocation();
            state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
        }
        None => {
            return Err(InterpreterError::InvalidBytecode {
                pc: state.pc() as usize,
                message: format!("Unknown TextExtended sub-opcode: 0x{:02X}", sub_op_byte),
            });
        }
    }

    Ok(DispatchResult::Continue)
}
