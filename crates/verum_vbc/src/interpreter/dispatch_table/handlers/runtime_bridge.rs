//! Verum runtime-bridge intercepts for the VBC interpreter (Tier 0).
//!
//! Backs the `verum_get_runtime_*` getter family declared as `extern fn`
//! in `core/async/executor.vr`. Under AOT (Tier 1) these getters are
//! emitted as LLVM-constant-folded reads from `__verum_runtime_*`
//! globals (see `crates/verum_codegen/src/llvm/platform_ir.rs::
//! emit_runtime_bridge_getters`). Under interpreter mode the symbols
//! are bodyless `extern fn` declarations — `dlsym` looks them up in
//! the host process and they don't exist, causing the FFI dispatch
//! to fail with "symbol not found".
//!
//! The interpreter intercept gives them sensible defaults so any
//! runtime-bridge consumer (`AsyncRuntimeConfig.default()`,
//! `AsyncRuntime.with_config(...)`, etc.) works unchanged in
//! interpreter mode without manifest input. AOT remains the
//! authoritative path for manifest-driven values.
//!
//! ## Intercept policy
//!
//! Both getters return `0` (the documented sentinel for "use the
//! built-in default"):
//!   * `verum_get_runtime_async_worker_threads()` — `0` keeps the
//!     scheduler in single-threaded mode (Phase 1A foundation
//!     contract: zero-overhead default).
//!   * `verum_get_runtime_task_stack_size()` — `0` keeps the
//!     1 MiB-default task stack documented in
//!     `AsyncRuntimeConfig.default()`.
//!
//! Both defaults align bit-for-bit with what AOT-with-no-manifest
//! produces, so a program that builds AND runs in both tiers
//! observes identical behaviour at this layer.

use crate::interpreter::error::InterpreterResult;
use crate::interpreter::state::InterpreterState;
use crate::value::Value;

/// Try to intercept a `verum_get_runtime_*` getter call. Returns
/// `Some(Value)` when the call is recognised (the caller should
/// install the result into `dst` and continue), `None` when it
/// isn't (the caller falls through to FFI dispatch).
///
/// All getters in this family take **zero arguments** and return
/// `Int`. `_state` is unused for now — these are pure constants in
/// interpreter mode — but the parameter is kept to match the
/// sibling intercept-fn signatures (`try_intercept_env_runtime`
/// etc.) so the calls.rs orchestration is uniform.
pub(in super::super) fn try_intercept_runtime_bridge(
    _state: &mut InterpreterState,
    func_name: &str,
    _args_start_reg: u16,
    arg_count: u8,
    _caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    if arg_count != 0 {
        return Ok(None);
    }
    let bare = func_name.rsplit('.').next().unwrap_or(func_name);
    match bare {
        "verum_get_runtime_async_worker_threads"
        | "verum_get_runtime_task_stack_size" => {
            // Both default to 0 — the documented "use the built-in
            // default" sentinel. AOT replaces these with manifest
            // values via `__verum_runtime_*` globals; the interpreter
            // ships the manifest-default behaviour.
            Ok(Some(Value::from_i64(0)))
        }
        _ => Ok(None),
    }
}
