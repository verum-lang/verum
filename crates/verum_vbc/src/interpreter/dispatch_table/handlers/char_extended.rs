//! Char extended opcode handler for VBC interpreter dispatch.

use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::*;
use super::envelope::dispatch_enveloped;
use crate::instruction::{CharSubOpcode, Opcode};
use crate::value::Value;
use crate::interpreter::lenient::{i64_or_zero as lenient_i64, char_or_nul as lenient_char};

/// CharExtended (0x2B) - Character classification and conversion.
///
/// Sub-opcodes organized by category:
/// - 0x00-0x0F: ASCII Classification (fast path, inline)
/// - 0x10-0x1F: ASCII Case Conversion (fast path, inline)
/// - 0x20-0x2F: Unicode Classification (runtime lookup)
/// - 0x30-0x3F: Unicode Case Conversion (runtime lookup)
/// - 0x40-0x4F: Char Value Operations
///
/// # Performance
///
/// ASCII operations are implemented inline with ~2ns overhead.
/// Unicode operations use Rust's char methods which may require
/// Unicode data lookup (~20-50ns).
///
/// CBGR tier analysis: char extended operations dispatched via sub-opcode byte after
/// the primary CharExtended (0xCA) opcode. Unicode lookups take ~20-50ns.

/// T0714 — TOTAL register reads for envelope-family arms.  The
/// envelope authority already guarantees stream re-alignment for a
/// malformed carrier (under/over-declared operand counts), but an arm
/// that reads a garbage register index gets a garbage VALUE — and the
/// panicking accessors' debug_asserts turned that into a debug-build
/// abort (the extended_envelope_pins contract is: wrong value maybe,
/// dead stream never).  Release semantics are unchanged (the asserts
/// compile out); these helpers make debug agree with release.
// (lenient_f64 deliberately absent here — char arms consume ints and
// chars only; the float twin lives in math_extended/arith_extended.)

pub(in super::super) fn handle_char_extended(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    dispatch_enveloped(state, char_extended_body)
}

/// `CharExtended` sub-op arms. Invoked through
/// [`dispatch_enveloped`](super::envelope::dispatch_enveloped), which owns the
/// sub-op byte, the operand-length envelope and the pc reposition — an arm may
/// read any number of operands, and may `return` early, without desynchronising
/// the instruction stream.
fn char_extended_body(
    state: &mut InterpreterState,
    sub_op_byte: u8,
) -> InterpreterResult<DispatchResult> {
    let sub_op = CharSubOpcode::from_byte(sub_op_byte);

    match sub_op {
        // ================================================================
        // ASCII Classification (0x00-0x0F) - Inline fast path
        // ================================================================
        Some(CharSubOpcode::IsAlphabeticAscii) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = lenient_char(state.get_reg(src_reg));
            state.set_reg(dst, Value::from_bool(c.is_ascii_alphabetic()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsNumericAscii) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = lenient_char(state.get_reg(src_reg));
            state.set_reg(dst, Value::from_bool(c.is_ascii_digit()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsAlphanumericAscii) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = lenient_char(state.get_reg(src_reg));
            state.set_reg(dst, Value::from_bool(c.is_ascii_alphanumeric()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsWhitespaceAscii) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = lenient_char(state.get_reg(src_reg));
            state.set_reg(dst, Value::from_bool(c.is_ascii_whitespace()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsControlAscii) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = lenient_char(state.get_reg(src_reg));
            state.set_reg(dst, Value::from_bool(c.is_ascii_control()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsPunctuationAscii) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = lenient_char(state.get_reg(src_reg));
            state.set_reg(dst, Value::from_bool(c.is_ascii_punctuation()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsGraphicAscii) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = lenient_char(state.get_reg(src_reg));
            state.set_reg(dst, Value::from_bool(c.is_ascii_graphic()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsHexDigitAscii) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = lenient_char(state.get_reg(src_reg));
            state.set_reg(dst, Value::from_bool(c.is_ascii_hexdigit()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsLowercaseAscii) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = lenient_char(state.get_reg(src_reg));
            state.set_reg(dst, Value::from_bool(c.is_ascii_lowercase()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsUppercaseAscii) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = lenient_char(state.get_reg(src_reg));
            state.set_reg(dst, Value::from_bool(c.is_ascii_uppercase()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsAscii) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = lenient_char(state.get_reg(src_reg));
            state.set_reg(dst, Value::from_bool(c.is_ascii()));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // ASCII Case Conversion (0x10-0x1F) - Inline fast path
        // ================================================================
        Some(CharSubOpcode::ToUppercaseAscii) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = lenient_char(state.get_reg(src_reg));
            state.set_reg(dst, Value::from_char(c.to_ascii_uppercase()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::ToLowercaseAscii) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = lenient_char(state.get_reg(src_reg));
            state.set_reg(dst, Value::from_char(c.to_ascii_lowercase()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::EqIgnoreCaseAscii) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = lenient_char(state.get_reg(src_reg));
            // Returns true if char equals its ASCII uppercase form
            // (i.e., it's already uppercase or not a letter)
            state.set_reg(dst, Value::from_bool(c == c.to_ascii_uppercase()));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Unicode Classification (0x20-0x2F) - Runtime lookup
        // ================================================================
        Some(CharSubOpcode::IsAlphabeticUnicode) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = lenient_char(state.get_reg(src_reg));
            state.set_reg(dst, Value::from_bool(c.is_alphabetic()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsNumericUnicode) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = lenient_char(state.get_reg(src_reg));
            state.set_reg(dst, Value::from_bool(c.is_numeric()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsAlphanumericUnicode) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = lenient_char(state.get_reg(src_reg));
            state.set_reg(dst, Value::from_bool(c.is_alphanumeric()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsWhitespaceUnicode) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = lenient_char(state.get_reg(src_reg));
            state.set_reg(dst, Value::from_bool(c.is_whitespace()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsControlUnicode) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = lenient_char(state.get_reg(src_reg));
            state.set_reg(dst, Value::from_bool(c.is_control()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsLowercaseUnicode) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = lenient_char(state.get_reg(src_reg));
            state.set_reg(dst, Value::from_bool(c.is_lowercase()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsUppercaseUnicode) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = lenient_char(state.get_reg(src_reg));
            state.set_reg(dst, Value::from_bool(c.is_uppercase()));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Unicode Case Conversion (0x30-0x3F) - Runtime lookup
        // ================================================================
        Some(CharSubOpcode::ToUppercaseUnicode) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = lenient_char(state.get_reg(src_reg));
            // Returns first char of uppercase mapping
            let result = c.to_uppercase().next().unwrap_or(c);
            state.set_reg(dst, Value::from_char(result));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::ToLowercaseUnicode) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = lenient_char(state.get_reg(src_reg));
            // Returns first char of lowercase mapping
            let result = c.to_lowercase().next().unwrap_or(c);
            state.set_reg(dst, Value::from_char(result));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::ToTitlecaseUnicode) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = lenient_char(state.get_reg(src_reg));
            // Titlecase often equals uppercase for most chars
            // For special cases like 'ǆ' → 'ǅ', we'd need Unicode tables
            // Fallback to uppercase which is correct for most chars
            let result = c.to_uppercase().next().unwrap_or(c);
            state.set_reg(dst, Value::from_char(result));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Char Value Operations (0x40-0x4F)
        // ================================================================
        Some(CharSubOpcode::ToCodePoint) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = lenient_char(state.get_reg(src_reg));
            state.set_reg(dst, Value::from_i64(c as u32 as i64));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::FromCodePoint) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let code_point = lenient_i64(state.get_reg(src_reg)) as u32;
            match char::from_u32(code_point) {
                Some(c) => {
                    state.set_reg(dst, Value::from_char(c));
                    Ok(DispatchResult::Continue)
                }
                None => {
                    // Invalid code point - set to replacement char
                    state.set_reg(dst, Value::from_char('\u{FFFD}'));
                    Ok(DispatchResult::Continue)
                }
            }
        }

        Some(CharSubOpcode::LenUtf8) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = lenient_char(state.get_reg(src_reg));
            state.set_reg(dst, Value::from_i64(c.len_utf8() as i64));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::LenUtf16) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = lenient_char(state.get_reg(src_reg));
            state.set_reg(dst, Value::from_i64(c.len_utf16() as i64));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // UTF-8 Encoding/Decoding (0x50-0x5F)
        // ================================================================
        Some(CharSubOpcode::EncodeUtf8) => {
            // `Char.encode_utf8(&mut buf: [Byte]) -> Int`
            //
            // Canonical semantics (matches stdlib body in
            // `core/text/char.vr::Char.encode_utf8`):
            //   * Side effect: writes 1..=4 UTF-8 bytes into the caller's
            //     mutable byte buffer.
            //   * Return: number of bytes written (1, 2, 3, or 4 — Int).
            //
            // Pre-fix the intercept IGNORED the buf argument, allocated a
            // FRESH list of bytes, and returned the list pointer. Every
            // caller that paid the cost of providing a stack-allocated
            // `[Byte; 4]` (TextBuilder.push_char, Text.push_char,
            // Formatter byte-level encoding, regex Unicode dispatch)
            // observed: (a) the local buffer never written, (b) the
            // return value not an Int. `let n = ch.encode_utf8(&mut tmp);
            // while i < n { … }` then iterated zero times → push_char
            // silently dropped every char.
            //
            // Fix: read both operands, write bytes into the caller's buf
            // via its backing-array pointer, return the byte count Int.
            let dst = read_reg(state)?;
            let src_char = read_reg(state)?;
            let src_buf = read_reg(state)?;

            let c = lenient_char(state.get_reg(src_char));
            let mut tmp = [0u8; 4];
            let encoded = c.encode_utf8(&mut tmp);
            let n_bytes = encoded.len();

            // Resolve the buf argument through the canonical helper so
            // all three Verum reference shapes (CBGR register-ref,
            // heap-interior pointer, ThinRef) collapse to the underlying
            // heap value first.
            let buf_val_raw = state.get_reg(src_buf);
            let buf_val =
                super::cbgr_helpers::resolve_arg_value(state, buf_val_raw)?;

            if buf_val.is_fat_ref() {
                // #53 IS-REGULAR-PTR-CONTRACT leg: the encode scratch is
                // a `&mut [Byte]` SLICE — a FatRef marker passed the old
                // `is_ptr` gate and the header probe below dereferenced
                // marker bits (the text/-mix SIGSEGV in this handler).
                // Stride-honouring store through the ONE write authority.
                let fr = buf_val.as_fat_ref();
                let cap = fr.len() as usize;
                for (i, b) in tmp.iter().enumerate().take(n_bytes.min(cap)) {
                    super::memory_collections::fat_ref_write_element(
                        &fr,
                        i,
                        Value::from_i64(*b as i64),
                    );
                }
            } else if buf_val.is_regular_ptr() && !buf_val.is_nil() {
                let buf_ptr = buf_val.as_ptr::<u8>();
                let header = unsafe {
                    super::super::super::heap::ObjectHeader::ref_or_stub(buf_ptr)
                };
                let header_data = unsafe {
                    buf_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE)
                        as *mut Value
                };
                if header.type_id == crate::types::TypeId::BYTE_LIST {
                    // BYTE_LIST: `[len, cap, backing_ptr]`, 1 byte/elem.
                    let backing_ptr =
                        unsafe { (*header_data.add(2)).as_ptr::<u8>() };
                    let dst_bytes = unsafe {
                        backing_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE)
                            as *mut u8
                    };
                    for (i, b) in tmp.iter().enumerate().take(n_bytes) {
                        unsafe {
                            *dst_bytes.add(i) = *b;
                        }
                    }
                } else if header.type_id == crate::types::TypeId::LIST {
                    // LIST: `[len, cap, backing_ptr]`, NaN-boxed Value/elem.
                    let backing_ptr =
                        unsafe { (*header_data.add(2)).as_ptr::<u8>() };
                    let dst_vals = unsafe {
                        backing_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE)
                            as *mut Value
                    };
                    for (i, b) in tmp.iter().enumerate().take(n_bytes) {
                        unsafe {
                            *dst_vals.add(i) = Value::from_i64(*b as i64);
                        }
                    }
                } else {
                    // Fixed-size byte array (no LIST/BYTE_LIST header) —
                    // the heap object's payload IS the byte buffer.
                    let dst_bytes = unsafe {
                        buf_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE)
                            as *mut u8
                    };
                    for (i, b) in tmp.iter().enumerate().take(n_bytes) {
                        unsafe {
                            *dst_bytes.add(i) = *b;
                        }
                    }
                }
            } else if buf_val.is_thin_ref() {
                let thin_ref = buf_val.raw_thin_ref()?;
                if !thin_ref.ptr.is_null() {
                    for (i, b) in tmp.iter().enumerate().take(n_bytes) {
                        unsafe {
                            *thin_ref.ptr.add(i) = *b;
                        }
                    }
                }
            }

            state.set_reg(dst, Value::from_i64(n_bytes as i64));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::DecodeUtf8) => {
            // Decode UTF-8 character at `bytes[byte_idx]`.
            //
            // Source intrinsic signature (core/intrinsics/runtime/text.vr:100):
            //   `fn utf8_decode_char(bytes: &[Byte], byte_idx: Int) -> Char`
            //
            // The handler MUST read TWO arg registers: the byte slice (a
            // FatRef carrying ptr+len, materialised by `Text.as_bytes()` /
            // every `&[Byte]` call site) and the byte index (Int). Pre-fix
            // the handler read ONE register and treated it as a code point
            // value — the `// For now, treat src as a code point value`
            // comment was a stub never finished. The first arg is actually
            // the bytes-slice value; casting it `as u32` truncated a
            // pointer/handle and `char::from_u32(garbage)` returned a
            // wrong character. `for c in s.chars()` then yielded random
            // chars and downstream `assert_eq(c, 'a')` failed.
            //
            // The fix: read both args, walk the byte slice properly,
            // decode the UTF-8 leading byte at `byte_idx` (1–4 bytes per
            // codepoint), assemble the codepoint and return as Char.
            let dst = read_reg(state)?;
            let bytes_reg = read_reg(state)?;
            let idx_reg = read_reg(state)?;

            let bytes_val = state.get_reg(bytes_reg);
            let idx = lenient_i64(state.get_reg(idx_reg));

            // Recover the byte slice. Four cases:
            //   1. BYTE_SLICE byte view — the canonical `&[Byte]` from
            //      `Text.as_bytes()` (ARCH-P5), raw `{ptr, len}` payload.
            //   2. FatRef: ptr + len (generic byte-stride slices).
            //   3. List heap object: `[ObjectHeader][len:i64][cap:i64][data:ptr-or-inline]`.
            //   4. Anything else: bail out as U+FFFD.
            let (ptr, slice_len): (*const u8, u64) = if let Some((p, l)) =
                crate::interpreter::heap::value_as_byte_slice(&bytes_val)
            {
                (p as *const u8, l)
            } else if bytes_val.is_fat_ref() {
                let fr = bytes_val.as_fat_ref();
                (fr.ptr() as *const u8, fr.len())
            } else if bytes_val.is_regular_ptr() && !bytes_val.is_nil() {
                let base = bytes_val.as_ptr::<u8>();
                if base.is_null() {
                    state.set_reg(dst, Value::from_char('\u{FFFD}'));
                    return Ok(DispatchResult::Continue);
                }
                // List heap layout: skip ObjectHeader, then len + cap + data_ptr.
                use super::super::super::heap;
                let header = match unsafe { heap::ObjectHeader::try_from_ptr(base) } {
                    Some(h) => h,
                    None => {
                        state.set_reg(dst, Value::from_char('\u{FFFD}'));
                        return Ok(DispatchResult::Continue);
                    }
                };
                let _ = header; // verified alignment
                // Layout immediately after the header for List/Vec-shaped
                // values: i64 len, i64 cap, ptr to backing data.
                let after_header = unsafe {
                    base.add(std::mem::size_of::<heap::ObjectHeader>())
                };
                let len = unsafe { *(after_header as *const i64) } as u64;
                let data_ptr = unsafe { *(after_header.add(16) as *const *const u8) };
                if data_ptr.is_null() || len == 0 {
                    state.set_reg(dst, Value::from_char('\u{FFFD}'));
                    return Ok(DispatchResult::Continue);
                }
                (data_ptr, len)
            } else {
                state.set_reg(dst, Value::from_char('\u{FFFD}'));
                return Ok(DispatchResult::Continue);
            };

            // Decode the UTF-8 codepoint at the given byte offset.
            // 1-byte (0xxxxxxx)   → ASCII
            // 2-byte (110xxxxx 10xxxxxx)
            // 3-byte (1110xxxx 10xxxxxx 10xxxxxx)
            // 4-byte (11110xxx 10xxxxxx 10xxxxxx 10xxxxxx)
            // Out-of-range or invalid leader → U+FFFD.
            if idx < 0 || (idx as u64) >= slice_len {
                state.set_reg(dst, Value::from_char('\u{FFFD}'));
                return Ok(DispatchResult::Continue);
            }
            let i = idx as usize;
            let b0 = unsafe { *ptr.add(i) };
            let cp: u32 = if b0 & 0x80 == 0 {
                b0 as u32
            } else if b0 & 0xE0 == 0xC0 && (i + 1) < slice_len as usize {
                let b1 = unsafe { *ptr.add(i + 1) };
                ((b0 as u32 & 0x1F) << 6) | (b1 as u32 & 0x3F)
            } else if b0 & 0xF0 == 0xE0 && (i + 2) < slice_len as usize {
                let b1 = unsafe { *ptr.add(i + 1) };
                let b2 = unsafe { *ptr.add(i + 2) };
                ((b0 as u32 & 0x0F) << 12)
                    | ((b1 as u32 & 0x3F) << 6)
                    | (b2 as u32 & 0x3F)
            } else if b0 & 0xF8 == 0xF0 && (i + 3) < slice_len as usize {
                let b1 = unsafe { *ptr.add(i + 1) };
                let b2 = unsafe { *ptr.add(i + 2) };
                let b3 = unsafe { *ptr.add(i + 3) };
                ((b0 as u32 & 0x07) << 18)
                    | ((b1 as u32 & 0x3F) << 12)
                    | ((b2 as u32 & 0x3F) << 6)
                    | (b3 as u32 & 0x3F)
            } else {
                0xFFFD
            };
            let c = char::from_u32(cp).unwrap_or('\u{FFFD}');
            state.set_reg(dst, Value::from_char(c));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::EscapeDebug) => {
            // Escape character for debug output
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = lenient_char(state.get_reg(src_reg));
            // Return escaped representation as code point value
            // (full implementation would return string)
            let escaped = c.escape_debug().next().unwrap_or(c);
            state.set_reg(dst, Value::from_char(escaped));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::GeneralCategory) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            if state.read_byte(state.pc()).is_some() {
                return Err(InterpreterError::InvalidBytecode {
                    pc: state.pc() as usize,
                    message: "Unicode general category expects two registers".into(),
                });
            }
            let code = general_category(state.get_reg(src), state.pc() as usize)?;
            state.set_reg(dst, code);
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Unimplemented sub-opcodes
        // ================================================================
        None => Err(InterpreterError::NotImplemented {
            feature: "char_extended sub-opcode",
            opcode: Some(Opcode::CharExtended),
        }),
    }
}

pub(super) fn general_category(value: Value, pc: usize) -> InterpreterResult<Value> {
    let scalar = value
        .try_as_i64()
        .and_then(|word| u32::try_from(word).ok())
        .and_then(char::from_u32)
        .ok_or_else(|| InterpreterError::InvalidBytecode {
            pc,
            message: "Unicode general category requires a valid Char".into(),
        })?;
    Ok(Value::from_i64(i64::from(crate::unicode_category::code(
        scalar,
    ))))
}
