//! Physical Deque slots hold one semantic Value, including for Byte and records.
use super::super::super::{
    error::{InterpreterError, InterpreterResult},
    heap,
    state::InterpreterState,
};
use super::super::DispatchResult;
use super::{
    bytecode_io::read_reg, cbgr_helpers::resolve_receiver_place, ffi_extended::value_as_addr,
};
use crate::{
    instruction::MemSubOpcode,
    types::TypeId,
    value::{Capabilities, Value},
};

fn invalid(message: &str) -> InterpreterError {
    InterpreterError::InvalidOperand {
        message: message.to_owned(),
    }
}

pub(super) struct Ring {
    backing: Value,
    pub(super) data: *mut Value,
    pub(super) head: usize,
    pub(super) len: usize,
    pub(super) capacity: usize,
}

impl Ring {
    pub(super) fn resolve(state: &InterpreterState, owner: Value) -> InterpreterResult<Self> {
        if !owner.is_regular_ptr() || !state.heap.contains(owner.as_ptr()) {
            return Err(invalid("Deque storage requires a live ring owner"));
        }
        // SAFETY: exact managed membership establishes a readable header.
        let header = unsafe { &*owner.as_ptr::<heap::ObjectHeader>() };
        let width = size_of::<Value>();
        if !header.is_valid()
            || header.type_id != TypeId::DEQUE
            || header.size as usize != 4 * width
        {
            return Err(invalid("Deque storage requires the four-field ring owner"));
        }
        // SAFETY: the exact extent includes the backing pointer and three Int fields.
        let fields = unsafe {
            owner
                .as_ptr::<u8>()
                .add(heap::OBJECT_HEADER_SIZE)
                .cast::<Value>()
        };
        let nonnegative = |word: Value| {
            word.try_as_i64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| invalid("Deque ring fields must be nonnegative integers"))
        };
        let (backing, head, len, capacity) = unsafe {
            (
                fields.read(),
                nonnegative(fields.add(1).read())?,
                nonnegative(fields.add(2).read())?,
                nonnegative(fields.add(3).read())?,
            )
        };
        let address = value_as_addr(backing) as *mut u8;
        if len > capacity
            || (capacity == 0 && (head != 0 || !address.is_null()))
            || (capacity > 0
                && (!capacity.is_power_of_two() || head >= capacity || address.is_null()))
        {
            return Err(invalid("invalid Deque ring bounds"));
        }
        let view = state.heap.backing_view(address);
        if capacity != 0 {
            let bytes = capacity
                .checked_mul(width)
                .ok_or_else(|| invalid("Deque capacity overflows"))?;
            let extent = view
                .payload_size
                .or_else(|| state.cbgr_bridge_extents.get(&(address as usize)).copied());
            if view.owner.is_some_and(|base| {
                base == owner.as_ptr() || (address != base && address != view.data)
            }) || extent != Some(bytes)
            {
                return Err(invalid("Deque backing extent differs from its capacity"));
            }
        }
        Ok(Self {
            backing,
            data: view.data.cast(),
            head,
            len,
            capacity,
        })
    }
}

/// Build the same four-field owner as the source constructor. A zero capacity
/// has no allocation and a nonzero hint reserves a power-of-two ring.
pub(super) fn new(state: &mut InterpreterState, requested: usize) -> InterpreterResult<Value> {
    let capacity = if requested == 0 {
        0
    } else {
        requested
            .checked_next_power_of_two()
            .filter(|cap| {
                *cap <= verum_common::layout::MAX_ALLOCATION_SIZE as usize / size_of::<Value>()
            })
            .ok_or_else(|| invalid("Deque capacity overflows its value slots"))?
    };
    let object = state.heap.alloc(TypeId::DEQUE, 4 * size_of::<Value>())?;
    state.record_allocation();
    let backing = if capacity == 0 {
        Value::from_i64(0)
    } else {
        let bytes = capacity * size_of::<Value>();
        let address = super::ffi_extended::cbgr_user_allocate(state, bytes as i64, 8);
        if address == 0 {
            // SAFETY: this unpublished owner has no initialized fields.
            unsafe {
                state.heap.free(object);
            }
            return Err(invalid("could not allocate Deque backing"));
        }
        let slots = address as *mut Value;
        for index in 0..capacity {
            // SAFETY: the CBGR allocation contains capacity aligned Value slots.
            unsafe {
                slots.add(index).write(Value::unit());
            }
        }
        Value::from_i64(address)
    };
    // SAFETY: the allocation contains the four declared slots in order.
    unsafe {
        let fields = object
            .as_ptr()
            .cast::<u8>()
            .add(heap::OBJECT_HEADER_SIZE)
            .cast::<Value>();
        fields.write(backing);
        fields.add(1).write(Value::from_i64(0));
        fields.add(2).write(Value::from_i64(0));
        fields.add(3).write(Value::from_i64(capacity as i64));
    }
    Ok(Value::from_ptr(object.as_ptr()))
}

pub(super) fn access(
    state: &mut InterpreterState,
    opcode: u8,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let owner = read_reg(state)?;
    let index = read_reg(state)?;
    let place = resolve_receiver_place(state, state.get_reg(owner))?.into_record(state)?;
    let operation =
        MemSubOpcode::from_byte(opcode).ok_or_else(|| invalid("unknown Deque operation"))?;
    let slice = matches!(
        operation,
        MemSubOpcode::DequeStorageSlice | MemSubOpcode::DequeStorageSliceMut
    );
    let shared = matches!(
        operation,
        MemSubOpcode::DequeStorageRef | MemSubOpcode::DequeStorageSlice
    );
    let project = shared
        || matches!(
            operation,
            MemSubOpcode::DequeStorageRefMut | MemSubOpcode::DequeStorageSliceMut
        );
    if !shared {
        place.require_write()?;
    }
    let ring = Ring::resolve(state, place.value)?;
    let natural = |value: Value| {
        value
            .try_as_i64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| invalid("Deque index and length must be nonnegative integers"))
    };
    let index = natural(state.get_reg(index))?;
    let count = if slice {
        let count = read_reg(state)?;
        natural(state.get_reg(count))?
    } else {
        1
    };
    if index > ring.capacity || count > ring.capacity - index {
        return Err(invalid("Deque storage range exceeds capacity"));
    }
    if project {
        if count != 0 {
            let logical = index.wrapping_sub(ring.head) & (ring.capacity - 1);
            if logical > ring.len || count > ring.len - logical {
                return Err(invalid("cannot borrow a vacant Deque slot"));
            }
        }
        let caps = if shared {
            place.capabilities.intersect(&Capabilities::READ_ONLY)
        } else {
            place.capabilities
        };
        let data = ring.data.cast();
        let extent = super::cbgr::bridge_extent(state, data as usize);
        let view = state.heap.project_sequence(
            place.value.as_ptr::<u8>() as usize,
            0,
            ring.backing,
            data,
            extent,
            ring.capacity,
            heap::ProjectionEncoding::Value,
            caps,
        )?;
        let element = if slice {
            state.heap.project_subsequence(view, index, count, caps)?
        } else {
            state.heap.project_sequence_element(view, index, caps)?
        };
        state.set_reg(dst, element);
        return Ok(DispatchResult::Continue);
    }
    // Element replacement removes the previously borrowed value even when
    // the allocation itself survives. Retire its projections before reuse.
    if state
        .cbgr_bridge_extents
        .contains_key(&(ring.data as usize))
    {
        state.heap.revoke_projections(ring.data as usize);
    } else {
        state
            .heap
            .revoke_projections(place.value.as_ptr::<u8>() as usize);
    }
    // SAFETY: the ring's entire allocation and this slot's index were validated.
    let slot = unsafe { ring.data.add(index) };
    let value = if opcode == MemSubOpcode::DequeStorageTake as u8 {
        // SAFETY: the source contract requires an initialized physical slot. Clearing
        // it transfers its sole ownership and removes the stale GC root.
        unsafe { slot.replace(Value::unit()) }
    } else {
        let src = read_reg(state)?;
        // SAFETY: the unsafe source operation requires a vacant or moved-out slot.
        unsafe { slot.write(state.get_reg(src)) };
        Value::unit()
    };
    state.set_reg(dst, value);
    Ok(DispatchResult::Continue)
}
