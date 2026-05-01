//! Async-runtime intercepts for the VBC interpreter (Tier 0).
//!
//! Backs `block_on` (top-level fn) and `AsyncRuntime.block_on`
//! (method) under interpreter mode where async fns are NOT
//! compiled to suspend/resume state machines. From
//! `crates/verum_vbc/src/codegen/expressions.rs:12778-12793`:
//!
//!     "Async fns in the current implementation are not compiled
//!      to suspend-/resume state machines — calling `add(1, 2)`
//!      runs the body inline and returns the value."
//!
//! Consequence: `rt.block_on(small_work())` receives the
//! already-computed value of `small_work()` (e.g. `Int(42)`),
//! NOT a Future state machine. The pure-Verum body of
//! `block_on` calls `.poll()` on this value, which the method
//! dispatcher (faced with a non-pointer receiver and a method
//! name with multiple `*.poll` candidates) silently picks the
//! first match — typically `Receiver.poll` from `core.async.channel`
//! — which then field-accesses through the bogus receiver and
//! dies on a null/invalid pointer dereference.
//!
//! ## Intercept policy
//!
//! Under the interpreter's documented "async-as-sync" semantics,
//! `block_on(x)` is by-construction a no-op: x IS the awaited
//! value. The intercept returns it directly, bypassing the
//! `.poll()` dispatch entirely.
//!
//! AOT mode keeps the full Future state-machine dispatch path —
//! this intercept only applies to interpreter execution.

use crate::instruction::Reg;
use crate::interpreter::error::InterpreterResult;
use crate::interpreter::state::InterpreterState;
use crate::value::Value;

/// Try to intercept `block_on` calls. Returns `Some(value)` when
/// the call is recognised (the caller installs the result into
/// `dst` and continues), `None` when it isn't (caller falls
/// through to compiled-bytecode dispatch).
///
/// Recognised forms:
///   * `block_on(future)` — top-level free function: 1 arg, the
///     already-computed value.
///   * `runtime.block_on(future)` — method on AsyncRuntime: the
///     method-dispatch handler (`method_dispatch.rs::handle_call_method`)
///     calls this with `func_name == "block_on"` AFTER
///     stripping the receiver-type prefix.
///
/// In both cases the future arg is the second register in
/// `args_start..args_start+arg_count` for the method form (after
/// the receiver), or the FIRST for the free-fn form.
pub(in super::super) fn try_intercept_async_runtime(
    state: &mut InterpreterState,
    func_name: &str,
    args_start_reg: u16,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let bare = func_name.rsplit('.').next().unwrap_or(func_name);
    if bare != "block_on" {
        return Ok(None);
    }
    // The "future" argument index depends on whether we caught
    // the free-fn `block_on(future)` (1 arg, idx 0) or the
    // method `runtime.block_on(future)` (2 args: self, future
    // — idx 1 from the original Call, but the calls.rs
    // intercept layer doesn't know about CallM receivers,
    // so this fast-path only fires for the free-fn form via
    // `Call`).
    if arg_count != 1 {
        return Ok(None);
    }
    // Read the single argument (the "future" — really the
    // already-computed value of the async fn body in interpreter
    // mode).
    let future_val = state.registers.get(caller_base, Reg(args_start_reg));
    Ok(Some(future_val))
}
