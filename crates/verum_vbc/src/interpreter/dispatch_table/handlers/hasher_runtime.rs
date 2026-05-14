//! High-level Rust intercepts for `core.base.protocols.DefaultHasher`.
//!
//! The user-side stdlib body for `Hasher.write(&mut self, bytes: &[Byte])`
//! performs `self.state = …` SetF on a `&mut self` whose method has a
//! slice argument — that SetF doesn't persist to the caller's frame on
//! the Tier-0 interpreter path (the slice argument's register allocation
//! collides with `self`'s CBGR-ref, so the writeback writes back to a
//! stale snapshot).  This is a separate architectural defect tracked
//! as task #11.
//!
//! Until that defect closes, we sidestep the broken body by running the
//! canonical FxHash 64-bit step directly on the `DefaultHasher` heap
//! record (`[ObjectHeader][state: Value(i64)]`).  Bit-equivalent to the
//! user-side body, observable behaviour identical at both tiers.
//!
//! # Methods intercepted
//!  * `DefaultHasher.write(&mut self, bytes: &[Byte])`
//!  * `DefaultHasher.write_byte(&mut self, byte: Byte)`
//!  * `DefaultHasher.write_int(&mut self, n: Int)`
//!  * `DefaultHasher.finish(&self) -> Int`
//!
//! All four mutate / read field 0 of the heap record in place, so the
//! caller's `&mut self` ref needs no separate writeback.

use super::super::super::error::InterpreterResult;
use super::super::super::heap;
use super::super::super::state::InterpreterState;
use super::cbgr_helpers::{decode_cbgr_ref, is_cbgr_ref};
use super::method_dispatch::fxhash_bytes;
use crate::instruction::Reg;
use crate::value::Value;

/// Try to intercept a DefaultHasher instance-method call by qualified
/// name.  Returns `Some(value)` when the interception fires, `None`
/// otherwise.  `func_name` is the canonical qualified form
/// (`DefaultHasher.write`, `core.base.protocols.DefaultHasher.finish`, …);
/// names that don't end in `DefaultHasher.<method>` short-circuit.
pub(in super::super) fn try_intercept_default_hasher(
    state: &mut InterpreterState,
    func_name: &str,
    args_start_reg: u16,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    // Accept both bare (`DefaultHasher.<method>`) and module-prefixed
    // (`core.base.protocols.DefaultHasher.<method>`) forms.
    let method = if let Some(idx) = func_name.rfind(".DefaultHasher.") {
        &func_name[idx + ".DefaultHasher.".len()..]
    } else if let Some(m) = func_name.strip_prefix("DefaultHasher.") {
        m
    } else {
        return Ok(None);
    };

    if arg_count == 0 {
        // `DefaultHasher.new()` — leave for the stdlib body.  Its
        // implementation is `DefaultHasher { state: 0 }`, which the
        // codegen handles correctly.
        return Ok(None);
    }

    // arg[0] is `self` (or `&self` / `&mut self`).  Deref CBGR refs +
    // ThinRefs so we land on the heap-record Value.
    let self_raw = state.registers.get(caller_base, Reg(args_start_reg));
    let self_val = deref_self(state, self_raw);

    // The DefaultHasher record is allocated as a single-field heap
    // object `[ObjectHeader][state: Value(i64)]`.  If we don't see a
    // heap pointer, the receiver isn't a DefaultHasher value — let the
    // stdlib body handle it (it can't fail any worse than the current
    // state).
    if !self_val.is_ptr() || self_val.is_nil() {
        return Ok(None);
    }
    let base = self_val.as_ptr::<u8>();
    if base.is_null()
        || !(base as usize).is_multiple_of(std::mem::align_of::<heap::ObjectHeader>())
    {
        return Ok(None);
    }
    let header = unsafe { heap::ObjectHeader::ref_or_stub(base) };
    if header.size as usize != std::mem::size_of::<Value>() {
        return Ok(None);
    }
    let state_ptr = unsafe { base.add(heap::OBJECT_HEADER_SIZE) as *mut Value };
    let mut state_i64 = unsafe { (*state_ptr).as_i64() };

    match method {
        "write" if arg_count == 2 => {
            // arg[1] is `&[Byte]` (typically FatRef).  Auto-deref
            // CBGR-ref / ThinRef first.
            let raw = state.registers.get(caller_base, Reg(args_start_reg + 1));
            let bytes_val = deref_self(state, raw);
            let bytes: Vec<u8> = if bytes_val.is_fat_ref() {
                let fr = bytes_val.as_fat_ref();
                let p = fr.ptr();
                let n = fr.len() as usize;
                if !p.is_null() && n > 0 && n <= 1_000_000 {
                    unsafe { std::slice::from_raw_parts(p, n) }.to_vec()
                } else {
                    Vec::new()
                }
            } else if bytes_val.is_ptr() && !bytes_val.is_nil() {
                // Heap-string Text view: `[hdr][len:u64][bytes…]`.
                let p = bytes_val.as_ptr::<u8>();
                if p.is_null()
                    || !(p as usize)
                        .is_multiple_of(std::mem::align_of::<heap::ObjectHeader>())
                {
                    Vec::new()
                } else {
                    let h = unsafe { heap::ObjectHeader::ref_or_stub(p) };
                    if h.type_id == crate::types::TypeId::TEXT
                        || h.type_id == crate::types::TypeId(0x0001)
                    {
                        let data = unsafe { p.add(heap::OBJECT_HEADER_SIZE) };
                        let len = unsafe { *(data as *const u64) } as usize;
                        let bytes_ptr = unsafe { data.add(8) };
                        if len > 0 && len <= 1_000_000 {
                            unsafe { std::slice::from_raw_parts(bytes_ptr, len) }
                                .to_vec()
                        } else {
                            Vec::new()
                        }
                    } else {
                        Vec::new()
                    }
                }
            } else {
                Vec::new()
            };
            state_i64 = fxhash_bytes(state_i64, &bytes);
            unsafe { *state_ptr = Value::from_i64(state_i64) };
            Ok(Some(Value::unit()))
        }
        "write_byte" if arg_count == 2 => {
            let b_arg = state.registers.get(caller_base, Reg(args_start_reg + 1));
            let b: u8 = if b_arg.is_int() {
                (b_arg.as_i64() & 0xFF) as u8
            } else {
                0
            };
            state_i64 = fxhash_bytes(state_i64, &[b]);
            unsafe { *state_ptr = Value::from_i64(state_i64) };
            Ok(Some(Value::unit()))
        }
        "write_int" if arg_count == 2 => {
            let n_arg = state.registers.get(caller_base, Reg(args_start_reg + 1));
            let n: i64 = if n_arg.is_int() { n_arg.as_i64() } else { 0 };
            state_i64 = fxhash_bytes(state_i64, &n.to_le_bytes());
            unsafe { *state_ptr = Value::from_i64(state_i64) };
            Ok(Some(Value::unit()))
        }
        "finish" if arg_count == 1 => Ok(Some(Value::from_i64(state_i64))),
        _ => Ok(None),
    }
}

/// Auto-deref CBGR ref / ThinRef.  Used for both `self` and slice args.
#[inline]
fn deref_self(state: &InterpreterState, v: Value) -> Value {
    if is_cbgr_ref(&v) {
        let (abs, _) = decode_cbgr_ref(v.as_i64());
        return state.registers.get_absolute(abs);
    }
    if v.is_thin_ref() {
        let tr = v.as_thin_ref();
        if !tr.ptr.is_null() {
            return unsafe { *(tr.ptr as *const Value) };
        }
    }
    v
}
