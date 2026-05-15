//! Char extended opcode handler for VBC interpreter dispatch.

use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::*;
use crate::instruction::{CharSubOpcode, Opcode};
use crate::value::Value;

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
pub(in super::super) fn handle_char_extended(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let sub_op_byte = read_u8(state)?;
    // Skip operand-length varint (see encode_instruction's
    // `Instruction::CharExtended` arm).
    let _operand_len = read_varint(state)?;
    let sub_op = CharSubOpcode::from_byte(sub_op_byte);

    match sub_op {
        // ================================================================
        // ASCII Classification (0x00-0x0F) - Inline fast path
        // ================================================================
        Some(CharSubOpcode::IsAlphabeticAscii) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = state.get_reg(src_reg).as_char();
            state.set_reg(dst, Value::from_bool(c.is_ascii_alphabetic()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsNumericAscii) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = state.get_reg(src_reg).as_char();
            state.set_reg(dst, Value::from_bool(c.is_ascii_digit()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsAlphanumericAscii) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = state.get_reg(src_reg).as_char();
            state.set_reg(dst, Value::from_bool(c.is_ascii_alphanumeric()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsWhitespaceAscii) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = state.get_reg(src_reg).as_char();
            state.set_reg(dst, Value::from_bool(c.is_ascii_whitespace()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsControlAscii) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = state.get_reg(src_reg).as_char();
            state.set_reg(dst, Value::from_bool(c.is_ascii_control()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsPunctuationAscii) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = state.get_reg(src_reg).as_char();
            state.set_reg(dst, Value::from_bool(c.is_ascii_punctuation()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsGraphicAscii) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = state.get_reg(src_reg).as_char();
            state.set_reg(dst, Value::from_bool(c.is_ascii_graphic()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsHexDigitAscii) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = state.get_reg(src_reg).as_char();
            state.set_reg(dst, Value::from_bool(c.is_ascii_hexdigit()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsLowercaseAscii) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = state.get_reg(src_reg).as_char();
            state.set_reg(dst, Value::from_bool(c.is_ascii_lowercase()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsUppercaseAscii) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = state.get_reg(src_reg).as_char();
            state.set_reg(dst, Value::from_bool(c.is_ascii_uppercase()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsAscii) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = state.get_reg(src_reg).as_char();
            state.set_reg(dst, Value::from_bool(c.is_ascii()));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // ASCII Case Conversion (0x10-0x1F) - Inline fast path
        // ================================================================
        Some(CharSubOpcode::ToUppercaseAscii) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = state.get_reg(src_reg).as_char();
            state.set_reg(dst, Value::from_char(c.to_ascii_uppercase()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::ToLowercaseAscii) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = state.get_reg(src_reg).as_char();
            state.set_reg(dst, Value::from_char(c.to_ascii_lowercase()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::EqIgnoreCaseAscii) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = state.get_reg(src_reg).as_char();
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
            let c = state.get_reg(src_reg).as_char();
            state.set_reg(dst, Value::from_bool(c.is_alphabetic()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsNumericUnicode) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = state.get_reg(src_reg).as_char();
            state.set_reg(dst, Value::from_bool(c.is_numeric()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsAlphanumericUnicode) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = state.get_reg(src_reg).as_char();
            state.set_reg(dst, Value::from_bool(c.is_alphanumeric()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsWhitespaceUnicode) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = state.get_reg(src_reg).as_char();
            state.set_reg(dst, Value::from_bool(c.is_whitespace()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsControlUnicode) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = state.get_reg(src_reg).as_char();
            state.set_reg(dst, Value::from_bool(c.is_control()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsLowercaseUnicode) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = state.get_reg(src_reg).as_char();
            state.set_reg(dst, Value::from_bool(c.is_lowercase()));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::IsUppercaseUnicode) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = state.get_reg(src_reg).as_char();
            state.set_reg(dst, Value::from_bool(c.is_uppercase()));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Unicode Case Conversion (0x30-0x3F) - Runtime lookup
        // ================================================================
        Some(CharSubOpcode::ToUppercaseUnicode) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = state.get_reg(src_reg).as_char();
            // Returns first char of uppercase mapping
            let result = c.to_uppercase().next().unwrap_or(c);
            state.set_reg(dst, Value::from_char(result));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::ToLowercaseUnicode) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = state.get_reg(src_reg).as_char();
            // Returns first char of lowercase mapping
            let result = c.to_lowercase().next().unwrap_or(c);
            state.set_reg(dst, Value::from_char(result));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::ToTitlecaseUnicode) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = state.get_reg(src_reg).as_char();
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
            let c = state.get_reg(src_reg).as_char();
            state.set_reg(dst, Value::from_i64(c as u32 as i64));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::FromCodePoint) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let code_point = state.get_reg(src_reg).as_i64() as u32;
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
            let c = state.get_reg(src_reg).as_char();
            state.set_reg(dst, Value::from_i64(c.len_utf8() as i64));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::LenUtf16) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = state.get_reg(src_reg).as_char();
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

            let c = state.get_reg(src_char).as_char();
            let mut tmp = [0u8; 4];
            let encoded = c.encode_utf8(&mut tmp);
            let n_bytes = encoded.len();

            // Resolve the buf argument through the canonical helper so
            // all three Verum reference shapes (CBGR register-ref,
            // heap-interior pointer, ThinRef) collapse to the underlying
            // heap value first.
            let buf_val_raw = state.get_reg(src_buf);
            let buf_val =
                super::cbgr_helpers::resolve_arg_value(state, buf_val_raw);

            if buf_val.is_ptr() && !buf_val.is_nil() {
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
                let thin_ref = buf_val.as_thin_ref();
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
            let idx = state.get_reg(idx_reg).as_i64();

            // Recover the byte slice. Three cases:
            //   1. FatRef: ptr + len (canonical `&[Byte]` from
            //      `Text.as_bytes`).
            //   2. List heap object: `[ObjectHeader][len:i64][cap:i64][data:ptr-or-inline]`.
            //   3. Anything else: bail out as U+FFFD.
            let (ptr, slice_len): (*const u8, u64) = if bytes_val.is_fat_ref() {
                let fr = bytes_val.as_fat_ref();
                (fr.ptr() as *const u8, fr.len())
            } else if bytes_val.is_ptr() && !bytes_val.is_nil() {
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
            let c = state.get_reg(src_reg).as_char();
            // Return escaped representation as code point value
            // (full implementation would return string)
            let escaped = c.escape_debug().next().unwrap_or(c);
            state.set_reg(dst, Value::from_char(escaped));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::GeneralCategory) => {
            // **Task #23 §D — Unicode general-category specific variant tag**.
            //
            // Pre-fix this returned a COARSE 7-category code (0=Letter,
            // 1=Mark, …, 6=Other) that mapped to the FIRST variant of
            // each category in the Verum `GeneralCategory` enum.  Tests
            // like `'a'.general_category() is GeneralCategory.Ll`
            // (tag 1) failed because the runtime returned 0 (= Lu's
            // tag), regardless of upper-vs-lower-case.
            //
            // Verum `GeneralCategory` variant order (pinned by
            // `core/text/char.vr:403`):
            //   Lu=0 Ll=1 Lt=2 Lm=3 Lo=4
            //   Mn=5 Mc=6 Me=7
            //   Nd=8 Nl=9 No=10
            //   Pc=11 Pd=12 Ps=13 Pe=14 Pi=15 Pf=16 Po=17
            //   Sm=18 Sc=19 Sk=20 So=21
            //   Zs=22 Zl=23 Zp=24
            //   Cc=25 Cf=26 Cs=27 Co=28 Cn=29
            //
            // Architectural rule: the runtime intrinsic MUST return
            // the exact variant tag matching the user-side enum's
            // declaration order — coarse categories are a downstream
            // helper (`is_letter()` walks via `match self { Lu | Ll
            // | … => true }`), not the intrinsic's contract.  Returning
            // a coarse code breaks the `is` operator at every specific-
            // variant test site.
            //
            // Coverage: every ASCII codepoint maps to its specific
            // Unicode general-category variant; non-ASCII falls back
            // through a sequence of property tests that produces the
            // best-available specific tag (Ll/Lu for alphabetic up/low,
            // Nd for is_numeric, Zs/Zl/Zp split for whitespace, etc.).
            //
            // Tags as ordered in `core/text/char.vr::GeneralCategory`:
            const TAG_LU: i64 = 0;
            const TAG_LL: i64 = 1;
            #[allow(dead_code)]
            const TAG_LT: i64 = 2;
            #[allow(dead_code)]
            const TAG_LM: i64 = 3;
            const TAG_LO: i64 = 4;
            const TAG_MN: i64 = 5;
            const TAG_ND: i64 = 8;
            #[allow(dead_code)]
            const TAG_NL: i64 = 9;
            #[allow(dead_code)]
            const TAG_NO: i64 = 10;
            const TAG_PC: i64 = 11;
            const TAG_PD: i64 = 12;
            const TAG_PS: i64 = 13;
            const TAG_PE: i64 = 14;
            #[allow(dead_code)]
            const TAG_PI: i64 = 15;
            #[allow(dead_code)]
            const TAG_PF: i64 = 16;
            const TAG_PO: i64 = 17;
            const TAG_SM: i64 = 18;
            const TAG_SC: i64 = 19;
            const TAG_SK: i64 = 20;
            const TAG_SO: i64 = 21;
            const TAG_ZS: i64 = 22;
            const TAG_ZL: i64 = 23;
            const TAG_ZP: i64 = 24;
            const TAG_CC: i64 = 25;
            const TAG_CF: i64 = 26;
            const TAG_CS: i64 = 27;
            #[allow(dead_code)]
            const TAG_CO: i64 = 28;
            const TAG_CN: i64 = 29;

            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = state.get_reg(src_reg).as_char();
            let cp = c as u32;

            // ASCII fast path — exact specific tag.
            let category = if cp < 128 {
                match c {
                    'A'..='Z' => TAG_LU,
                    'a'..='z' => TAG_LL,
                    '0'..='9' => TAG_ND,
                    ' ' => TAG_ZS,
                    // ASCII control chars (0x00-0x1F + 0x7F)
                    '\u{00}'..='\u{1F}' | '\u{7F}' => TAG_CC,
                    // Connector punctuation
                    '_' => TAG_PC,
                    // Dash punctuation
                    '-' => TAG_PD,
                    // Open punctuation
                    '(' | '[' | '{' => TAG_PS,
                    // Close punctuation
                    ')' | ']' | '}' => TAG_PE,
                    // Currency symbol
                    '$' => TAG_SC,
                    // Math symbols
                    '+' | '<' | '=' | '>' | '|' | '~' => TAG_SM,
                    // Modifier symbol
                    '^' | '`' => TAG_SK,
                    // Other punctuation (catch-all for !, ", #, %, &, ',
                    // *, ,, ., /, :, ;, ?, @, \, ASCII default)
                    '!' | '"' | '#' | '%' | '&' | '\'' | '*' | ',' | '.' | '/'
                    | ':' | ';' | '?' | '@' | '\\' => TAG_PO,
                    _ => TAG_CN,
                }
            } else {
                // Non-ASCII — best-available specific mapping via
                // Rust char properties + Unicode block heuristics.
                // The full Unicode UCD table is ~1500 entries and
                // lives in the `unicode-properties` crate; this
                // inline mapping covers the common cases and falls
                // back to coarse categories for the rest.
                if c.is_uppercase() {
                    TAG_LU
                } else if c.is_lowercase() {
                    TAG_LL
                } else if c.is_alphabetic() {
                    TAG_LO // Letter, other — Han / Hiragana / etc.
                } else if c.is_numeric() {
                    TAG_ND // Number, decimal digit — extended digits
                } else if c == '\u{2028}' {
                    TAG_ZL
                } else if c == '\u{2029}' {
                    TAG_ZP
                } else if c.is_whitespace() {
                    TAG_ZS
                } else if matches!(c,
                    '\u{0300}'..='\u{036F}' |   // Combining diacriticals
                    '\u{0483}'..='\u{0489}' |   // Cyrillic combining
                    '\u{0591}'..='\u{05BD}' |   // Hebrew combining
                    '\u{0610}'..='\u{061A}' |   // Arabic combining
                    '\u{064B}'..='\u{065F}' |
                    '\u{0670}' |
                    '\u{20D0}'..='\u{20FF}' |
                    '\u{FE20}'..='\u{FE2F}'
                ) {
                    TAG_MN
                } else if matches!(c,
                    '\u{00A2}'..='\u{00A5}' |
                    '\u{20A0}'..='\u{20CF}'
                ) {
                    TAG_SC
                } else if matches!(c,
                    '\u{2190}'..='\u{21FF}' |
                    '\u{2200}'..='\u{22FF}' |
                    '\u{2300}'..='\u{23FF}'
                ) {
                    TAG_SM
                } else if matches!(c,
                    '\u{25A0}'..='\u{25FF}' |
                    '\u{2600}'..='\u{26FF}' |
                    '\u{2700}'..='\u{27BF}' |
                    '\u{1F300}'..='\u{1F9FF}'
                ) {
                    TAG_SO
                } else if matches!(c,
                    '\u{00A1}' | '\u{00BF}' |
                    '\u{2010}'..='\u{2027}' |
                    '\u{2030}'..='\u{205E}'
                ) {
                    TAG_PO
                } else if c.is_control() {
                    TAG_CC
                } else if cp >= 0xE000 && cp <= 0xF8FF {
                    // Private Use Area
                    TAG_CO
                } else if cp >= 0xD800 && cp <= 0xDFFF {
                    TAG_CS
                } else if (0xFFF9..=0xFFFB).contains(&cp)
                    || (0x2060..=0x206F).contains(&cp)
                {
                    TAG_CF
                } else {
                    TAG_CN
                }
            };

            state.set_reg(dst, Value::from_i64(category));
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
