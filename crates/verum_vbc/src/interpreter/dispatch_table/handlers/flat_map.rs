//! Checked view of the VM's flat, allocator-owned key/value table. The source
//! Robin Hood table has a different allocation contract and never enters here.
use super::super::super::{
    error::{InterpreterError, InterpreterResult},
    heap,
    state::InterpreterState,
};
use super::memory_collections::{value_eq, value_hash};
use crate::{
    drop_plan::{self, DropPlan},
    types::TypeId,
    value::Value,
};
use verum_common::well_known_types::WellKnownType;

pub(super) struct FlatMap<'heap> {
    _heap: std::marker::PhantomData<&'heap heap::Heap>,
    header: *mut Value,
    pub(super) entries: *mut Value,
    pub(super) capacity: usize,
    pub(super) len: usize,
    backing: Option<Value>,
}

impl<'heap> FlatMap<'heap> {
    pub(super) fn is_carrier(state: &InterpreterState, value: Value) -> bool {
        if !value.is_regular_ptr() || value.is_nil() || !state.heap.contains(value.as_ptr()) {
            return false;
        }
        // SAFETY: exact heap membership precedes the header access.
        unsafe { (*value.as_ptr::<heap::ObjectHeader>()).type_id == TypeId::MAP }
    }

    pub(super) fn checked(
        state: &'heap InterpreterState,
        receiver: Value,
    ) -> InterpreterResult<Self> {
        let invalid = |detail: &str| InterpreterError::Panic {
            message: format!("invalid Map storage: {detail}"),
        };
        if !receiver.is_regular_ptr()
            || receiver.is_nil()
            || !state.heap.contains(receiver.as_ptr::<heap::ObjectHeader>())
        {
            return Err(invalid("receiver is not an owned object"));
        }
        // SAFETY: exact heap membership establishes a live ObjectHeader.
        let header = unsafe { &*receiver.as_ptr::<heap::ObjectHeader>() };
        let value_size = std::mem::size_of::<Value>();
        if header.type_id != TypeId::MAP
            || !header.is_valid()
            || header.flags.contains(heap::ObjectFlags::VARIANT)
            || (header.size as usize != 3 * value_size && header.size as usize != 4 * value_size)
        {
            return Err(invalid("receiver does not have a flat Map header"));
        }
        // SAFETY: both supported runtime headers contain these three Value slots.
        let fields = unsafe {
            receiver
                .as_ptr::<u8>()
                .add(heap::OBJECT_HEADER_SIZE)
                .cast::<Value>()
        };
        // SAFETY: the same three-slot extent was established above.
        let (length, capacity, backing) = unsafe { (*fields, *fields.add(1), *fields.add(2)) };
        if !length.is_int() || !capacity.is_int() || length.as_i64() < 0 || capacity.as_i64() < 0 {
            return Err(invalid("length and capacity must be nonnegative integers"));
        }
        let capacity = usize::try_from(capacity.as_i64())
            .map_err(|_| invalid("capacity exceeds the address space"))?;
        if length.as_i64() as u64 > capacity as u64 {
            return Err(invalid("length exceeds capacity"));
        }
        if capacity == 0 {
            // The source constructor uses integer zero; runtime constructors may
            // use nil. Neither authorizes a header read from an absent allocation.
            if backing.is_nil() || (backing.is_int() && backing.as_i64() == 0) {
                return Ok(Self {
                    _heap: std::marker::PhantomData,
                    header: fields as *mut Value,
                    entries: std::ptr::null_mut(),
                    capacity,
                    len: 0,
                    backing: None,
                });
            }
            return Err(invalid("zero capacity has a backing allocation"));
        }
        let bytes = capacity
            .checked_mul(2)
            .and_then(|slots| slots.checked_mul(value_size))
            .ok_or_else(|| invalid("capacity byte size overflows"))?;
        if !backing.is_regular_ptr()
            || backing.is_nil()
            || !state.heap.contains(backing.as_ptr::<heap::ObjectHeader>())
        {
            return Err(invalid("backing is not an owned object"));
        }
        // SAFETY: exact heap membership establishes the backing header. The
        // allocation role distinguishes real buckets from identically sized
        // ordinary records and arrays.
        let backing_header = unsafe { &*backing.as_ptr::<heap::ObjectHeader>() };
        if backing == receiver
            || !backing_header
                .flags
                .contains(heap::ObjectFlags::HASH_BUCKETS)
            || !backing_header.is_valid()
            || backing_header.flags.contains(heap::ObjectFlags::VARIANT)
            || backing_header.size as usize != bytes
        {
            return Err(invalid("backing does not match the flat key/value extent"));
        }
        // SAFETY: the validated allocation contains exactly capacity key/value
        // pairs. Probing is finite even for a full table, and no user callback or
        // table mutation occurs during the existing semantic hash/equality checks.
        let entries = unsafe {
            backing
                .as_ptr::<u8>()
                .add(heap::OBJECT_HEADER_SIZE)
                .cast::<Value>()
        };
        Ok(Self {
            _heap: std::marker::PhantomData,
            header: fields as *mut Value,
            entries,
            capacity,
            len: length.as_i64() as usize,
            backing: Some(backing),
        })
    }

    fn insertion_growth(&self) -> Option<usize> {
        if self.capacity == 0 {
            Some(16)
        } else if self.len >= self.capacity - self.capacity / 4 {
            // The validated bucket byte extent fits the u32 ObjectHeader.
            Some(self.capacity * 2)
        } else {
            None
        }
    }

    fn probe(&self, key: Value, state: &InterpreterState) -> Option<(usize, bool)> {
        if self.capacity == 0 {
            return None;
        }
        let mut index = value_hash(key, state) % self.capacity;
        for _ in 0..self.capacity {
            // SAFETY: the checked backing contains two Values per bucket.
            let stored = unsafe { *self.entries.add(index * 2) };
            if stored.is_vacant_hash_key() {
                return Some((index, false));
            }
            if value_eq(stored, key, state) {
                return Some((index, true));
            }
            index = (index + 1) % self.capacity;
        }
        None
    }

    fn pair(&self, index: usize) -> [*mut Value; 2] {
        debug_assert!(index < self.capacity);
        // SAFETY: callers obtain the index from a bounded probe of this table.
        unsafe { [self.entries.add(index * 2), self.entries.add(index * 2 + 1)] }
    }

    pub(super) fn lookup(&self, key: Value, state: &InterpreterState) -> Option<[*mut Value; 2]> {
        self.probe(key, state)
            .and_then(|(index, occupied)| occupied.then(|| self.pair(index)))
    }

    /// The checked allocation contains exactly two Values per bucket. Consume
    /// this view before executing user code that can mutate or release storage.
    pub(super) fn occupied(&self) -> impl Iterator<Item = [Value; 2]> + '_ {
        (0..self.capacity).filter_map(|index| {
            // SAFETY: checked() validated both allocation extents.
            let pair = unsafe { *self.entries.add(index * 2).cast::<[Value; 2]>() };
            (!pair[0].is_vacant_hash_key()).then_some(pair)
        })
    }
}

/// Allocate and populate a replacement before publishing it. Failure leaves
/// existing entry witnesses and the original table intact.
pub(super) fn resize(
    state: &mut InterpreterState,
    receiver: Value,
    capacity: usize,
) -> InterpreterResult<()> {
    let old = FlatMap::checked(state, receiver)?;
    if capacity == 0 || capacity < old.len {
        return Err(InterpreterError::InvalidOperand {
            message: "Map resize would discard entries".into(),
        });
    }
    let storage = state.heap.alloc_hash_buckets(capacity)?;
    state.record_allocation();
    let entries = storage.data_ptr().cast::<[Value; 2]>();
    // SAFETY: alloc_hash_buckets allocated exactly capacity pairs.
    let entries = unsafe { std::slice::from_raw_parts_mut(entries, capacity) };
    let old = FlatMap::checked(state, receiver)?;
    let populated = old.occupied().try_for_each(|pair| {
        let start = value_hash(pair[0], state) % capacity;
        let index = (start..capacity)
            .chain(0..start)
            .find(|index| entries[*index][0].is_vacant_hash_key())?;
        entries[index] = pair;
        Some(())
    });
    if populated.is_none() {
        // SAFETY: storage is still private to this operation.
        unsafe { state.heap.free(storage) };
        return Err(InterpreterError::InvalidOperand {
            message: "Map length disagrees with occupied buckets".into(),
        });
    }
    let backing = old.backing;
    let header = old.header;
    state
        .heap
        .revoke_projections(receiver.as_ptr::<u8>() as usize);
    // SAFETY: fields and backing belong to this live map; no user code runs
    // between installing the new backing and freeing the superseded one.
    unsafe {
        *header.add(1) = Value::from_i64(capacity as i64);
        *header.add(2) = Value::from_ptr(storage.as_ptr().cast::<u8>());
        if let Some(backing) = backing {
            state
                .heap
                .free(heap::Object::from_raw(backing.as_ptr()).expect("checked map backing"));
        }
    }
    Ok(())
}

pub(super) fn ensure_capacity(
    state: &mut InterpreterState,
    receiver: Value,
) -> InterpreterResult<()> {
    if let Some(capacity) = FlatMap::checked(state, receiver)?.insertion_growth() {
        resize(state, receiver, capacity)?;
    }
    Ok(())
}

pub(super) enum Insertion {
    Inserted(*mut Value),
    Replaced(Value),
    Occupied,
}

/// Probe before growing: replacing an existing value and rejecting an occupied
/// entry cannot consume capacity. The returned slot is the actual chosen bucket.
pub(super) fn insert(
    state: &mut InterpreterState,
    receiver: Value,
    key: Value,
    value: Value,
    replace: bool,
) -> InterpreterResult<Insertion> {
    let table = FlatMap::checked(state, receiver)?;
    let probe = table.probe(key, state);
    if let Some((index, true)) = probe {
        if !replace {
            return Ok(Insertion::Occupied);
        }
        let slot = table.pair(index)[1];
        state
            .heap
            .revoke_projections(receiver.as_ptr::<u8>() as usize);
        // SAFETY: no rehashing or user code between the probe and this store.
        return Ok(Insertion::Replaced(unsafe {
            std::ptr::replace(slot, value)
        }));
    }
    let (table, vacant) = if let Some(capacity) = table.insertion_growth() {
        resize(state, receiver, capacity)?;
        let table = FlatMap::checked(state, receiver)?;
        let vacant = table.probe(key, state);
        (table, vacant)
    } else {
        (table, probe)
    };
    let Some((index, false)) = vacant else {
        return Err(InterpreterError::InvalidOperand {
            message: "Map has no vacant bucket after growth".into(),
        });
    };
    let [key_slot, value_slot] = table.pair(index);
    let (header, length) = (table.header, table.len);
    state
        .heap
        .revoke_projections(receiver.as_ptr::<u8>() as usize);
    // SAFETY: both slots came from the vacant probe of the current backing.
    unsafe {
        *key_slot = key;
        *value_slot = value;
        *header = Value::from_i64((length + 1) as i64);
    }
    Ok(Insertion::Inserted(value_slot))
}

/// Select the VM representation only when both the allocation and the exact
/// source owner authorize it. Native compilation and source-layout records use
/// the original declaration's Drop unconditionally.
pub(super) fn entry_plans(
    state: &InterpreterState,
    value: Value,
    plan: &DropPlan,
) -> InterpreterResult<Option<[DropPlan; 2]>> {
    if !FlatMap::is_carrier(state, value) {
        return Ok(None);
    }
    let Some(args) = plan.standard_record_arguments(&state.module, WellKnownType::Map) else {
        return Ok(None);
    };
    let [key, value] = args else {
        return Err(InterpreterError::Panic {
            message: "Map carrier requires its declared key and value witnesses".into(),
        });
    };
    let plan = |ty| {
        drop_plan::build(&state.module, ty).map_err(|message| InterpreterError::Panic {
            message: message.to_string(),
        })
    };
    Ok(Some([plan(key)?, plan(value)?]))
}

/// Issue a slot witness only after establishing the actual owner/backing
/// path and deriving a full Value-sized offset inside that allocation.
pub(super) fn borrow_slot(
    state: &mut InterpreterState,
    receiver: Value,
    slot: *mut Value,
    caps: crate::value::Capabilities,
) -> InterpreterResult<Value> {
    let table = FlatMap::checked(state, receiver)?;
    let backing = table.backing.ok_or(InterpreterError::NullPointer)?;
    let offset = (slot as usize)
        .checked_sub(table.entries as usize)
        .ok_or_else(|| InterpreterError::InvalidOperand {
            message: "slot precedes its backing".into(),
        })?;
    state.heap.project_slot(
        receiver.as_ptr::<u8>() as usize,
        2,
        backing.as_ptr::<u8>() as usize,
        offset,
        caps,
    )
}

/// Move the entries out of an exclusively borrowed map. No raw pointer crosses
/// a destructor callback. Trivial keys and values require no staging allocation.
pub(super) fn take_entries(
    state: &mut InterpreterState,
    value: Value,
    retain: bool,
) -> InterpreterResult<Vec<[Value; 2]>> {
    state.heap.revoke_projections(value.as_ptr::<u8>() as usize);
    let table = FlatMap::checked(state, value)?;
    let mut entries = Vec::new();
    if retain {
        entries.reserve(table.len);
        entries.extend(table.occupied());
    }
    // SAFETY: checked() validated both extents. No other heap operation or
    // callback can intervene during the exclusive state borrow. Invalidate the
    // backing pointer before freeing it so aliases see the empty map.
    let allocation = unsafe {
        *table.header = Value::from_i64(0);
        *table.header.add(1) = Value::from_i64(0);
        *table.header.add(2) = Value::nil();
        table
            .backing
            .and_then(|backing| heap::Object::from_raw(backing.as_ptr()))
    };
    if let Some(allocation) = allocation {
        // SAFETY: the receiver was detached above, before any user callback.
        unsafe { state.heap.free(allocation) };
    }
    Ok(entries)
}
