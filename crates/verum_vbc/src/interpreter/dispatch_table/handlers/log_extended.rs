//! Log and memory extended opcode handlers for VBC interpreter dispatch.

use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::*;
use super::envelope::dispatch_enveloped;
use crate::instruction::{LogSubOpcode, Opcode};
use crate::value::Value;
// LIST-REALLOC-CANONICAL-1: realloc must recognise interpreter-heap backing
// objects (NewList/ListPush arrays) vs opaque std::alloc buffers.

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
pub(in super::super) fn handle_log_extended(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    dispatch_enveloped(state, log_extended_body)
}

/// `LogExtended` sub-op arms. Invoked through
/// [`dispatch_enveloped`](super::envelope::dispatch_enveloped), which owns the
/// sub-op byte, the operand-length envelope and the pc reposition — an arm may
/// read any number of operands, and may `return` early, without desynchronising
/// the instruction stream.
fn log_extended_body(
    state: &mut InterpreterState,
    sub_op_byte: u8,
) -> InterpreterResult<DispatchResult> {
    let sub_op = LogSubOpcode::from_byte(sub_op_byte);

    match sub_op {
        // ================================================================
        // Log Levels (0x00-0x04)
        //
        // Wire is [dst][msg]. The canonical `encode_operands` helper
        // prefixes the destination register unconditionally — even for
        // void-returning sub-ops like these, which never write it. Reading
        // `msg` first therefore consumed the DST byte and logged the
        // uninitialised destination temp instead of the message, leaving
        // the real operand unread (T0418).
        // ================================================================
        Some(LogSubOpcode::Info) => {
            let _dst = read_reg(state)?;
            let msg_reg = read_reg(state)?;
            let msg = state.get_reg(msg_reg);
            let msg_str = format_value_for_log(&msg);
            // Use eprintln for now; in production, this would use the log crate
            eprintln!("[INFO] {}", msg_str);
            Ok(DispatchResult::Continue)
        }

        Some(LogSubOpcode::Warning) => {
            let _dst = read_reg(state)?;
            let msg_reg = read_reg(state)?;
            let msg = state.get_reg(msg_reg);
            let msg_str = format_value_for_log(&msg);
            eprintln!("[WARN] {}", msg_str);
            Ok(DispatchResult::Continue)
        }

        Some(LogSubOpcode::Error) => {
            let _dst = read_reg(state)?;
            let msg_reg = read_reg(state)?;
            let msg = state.get_reg(msg_reg);
            let msg_str = format_value_for_log(&msg);
            eprintln!("[ERROR] {}", msg_str);
            Ok(DispatchResult::Continue)
        }

        Some(LogSubOpcode::Debug) => {
            let _dst = read_reg(state)?;
            let msg_reg = read_reg(state)?;
            let msg = state.get_reg(msg_reg);
            if state.log_level >= 3 {
                let msg_str = format_value_for_log(&msg);
                eprintln!("[DEBUG] {}", msg_str);
            }
            Ok(DispatchResult::Continue)
        }

        Some(LogSubOpcode::Trace) => {
            let _dst = read_reg(state)?;
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
        None => Err(InterpreterError::NotImplemented {
            feature: "log_extended sub-opcode",
            opcode: Some(Opcode::LogExtended),
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
        if raw < 0x110000
            && let Some(c) = char::from_u32(raw as u32)
        {
            return format!("{}", c);
        }
        format!("<ptr:{:p}>", value.as_ptr::<()>())
    } else {
        format!("{:?}", value)
    }
}
