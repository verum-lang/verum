//! VBC interpreter intercept for `catch_unwind(f)`.
//!
//! `core/base/panic.vr` declares:
//! ```verum
//! @intrinsic("catch_unwind")
//! public fn catch_unwind<T, F: fn() -> T>(f: F) -> Result<T, PanicInfo>
//! ```
//!
//! The codegen for `CatchUnwind` (expressions.rs:20709) currently emits a
//! plain `Call` with no exception or panic protection — panics from `f`
//! propagate up as `InterpreterError::Panic` and are never turned into
//! `Err(PanicInfo)`.
//!
//! This intercept runs `f` via `execute_table`, catches
//! `InterpreterError::Panic { message }`, and builds a Verum-side
//! `Result<T, PanicInfo>` value:
//!
//!   - Normal return → `Result.Ok(return_value)`  (variant tag 0)
//!   - Panic         → `Result.Err(PanicInfo { message, location: None })` (tag 1)
//!
//! ## PanicInfo layout (declaration order in panic.vr)
//!
//!   [0] message  : Text
//!   [1] location : Maybe<Location>   ← always `None` (tag 0) here
//!
//! ## Why an intercept and not a TryBegin/TryEnd wrapper in codegen?
//!
//! The VBC TryBegin/TryEnd mechanism handles explicit `Throw` instructions
//! (Verum exceptions), NOT `InterpreterError::Panic` (which is a Rust-level
//! error that propagates up through Rust call frames, not VBC frames). An
//! intercept that calls `execute_table` from Rust can catch it with a normal
//! Rust `match`.

use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use crate::interpreter::execute_table;
use super::heap_helpers::{alloc_record_n_fields, wrap_in_variant};
use super::string_helpers::alloc_string_value;
use crate::instruction::Reg;
use crate::module::FunctionId;
use crate::value::Value;

/// Try to intercept a `catch_unwind(f)` call.
///
/// Returns `Some(Result<T, PanicInfo>)` when the call is recognised,
/// `None` otherwise.
pub(in super::super) fn try_intercept_catch_unwind(
    state: &mut InterpreterState,
    func_name: &str,
    args_start_reg: u16,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let bare = func_name.rsplit('.').next().unwrap_or(func_name);
    if bare != "catch_unwind" {
        return Ok(None);
    }
    if arg_count != 1 {
        return Ok(None);
    }

    // Retrieve the closure argument.
    let closure_val = state
        .registers
        .get(caller_base, Reg(args_start_reg));
    if !closure_val.is_func_ref() {
        // Not a function value — fall through to normal dispatch.
        return Ok(None);
    }
    let func_id: FunctionId = closure_val.as_func_id();

    // Save the current call-stack depth so we can unwind on panic.
    let saved_depth = state.call_stack.depth();

    // Execute the closure, capturing any panic.
    let inner_result = execute_table(state, func_id);

    match inner_result {
        Ok(val) => {
            // Wrap in Result.Ok (tag 0).
            let ok = wrap_in_variant(state, "Result", 0, &[val])?;
            Ok(Some(ok))
        }
        Err(InterpreterError::Panic { message }) => {
            // Unwind the call stack back to the pre-call depth.
            while state.call_stack.depth() > saved_depth {
                let _ = state.call_stack.pop_frame();
            }

            // Build PanicInfo { message, location: None }.
            let msg_val = alloc_string_value(state, &message)?;
            let none_loc = wrap_in_variant(state, "Maybe", 0, &[])?;
            let panic_info = alloc_record_n_fields(state, "PanicInfo", &[msg_val, none_loc])?;

            // Wrap in Result.Err (tag 1).
            let err = wrap_in_variant(state, "Result", 1, &[panic_info])?;
            Ok(Some(err))
        }
        Err(other) => {
            // Unwind the call stack before re-propagating.
            while state.call_stack.depth() > saved_depth {
                let _ = state.call_stack.pop_frame();
            }
            // Non-panic interpreter errors (StackOverflow, BadOpcode, …)
            // are not catchable by user code — propagate them.
            Err(other)
        }
    }
}
