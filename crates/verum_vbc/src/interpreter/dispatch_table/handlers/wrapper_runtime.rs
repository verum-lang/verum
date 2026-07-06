//! High-level Rust intercepts for the allocating-wrapper surface
//! (`Heap<T>` / `Shared<T>`) over their Tier-0 runtime representations.
//!
//! The interpreter does NOT materialise the source-level wrapper records:
//!
//!  * `Heap.new(v)` produces a **CBGR data pointer** — the address just
//!    past a 32-byte `AllocationHeader`, tracked in
//!    `state.cbgr_allocations` (see the `Heap.new` intrinsic path in
//!    `method_dispatch.rs`).  There is no `Heap { ptr, generation,
//!    epoch }` record at runtime.
//!  * `Shared.new(v)` produces a `TypeId::SHARED` heap object with the
//!    layout `[ObjectHeader][refcount: i64][value: Value]`.  There is no
//!    `SharedInner { strong_count, weak_count, value }` record.
//!
//! The COMPILED stdlib bodies (`core/base/memory.vr`) read the
//! source-level layouts (`self.ptr`, `(*self.ptr).strong_count`), so
//! letting a wrapper method reach its compiled body against the runtime
//! repr mis-reads memory — `Heap.into_raw`'s `self.ptr` field access
//! surfaces the stored payload as a bogus `type_id` ("field access out
//! of bounds … type_id=<the value>"), and `Shared.strong_count` dies in
//! dispatch ("method not found … runtime kind Object").
//!
//! `method_dispatch.rs` already intercepts these names on the `CallM`
//! (runtime-dispatch) path.  This module is the **`Call` (statically
//! resolved) twin**: when codegen pre-resolves `boxed.into_raw()` to a
//! direct `Call Heap.into_raw [recv]`, the receiver arrives as arg 0
//! and never passes through `CallM` dispatch.  Same architectural
//! pattern as `hasher_runtime` / `char_runtime` — qualified-name
//! intercepts at the call boundary.
//!
//! Every intercept is **shape-guarded**: it only fires when the
//! receiver actually carries the runtime repr (a tracked CBGR data
//! pointer / a `TypeId::SHARED` object).  A genuine source-level record
//! (e.g. constructed inside stdlib internals) falls through to the
//! compiled body untouched.
//!
//! # Functions intercepted
//!  * `Heap.into_raw(self) -> &unsafe T`   — identity on the data ptr
//!    (`forget(self)` semantics: no dealloc, no generation bump).
//!  * `Heap.from_raw(ptr) -> Heap<T>`      — identity on the data ptr.
//!  * `Shared.strong_count(&self) -> Int`  — refcount slot read.
//!  * `Shared.weak_count(&self) -> Int`    — 0 (repr tracks no weaks).
//!  * `Shared.is_unique(&self) -> Bool`    — refcount == 1.
//!
//! Pinned by `core-tests/mem/allocator/integration_test.vr` §4/§5
//! (HEAP-INTORAW-1 / SHARED-STRONGCOUNT-1).

use super::super::super::error::InterpreterResult;
use super::super::super::heap;
use super::super::super::state::InterpreterState;
use super::cbgr_helpers::{decode_cbgr_ref, is_cbgr_ref};
use crate::instruction::Reg;
use crate::types::TypeId;
use crate::value::Value;

/// Strip a qualified function name down to `<Type>.<method>` when the
/// type segment matches, accepting both bare (`Heap.into_raw`) and
/// module-prefixed (`core.base.memory.Heap.into_raw`) forms.
fn method_of<'n>(func_name: &'n str, type_seg: &str) -> Option<&'n str> {
    let dotted = format!(".{}.", type_seg);
    if let Some(idx) = func_name.rfind(&dotted) {
        return Some(&func_name[idx + dotted.len()..]);
    }
    let bare = format!("{}.", type_seg);
    func_name.strip_prefix(bare.as_str())
}

/// Fetch an argument register, auto-dereffing CBGR register refs so a
/// `&self` receiver reaches the underlying wrapper value.
fn arg_value(state: &InterpreterState, caller_base: u32, args_start: u16, idx: u16) -> Value {
    let raw = state
        .registers
        .get(caller_base, Reg(args_start + idx));
    if is_cbgr_ref(&raw) {
        let (abs_index, _gen) = decode_cbgr_ref(raw.as_i64());
        state.registers.get_absolute(abs_index)
    } else {
        raw
    }
}

/// Returns `true` when `v` is a tracked CBGR data pointer (the runtime
/// repr of `Heap<T>`).
fn is_cbgr_data_ptr(state: &InterpreterState, v: &Value) -> bool {
    if !v.is_ptr() || v.is_nil() {
        return false;
    }
    let data_ptr = v.as_ptr::<u8>() as usize;
    state
        .cbgr_allocations
        .contains(&data_ptr.wrapping_sub(verum_common::layout::ALLOCATION_HEADER_SIZE as usize))
}

/// Returns the refcount-slot pointer when `v` is a `TypeId::SHARED`
/// object (the runtime repr of `Shared<T>`), else `None`.
fn shared_refcount_ptr(v: &Value) -> Option<*mut Value> {
    if !v.is_ptr() || v.is_nil() {
        return None;
    }
    let ptr = v.as_ptr::<u8>();
    if ptr.is_null()
        || !(ptr as usize).is_multiple_of(std::mem::align_of::<heap::ObjectHeader>())
    {
        return None;
    }
    // SAFETY: alignment verified; every heap object begins with an
    // ObjectHeader.
    let header = unsafe { heap::ObjectHeader::ref_or_stub(ptr) };
    if header.type_id != TypeId::SHARED {
        return None;
    }
    Some(unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value })
}

/// Try to intercept a statically-resolved wrapper-method `Call` by
/// qualified name.  Returns `Some(value)` when the interception fires.
pub(in super::super) fn try_intercept_wrapper_call(
    state: &mut InterpreterState,
    func_name: &str,
    args_start_reg: u16,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    if let Some(method) = method_of(func_name, "Heap") {
        match method {
            "into_raw" | "from_raw" if arg_count == 1 => {
                let v = arg_value(state, caller_base, args_start_reg, 0);
                if is_cbgr_data_ptr(state, &v) {
                    // Identity on the pointer bits — see module doc.
                    return Ok(Some(v));
                }
            }
            _ => {}
        }
        return Ok(None);
    }
    if let Some(method) = method_of(func_name, "Shared") {
        if arg_count == 1 {
            let v = arg_value(state, caller_base, args_start_reg, 0);
            if let Some(rc_ptr) = shared_refcount_ptr(&v) {
                match method {
                    "strong_count" => {
                        // SAFETY: rc_ptr derives from a validated SHARED
                        // object; slot 0 is the refcount Value.
                        let rc = unsafe { (*rc_ptr).as_i64() };
                        return Ok(Some(Value::from_i64(rc)));
                    }
                    "weak_count" => return Ok(Some(Value::from_i64(0))),
                    "is_unique" => {
                        let rc = unsafe { (*rc_ptr).as_i64() };
                        return Ok(Some(Value::from_bool(rc == 1)));
                    }
                    // Statically-resolved `Shared.clone` — bump the
                    // strong count and return the same pointer (the
                    // CallM twin lives in method_dispatch.rs; a call
                    // site that pre-resolved to `Call Shared.clone`
                    // otherwise reaches the compiled body, which
                    // reads the nonexistent SharedInner layout and
                    // leaves the count untouched).
                    "clone" => {
                        let rc = unsafe { (*rc_ptr).as_i64() };
                        // SAFETY: same validated SHARED object.
                        unsafe { *rc_ptr = Value::from_i64(rc + 1) };
                        return Ok(Some(v));
                    }
                    // `borrow` / `borrow_mut` — inner value lives at
                    // slot 1 (single-threaded Tier 0: no borrow
                    // tracking, mirrors the CallM twin).
                    "borrow" | "borrow_mut" => {
                        // SAFETY: slot 1 of the validated SHARED object.
                        let inner = unsafe { *rc_ptr.add(1) };
                        return Ok(Some(inner));
                    }
                    _ => {}
                }
            }
        }
        return Ok(None);
    }
    Ok(None)
}
