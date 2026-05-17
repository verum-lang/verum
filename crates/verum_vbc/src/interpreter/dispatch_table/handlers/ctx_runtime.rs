//! High-level Rust intercepts for V-LLSI context system raw intrinsics
//! (`__ctx_get_raw` / `__ctx_provide_raw` / `__ctx_end_raw`) and the
//! defer-stack family (`__defer_push_raw` / `__defer_pop_raw` /
//! `__defer_depth_raw` / `__defer_run_to_raw`).
//!
//! ## Why an intercept, not the intrinsic registry?
//!
//! These declarations live at `core/intrinsics/runtime/os.vr` and carry
//! `@intrinsic("ctx_get")` / `@intrinsic("ctx_provide")` / etc.
//! annotations.  The intrinsic registry has NO entry for these names,
//! so `lookup_intrinsic("ctx_get")` returns `None` — and the codegen
//! falls through to compile the function body.  Because the body is a
//! forward declaration (no `{ ... }` block), the codegen emits a
//! single `Return` opcode (bytecode_length = 1) so the function is
//! callable but a no-op.
//!
//! That defeats the `bytecode_length == 0` gate that
//! `try_dispatch_intrinsic_by_name` keys off, so the intrinsic name
//! arm in `calls.rs` never fires.  The intercept here mirrors the
//! pattern used by `hasher_runtime` / `char_runtime` / `file_runtime`:
//! fire BEFORE the `Call` opcode reaches the bytecode body.
//!
//! ## Architectural reuse
//!
//! All three context-stack operations route through the existing
//! `state.context_stack: ContextStack` machinery that the opcode-level
//! `CtxProvide`/`CtxGet`/`CtxEnd` (0xB0-0xB2) handlers also use.  No
//! new storage; no duplication.  The defer stack uses the
//! `state.defer_stack: Vec<(i64, i64)>` field added on the same line
//! as `context_stack` so the structural invariant is "every V-LLSI
//! scoped resource lives next to its peer on InterpreterState".
//!
//! ## What's NOT intercepted here
//!
//! `defer_execute` invocation — actually calling the registered
//! `fn(Int) -> Int` cleanup callback — is deferred to Tier-1 AOT.
//! The interpreter would need to synthesise an indirect dispatch
//! across the call-frame machinery, which crosses the intercept's
//! shape.  The stack-bookkeeping pieces (`__defer_push_raw` /
//! `__defer_pop_raw` / `__defer_depth_raw` / `__defer_run_to_raw`)
//! are all here so depth tracking is accurate; the callback chase
//! happens via AOT.

use super::super::super::error::InterpreterResult;
use super::super::super::state::InterpreterState;
use crate::instruction::Reg;
use crate::value::Value;

/// Match a (possibly-qualified) function name against an unqualified
/// stem (`__ctx_get_raw`, `__defer_push_raw`, …).  Qualified shapes
/// like `core.intrinsics.runtime.os.__ctx_get_raw` are accepted via
/// the same trailing-segment rule the other runtime intercepts use.
#[inline]
fn name_ends_with(func_name: &str, stem: &str) -> bool {
    func_name == stem
        || func_name.ends_with(&format!(".{}", stem))
        || func_name.ends_with(&format!("::{}", stem))
}

/// Read the i64 value from the caller's frame at argument index
/// `idx`.  Mirrors the helper closure inside
/// `try_dispatch_intrinsic_by_name` in `calls.rs`.
#[inline]
fn get_i64_arg(state: &InterpreterState, args_start: u16, idx: u8, caller_base: u32) -> i64 {
    state
        .registers
        .get(caller_base, Reg(args_start + u16::from(idx)))
        .as_i64()
}

/// Intercept entry point for V-LLSI context-system raw intrinsics.
///
/// Returns `Ok(Some(...))` when the call was handled; `Ok(None)` when
/// the function name did not match and the dispatcher should continue
/// to the next intercept layer.
pub(in super::super) fn try_intercept_ctx_runtime(
    state: &mut InterpreterState,
    func_name: &str,
    args_start: u16,
    args_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    // ---- Context system (`core.sys.context_ops` + V-LLSI) ----------

    if name_ends_with(func_name, "__ctx_get_raw") && args_count >= 1 {
        let type_id = get_i64_arg(state, args_start, 0, caller_base);
        let ctx_type = (type_id as u32) & 0x7fff_ffff;
        let v = state
            .context_stack
            .get(ctx_type)
            .map(|val| val.as_i64())
            .unwrap_or(0);
        return Ok(Some(Value::from_i64(v)));
    }

    if name_ends_with(func_name, "__ctx_provide_raw") && args_count >= 2 {
        let type_id = get_i64_arg(state, args_start, 0, caller_base);
        let value = get_i64_arg(state, args_start, 1, caller_base);
        let ctx_type = (type_id as u32) & 0x7fff_ffff;
        let depth = state.call_stack.depth();
        state
            .context_stack
            .provide(ctx_type, Value::from_i64(value), depth);
        return Ok(Some(Value::from_i64(0)));
    }

    if name_ends_with(func_name, "__ctx_end_raw") && args_count >= 1 {
        let type_id = get_i64_arg(state, args_start, 0, caller_base);
        let ctx_type = (type_id as u32) & 0x7fff_ffff;
        state.context_stack.end_by_type(ctx_type);
        return Ok(Some(Value::from_i64(0)));
    }

    // ---- Defer stack (`core.sys.context_ops` RAII cleanup) ---------

    if name_ends_with(func_name, "__defer_push_raw") && args_count >= 2 {
        let fn_id = get_i64_arg(state, args_start, 0, caller_base);
        let arg = get_i64_arg(state, args_start, 1, caller_base);
        state.defer_stack.push((fn_id, arg));
        return Ok(Some(Value::from_i64(0)));
    }

    if name_ends_with(func_name, "__defer_pop_raw") {
        state.defer_stack.pop();
        return Ok(Some(Value::from_i64(0)));
    }

    if name_ends_with(func_name, "__defer_depth_raw") {
        return Ok(Some(Value::from_i64(state.defer_stack.len() as i64)));
    }

    if name_ends_with(func_name, "__defer_run_to_raw") && args_count >= 1 {
        let target_depth = get_i64_arg(state, args_start, 0, caller_base);
        let target = if target_depth < 0 {
            0
        } else {
            target_depth as usize
        };
        while state.defer_stack.len() > target {
            state.defer_stack.pop();
        }
        return Ok(Some(Value::from_i64(0)));
    }

    Ok(None)
}
