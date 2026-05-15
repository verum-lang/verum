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
            // Get Unicode general category using Rust's char properties
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = state.get_reg(src_reg).as_char();
            // Return a category code:
            // 0 = Letter, 1 = Mark, 2 = Number, 3 = Punctuation,
            // 4 = Symbol, 5 = Separator, 6 = Other/Control
            let category = if c.is_alphabetic() {
                0 // Letter (Lu, Ll, Lt, Lm, Lo)
            } else if c.is_numeric() {
                2 // Number (Nd, Nl, No)
            } else if c.is_whitespace() {
                5 // Separator (Zs, Zl, Zp) — whitespace is a superset
            } else if c.is_ascii_punctuation()
                || matches!(c,
                    '\u{00A1}'..='\u{00BF}' |  // Latin-1 punctuation
                    '\u{2010}'..='\u{2027}' |  // General punctuation
                    '\u{2030}'..='\u{205E}' |  // More general punctuation
                    '\u{3001}'..='\u{3003}' |  // CJK punctuation
                    '\u{FE50}'..='\u{FE6F}' |  // Small form variants
                    '\u{FF01}'..='\u{FF0F}' |  // Fullwidth punctuation
                    '\u{FF1A}'..='\u{FF20}' |
                    '\u{FF3B}'..='\u{FF40}' |
                    '\u{FF5B}'..='\u{FF65}'
                )
            {
                3 // Punctuation (Pc, Pd, Ps, Pe, Pi, Pf, Po)
            } else if matches!(c,
                '$' | '+' | '<' | '=' | '>' | '^' | '`' | '|' | '~' |
                '\u{00A2}'..='\u{00A9}' |  // Currency/math symbols
                '\u{00AC}' | '\u{00AE}' | '\u{00AF}' |
                '\u{00B0}' | '\u{00B1}' |
                '\u{00D7}' | '\u{00F7}' |
                '\u{2190}'..='\u{21FF}' |  // Arrows
                '\u{2200}'..='\u{22FF}' |  // Mathematical operators
                '\u{2300}'..='\u{23FF}' |  // Misc technical
                '\u{25A0}'..='\u{25FF}' |  // Geometric shapes
                '\u{2600}'..='\u{26FF}' |  // Misc symbols
                '\u{2700}'..='\u{27BF}' |  // Dingbats
                '\u{1F300}'..='\u{1F9FF}'  // Emoji
            ) {
                4 // Symbol (Sm, Sc, Sk, So)
            } else if c.is_control() {
                6 // Other/Control (Cc, Cf, Cs, Co, Cn)
            } else if matches!(c,
                '\u{0300}'..='\u{036F}' |   // Combining diacriticals
                '\u{0483}'..='\u{0489}' |   // Cyrillic combining marks
                '\u{0591}'..='\u{05BD}' |   // Hebrew combining marks
                '\u{0610}'..='\u{061A}' |   // Arabic combining marks
                '\u{064B}'..='\u{065F}' |   // Arabic combining marks
                '\u{0670}' |                // Arabic superscript alef
                '\u{20D0}'..='\u{20FF}' |   // Combining marks for symbols
                '\u{FE20}'..='\u{FE2F}'     // Combining half marks
            ) {
                1 // Mark (Mn, Mc, Me) — combining marks
            } else {
                // Default: treat remaining as Letter if alphabetic-like,
                // otherwise Other
                6
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
