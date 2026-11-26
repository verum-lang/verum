//! Borrow or transfer a Heap whose VBC carrier is a tracked Value cell.
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
use crate::value::{Capabilities, Value};
use verum_common::well_known_types::WellKnownType;

#[derive(Clone, Copy)]
pub(super) enum HeapCellMethod {
    Transfer,
    Borrow { mutable: bool },
}

/// Qualified selectors belong to the canonical declaration. The unqualified
/// type spelling is the legacy VBC encoding.
pub(super) fn heap_cell_method(name: &str) -> Option<HeapCellMethod> {
    let (ty, method) = name.strip_prefix("dyn:").unwrap_or(name).rsplit_once('.')?;
    let operation = match method {
        "into_raw" | "from_raw" => HeapCellMethod::Transfer,
        "as_ref" => HeapCellMethod::Borrow { mutable: false },
        "as_mut" => HeapCellMethod::Borrow { mutable: true },
        _ => return None,
    };
    let owner = match ty.rsplit_once('.') {
        Some((owner, ty)) => WellKnownType::from_declaration(ty, owner),
        None => WellKnownType::from_name(ty),
    };
    (owner == Some(WellKnownType::Heap)).then_some(operation)
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
    heap_cell_method(name).is_some()
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

/// Managed Heap cells have one Value slot, while source-constructed wrappers
/// have ordinary fields. Only the former require ABI adaptation.
pub(super) fn borrow_heap_cell(
    state: &mut InterpreterState,
    receiver: Value,
    mutable: bool,
) -> InterpreterResult<Option<Value>> {
    use super::cbgr_helpers::resolve_receiver_place;
    use crate::interpreter::InterpreterError;
    use verum_common::layout::{ALLOCATION_HEADER_CAPABILITIES_OFFSET, ALLOCATION_HEADER_SIZE};

    let place = resolve_receiver_place(state, receiver)?;
    let cell = place.value;
    if !is_heap_cell(state, cell) {
        return Ok(None);
    }
    validate_reference(state, cell)?;
    let address = cell.as_ptr::<u8>() as usize;
    let header = address - ALLOCATION_HEADER_SIZE as usize;
    // SAFETY: the live Heap stamp and CBGR validation establish this header.
    let caps =
        unsafe { ((header + ALLOCATION_HEADER_CAPABILITIES_OFFSET as usize) as *const u16).read() };
    let available = place.capabilities.intersect(&Capabilities(caps));
    let requested = if mutable {
        Capabilities::READ | Capabilities::WRITE
    } else {
        Capabilities::READ
    };
    if available.0 & requested != requested {
        return Err(InterpreterError::CapabilityDenied {
            required: u64::from(requested),
            actual: u64::from(available.0),
        });
    }
    state
        .heap
        .project_raw_slot(
            address,
            std::mem::size_of::<Value>(),
            0,
            Capabilities(requested),
        )
        .map(Some)
}

pub(super) fn try_intercept_wrapper_call(
    state: &mut InterpreterState,
    method: Option<HeapCellMethod>,
    args_start: u16,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    if arg_count != 1 {
        return Ok(None);
    }
    let receiver = state.registers.get(caller_base, Reg(args_start));
    match method {
        Some(HeapCellMethod::Borrow { mutable }) => borrow_heap_cell(state, receiver, mutable),
        Some(HeapCellMethod::Transfer) => transfer_heap_cell(state, receiver),
        None => Ok(None),
    }
}
