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

// =====================================================================
// `Char.encode_utf8` / `Char.encode_utf16` CallM-path intercept
// (audit text/text §B)
//
// Closes the architectural class where a `ch.encode_utf8(&mut buf)`
// call dispatched from a Verum body (e.g. `Text.insert`) reaches
// `handle_call_method` with `receiver_kind = Int` — Char is NaN-boxed
// into `Value::Int` for storage, but the CallM lookup keyed on the
// receiver's runtime kind has no entry for "Char" → fails with
// `method 'Char.encode_utf8' not found on receiver of runtime kind
// 'Int'`.  The user-side body in `core/text/char.vr::encode_utf8`
// (lines 296-322) exists and is correct; the failure is purely in
// the dispatch path's inability to recognise an `Int`-shaped Char.
//
// Fix shape: a Tier-0 intercept that consumes the Char codepoint
// from the receiver and writes the UTF-8 / UTF-16 bytes into the
// caller's buffer through the canonical three-shape resolver
// (BYTE_LIST / LIST / direct byte array, plus ThinRef).  Mirrors the
// existing `CharSubOpcode::EncodeUtf8` intrinsic handler in
// `char_extended.rs` — the encoding logic is identical, only the
// register-decoding wrapper differs.
//
// The intercept is a no-op (returns `None`) when the receiver is
// not Int-shaped — e.g. when the dispatch already resolved via the
// intrinsic-opcode path or when the receiver is some other type
// with an unrelated `encode_utf8` method.
//
// # Returns
//
// `Some(Value::from_i64(n_bytes))` on hit; `None` to defer to the
// regular dispatch path (compiled body / "method not found").
pub(in super::super) fn try_intercept_char_encode(
    state: &mut InterpreterState,
    bare_method_name: &str,
    receiver: Value,
    buf_reg: Reg,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let kind = match bare_method_name {
        "encode_utf8" => EncodeKind::Utf8,
        "encode_utf16" => EncodeKind::Utf16,
        _ => return Ok(None),
    };

    // Receiver must be an Int-shaped Char NaN-box.  Auto-deref one
    // level of CBGR-ref / ThinRef so that `(&ch).encode_utf8(...)`
    // also reaches us — same pattern as the existing intercept
    // siblings.  `resolve_arg_value` collapses all three reference
    // shapes; for a non-reference value it's a no-op.
    let receiver_resolved = super::cbgr_helpers::resolve_arg_value(state, receiver);
    let codepoint = if receiver_resolved.is_int() {
        receiver_resolved.as_i64()
    } else if is_cbgr_ref(&receiver_resolved) {
        let (abs_index, _) = decode_cbgr_ref(receiver_resolved.as_i64());
        let inner = state.registers.get_absolute(abs_index);
        if !inner.is_int() {
            return Ok(None);
        }
        inner.as_i64()
    } else {
        return Ok(None);
    };

    let c = match char::from_u32(codepoint as u32) {
        Some(c) => c,
        None => return Ok(None),
    };

    // Resolve the buf argument through the canonical helper so all
    // three Verum reference shapes (CBGR register-ref, heap-interior
    // pointer, ThinRef) collapse to the underlying heap value first.
    let buf_val_raw = state.registers.get(caller_base, buf_reg);
    let buf_val = super::cbgr_helpers::resolve_arg_value(state, buf_val_raw);

    match kind {
        EncodeKind::Utf8 => {
            let mut tmp = [0u8; 4];
            let encoded = c.encode_utf8(&mut tmp);
            let n_bytes = encoded.len();
            write_utf8_bytes_to_buf(buf_val, &tmp, n_bytes);
            state.cbgr_epoch = state.cbgr_epoch.wrapping_add(1);
            Ok(Some(Value::from_i64(n_bytes as i64)))
        }
        EncodeKind::Utf16 => {
            let mut tmp = [0u16; 2];
            let encoded = c.encode_utf16(&mut tmp);
            let n_units = encoded.len();
            write_utf16_units_to_buf(buf_val, &tmp, n_units);
            state.cbgr_epoch = state.cbgr_epoch.wrapping_add(1);
            Ok(Some(Value::from_i64(n_units as i64)))
        }
    }
}

#[derive(Clone, Copy)]
enum EncodeKind {
    Utf8,
    Utf16,
}

fn write_utf8_bytes_to_buf(buf_val: Value, src: &[u8; 4], n_bytes: usize) {
    if !buf_val.is_ptr() || buf_val.is_nil() {
        if buf_val.is_thin_ref() {
            let thin = buf_val.as_thin_ref();
            if !thin.ptr.is_null() {
                for (i, b) in src.iter().enumerate().take(n_bytes) {
                    unsafe {
                        *thin.ptr.add(i) = *b;
                    }
                }
            }
        }
        return;
    }

    let buf_ptr = buf_val.as_ptr::<u8>();
    let header =
        unsafe { super::super::super::heap::ObjectHeader::ref_or_stub(buf_ptr) };
    let header_data = unsafe {
        buf_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE) as *mut Value
    };
    if header.type_id == crate::types::TypeId::BYTE_LIST {
        let backing_ptr = unsafe { (*header_data.add(2)).as_ptr::<u8>() };
        let dst_bytes = unsafe {
            backing_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE) as *mut u8
        };
        for (i, b) in src.iter().enumerate().take(n_bytes) {
            unsafe {
                *dst_bytes.add(i) = *b;
            }
        }
    } else if header.type_id == crate::types::TypeId::LIST {
        let backing_ptr = unsafe { (*header_data.add(2)).as_ptr::<u8>() };
        let dst_vals = unsafe {
            backing_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE)
                as *mut Value
        };
        for (i, b) in src.iter().enumerate().take(n_bytes) {
            unsafe {
                *dst_vals.add(i) = Value::from_i64(*b as i64);
            }
        }
    } else {
        let dst_bytes = unsafe {
            buf_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE) as *mut u8
        };
        for (i, b) in src.iter().enumerate().take(n_bytes) {
            unsafe {
                *dst_bytes.add(i) = *b;
            }
        }
    }
}

fn write_utf16_units_to_buf(buf_val: Value, src: &[u16; 2], n_units: usize) {
    if !buf_val.is_ptr() || buf_val.is_nil() {
        if buf_val.is_thin_ref() {
            let thin = buf_val.as_thin_ref();
            if !thin.ptr.is_null() {
                for (i, u) in src.iter().enumerate().take(n_units) {
                    unsafe {
                        *(thin.ptr as *mut u16).add(i) = *u;
                    }
                }
            }
        }
        return;
    }

    let buf_ptr = buf_val.as_ptr::<u8>();
    let header =
        unsafe { super::super::super::heap::ObjectHeader::ref_or_stub(buf_ptr) };
    let header_data = unsafe {
        buf_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE) as *mut Value
    };
    if header.type_id == crate::types::TypeId::LIST {
        // Verum-side `&mut [Int]`: NaN-boxed Value per element.
        let backing_ptr = unsafe { (*header_data.add(2)).as_ptr::<u8>() };
        let dst_vals = unsafe {
            backing_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE)
                as *mut Value
        };
        for (i, u) in src.iter().enumerate().take(n_units) {
            unsafe {
                *dst_vals.add(i) = Value::from_i64(*u as i64);
            }
        }
    } else {
        // Fixed-size [Int; 2] array — payload is Value-per-elem.
        let dst_vals = unsafe {
            buf_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE) as *mut Value
        };
        for (i, u) in src.iter().enumerate().take(n_units) {
            unsafe {
                *dst_vals.add(i) = Value::from_i64(*u as i64);
            }
        }
    }
}
