//! Exception handling instruction handlers for VBC interpreter.

use crate::value::Value;
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::super::format_value_for_print;
use super::bytecode_io::*;

// ============================================================================
// Exception Handling Operations
// ============================================================================

/// Throw (0xD0) - Throw an exception.
///
/// Format: `Throw error_reg`
/// Throws an exception with the value from error_reg.
/// Unwinds the stack to the nearest exception handler or errors if none.
pub(in super::super) fn handle_throw(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let error_reg = read_reg(state)?;
    let error_value = state.get_reg(error_reg);

    // Store the exception value
    state.current_exception = Some(error_value);

    let debug = std::env::var("VBC_EXC_TRACE").is_ok();
    if debug {
        eprintln!("[THROW] pc={} error_reg={:?} value=0x{:x} stack_depth={} handlers={}",
            state.pc(), error_reg, error_value.to_bits(),
            state.call_stack.depth(), state.exception_handlers.len());
    }

    // Find the nearest exception handler
    if let Some(handler) = state.exception_handlers.pop() {
        if debug {
            eprintln!("[THROW] jumping to handler_pc={} (stack_depth={}, func={:?})",
                handler.handler_pc, handler.stack_depth, handler.func_id);
        }
        // Unwind stack to handler's depth
        while state.call_stack.depth() > handler.stack_depth {
            let _ = state.call_stack.pop_frame();
        }

        // Jump to the exception handler
        state.call_stack.set_pc(handler.handler_pc as u32);
        Ok(DispatchResult::Continue)
    } else {
        // No exception handler - propagate as a panic
        // Format the exception value for the error message
        let value_str = format_value_for_print(state, error_value);
        let message = format!("unhandled exception: {}", value_str);
        Err(InterpreterError::Panic { message })
    }
}

/// TryBegin (0xD1) - Begin a try block.
///
/// Format: `TryBegin handler_offset:i32`
/// Sets up an exception handler at the given offset from current PC.
pub(in super::super) fn handle_try_begin(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    // Read the handler offset (signed 32-bit varint)
    let offset = read_signed_varint(state)?;

    // Calculate handler PC relative to current instruction
    let handler_pc = (state.pc() as i64 + offset as i64) as usize;

    // Get current context
    let stack_depth = state.call_stack.depth();
    let reg_base = state.reg_base() as usize;
    let func_id = state.call_stack.current()
        .map(|f| f.function)
        .unwrap_or(crate::module::FunctionId(0));

    // Push exception handler
    let handler = super::super::super::state::ExceptionHandler {
        handler_pc,
        stack_depth,
        reg_base,
        func_id,
    };
    if std::env::var("VBC_EXC_TRACE").is_ok() {
        eprintln!("[TRY_BEGIN] pc={} handler_pc={} stack_depth={} reg_base={} func={:?}",
            state.pc(), handler_pc, stack_depth, reg_base, func_id);
    }
    state.exception_handlers.push(handler);

    Ok(DispatchResult::Continue)
}

/// TryEnd (0xD2) - End a try block.
///
/// Format: `TryEnd`
/// Pops the current exception handler (normal exit from try block).
pub(in super::super) fn handle_try_end(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    // Pop the exception handler - we're leaving the try block normally
    let _ = state.exception_handlers.pop();
    Ok(DispatchResult::Continue)
}

/// GetException (0xD3) - Get the current exception value.
///
/// Format: `GetException dst`
/// Gets the current exception value (set by Throw) into dst register.
pub(in super::super) fn handle_get_exception(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;

    // Get the current exception value (or unit if none)
    let exception = state.current_exception.take().unwrap_or(Value::unit());
    if std::env::var("VBC_EXC_TRACE").is_ok() {
        eprintln!("[GET_EXC] pc={} dst={:?} value=0x{:x}",
            state.pc(), dst, exception.to_bits());
    }
    state.set_reg(dst, exception);

    Ok(DispatchResult::Continue)
}

