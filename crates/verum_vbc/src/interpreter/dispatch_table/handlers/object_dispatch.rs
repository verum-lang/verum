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

use super::super::super::error::{InterpreterError, InterpreterResult};
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
pub(super) fn runtime_type_for_dispatch(v: &Value, state: &InterpreterState) -> Option<(u32, String)> {
    let (id, owner) = heap_operator_owner(v, state)?;
    Some((id.0, state.module.get_string(owner.name)?.into()))
}

fn heap_operator_owner<'a>(
    v: &Value, state: &'a InterpreterState,
) -> Option<(crate::types::TypeId, &'a crate::types::TypeDescriptor)> {
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
    let id = crate::types::TypeId(raw_id);
    Some((id, state.module.get_type(id)?))
}

/// Resolve the operator method `<ReceiverType>.<method>` for a heap
/// operand, memoised in `operator_method_cache` (negative results
/// included). Returns the function to invoke, or `None` when the operand
/// is not a dispatchable object or its type has no such method — the
/// caller then falls through to its legacy arm.
fn resolve_operator_method(
    state: &mut InterpreterState, receiver: &Value, rhs: Option<&Value>, method: &'static str,
) -> InterpreterResult<Option<FunctionId>> {
    let Some((receiver_id, descriptor)) = heap_operator_owner(receiver, state) else { return Ok(None) };
    let type_id = receiver_id.0;
    let rhs_type = rhs.map(|value| heap_operator_owner(value, state).map(|(id, _)| id)
        .unwrap_or_else(|| super::calls::get_builtin_type_id(*value)));
    let key = (type_id, rhs_type, method);
    if let Some(cached) = state.operator_method_cache.get(&key) { return Ok(*cached); }
    let module = &state.module;
    let standard_protocol = verum_ast::BinOp::ALL.iter().filter_map(|op| op.standard_protocol_member())
        .find_map(|(protocol, member)| (member == method).then_some(protocol));
    let mut declared = None;
    if let Some(protocol) = standard_protocol {
        for row in &descriptor.protocols {
            if module.get_type(crate::types::TypeId(row.protocol.0)).and_then(|ty| module.get_string(ty.name)) != Some(protocol) { continue; }
            if declared.is_some_and(|prior| prior != row.protocol) {
                return Err(InterpreterError::InvalidBytecode { pc: state.pc() as usize,
                    message: format!("ambiguous declaring protocol for operator `{method}`") });
            }
            declared = Some(row.protocol);
        }
    }
    let resolved = if let Some(protocol) = declared {
        Some(module.implementation_method(crate::types::TypeId(type_id), protocol, method, rhs_type)
            .map_err(|message| InterpreterError::InvalidBytecode { pc: state.pc() as usize, message })?
            .ok_or_else(|| InterpreterError::InvalidBytecode { pc: state.pc() as usize,
                message: format!("operator `{method}` has no implementation for these operands") })?)
    } else {
        let Some(type_name) = module.get_string(descriptor.name) else { return Ok(None) };
        let base = crate::module::strip_generic_args(type_name);
        module.find_function_by_name(&format!("{base}.{method}"))
            .filter(|id| module.get_function(*id).is_some_and(|f| f.has_definition()))
    };
    state.operator_method_cache.insert(key, resolved);
    Ok(resolved)
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
    let Some(func_id) = resolve_operator_method(state, &receiver, rhs.as_ref(), method)? else {
        return Ok(false);
    };
    push_operator_frame(state, dst, receiver, rhs, func_id)
}

/// Generic arithmetic carries a protocol ID on the wire. A heap operator may
/// only enter the vtable identified by that operand.
pub(super) fn try_push_declared_binary(
    state: &mut InterpreterState, dst: Reg, left: Value, right: Value,
    instruction_protocol: u32, operation: verum_ast::BinOp,
) -> InterpreterResult<bool> {
    let Some((protocol_name, method)) = operation.standard_protocol_member() else { return Ok(false) };
    let invalid = |message| InterpreterError::InvalidBytecode { pc: state.pc() as usize, message };
    if instruction_protocol != 0 {
        let declared = state.module.get_type(crate::types::TypeId(instruction_protocol))
            .filter(|protocol| protocol.kind == crate::types::TypeKind::Protocol)
            .and_then(|protocol| state.module.get_string(protocol.name));
        if declared != Some(protocol_name) {
            return Err(invalid(format!("generic operator `{method}` carries a different protocol")));
        }
    }
    let Some((receiver, _)) = heap_operator_owner(&left, state) else { return Ok(false) };
    if receiver.is_builtin() { return Ok(false); }
    if instruction_protocol == 0 {
        return try_push_operator_method_frame(state, dst, left, Some(right), method);
    }
    let operand = heap_operator_owner(&right, state).map(|(id, _)| id)
        .unwrap_or_else(|| super::calls::get_builtin_type_id(right));
    let target = state.module.implementation_method(receiver,
        crate::types::ProtocolId(instruction_protocol), method, Some(operand)).map_err(invalid)?
        .ok_or_else(|| invalid(format!("missing implementation of generic operator `{method}`")))?;
    push_operator_frame(state, dst, left, Some(right), target)
}

fn push_operator_frame(
    state: &mut InterpreterState, dst: Reg, receiver: Value, rhs: Option<Value>, func_id: FunctionId,
) -> InterpreterResult<bool> {
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
