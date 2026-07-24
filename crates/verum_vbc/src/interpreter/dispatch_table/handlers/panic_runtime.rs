//! VBC interpreter intercepts for the `core.base.panic` surface:
//! `catch_unwind(f)` and the low-level `panic_impl(msg, file, line, col)`.
//!
//! `core/base/panic.vr` declares:
//! ```verum
//! @intrinsic("catch_unwind")
//! public fn catch_unwind<T, F: fn() -> T>(f: F) -> Result<T, PanicInfo>
//! ```
//! (`core/intrinsics/control.vr` declares a second spelling returning
//! `Result<T, IntrinsicPanicInfo>` — same field shape `{message,
//! location}`, same intercept.)
//!
//! ## Why an intercept and not the TryBegin/TryEnd VBC body?
//!
//! The VBC TryBegin/TryEnd mechanism handles explicit `Throw` instructions
//! (Verum exceptions): the dispatch loop never consults
//! `state.exception_handlers` when a HANDLER returns an error, so
//! `InterpreterError::Panic` (a Rust-level error propagating through Rust
//! call frames) sails past the `TryBegin` the CatchUnwind inline sequence
//! emits. An intercept that runs the closure via `call_function_sync` from
//! Rust can catch it with a normal Rust `match`. Tier-1 keeps the compiled
//! body.
//!
//! ## Why `call_function_sync`, NOT `execute_table_with_args` (T0619)
//!
//! `execute_table_with_args` is the TOP-LEVEL entry primitive: it pushes the
//! entry frame with `return_pc = 0` and does not preserve the caller's r0.
//! That is correct only when the callee's final `Ret` empties the stack
//! (`do_return` hits its `is_empty()` → `FinalReturn` arm before touching a
//! caller). `catch_unwind` runs the closure NESTED inside the live `main`
//! frame, so a NORMAL-returning closure's `Ret` reaches `do_return`'s
//! caller-return tail instead: it writes the result into `main`'s r0 and
//! restores `main`'s pc to the entry frame's bogus `return_pc = 0` — `main`
//! restarts from the top and spins until `InstructionLimitExceeded`. (The
//! panic path dodged it: an `Err` unwinds without running `do_return`.)
//! `call_function_sync` is the CANONICAL nested-execution primitive — it
//! captures the real `return_pc = state.pc()` and saves/restores the
//! caller's r0 (`CALLSYNC-R0-CLOBBER-1`), so the closure returns to `main`'s
//! true resume point with r0 intact.
//!
//! ## Closure shapes (CATCH-UNWIND-CLOSURE-1, T0148)
//!
//! Since #110 the codegen emits `NewClosure` heap objects for EVERY
//! lambda and named-fn reference — including zero-capture ones — so the
//! argument is virtually never a bare `FuncRef`. The pre-fix intercept
//! gated on `is_func_ref()` and therefore never fired on real code:
//! every panic escaped `catch_unwind`, failing the whole
//! assert_panics/catch_unwind conformance surface (base/panic 20+ red).
//! Both shapes are handled now:
//!   * bare `FuncRef` (NaN-boxed function id) — zero captures;
//!   * heap closure `[ObjectHeader][func_id:u32][capture_count:u32]
//!     [captures…]` (TypeId 0xC000) — captures are seeded into the
//!     callee frame's registers 0.. exactly like `handle_call_closure`.
//!
//! ## Catchable error class (PANIC-CLASS-1)
//!
//! The panic SURFACE maps to three interpreter errors:
//!   * `Panic`            — `Instruction::Panic` (builtin `panic("…")`,
//!                          stdlib assert_* via `panic(msg)`);
//!   * `AssertionFailed`  — `Instruction::Assert` (builtin `assert`);
//!   * `Unreachable`      — `Instruction::Unreachable` (builtin
//!                          `unreachable()`).
//! All three are catchable — they ARE panics in language terms.
//! Runtime faults (NullPointerAt, TypeMismatch, FunctionNotFound,
//! StackOverflow, …) and `ProcessExit` (`exit(code)` is termination,
//! not a panic) propagate unchanged: catchability there would be a
//! language-semantics decision, not an interpreter convenience.
//!
//! ## Result shapes
//!
//!   - Normal return → `Result.Ok(return_value)`  (variant tag 0)
//!   - Caught panic  → `Result.Err(PanicInfo { message, location: None })`
//!     (tag 1; PanicInfo layout: [0] message: Text, [1] location:
//!     Maybe<Location> — declaration order in panic.vr)

use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::call_function_sync;
use super::heap_helpers::{alloc_record_n_fields, extract_text_arg, wrap_in_variant};
use super::string_helpers::alloc_string_value;
use crate::instruction::Reg;
use crate::module::FunctionId;
use crate::value::Value;

/// The panic-class subset of interpreter errors — the errors
/// `catch_unwind` converts to `Result.Err(PanicInfo)`. ONE authority
/// for "what is a panic" on the Tier-0 catch path.
fn panic_class_message(err: &InterpreterError) -> Option<String> {
    match err {
        InterpreterError::Panic { message } => Some(message.clone()),
        InterpreterError::AssertionFailed { message, .. } => Some(message.clone()),
        InterpreterError::Unreachable { .. } => Some("entered unreachable code".to_string()),
        _ => None,
    }
}

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

    // Retrieve the closure argument. Two callable shapes reach us
    // (CATCH-UNWIND-CLOSURE-1): bare FuncRef and heap closure.
    let closure_val = state
        .registers
        .get(caller_base, Reg(args_start_reg));
    let (func_id, captures): (FunctionId, Vec<Value>) = if closure_val.is_func_ref() {
        (closure_val.as_func_id(), Vec::new())
    } else if closure_val.is_ptr() && !closure_val.is_nil() {
        let base_ptr = closure_val.as_ptr::<u8>();
        // Closure discrimination: only genuine closure objects
        // (TypeId 0xC000, the id `handle_make_closure` stamps). Any
        // other heap shape falls through to normal dispatch — loud
        // downstream, never a silent misread of a non-closure object.
        // SAFETY: non-null heap pointer from a live register;
        // `ref_or_stub` returns an all-zero stub header (type_id 0)
        // for unaligned/garbage pointers, which fails the check.
        let header_tid =
            unsafe { super::super::super::heap::ObjectHeader::ref_or_stub(base_ptr) }.type_id;
        if header_tid != crate::types::TypeId(0xC000) {
            return Ok(None);
        }
        // SAFETY: verified closure object; canonical layout readers.
        let (raw_func_id, capture_count) =
            unsafe { super::super::super::heap::closure_header(base_ptr) };
        let mut caps: Vec<Value> = Vec::with_capacity(capture_count as usize);
        unsafe {
            for i in 0..capture_count as usize {
                caps.push(std::ptr::read(
                    super::super::super::heap::closure_captures_ptr(base_ptr, i),
                ));
            }
        }
        (FunctionId(raw_func_id), caps)
    } else {
        // Not a callable value — fall through to normal dispatch.
        return Ok(None);
    };

    // Snapshot both stacks so a caught panic unwinds cleanly
    // (CTOR-UNWIND discipline: frames AND register windows — popping
    // call frames without releasing register windows desyncs the
    // register file; `Registers::pop_frame` also bumps generations so
    // CBGR refs into the dead windows go stale instead of dangling).
    let saved_depth = state.call_stack.depth();
    let saved_reg_top = state.registers.top();

    // Execute the closure, seeding captures into registers 0..N — the same
    // convention `handle_call_closure` uses (captures first; `f` takes no
    // parameters). `call_function_sync` (NOT `execute_table_with_args`) is
    // the correct NESTED-execution primitive here: it preserves the caller's
    // pc/r0 so a normally-returning closure returns to `main`'s real resume
    // point instead of restarting it (T0619 — see module docs).
    let inner_result = call_function_sync(state, func_id, &captures);

    match inner_result {
        Ok(val) => {
            // Wrap in Result.Ok (tag 0).
            let ok = wrap_in_variant(state, "Result", 0, &[val])?;
            Ok(Some(ok))
        }
        Err(err) => {
            // Unwind BOTH stacks back to the pre-call snapshot before
            // building the result or re-propagating.
            while state.call_stack.depth() > saved_depth {
                if state.call_stack.pop_frame().is_err() {
                    break;
                }
            }
            if state.registers.top() > saved_reg_top {
                state.registers.pop_frame(saved_reg_top as u32);
            }

            match panic_class_message(&err) {
                Some(message) => {
                    // Build PanicInfo { message, location: None }.
                    let msg_val = alloc_string_value(state, &message)?;
                    let none_loc = wrap_in_variant(state, "Maybe", 0, &[])?;
                    let panic_info =
                        alloc_record_n_fields(state, "PanicInfo", &[msg_val, none_loc])?;

                    // Wrap in Result.Err (tag 1).
                    let err_val = wrap_in_variant(state, "Result", 1, &[panic_info])?;
                    Ok(Some(err_val))
                }
                None => {
                    // Non-panic interpreter errors (StackOverflow,
                    // ProcessExit, BadOpcode, …) are not catchable by
                    // user code — propagate them.
                    Err(err)
                }
            }
        }
    }
}

/// Try to intercept a `panic_impl(msg, file, line, column)` call
/// (PANIC-IMPL-EXIT-1, T0148).
///
/// The stdlib body (`core/base/panic.vr::panic_impl`) writes the
/// message to stderr and calls `exit_process(101)` — the right
/// behaviour for a TOP-LEVEL panic at Tier 1, but under the
/// interpreter it turned every `panic_at(...)` / `resume_unwind(...)`
/// into `InterpreterError::ProcessExit(101)`: uncatchable by
/// `catch_unwind` (a panic MUST be catchable) and lethal to the whole
/// per-file test batch (quarantine collateral for every sibling test).
/// Direct `panic("literal")` calls never reach the body — codegen
/// compiles the bare name to `Instruction::Panic` — so this intercept
/// closes exactly the qualified/indirect callers: `panic_at`,
/// `resume_unwind`, and both `panic_impl` spellings (base.panic and
/// intrinsics.control).
///
/// Raises `InterpreterError::Panic` with the same `msg at
/// file:line:column` rendering the stdlib body writes to stderr; the
/// harness prints uncaught panics, so no diagnostic is lost.
pub(in super::super) fn try_intercept_panic_impl(
    state: &mut InterpreterState,
    func_name: &str,
    args_start_reg: u16,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let bare = func_name.rsplit('.').next().unwrap_or(func_name);
    if bare != "panic_impl" {
        return Ok(None);
    }
    if arg_count != 4 {
        return Ok(None);
    }

    let msg = extract_text_arg(state, args_start_reg, caller_base);
    let file = extract_text_arg(state, args_start_reg + 1, caller_base);
    let line = state
        .registers
        .get(caller_base, Reg(args_start_reg + 2))
        .as_i64();
    let column = state
        .registers
        .get(caller_base, Reg(args_start_reg + 3))
        .as_i64();

    let message = if file.is_empty() {
        msg
    } else {
        format!("{msg} at {file}:{line}:{column}")
    };
    Err(InterpreterError::Panic { message })
}
