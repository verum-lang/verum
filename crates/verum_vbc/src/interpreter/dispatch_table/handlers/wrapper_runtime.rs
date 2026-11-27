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

pub(super) fn allocate_heap_cell(
    state: &mut InterpreterState,
    value: Value,
    witnesses: Option<&[crate::types::TypeRef]>,
) -> InterpreterResult<Value> {
    use verum_common::layout::{ALLOCATION_HEADER_SIZE, ALLOCATION_HEADER_TYPE_ID_OFFSET};
    let element = match witnesses {
        Some([element]) => Some(element),
        None if value.is_int() || value.is_bool() || value.is_float() || value.is_unit()
            || value.is_small_string() => None,
        _ => return Err(crate::interpreter::InterpreterError::InvalidOperand {
            message: "managed allocation requires its payload type witness".into(),
        }),
    };
    let size = std::mem::size_of::<Value>();
    let address = super::ffi_extended::cbgr_user_allocate(
        state,
        size as i64,
        std::mem::align_of::<Value>() as i64,
    );
    if address == 0 {
        return Err(crate::interpreter::InterpreterError::OutOfMemory {
            requested: size,
            available: 0,
        });
    }
    let data = address as *mut Value;
    let header = address as usize - ALLOCATION_HEADER_SIZE as usize;
    // SAFETY: this allocation owns one aligned Value and its initialized CBGR header.
    unsafe {
        ((header + ALLOCATION_HEADER_TYPE_ID_OFFSET as usize) as *mut u32).write(TypeId::HEAP.0);
        data.write(value);
    }
    state.record_allocation();
    if let Some(element) = element {
        state.heap_cell_elements.insert(address as usize, element.clone());
    }
    Ok(Value::from_ptr(data))
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
    let place = resolve_receiver_place(state, receiver)?;
    let cell = place.value;
    if !is_heap_cell(state, cell) {
        return Ok(None);
    }
    validate_reference(state, cell)?;
    let address = cell.as_ptr::<u8>() as usize;
    borrow_live_heap_slot(state, address, std::mem::size_of::<Value>(), place.capabilities, mutable).map(Some)
}

/// Both a bootstrap cell and a declared Heap borrow their live allocation's
/// Value slot. The allocation header limits the permissions of the new ticket.
pub(super) fn borrow_live_heap_slot(
    state: &mut InterpreterState, address: usize, len: usize,
    inherited: Capabilities, mutable: bool,
) -> InterpreterResult<Value> {
    use crate::interpreter::InterpreterError;
    use verum_common::layout::{ALLOCATION_HEADER_CAPABILITIES_OFFSET, ALLOCATION_HEADER_SIZE};
    let header = address - ALLOCATION_HEADER_SIZE as usize;
    // SAFETY: the caller verified that address is the base of a live CBGR allocation.
    let caps =
        unsafe { ((header + ALLOCATION_HEADER_CAPABILITIES_OFFSET as usize) as *const u16).read() };
    let available = inherited.intersect(&Capabilities(caps));
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
    state.heap.project_raw_slot(address, len, 0, Capabilities(requested))
}

/// Resolve the owned payload only when both the declared plan and the live
/// allocation identify the standard Heap. A source record keeps its own glue.
pub(super) fn cell_drop_payload(
    state: &InterpreterState,
    cell: Value,
    plan: &crate::drop_plan::DropPlan,
) -> InterpreterResult<Option<(Value, crate::drop_plan::DropPlan)>> {
    use crate::interpreter::InterpreterError;
    let Some(args) = plan.standard_record_arguments(&state.module, WellKnownType::Heap) else {
        return Ok(None);
    };
    if !is_heap_cell(state, cell) {
        return Ok(None);
    }
    validate_reference(state, cell)?;
    let [element] = args else {
        return Err(InterpreterError::InvalidOperand {
            message: "Heap destruction requires one payload type".into(),
        });
    };
    if state.heap_cell_elements.get(&(cell.as_ptr::<Value>() as usize))
        .is_some_and(|issued| issued != element)
    {
        return Err(InterpreterError::InvalidOperand {
            message: "Heap payload type differs from the allocating constructor".into(),
        });
    }
    let element = crate::drop_plan::build(&state.module, element).map_err(|message| {
        InterpreterError::InvalidOperand {
            message: message.into_string(),
        }
    })?;
    // SAFETY: the live runtime cell owns exactly one initialized Value.
    Ok(Some((unsafe { cell.as_ptr::<Value>().read() }, element)))
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
