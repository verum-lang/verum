//! Runtime operator-method dispatch for heap-record operands.
//!
//! Generic arithmetic (`fn f<T: Add>(a: T, b: T) { a + b }`) erases the
//! operand type, so codegen emits the integer `AddI`/`SubI`/… opcodes for
//! type-param operands. The handlers carry polymorphic arms for the
//! primitive shapes (inline int fast path, float, i128, string concat);
//! before this module existed, a HEAP RECORD operand — a user type with an
//! `Add`/`Mul`/… implementation, e.g. `Complex<Float>` inside a generic
//! `matmul` — fell through to the integer-extract arm and was summed as
//! raw NaN-box pointer bits.
//!
//! The object arm completes the intended polymorphism: resolve the
//! receiver's runtime type from its `ObjectHeader`, look up the
//! generic-stripped operator method (`Complex.add`), and dispatch by
//! pushing a call frame — the same in-loop tail-call shape `handle_eqg`
//! uses for `<T: Eq>` dispatch (no nested execution; the frame's return
//! value lands in `dst`).
//!
//! Resolution is memoised per `(type_id, method)` in
//! `InterpreterState::operator_method_cache`, including negative results,
//! so hot loops (`matmul` inner products) pay one function-table scan per
//! (type, operator) pair, not one per element operation.
//!
//! This module is the ONE authority for "runtime type name of a heap
//! value for method dispatch" — `comparison.rs`'s Eq/Ord fallback
//! delegates here rather than keeping its own copy of the header-probe
//! guards.

use super::super::super::error::InterpreterResult;
use super::super::super::state::InterpreterState;
use super::string_helpers::is_byte_slice_value;
use crate::instruction::Reg;
use crate::module::FunctionId;
use crate::value::Value;

/// Read the runtime TypeId from a heap-allocated `Value`'s ObjectHeader
/// and resolve it to the declared type name via `state.module.types`.
/// Returns `None` for primitives (NaN-boxed ints, bools, floats,
/// pointers without a valid header) and for TypeIds that aren't
/// registered in the module's type table.
///
/// Guards (kept in ONE place; the Eq/Ord fallback shares them):
/// * byte/raw-element slice FatRefs (`reserved != 0`) carry the
///   FAT_REF_MARKER payload in `as_ptr()`, which a header probe would
///   dereference — SIGSEGV. They have no nominal pointee type.
/// * BYTE_SLICE byte-view objects (`Text.as_bytes()`) are structural
///   byte ranges, not nominal method carriers.
/// * `ObjectHeader::ref_or_stub` returns a benign stub (type_id
///   INVALID → 0) for pointers failing the alignment/sentinel check, so
///   mis-tagged integers fall through to the caller's fallback arm
///   instead of faulting.
pub(super) fn runtime_type_for_dispatch(
    v: &Value,
    state: &InterpreterState,
) -> Option<(u32, String)> {
    use crate::interpreter::heap;
    if v.is_fat_ref() && v.as_fat_ref().reserved != 0 {
        return None;
    }
    if is_byte_slice_value(v) {
        return None;
    }
    if !v.is_ptr() || v.is_nil() {
        return None;
    }
    let ptr = v.as_ptr::<u8>();
    if ptr.is_null() {
        return None;
    }
    // Safety: any non-null pointer Value in a well-formed module points
    // at a heap allocation whose first `OBJECT_HEADER_SIZE` bytes are an
    // ObjectHeader; `ref_or_stub` degrades unaligned/foreign pointers to
    // a stub header instead of trusting them.
    let header = unsafe { heap::ObjectHeader::ref_or_stub(ptr) };
    let raw_id = header.type_id.0;
    if raw_id == 0 {
        return None;
    }
    let name = state
        .module
        .types
        .iter()
        .find(|td| td.id.0 == raw_id)
        .and_then(|td| state.module.strings.get(td.name))
        .map(|s| s.to_string())?;
    Some((raw_id, name))
}

/// Resolve the operator method `<ReceiverType>.<method>` for a heap
/// operand, memoised in `operator_method_cache` (negative results
/// included). Returns the function to invoke, or `None` when the operand
/// is not a dispatchable object or its type has no such method — the
/// caller then falls through to its legacy arm.
fn resolve_operator_method(
    state: &mut InterpreterState,
    receiver: &Value,
    method: &'static str,
) -> Option<FunctionId> {
    let (type_id, type_name) = runtime_type_for_dispatch(receiver, state)?;
    if let Some(cached) = state.operator_method_cache.get(&(type_id, method)) {
        return *cached;
    }
    // Descriptor names can be generic-stamped ("Complex<Float>");
    // function keys are generic-stripped ("Complex.add").
    let base = crate::module::strip_generic_args(&type_name);
    let qualified = format!("{}.{}", base, method);
    let resolved = state.module.find_function_by_name(&qualified).filter(|fid| {
        // A forward declaration with an empty body is not a dispatch
        // target — landing in one would silently return nil. Require a
        // real body, mirroring `find_function_by_name`'s own ranking.
        state
            .module
            .get_function(*fid)
            .map(|f| {
                f.bytecode_length > 0
                    || f.instructions.as_ref().map(|i| !i.is_empty()).unwrap_or(false)
            })
            .unwrap_or(false)
    });
    state
        .operator_method_cache
        .insert((type_id, method), resolved);
    resolved
}

/// Object arm entry point: when `receiver` is a heap record whose type
/// implements `<method>`, push the method's call frame (receiver in r0,
/// `rhs` — when present — in r1) and report `true`; the main loop then
/// executes the body and its return lands in `dst`. Reports `false` when
/// this operand is not object-dispatchable, so the handler continues into
/// its legacy arms.
pub(super) fn try_push_operator_method_frame(
    state: &mut InterpreterState,
    dst: Reg,
    receiver: Value,
    rhs: Option<Value>,
    method: &'static str,
) -> InterpreterResult<bool> {
    let Some(func_id) = resolve_operator_method(state, &receiver, method) else {
        return Ok(false);
    };
    let Some(func) = state.module.get_function(func_id) else {
        return Ok(false);
    };
    let reg_count = func.register_count;
    let return_pc = state.pc();
    let new_base = state
        .call_stack
        .push_frame(func_id, reg_count, return_pc, dst)?;
    state.registers.push_frame(reg_count);
    state.registers.set(new_base, Reg(0), receiver);
    if let Some(rhs) = rhs {
        state.registers.set(new_base, Reg(1), rhs);
    }
    state.set_pc(0);
    state.record_call();
    Ok(true)
}
