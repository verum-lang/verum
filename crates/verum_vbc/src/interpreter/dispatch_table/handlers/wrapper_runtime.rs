//! Transfers ownership of a live interpreter Heap cell across raw-pointer calls.
//! Static, generic and dynamic calls use the same declaration and allocation header.
//! A source-level wrapper with a different representation runs its own body.

use super::super::super::error::InterpreterResult;
use super::super::super::state::InterpreterState;
use super::{
    cbgr::validate_reference,
    cbgr_helpers::{decode_cbgr_ref, is_cbgr_ref, validate_cbgr_generation},
};
use crate::instruction::Reg;
use crate::types::TypeId;
use crate::value::Value;
use verum_common::well_known_types::WellKnownType;

/// Older bytecode carries the bare type name. Qualified bytecode identifies
/// the declaring source module, so a user-defined Heap retains its own calls.
pub(super) fn heap_raw_method(name: &str) -> Option<&str> {
    let (ty, method) = name.strip_prefix("dyn:").unwrap_or(name).rsplit_once('.')?;
    if !matches!(method, "into_raw" | "from_raw") {
        return None;
    }
    let owner = match ty.rsplit_once('.') {
        Some((owner, ty)) => WellKnownType::from_declaration(ty, owner),
        None => WellKnownType::from_name(ty),
    };
    (owner == Some(WellKnownType::Heap)).then_some(method)
}

pub(super) fn is_heap_cell(state: &InterpreterState, value: Value) -> bool {
    use verum_common::layout::{ALLOCATION_HEADER_SIZE, ALLOCATION_HEADER_TYPE_ID_OFFSET};

    if !value.is_regular_ptr() || value.is_nil() {
        return false;
    }
    let Some(header) = (value.as_ptr::<u8>() as usize).checked_sub(ALLOCATION_HEADER_SIZE as usize)
    else {
        return false;
    };
    if !state.cbgr_allocations.contains(&header) {
        return false;
    }
    // SAFETY: the live allocation owns a complete, aligned AllocationHeader.
    // Heap.new writes its type stamp at this offset.
    let stamp = (header + ALLOCATION_HEADER_TYPE_ID_OFFSET as usize) as *const u32;
    unsafe { stamp.read() == TypeId::HEAP.0 }
}

pub(in super::super) fn wrapper_name_matches(name: &str) -> bool {
    heap_raw_method(name).is_some()
}

pub(super) fn transfer_heap_cell(
    state: &InterpreterState,
    mut value: Value,
) -> InterpreterResult<Option<Value>> {
    if is_cbgr_ref(&value) {
        let (slot, generation) = decode_cbgr_ref(value);
        validate_cbgr_generation(state, slot, generation)?;
        value = state.registers.get_absolute(slot);
    }
    if !is_heap_cell(state, value) {
        return Ok(None);
    }
    validate_reference(state, value)?;
    Ok(Some(value))
}

pub(in super::super) fn try_intercept_wrapper_call(
    state: &InterpreterState,
    name: &str,
    args_start: u16,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    if arg_count != 1 || heap_raw_method(name).is_none() {
        return Ok(None);
    }
    transfer_heap_cell(state, state.registers.get(caller_base, Reg(args_start)))
}
