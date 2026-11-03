//! Checked view of the VM's flat, allocator-owned key/value table. The source
//! Robin Hood table has a different allocation contract and never enters here.
use super::super::super::{
    error::{InterpreterError, InterpreterResult},
    heap,
    state::InterpreterState,
};
use crate::{
    drop_plan::{self, DropPlan},
    types::{TypeId, TypeKind},
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
            || !backing_header.flags.contains(heap::ObjectFlags::HASH_BUCKETS)
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

    /// The checked allocation contains exactly two Values per bucket. Consume
    /// this view before executing user code that can mutate or release storage.
    pub(super) fn occupied(&self) -> impl Iterator<Item = [Value; 2]> + '_ {
        (0..self.capacity).filter_map(|index| {
            // SAFETY: checked() validated both allocation extents.
            let pair = unsafe { *self.entries.add(index * 2).cast::<[Value; 2]>() };
            (!pair[0].is_unit()).then_some(pair)
        })
    }
}

/// Select the VM representation only when both the allocation and the exact
/// source owner authorize it. Native compilation and source-layout records use
/// the original declaration's Drop unconditionally.
pub(super) fn entry_plans(
    state: &InterpreterState,
    value: Value,
    plan: &DropPlan,
) -> InterpreterResult<Option<[DropPlan; 2]>> {
    let (owner, args) = match plan {
        DropPlan::Glue {
            owner, owner_args, ..
        }
        | DropPlan::Record {
            owner, owner_args, ..
        } if !owner_args.is_empty() => (*owner, owner_args.as_slice()),
        _ => return Ok(None),
    };
    if !FlatMap::is_carrier(state, value) {
        return Ok(None);
    }
    let declaration = state
        .module
        .get_type(owner)
        .expect("DropPlan carries a declared owner");
    let name = state.module.get_string(declaration.name);
    let origin = declaration
        .origin_module
        .and_then(|id| state.module.get_string(id))
        .unwrap_or(&state.module.name);
    if declaration.kind != TypeKind::Record
        || name.and_then(|name| WellKnownType::from_declaration(name, origin))
            != Some(WellKnownType::Map)
    {
        return Ok(None);
    }
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

/// Move the entries out of an exclusively borrowed map. No raw pointer crosses
/// a destructor callback. Trivial keys and values require no staging allocation.
pub(super) fn take_entries(
    state: &mut InterpreterState,
    value: Value,
    retain: bool,
) -> InterpreterResult<Vec<[Value; 2]>> {
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
