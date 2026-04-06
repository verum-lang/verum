//! Log and memory extended opcode handlers for VBC interpreter dispatch.

use crate::instruction::{Opcode, LogSubOpcode};
use crate::value::Value;
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::*;

/// LogExtended (0xBE) - Structured logging operations.
///
/// Sub-opcodes organized by category:
/// - 0x00-0x04: Log levels (Info, Warning, Error, Debug, Trace)
/// - 0x10: Structured logging with key-value pairs
/// - 0x20-0x22: Control operations (Flush, SetLevel, GetLevel)
///
/// # Performance
///
/// Logging is inherently I/O-bound, so the runtime overhead (~50ns)
/// is negligible compared to actual I/O operations.
///
/// Extended logging opcode (0xCB + sub-opcode): structured logging with levels (Debug, Info,
/// Warn, Error, Fatal), structured fields, and context integration. ~50ns runtime overhead
/// is negligible vs I/O cost.
pub(in super::super) fn handle_log_extended(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let sub_op_byte = read_u8(state)?;
    let sub_op = LogSubOpcode::from_byte(sub_op_byte);

    match sub_op {
        // ================================================================
        // Log Levels (0x00-0x04)
        // ================================================================
        Some(LogSubOpcode::Info) => {
            let msg_reg = read_reg(state)?;
            let msg = state.get_reg(msg_reg);
            let msg_str = format_value_for_log(&msg);
            // Use eprintln for now; in production, this would use the log crate
            eprintln!("[INFO] {}", msg_str);
            Ok(DispatchResult::Continue)
        }

        Some(LogSubOpcode::Warning) => {
            let msg_reg = read_reg(state)?;
            let msg = state.get_reg(msg_reg);
            let msg_str = format_value_for_log(&msg);
            eprintln!("[WARN] {}", msg_str);
            Ok(DispatchResult::Continue)
        }

        Some(LogSubOpcode::Error) => {
            let msg_reg = read_reg(state)?;
            let msg = state.get_reg(msg_reg);
            let msg_str = format_value_for_log(&msg);
            eprintln!("[ERROR] {}", msg_str);
            Ok(DispatchResult::Continue)
        }

        Some(LogSubOpcode::Debug) => {
            let msg_reg = read_reg(state)?;
            let msg = state.get_reg(msg_reg);
            if state.log_level >= 3 {
                let msg_str = format_value_for_log(&msg);
                eprintln!("[DEBUG] {}", msg_str);
            }
            Ok(DispatchResult::Continue)
        }

        Some(LogSubOpcode::Trace) => {
            let msg_reg = read_reg(state)?;
            let msg = state.get_reg(msg_reg);
            if state.log_level >= 4 {
                let msg_str = format_value_for_log(&msg);
                eprintln!("[TRACE] {}", msg_str);
            }
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Structured Logging (0x10)
        // ================================================================
        Some(LogSubOpcode::Structured) => {
            let _level = read_u8(state)?;
            let msg_reg = read_reg(state)?;
            let _kvs_reg = read_reg(state)?;
            // For now, just log the message; full structured logging TBD
            let msg = state.get_reg(msg_reg);
            let msg_str = format_value_for_log(&msg);
            eprintln!("[STRUCTURED] {}", msg_str);
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Control Operations (0x20-0x22)
        // ================================================================
        Some(LogSubOpcode::Flush) => {
            // Flush stderr (logs go to stderr)
            use std::io::Write;
            let _ = std::io::stderr().flush();
            Ok(DispatchResult::Continue)
        }

        Some(LogSubOpcode::SetLevel) => {
            let level_reg = read_reg(state)?;
            let level = state.get_reg(level_reg).as_i64();
            state.log_level = level.clamp(0, 4);
            Ok(DispatchResult::Continue)
        }

        Some(LogSubOpcode::GetLevel) => {
            let dst = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(state.log_level));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Unimplemented sub-opcodes
        // ================================================================
        None => {
            Err(InterpreterError::NotImplemented {
                feature: "log_extended sub-opcode",
                opcode: Some(Opcode::LogExtended),
            })
        }
    }
}

/// MemExtended (0xBF) - Memory allocation operations.
///
/// Sub-opcodes:
/// - 0x00: Alloc - allocate heap memory
/// - 0x01: AllocZeroed - allocate zeroed heap memory
/// - 0x02: Dealloc - deallocate heap memory
/// - 0x03: Realloc - reallocate heap memory
/// - 0x04: Swap - swap two values in place
/// - 0x05: Replace - replace value and return old
pub(in super::super) fn handle_mem_extended(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let sub_op = read_u8(state)?;

    match sub_op {
        // Alloc: [dst, size, align]
        0x00 => {
            let dst = read_reg(state)?;
            let size_reg = read_reg(state)?;
            let _align_reg = read_reg(state)?;

            let size = state.get_reg(size_reg).as_i64() as usize;

            // Allocate memory using system allocator
            let layout = std::alloc::Layout::from_size_align(size.max(1), 8)
                .map_err(|_| InterpreterError::Panic {
                    message: "invalid allocation layout".into(),
                })?;
            let ptr = unsafe { std::alloc::alloc(layout) };
            if ptr.is_null() {
                return Err(InterpreterError::Panic {
                    message: "allocation failed".into(),
                });
            }

            state.set_reg(dst, Value::from_ptr(ptr as *mut ()));
            Ok(DispatchResult::Continue)
        }

        // AllocZeroed: [dst, size, align]
        0x01 => {
            let dst = read_reg(state)?;
            let size_reg = read_reg(state)?;
            let _align_reg = read_reg(state)?;

            let size = state.get_reg(size_reg).as_i64() as usize;

            // Allocate zeroed memory using system allocator
            let layout = std::alloc::Layout::from_size_align(size.max(1), 8)
                .map_err(|_| InterpreterError::Panic {
                    message: "invalid allocation layout".into(),
                })?;
            let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
            if ptr.is_null() {
                return Err(InterpreterError::Panic {
                    message: "allocation failed".into(),
                });
            }

            state.set_reg(dst, Value::from_ptr(ptr as *mut ()));
            Ok(DispatchResult::Continue)
        }

        // Dealloc: [ptr, size, align]
        0x02 => {
            let ptr_reg = read_reg(state)?;
            let size_reg = read_reg(state)?;
            let _align_reg = read_reg(state)?;

            let ptr = state.get_reg(ptr_reg).as_ptr::<u8>() as *mut u8;
            let size = state.get_reg(size_reg).as_i64() as usize;

            if !ptr.is_null() && size > 0 {
                let layout = std::alloc::Layout::from_size_align(size, 8)
                    .map_err(|_| InterpreterError::Panic {
                        message: "invalid deallocation layout".into(),
                    })?;
                unsafe { std::alloc::dealloc(ptr, layout) };
            }

            Ok(DispatchResult::Continue)
        }

        // Realloc: [dst, ptr, old_size, new_size, align]
        0x03 => {
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;
            let old_size_reg = read_reg(state)?;
            let new_size_reg = read_reg(state)?;
            let align_reg = read_reg(state)?;

            let ptr = state.get_reg(ptr_reg).as_ptr::<u8>() as *mut u8;
            let old_size = state.get_reg(old_size_reg).as_i64() as usize;
            let new_size = state.get_reg(new_size_reg).as_i64() as usize;
            let align = {
                let a = state.get_reg(align_reg).as_i64() as usize;
                if a == 0 { 8 } else { a }
            };

            let new_layout = std::alloc::Layout::from_size_align(new_size.max(1), align)
                .map_err(|_| InterpreterError::Panic {
                    message: "invalid reallocation layout".into(),
                })?;
            let new_ptr = unsafe { std::alloc::alloc(new_layout) };
            if new_ptr.is_null() {
                return Err(InterpreterError::Panic {
                    message: "reallocation failed".into(),
                });
            }

            // Copy old data to new allocation (up to the smaller of old/new size)
            if !ptr.is_null() && old_size > 0 {
                let copy_size = old_size.min(new_size);
                unsafe { std::ptr::copy_nonoverlapping(ptr, new_ptr, copy_size) };
                // Zero the extra bytes if growing
                if new_size > old_size {
                    unsafe {
                        std::ptr::write_bytes(new_ptr.add(old_size), 0, new_size - old_size);
                    }
                }
                // Free old allocation
                if let Ok(old_layout) = std::alloc::Layout::from_size_align(old_size.max(1), align) {
                    unsafe { std::alloc::dealloc(ptr, old_layout) };
                }
            }

            state.set_reg(dst, Value::from_ptr(new_ptr as *mut ()));
            Ok(DispatchResult::Continue)
        }

        // Swap: [a, b]
        0x04 => {
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;

            // Get pointers from the registers
            let a_ptr = state.get_reg(a_reg).as_ptr::<u64>() as *mut u64;
            let b_ptr = state.get_reg(b_reg).as_ptr::<u64>() as *mut u64;

            // Swap the values at those pointers
            unsafe {
                let tmp = *a_ptr;
                *a_ptr = *b_ptr;
                *b_ptr = tmp;
            }

            Ok(DispatchResult::Continue)
        }

        // Replace: [dst, dest, src]
        0x05 => {
            let dst = read_reg(state)?;
            let dest_reg = read_reg(state)?;
            let src_reg = read_reg(state)?;

            let dest_ptr = state.get_reg(dest_reg).as_ptr::<u64>() as *mut u64;
            let new_val = state.get_reg(src_reg).as_i64() as u64;

            // Read old value and write new value
            let old_val = unsafe { *dest_ptr };
            unsafe { *dest_ptr = new_val };

            state.set_reg(dst, Value::from_i64(old_val as i64));
            Ok(DispatchResult::Continue)
        }

        _ => Err(InterpreterError::NotImplemented {
            feature: "mem_extended sub-opcode",
            opcode: Some(Opcode::MemExtended),
        }),
    }
}

/// Format a Value for logging output.
pub(in super::super) fn format_value_for_log(value: &Value) -> String {
    if value.is_small_string() {
        // For small strings, extract the content
        value.as_small_string().as_str().to_string()
    } else if value.is_int() {
        format!("{}", value.as_i64())
    } else if value.is_float() {
        format!("{}", value.as_f64())
    } else if value.is_bool() {
        format!("{}", value.as_bool())
    } else if value.is_nil() {
        "nil".to_string()
    } else if value.is_unit() {
        "()".to_string()
    } else if value.is_ptr() {
        // Could be a char or other pointer value
        // Try to interpret as char if it looks like a valid code point
        let raw = value.as_ptr::<()>() as u64;
        if raw < 0x110000 {
            if let Some(c) = char::from_u32(raw as u32) {
                return format!("{}", c);
            }
        }
        format!("<ptr:{:p}>", value.as_ptr::<()>())
    } else {
        format!("{:?}", value)
    }
}
