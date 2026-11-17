//! Checked Text storage shared by copying, mutation and destruction.
//! Live allocation membership precedes every header or payload read. Capacity
//! bounds bytes; it never supplies permission to release an allocation.
use super::super::super::{
    error::{InterpreterError, InterpreterResult}, heap, state::InterpreterState,
};
use crate::{types::TypeId, value::Value};

fn invalid(message: &str) -> InterpreterError {
    InterpreterError::InvalidOperand { message: message.into() }
}

/// Consume a fresh replacement, publish it through the reference writer, then
/// retire the previous owned value. Refusal retires only the replacement. A
/// caller receiving false writes its direct slot without invoking user code.
pub(super) fn replace(
    state: &mut InterpreterState, previous: Value, target: Value, replacement: Value,
) -> InterpreterResult<bool> {
    let fresh = if replacement.is_small_string() { None } else {
        let storage = TextStorage::checked(state, replacement)?;
        storage.validate_release(state)?;
        Some(storage)
    };
    let result = (|| {
        let previous_storage = if previous.is_small_string() { None } else {
            let storage = TextStorage::checked(state, previous)?;
            storage.validate_release(state)?;
            Some(storage)
        };
        let written = super::cbgr_helpers::write_through_ref(state, target, replacement)?;
        if let Some(storage) = previous_storage {
            if previous != replacement { storage.destroy(state)?; }
        }
        Ok(written)
    })();
    if result.is_err() && previous != replacement {
        if let Some(storage) = fresh { storage.destroy(state)?; }
    }
    result
}

/// The owned-buffer operation transfers a complete allocation, never a slice
/// or an address inferred to be plausible. On refusal the caller retains it.
pub(super) fn adopt_owned(
    state: &mut InterpreterState, data: Value, len: Value, capacity: Value,
) -> InterpreterResult<Value> {
    let natural = |value: Value| value.try_as_i64()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| invalid("Text adoption requires nonnegative length and capacity"));
    let len = natural(len)?;
    let capacity = natural(capacity)?;
    let address = super::ffi_extended::value_as_addr(data);
    if len > capacity { return Err(invalid("Text adoption length exceeds capacity")); }
    let extent = capacity.checked_add(1).ok_or_else(|| invalid("Text adoption extent overflow"))?;
    if address == 0 {
        if len != 0 || capacity != 0 { return Err(invalid("Text adoption has no byte storage")); }
    } else if state.cbgr_bridge_extents.get(&address) != Some(&extent) {
        return Err(invalid("Text adoption requires the complete owned CBGR byte allocation"));
    }
    let object = state.heap.alloc(TypeId::TEXT, heap::TEXT_RECORD_SIZE)?;
    state.record_allocation();
    // SAFETY: the fresh descriptor has three slots, and the checked producer
    // supplied the complete live byte allocation transferred into those slots.
    unsafe {
        if address != 0 { (address as *mut u8).add(len).write(0); }
        let fields = object.data_ptr().cast::<Value>();
        fields.write(if address == 0 { Value::nil() } else { Value::from_i64(address as i64) });
        fields.add(1).write(Value::from_i64(len as i64));
        fields.add(2).write(Value::from_i64(capacity as i64));
        (*object.as_ptr()).flags.insert(heap::ObjectFlags::TEXT_OWNED_BUFFER);
    }
    Ok(Value::from_ptr(object.as_ptr().cast::<u8>()))
}

#[cfg(test)]
#[path = "../../../../tests/unit/text_storage_lifecycle.rs"]
mod tests;

/// A read capability obtained from live storage, valid until the next mutation.
pub(super) struct TextStorage {
    pub object: heap::Object,
    pub data: *const u8,
    pub len: usize,
    pub capacity: usize,
    inline: bool,
}

impl TextStorage {
    pub fn checked(state: &InterpreterState, value: Value) -> InterpreterResult<Self> {
        if !value.is_regular_ptr() || value.is_nil() || !state.heap.contains(value.as_ptr()) {
            return Err(invalid("Text requires a live record allocation"));
        }
        let pointer = value.as_ptr::<heap::ObjectHeader>();
        // SAFETY: exact live allocation membership precedes all reads.
        let header = unsafe { &*pointer };
        if !header.is_valid() || header.type_id != TypeId::TEXT
            || (header.size as usize) < heap::TEXT_RECORD_SIZE
        {
            return Err(invalid("Text requires its complete declared record"));
        }
        // SAFETY: the entire three-slot record is in the live allocation.
        let (data, len, capacity) = unsafe {
            let fields = pointer.cast::<u8>().add(heap::OBJECT_HEADER_SIZE).cast::<Value>();
            let natural = |value: Value| value.try_as_i64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| invalid("Text length and capacity must be nonnegative Int values"));
            let len = natural(fields.add(1).read_unaligned())?;
            let capacity = natural(fields.add(2).read_unaligned())?;
            let (data, _) = heap::text_record_payload(pointer.cast())
                .ok_or_else(|| invalid("Text requires its byte-buffer codec"))?;
            (data, len, capacity)
        };
        if capacity != 0 && len > capacity {
            return Err(invalid("Text length exceeds its capacity"));
        }
        let readable = if capacity == 0 { len } else {
            capacity.checked_add(1).ok_or_else(|| invalid("Text extent overflow"))?
        };
        let address = data as usize;
        let inline_start = pointer as usize + heap::OBJECT_HEADER_SIZE + heap::TEXT_RECORD_SIZE;
        let inline_end = pointer as usize + heap::OBJECT_HEADER_SIZE + header.size as usize;
        let inline = address >= inline_start
            && address.checked_add(readable).is_some_and(|end| end <= inline_end);
        if readable != 0 && !inline {
            let backing = state.heap.backing_view(data.cast_mut());
            let managed = backing.data == data.cast_mut()
                && backing.payload_size.is_some_and(|size| readable <= size);
            let raw = super::cbgr::bridge_extent_room(state, address)
                .is_some_and(|size| readable <= size);
            if !managed && !raw {
                return Err(invalid("Text byte buffer exceeds its live allocation"));
            }
        }
        // SAFETY: exact membership and the complete header were checked above.
        let object = unsafe { heap::Object::from_raw(pointer) }
            .ok_or_else(|| invalid("Text record pointer is null"))?;
        Ok(Self { object, data, len, capacity, inline })
    }

    /// Only a storage producer can set these flags. In particular neither a
    /// nominal TEXT tag nor a positive capacity is a destruction receipt.
    pub fn validate_release(&self, state: &InterpreterState) -> InterpreterResult<()> {
        let flags = self.object.header().flags;
        let receipt = flags & heap::ObjectFlags::TEXT_STORAGE;
        if receipt == heap::ObjectFlags::TEXT_INLINE {
            if self.inline || self.len == 0 && self.capacity == 0 { return Ok(()); }
        } else if receipt == heap::ObjectFlags::TEXT_BORROWED {
            if self.capacity == 0 { return Ok(()); }
        } else if receipt == heap::ObjectFlags::TEXT_OWNED_BUFFER {
            if self.data.is_null() && self.len == 0 && self.capacity == 0 { return Ok(()); }
            let extent = self.capacity.checked_add(1)
                .ok_or_else(|| invalid("Text owned extent overflow"))?;
            if state.cbgr_bridge_extents.get(&(self.data as usize)) == Some(&extent) {
                return Ok(());
            }
        }
        Err(invalid("Text destruction requires its producer-issued storage receipt"))
    }

    /// Retire contents after validation, leaving an empty mutable descriptor.
    /// A subsequent typed drop can release that descriptor without double free.
    pub fn release_contents(&self, state: &mut InterpreterState) -> InterpreterResult<()> {
        self.validate_release(state)?;
        if self.object.header().flags.contains(heap::ObjectFlags::MODULE_LITERAL) {
            return Err(invalid("module-owned Text storage cannot be mutated"));
        }
        if self.object.header().flags.contains(heap::ObjectFlags::TEXT_OWNED_BUFFER)
            && !self.data.is_null()
        {
            super::ffi_extended::cbgr_user_deallocate(state, self.data as i64);
        }
        // SAFETY: this checked live descriptor is exclusively borrowed by the
        // executing mutation. No user code runs between validation and clearing.
        unsafe {
            let fields = self.object.data_ptr().cast::<Value>();
            fields.write(Value::nil());
            fields.add(1).write(Value::from_i64(0));
            fields.add(2).write(Value::from_i64(0));
            let header = &mut *self.object.as_ptr();
            header.flags.remove(heap::ObjectFlags::TEXT_STORAGE);
            header.flags.insert(heap::ObjectFlags::TEXT_BORROWED);
        }
        Ok(())
    }

    pub fn destroy(self, state: &mut InterpreterState) -> InterpreterResult<()> {
        self.validate_release(state)?;
        if self.object.header().flags.contains(heap::ObjectFlags::MODULE_LITERAL) {
            return Ok(());
        }
        self.release_contents(state)?;
        // SAFETY: validation retained the exact owned record and retired its
        // backing. Heap::free also revokes projections and allocation membership.
        unsafe { state.heap.free(self.object); }
        Ok(())
    }
}
