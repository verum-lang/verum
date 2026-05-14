//! High-level Rust intercepts for `core.text.char.Char` `&mut self`
//! mutators.
//!
//! The user-side stdlib body for `Char.make_ascii_uppercase` /
//! `make_ascii_lowercase` is `*self = self.to_ascii_uppercase();` —
//! a DerefMut-through-`&mut self` assignment that should propagate
//! the new Char back to the caller's slot.  On the Tier-0
//! interpreter path the precompiled body fails to emit a
//! `DerefMut` (verified via diagnostic trace `[deref-mut-rt]`
//! never firing for `c.make_ascii_uppercase()`), so the mutation
//! is silently lost.  Same architectural class as the
//! `DefaultHasher.write` defect closed by `hasher_runtime` — a
//! Tier-0 intercept that operates directly on the caller's slot
//! via the CBGR-ref writeback discipline is the canonical fix
//! until the body-side codegen is repaired.
//!
//! The call site emits `RefMut(ref_reg, c_reg)` so the receiver
//! arrives at the call as a CBGR ref to the caller's Char slot —
//! the intercept decodes the ref, computes the new Char, writes
//! it through `state.registers.set_absolute`.
//!
//! # Methods intercepted
//!  * `Char.make_ascii_uppercase(&mut self)`
//!  * `Char.make_ascii_lowercase(&mut self)`

use super::super::super::error::InterpreterResult;
use super::super::super::state::InterpreterState;
use super::cbgr_helpers::{decode_cbgr_ref, is_cbgr_ref};
use crate::instruction::Reg;
use crate::value::Value;

/// Try to intercept a `Char.<method>` `&mut self` call.  Returns
/// `Some(Value::unit())` when the interception fires (the mutation
/// is applied in place via `set_absolute`), `None` otherwise.
pub(in super::super) fn try_intercept_char_mutator(
    state: &mut InterpreterState,
    func_name: &str,
    args_start_reg: u16,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    // Accept both bare (`Char.<method>`) and module-prefixed
    // (`core.text.char.Char.<method>`) forms.
    let method = if let Some(idx) = func_name.rfind(".Char.") {
        &func_name[idx + ".Char.".len()..]
    } else if let Some(m) = func_name.strip_prefix("Char.") {
        m
    } else {
        return Ok(None);
    };

    // Only the in-place ASCII case mutators are handled here.  The
    // pure-conversion `to_ascii_uppercase` / `to_ascii_lowercase`
    // surfaces work correctly through the user-side body (they're
    // by-value, no DerefMut involved).
    let kind = match method {
        "make_ascii_uppercase" if arg_count == 1 => CaseKind::Upper,
        "make_ascii_lowercase" if arg_count == 1 => CaseKind::Lower,
        _ => return Ok(None),
    };

    // arg[0] is `self` (CBGR ref to caller's Char slot, courtesy of
    // the call-site `RefMut` emitted when `takes_self_mut_ref` is
    // set).  Decode the ref to find the absolute register holding
    // the caller's Char value.
    let self_raw = state.registers.get(caller_base, Reg(args_start_reg));
    let (abs_index, current_val) = if is_cbgr_ref(&self_raw) {
        let (abs, _) = decode_cbgr_ref(self_raw.as_i64());
        (Some(abs), state.registers.get_absolute(abs))
    } else if self_raw.is_thin_ref() {
        // ThinRef writeback isn't a register slot — fall through.
        // The body's `*self = X` would need a heap-pointer write
        // which the canonical DerefMut handler already supports.
        return Ok(None);
    } else if self_raw.is_int() {
        // The CallM-path intercept passes receiver_reg DIRECTLY
        // (not via RefMut wrapping) — bare Char-as-Int Value.
        // Writeback target is the receiver's register itself in
        // the caller's frame.
        (None, self_raw)
    } else {
        return Ok(None);
    };

    // Char is NaN-boxed as Int (the runtime collapses Char into the
    // Int slot for storage).  Extract the codepoint, apply the
    // ASCII case fold, write back.
    let codepoint = if current_val.is_int() {
        current_val.as_i64()
    } else {
        // Receiver isn't an Int-shaped Char — let the body run
        // (it'll surface its own error if applicable).
        return Ok(None);
    };

    let new_codepoint = match kind {
        CaseKind::Upper => {
            if (97..=122).contains(&codepoint) {
                codepoint - 32
            } else {
                codepoint
            }
        }
        CaseKind::Lower => {
            if (65..=90).contains(&codepoint) {
                codepoint + 32
            } else {
                codepoint
            }
        }
    };

    if let Some(abs) = abs_index {
        state.registers.set_absolute(abs, Value::from_i64(new_codepoint));
        state.cbgr_epoch = state.cbgr_epoch.wrapping_add(1);
    } else {
        // CallM path: writeback to the receiver's caller-frame slot.
        // The caller passed receiver_reg as the "args_start_reg"
        // (see method_dispatch.rs).  Update it in place.
        state
            .registers
            .set(caller_base, Reg(args_start_reg), Value::from_i64(new_codepoint));
    }

    Ok(Some(Value::unit()))
}

#[derive(Clone, Copy)]
enum CaseKind {
    Upper,
    Lower,
}
