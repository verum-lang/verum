//! Context system and capability operation handlers for VBC interpreter dispatch.
//!
//! Handles: CtxGet (0xB0), CtxProvide (0xB1), CtxEnd (0xB2), PushContext (0xB3),
//! PopContext (0xB4), Attenuate (0xB5), HasCapability (0xB6), RequireCapability (0xB7)
//!
//! Verum context system: capability-based dependency injection via `using [Ctx]` / `provide`.
//! Contexts are runtime-varying dependencies stored in task-local storage (theta). Functions
//! declare required contexts with `using [Logger, Database]`; providers are installed with
//! `provide Context = impl`. Lookup cost is ~5-30ns via vtable in task-local context stack.
//! This is NOT algebraic effects -- it is pure dependency injection with lexical scoping.

use crate::value::Value;
use super::super::super::error::{CbgrViolationKind, InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::*;
use super::cbgr_helpers::{is_cbgr_ref, is_cbgr_ref_mutable, decode_cbgr_ref, strip_cbgr_ref_mutability};
use verum_common::cbgr::caps;

// ============================================================================
// Context System Operations (0xB0-0xB4)
// ============================================================================

/// CtxGet (0xB0) - Retrieve a context value by type.
///
/// Panics with `Context X not provided` when the context is missing.
/// Accessing a context that was never provided is a programming error —
/// the caller's `using [X]` clause is a hard requirement, and the
/// previous fallback to `nil` silently propagated a null through the
/// next method call, producing a much less informative
/// `NullPointerDereference` downstream. The panic carries the actual
/// context name so `@expected-panic: Context X not provided` tests
/// can match against it.
pub(in super::super) fn handle_ctx_get(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let ctx_type = read_varint(state)? as u32;

    // Look up the context value from the stack
    match state.context_stack.get(ctx_type) {
        Some(value) => {
            state.set_reg(dst, value);
        }
        None => {
            let ctx_name = state.module.strings.get(crate::types::StringId(ctx_type))
                .unwrap_or("unknown");
            return Err(InterpreterError::Panic {
                message: format!("Context {} not provided", ctx_name),
            });
        }
    }

    Ok(DispatchResult::Continue)
}

/// CtxProvide (0xB1) - Provide a context value for a scope.
pub(in super::super) fn handle_ctx_provide(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let ctx_type = read_varint(state)? as u32;
    let value_reg = read_reg(state)?;
    let _body_offset = read_signed_varint(state)?; // Reserved for future use

    let value = state.get_reg(value_reg);
    let stack_depth = state.call_stack.depth();
    state.context_stack.provide(ctx_type, value, stack_depth);

    Ok(DispatchResult::Continue)
}

/// CtxEnd (0xB2) / PopContext (0xB4) - End a context scope.
///
/// Pops the most recently-provided context entry. CtxProvide + CtxEnd come
/// in matched pairs for `provide X = v in { body }` blocks, so a LIFO pop
/// correctly handles both flat and nested provides.
pub(in super::super) fn handle_ctx_pop(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    state.context_stack.pop_one();
    Ok(DispatchResult::Continue)
}

/// PushContext (0xB3) - Push a context handler (for advanced context patterns).
///
/// In the simple implementation, this is a no-op.
pub(in super::super) fn handle_ctx_push(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let _handler_offset = read_signed_varint(state)?;
    Ok(DispatchResult::Continue)
}

// ============================================================================
// Capability Operations (0xB5-0xB7)
// ============================================================================

/// Attenuate (0xB5) - Attenuate reference capabilities.
pub(in super::super) fn handle_attenuate(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let mask = read_varint(state)? as u32;

    let src_val = state.get_reg(src);

    if is_cbgr_ref(&src_val) {
        let can_write = (mask & caps::WRITE) != 0;
        if can_write {
            state.set_reg(dst, src_val);
        } else {
            let attenuated = strip_cbgr_ref_mutability(src_val.as_i64());
            state.set_reg(dst, Value::from_i64(attenuated));
        }
    } else if src_val.is_ptr() && !src_val.is_nil() {
        let ptr_addr = src_val.as_ptr::<u8>() as usize;
        let can_write = (mask & caps::WRITE) != 0;
        if !can_write {
            state.cbgr_mutable_ptrs.remove(&ptr_addr);
        }
        state.set_reg(dst, src_val);
    } else {
        state.set_reg(dst, src_val);
    }

    Ok(DispatchResult::Continue)
}

/// HasCapability (0xB6) - Check if reference has specific capability.
pub(in super::super) fn handle_has_capability(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let ref_reg = read_reg(state)?;
    let cap_mask = read_varint(state)? as u32;

    let ref_val = state.get_reg(ref_reg);

    let has_all_caps = if is_cbgr_ref(&ref_val) {
        let is_mut = is_cbgr_ref_mutable(ref_val.as_i64());
        check_capabilities_for_mutability(cap_mask, is_mut)
    } else if ref_val.is_ptr() && !ref_val.is_nil() {
        let ptr_addr = ref_val.as_ptr::<u8>() as usize;
        let is_mut = state.cbgr_mutable_ptrs.contains(&ptr_addr);
        check_capabilities_for_mutability(cap_mask, is_mut)
    } else { !ref_val.is_nil() };

    state.set_reg(dst, Value::from_bool(has_all_caps));
    Ok(DispatchResult::Continue)
}

/// Check if all requested capabilities are satisfied given mutability status.
#[inline(always)]
pub(in super::super) fn check_capabilities_for_mutability(cap_mask: u32, is_mut: bool) -> bool {
    if (cap_mask & caps::WRITE) != 0 && !is_mut {
        return false;
    }
    if (cap_mask & caps::MUTABLE) != 0 && !is_mut {
        return false;
    }
    if (cap_mask & caps::REVOKE) != 0 && !is_mut {
        return false;
    }
    true
}

/// RequireCapability (0xB7) - Require capability, panic if not present.
pub(in super::super) fn handle_require_capability(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let ref_reg = read_reg(state)?;
    let cap_mask = read_varint(state)? as u32;

    let ref_val = state.get_reg(ref_reg);

    let (has_caps, ptr_addr) = if is_cbgr_ref(&ref_val) {
        let is_mut = is_cbgr_ref_mutable(ref_val.as_i64());
        let (abs_index, _) = decode_cbgr_ref(ref_val.as_i64());
        (check_capabilities_for_mutability(cap_mask, is_mut), abs_index as usize)
    } else if ref_val.is_ptr() && !ref_val.is_nil() {
        let ptr_addr = ref_val.as_ptr::<u8>() as usize;
        let is_mut = state.cbgr_mutable_ptrs.contains(&ptr_addr);
        (check_capabilities_for_mutability(cap_mask, is_mut), ptr_addr)
    } else if ref_val.is_nil() {
        (false, 0)
    } else {
        (true, 0)
    };

    if !has_caps {
        return Err(InterpreterError::CbgrViolation {
            kind: CbgrViolationKind::CapabilityDenied,
            ptr: ptr_addr,
        });
    }

    Ok(DispatchResult::Continue)
}
