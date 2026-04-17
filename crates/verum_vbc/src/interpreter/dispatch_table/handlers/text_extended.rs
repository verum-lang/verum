//! Text extended opcode handler for VBC interpreter dispatch.

use crate::value::Value;
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::*;

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
pub(in super::super) fn handle_text_extended(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    use crate::instruction::TextSubOpcode;

    let sub_op_byte = read_u8(state)?;
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
        Some(TextSubOpcode::ParseInt) => {
            // Parse integer from Text
            let text_reg = read_reg(state)?;
            let text = state.get_reg(text_reg);

            if text.is_small_string() {
                let small = text.as_small_string();
                let s = small.as_str();
                if let Ok(n) = s.trim().parse::<i64>() {
                    state.set_reg(dst, Value::from_i64(n));
                } else {
                    // Parse error - return 0 (or could return error value)
                    state.set_reg(dst, Value::from_i64(0));
                }
            } else {
                state.set_reg(dst, Value::from_i64(0));
            }
        }
        Some(TextSubOpcode::ParseFloat) => {
            // Parse float from Text
            let text_reg = read_reg(state)?;
            let text = state.get_reg(text_reg);

            if text.is_small_string() {
                let small = text.as_small_string();
                let s = small.as_str();
                if let Ok(f) = s.trim().parse::<f64>() {
                    state.set_reg(dst, Value::from_f64(f));
                } else {
                    // Parse error - return 0.0
                    state.set_reg(dst, Value::from_f64(0.0));
                }
            } else {
                state.set_reg(dst, Value::from_f64(0.0));
            }
        }
        Some(TextSubOpcode::IntToText) => {
            // Convert integer to Text
            let value_reg = read_reg(state)?;
            let value = state.get_reg(value_reg);
            let n = value.as_i64();

            // Format as string
            let s = format!("{}", n);
            // Use small string if possible (up to 6 chars)
            if let Some(small) = Value::from_small_string(&s) {
                state.set_reg(dst, small);
            } else {
                // Truncate for small string (numbers up to 6 digits)
                let truncated = if s.len() > 6 { &s[..6] } else { &s };
                state.set_reg(dst, Value::from_small_string(truncated).unwrap_or(Value::nil()));
            }
        }
        Some(TextSubOpcode::FloatToText) => {
            // Convert float to Text
            let value_reg = read_reg(state)?;
            let value = state.get_reg(value_reg);
            let f = value.as_f64();

            // Format as string (compact representation)
            let s = if f.fract() == 0.0 && f.abs() < 1e6 {
                format!("{:.0}", f)
            } else {
                format!("{:.6}", f)
            };
            // Use small string if possible
            if let Some(small) = Value::from_small_string(&s) {
                state.set_reg(dst, small);
            } else {
                // Truncate for small string
                let truncated = if s.len() > 6 { &s[..6] } else { &s };
                state.set_reg(dst, Value::from_small_string(truncated).unwrap_or(Value::nil()));
            }
        }
        Some(TextSubOpcode::ByteLen) => {
            // Get Text length in bytes
            let text_reg = read_reg(state)?;
            let text = state.get_reg(text_reg);

            let len = if text.is_small_string() {
                text.as_small_string().len()
            } else {
                0
            };
            state.set_reg(dst, Value::from_i64(len as i64));
        }
        Some(TextSubOpcode::CharLen) => {
            // Get Text length in characters
            let text_reg = read_reg(state)?;
            let text = state.get_reg(text_reg);

            let len = if text.is_small_string() {
                text.as_small_string().as_str().chars().count()
            } else {
                0
            };
            state.set_reg(dst, Value::from_i64(len as i64));
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
            // Borrow a Text as a byte slice (FatRef with elem_size=1),
            // handling both NaN-boxed small strings and heap-allocated text.
            //
            // The runtime representation of Text is not the same as the
            // Verum struct `{ptr, len, cap}`:
            //   small string → 6 bytes packed into the NaN-boxed Value itself
            //   heap string  → pointer to `[ObjectHeader][len:u64][bytes...]`
            // Reading `self.ptr` via GetF is wrong in both cases, so we
            // materialise the byte view here.
            let text_reg = read_reg(state)?;
            let text = state.get_reg(text_reg);
            use crate::value::{FatRef, Capabilities};
            use crate::types::TypeId;
            use super::super::super::heap;

            let (ptr, len): (*mut u8, u64) = if text.is_small_string() {
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
                    let data_ptr = obj.data_ptr() as *mut u8;
                    unsafe {
                        std::ptr::copy_nonoverlapping(bytes.as_ptr(), data_ptr, n);
                    }
                    (data_ptr, n as u64)
                }
            } else if text.is_ptr() && !text.is_nil() {
                let base = text.as_ptr::<u8>();
                let header = unsafe { &*(base as *const heap::ObjectHeader) };
                if header.type_id == TypeId::TEXT || header.type_id == TypeId(0x0001) {
                    // Heap string layout: [ObjectHeader][len:u64][bytes...]
                    let len_ptr = unsafe { base.add(heap::OBJECT_HEADER_SIZE) as *const u64 };
                    let len = unsafe { *len_ptr };
                    let bytes_ptr = unsafe { base.add(heap::OBJECT_HEADER_SIZE + 8) };
                    (bytes_ptr, len)
                } else {
                    // Unknown pointer type — return empty slice rather than
                    // corrupt memory.
                    (std::ptr::null_mut(), 0)
                }
            } else {
                (std::ptr::null_mut(), 0)
            };

            let mut fat_ref = FatRef::slice(
                ptr,
                0,
                (state.cbgr_epoch & 0xFFFF) as u16,
                Capabilities::MUT_EXCLUSIVE,
                len,
            );
            fat_ref.reserved = 1; // byte-sized elements
            state.set_reg(dst, Value::from_fat_ref(fat_ref));
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
