//! Char extended opcode handler for VBC interpreter dispatch.

use crate::instruction::{CharSubOpcode, Opcode};
use crate::value::Value;
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::*;

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
pub(in super::super) fn handle_char_extended(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let sub_op_byte = read_u8(state)?;
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
            // Encode character as UTF-8 bytes and return a heap-allocated list of byte values
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let c = state.get_reg(src_reg).as_char();
            let mut buf = [0u8; 4];
            let encoded = c.encode_utf8(&mut buf);
            let len = encoded.len();

            // Allocate backing array for bytes
            let backing_layout = std::alloc::Layout::from_size_align(len * 8, 8)
                .map_err(|_| InterpreterError::Panic { message: "bad layout for utf8 bytes".into() })?;
            let backing_ptr = unsafe { std::alloc::alloc_zeroed(backing_layout) };
            if backing_ptr.is_null() {
                return Err(InterpreterError::Panic { message: "alloc failed for utf8 bytes".into() });
            }
            // Fill backing array with byte values
            for i in 0..len {
                let val = Value::from_i64(buf[i] as i64);
                unsafe {
                    std::ptr::write((backing_ptr as *mut Value).add(i), val);
                }
            }

            // Allocate list header: [length, capacity, backing_ptr]
            let header_layout = std::alloc::Layout::from_size_align(3 * 8, 8)
                .map_err(|_| InterpreterError::Panic { message: "bad layout for utf8 header".into() })?;
            let header_ptr = unsafe { std::alloc::alloc_zeroed(header_layout) };
            if header_ptr.is_null() {
                return Err(InterpreterError::Panic { message: "alloc failed for utf8 header".into() });
            }
            unsafe {
                std::ptr::write(header_ptr as *mut i64, len as i64);
                std::ptr::write((header_ptr as *mut i64).add(1), len as i64);
                std::ptr::write((header_ptr as *mut i64).add(2), backing_ptr as i64);
            }
            state.set_reg(dst, Value::from_ptr(header_ptr));
            Ok(DispatchResult::Continue)
        }

        Some(CharSubOpcode::DecodeUtf8) => {
            // Decode UTF-8 bytes to character
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            // For now, treat src as a code point value
            let v = state.get_reg(src_reg).as_i64() as u32;
            let c = char::from_u32(v).unwrap_or('\u{FFFD}');
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
        None => {
            Err(InterpreterError::NotImplemented {
                feature: "char_extended sub-opcode",
                opcode: Some(Opcode::CharExtended),
            })
        }
    }
}
