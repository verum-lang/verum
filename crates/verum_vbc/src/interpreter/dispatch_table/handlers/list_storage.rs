//! Unsafe storage operations retain their container's encoding and extent.
use super::super::super::{
    error::{InterpreterError, InterpreterResult},
    heap,
    state::InterpreterState,
};
use super::super::DispatchResult;
use super::{bytecode_io::read_reg, cbgr_helpers::resolve_receiver, ffi_extended::value_as_addr};
use crate::value::Value;

fn invalid(message: impl Into<verum_common::Text>) -> InterpreterError {
    InterpreterError::Panic {
        message: message.into().into_string(),
    }
}

struct Storage {
    data: *mut u8,
    width: usize,
    capacity: usize,
}
impl Storage {
    fn resolve(state: &InterpreterState, value: Value) -> InterpreterResult<Self> {
        let value = resolve_receiver(state, value);
        if !value.is_regular_ptr() {
            return Err(invalid("List storage requires a live List object"));
        }
        let owner = value.as_ptr::<heap::ObjectHeader>();
        if owner.is_null() || !state.heap.contains(owner) {
            return Err(invalid("List storage requires a live List object"));
        }
        // SAFETY: exact membership proves the live object header.
        let header = unsafe { &*owner };
        let width =
            header.type_id.list_storage_stride().ok_or_else(|| {
                invalid(format!("unknown List storage type: {:?}", header.type_id))
            })? as usize;
        if header.size < 3 * std::mem::size_of::<Value>() as u32 {
            return Err(invalid("List storage object has a truncated header"));
        }
        // SAFETY: the declared List header contains these three Value fields.
        let fields = unsafe { (owner as *const u8).add(heap::OBJECT_HEADER_SIZE) as *const Value };
        let capacity = unsafe { fields.add(1).read_unaligned() }
            .try_as_i64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| invalid("List storage capacity is not a nonnegative Int"))?;
        let data = value_as_addr(unsafe { fields.add(2).read_unaligned() }) as *mut u8;
        let backing = state.heap.backing_view(data);
        if capacity == 0 && data.is_null() {
            return Ok(Self {
                data,
                width,
                capacity,
            });
        }
        let extent = backing
            .payload_size
            .or_else(|| super::cbgr::bridge_extent_room(state, data as usize))
            .ok_or_else(|| invalid("List storage backing has no live allocation extent"))?;
        let required = capacity
            .checked_mul(width)
            .ok_or_else(|| invalid("List storage capacity byte extent overflow"))?;
        if required > extent {
            return Err(invalid(
                "List storage capacity exceeds its allocation extent",
            ));
        }
        Ok(Self {
            data: backing.data,
            width,
            capacity,
        })
    }
    fn range(&self, index: i64, count: i64) -> InterpreterResult<(*mut u8, usize)> {
        let index =
            usize::try_from(index).map_err(|_| invalid("List storage range has negative index"))?;
        let count = usize::try_from(count)
            .map_err(|_| invalid("List storage range has negative length"))?;
        if index > self.capacity || count > self.capacity - index {
            return Err(invalid("List storage range exceeds capacity"));
        }
        if count == 0 {
            return Ok((self.data, 0));
        }
        // SAFETY: resolve proved capacity*width lies in one live allocation;
        // the subtraction-based range check avoids wrapping index+count.
        Ok((
            unsafe { self.data.add(index * self.width) },
            count * self.width,
        ))
    }
}

pub(super) fn access(
    state: &mut InterpreterState,
    opcode: u8,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let owner = read_reg(state)?;
    let first = read_reg(state)?;
    let storage = Storage::resolve(state, state.get_reg(owner))?;
    let index = state
        .get_reg(first)
        .try_as_i64()
        .ok_or_else(|| invalid("List storage range index is not Int"))?;
    match opcode {
        0x08 => {
            let (address, _) = storage.range(index, 1)?;
            // SAFETY: the checked range names one initialized element, as
            // required by the intrinsic's unsafe source contract.
            let value = unsafe {
                if storage.width == 1 {
                    Value::from_i64(address.read() as i64)
                } else {
                    (address as *const Value).read_unaligned()
                }
            };
            state.set_reg(dst, value);
        }
        0x09 => {
            let value = read_reg(state)?;
            let value = state.get_reg(value);
            let (address, _) = storage.range(index, 1)?;
            if storage.width == 1 {
                let byte = value
                    .try_as_i64()
                    .ok_or_else(|| invalid("packed List storage requires a byte value"))?
                    as u8;
                // SAFETY: range proved one byte writable in the owned buffer.
                unsafe { address.write(byte) };
            } else {
                // SAFETY: range proved a full Value slot writable.
                unsafe { (address as *mut Value).write_unaligned(value) };
            }
            state.set_reg(dst, Value::unit());
        }
        0x0A => {
            let target = read_reg(state)?;
            let count = read_reg(state)?;
            let target = state
                .get_reg(target)
                .try_as_i64()
                .ok_or_else(|| invalid("List storage range target is not Int"))?;
            let count = state
                .get_reg(count)
                .try_as_i64()
                .ok_or_else(|| invalid("List storage range length is not Int"))?;
            let (source, bytes) = storage.range(index, count)?;
            let (target, _) = storage.range(target, count)?;
            if bytes != 0 {
                // SAFETY: both complete ranges are inside the same allocation.
                // copy deliberately supports overlapping source/destination.
                unsafe { std::ptr::copy(source, target, bytes) };
            }
            state.set_reg(dst, Value::unit());
        }
        _ => return Err(invalid("unsupported List storage operation")),
    }
    Ok(DispatchResult::Continue)
}

/// Resize only the initialized prefix of a canonical owner. All fallible
/// work precedes publication; allocation failure preserves the old fields.
pub(super) fn resize(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let list = read_reg(state)?;
    let requested = read_reg(state)?;
    let value = resolve_receiver(state, state.get_reg(list));
    let storage = Storage::resolve(state, value)?;
    let owner = value.as_ptr::<heap::ObjectHeader>();
    // SAFETY: Storage::resolve proved the exact live owner and field extent.
    let fields = unsafe { (owner as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value };
    let len = unsafe { fields.read_unaligned() }
        .try_as_i64()
        .and_then(|n| usize::try_from(n).ok())
        .filter(|&n| n <= storage.capacity)
        .ok_or_else(|| invalid("List storage length exceeds capacity"))?;
    let requested = state
        .get_reg(requested)
        .try_as_i64()
        .ok_or_else(|| invalid("List storage capacity is not Int"))?;
    let limit = (u32::MAX as usize - 3 * verum_common::layout::ALLOCATION_HEADER_SIZE as usize)
        / storage.width;
    let Some(capacity) = usize::try_from(requested)
        .ok()
        .filter(|&n| n >= len && n <= limit)
    else {
        state.set_reg(dst, Value::from_bool(false));
        return Ok(DispatchResult::Continue);
    };
    if capacity == storage.capacity {
        state.set_reg(dst, Value::from_bool(true));
        return Ok(DispatchResult::Continue);
    }
    // A bridge interior address is readable, but does not own the enclosing
    // allocation. Resizing requires the exact tracked allocation base.
    let managed = state.heap.backing_view(storage.data).owner.is_some();
    let bridge = state
        .cbgr_bridge_extents
        .contains_key(&(storage.data as usize));
    if !storage.data.is_null() && !managed && !bridge {
        return Err(invalid("List resize requires an owned allocation base"));
    }
    let pointer = if capacity == 0 {
        Value::from_ptr(std::ptr::null_mut::<()>())
    } else {
        let allocation = if storage.width == 1 {
            state.heap.alloc(crate::types::TypeId::BYTE_LIST, capacity)
        } else {
            state.heap.alloc_array(crate::types::TypeId::UNIT, capacity)
        };
        let allocation = match allocation {
            Ok(allocation) => allocation,
            Err(InterpreterError::OutOfMemory { .. }) => {
                state.set_reg(dst, Value::from_bool(false));
                return Ok(DispatchResult::Continue);
            }
            Err(error) => return Err(error),
        };
        state.record_allocation();
        if len != 0 {
            // SAFETY: both complete initialized prefixes were proven; the
            // fresh allocation cannot overlap a live source allocation.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    storage.data,
                    (allocation.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE),
                    len * storage.width,
                );
            }
        }
        Value::from_ptr(allocation.as_ptr() as *mut ())
    };
    if bridge && !managed {
        super::ffi_extended::cbgr_user_deallocate(state, storage.data as i64);
    }
    // Managed old backing remains owned by the GC. No field changed before
    // the allocation and initialized-prefix copy succeeded.
    unsafe {
        fields.add(2).write_unaligned(pointer);
        fields
            .add(1)
            .write_unaligned(Value::from_i64(capacity as i64));
    }
    state.set_reg(dst, Value::from_bool(true));
    Ok(DispatchResult::Continue)
}
