//! Declaration-owned copying is prepared privately, then committed atomically.
//! No user callback runs between source inspection and publication.
use super::super::super::{
    error::{InterpreterError, InterpreterResult},
    heap,
    state::InterpreterState,
};
use super::super::DispatchResult;
use super::{
    bytecode_io::{read_reg, read_type_ref},
    cbgr_helpers, list_storage,
    retain_copy::{PreparedRetain, RetainedCopy},
};
use crate::{
    copy_plan::{self, CopyFailure, CopyLimits, CopyNode, CopyPlan, NodeId},
    types::{CbgrTier, TypeId, TypeRef},
    value::Value,
};
use verum_common::{List, Set, Text};

fn invalid(message: impl Into<Text>) -> InterpreterError {
    InterpreterError::InvalidOperand {
        message: format!(
            "{}: {}",
            CopyFailure::InvalidStorage.message(),
            message.into()
        ),
    }
}
fn failed(reason: CopyFailure) -> InterpreterError {
    InterpreterError::InvalidOperand {
        message: reason.message().into(),
    }
}
fn allocation_error(error: InterpreterError) -> InterpreterError {
    InterpreterError::InvalidOperand {
        message: format!("{}: {error}", CopyFailure::AllocationFailure.message()),
    }
}
fn preparation_error(error: InterpreterError) -> InterpreterError {
    match error {
        error @ (InterpreterError::OutOfMemory { .. }
        | InterpreterError::AllocationTooLarge { .. }) => allocation_error(error),
        error => invalid(error.to_string()),
    }
}
fn push<T>(items: &mut List<T>, item: T) -> InterpreterResult<()> {
    items
        .try_reserve(1)
        .map_err(|_| failed(CopyFailure::AllocationFailure))?;
    items.push(item);
    Ok(())
}

struct Budget {
    work: usize,
    bytes: usize,
}
impl Budget {
    fn allocation(&mut self, bytes: usize) -> InterpreterResult<()> {
        self.bytes = self
            .bytes
            .checked_sub(bytes)
            .ok_or_else(|| failed(CopyFailure::AllocationLimit))?;
        Ok(())
    }
    fn step(&mut self) -> InterpreterResult<()> {
        self.work = self
            .work
            .checked_sub(1)
            .ok_or_else(|| failed(CopyFailure::WorkLimit))?;
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Destination {
    Root,
    Field { parent: usize, index: usize },
    Element { parent: usize, index: usize },
}
struct Slot {
    source: *const Value,
    index: usize,
    node: NodeId,
    element: bool,
}
enum Children<'a> {
    None,
    Fields {
        source: *const Value,
        fields: &'a [NodeId],
    },
    Elements {
        source: *const Value,
        length: usize,
        node: NodeId,
    },
    Map {
        source: *const Value,
        capacity: usize,
        stride: usize,
        key: NodeId,
        value: NodeId,
    },
    Ring {
        source: *const Value,
        capacity: usize,
        length: usize,
        head: usize,
        node: NodeId,
    },
}
impl Children<'_> {
    fn vacant(&self, index: usize) -> bool {
        let Self::Map {
            source,
            capacity,
            stride,
            ..
        } = self
        else {
            return false;
        };
        if index >= capacity * 2 {
            return false;
        }
        // SAFETY: inspection proved the complete bucket extent and typed hashes.
        unsafe {
            let entry = source.add(index / 2 * stride);
            if *stride == 2 {
                entry.read_unaligned().is_vacant_hash_key()
            } else {
                entry.add(2).read_unaligned().as_i64() <= 0
            }
        }
    }
    fn at(&self, index: usize) -> Option<Slot> {
        let (source, index, node, element) = match self {
            Self::None => return None,
            Self::Fields { source, fields } => (*source, index, *fields.get(index)?, false),
            Self::Elements {
                source,
                length,
                node,
            } => {
                if index >= *length {
                    return None;
                }
                (*source, index, *node, true)
            }
            Self::Map {
                source,
                capacity,
                stride,
                key,
                value,
            } => {
                if index >= capacity * 2 {
                    return None;
                }
                (
                    *source,
                    index / 2 * stride + index % 2,
                    if index % 2 == 0 { *key } else { *value },
                    true,
                )
            }
            Self::Ring {
                source,
                capacity,
                length,
                head,
                node,
            } => {
                if index >= *length {
                    return None;
                }
                (*source, (head + index) % capacity, *node, true)
            }
        };
        // SAFETY: inspection proved each initialized field/backing extent.
        Some(Slot {
            source: unsafe { source.add(index) },
            index,
            node,
            element,
        })
    }
}
struct Frame<'a> {
    owner: usize,
    operation: usize,
    next: usize,
    children: Children<'a>,
}
#[derive(Clone, Copy)]
struct ObjectCopy {
    source: *const u8,
    owner: TypeId,
    bytes: usize,
    flags: heap::ObjectFlags,
    field_offset: usize,
}
#[derive(Clone, Copy)]
enum Backing {
    Raw,
    HashBuckets { capacity: usize },
}
enum Operation {
    Identity(Value),
    Object(ObjectCopy),
    List(list_storage::CopyElements),
    Backed {
        object: ObjectCopy,
        source: *const u8,
        extent: usize,
        field: usize,
        backing: Backing,
    },
    Retained {
        source: Value,
        counter: Value,
        layout: crate::retain_copy::RetainLayout,
    },
}
struct Step {
    operation: Operation,
    destination: Destination,
}
struct Schedule {
    steps: List<Step>,
    allocations: usize,
}
#[derive(Clone, Copy)]
struct Output {
    value: Value,
    fields: *mut Value,
    elements: *mut Value,
}
impl Output {
    fn scalar(value: Value) -> Self {
        Self {
            value,
            fields: std::ptr::null_mut(),
            elements: std::ptr::null_mut(),
        }
    }
}

fn object(state: &InterpreterState, value: Value, owner: TypeId) -> InterpreterResult<ObjectCopy> {
    if !value.is_regular_ptr() || value.is_nil() || !state.heap.contains(value.as_ptr()) {
        return Err(invalid("typed copy requires a live owned object"));
    }
    let pointer = value.as_ptr::<heap::ObjectHeader>();
    // SAFETY: exact allocation membership proves this header remains live.
    let header = unsafe { &*pointer };
    if !header.is_valid() || header.type_id != owner {
        return Err(invalid(
            "typed copy nominal owner differs from its declaration",
        ));
    }
    Ok(ObjectCopy {
        source: pointer.cast(),
        owner,
        bytes: header.size as usize,
        flags: header.flags,
        field_offset: 0,
    })
}
fn fields<'a>(
    state: &InterpreterState,
    source: Value,
    owner: TypeId,
    fields: &'a [NodeId],
    offset: usize,
    budget: &mut Budget,
) -> InterpreterResult<(Operation, Children<'a>, usize)> {
    let mut copy = object(state, source, owner)?;
    let slots = if offset == 0 {
        fields.len().max(1)
    } else {
        fields.len()
    };
    if slots.checked_mul(8).and_then(|n| n.checked_add(offset)) != Some(copy.bytes) {
        return Err(invalid(
            "typed copy object extent differs from its declaration",
        ));
    }
    // Empty records carry a VM padding slot, which is not logical payload.
    budget.allocation(offset + fields.len() * 8)?;
    copy.field_offset = offset;
    // SAFETY: the exact extent includes every declared field and the sum tag.
    let source = unsafe { copy.source.add(heap::OBJECT_HEADER_SIZE + offset).cast() };
    Ok((
        Operation::Object(copy),
        Children::Fields { source, fields },
        1,
    ))
}

/// Execute only the checked field path, using the existing reference/extent
/// authorities. The path cannot run arbitrary Clone code or change its source.
fn counter(
    state: &mut InterpreterState,
    source: Value,
    contract: &crate::retain_forward::RetainForward,
    budget: &mut Budget,
) -> InterpreterResult<Value> {
    let mut current = source;
    let mut raw_allowed = false;
    for (index, field) in contract.counter_path().iter().enumerate() {
        budget.step()?;
        let place = cbgr_helpers::resolve_receiver_place(state, current)
            .map_err(preparation_error)?
            .into_record(state)
            .map_err(preparation_error)?;
        let last = index + 1 == contract.counter_path().len();
        if last {
            place.require_write().map_err(preparation_error)?;
        }
        let offset = usize::from(field.index) * std::mem::size_of::<Value>();
        if let Some(slot) = cbgr_helpers::raw_record_field_slot(state, place.value, offset)
            .map_err(preparation_error)?
        {
            if !raw_allowed {
                return Err(invalid(
                    "retained counter raw projection lacks its unsafe declaration",
                ));
            }
            if last {
                let (owner, len) = super::cbgr::bridge_extent(state, slot as usize)
                    .ok_or_else(|| invalid("retained counter allocation disappeared"))?;
                return state
                    .heap
                    .project_raw_slot(owner, len, slot as usize - owner, place.capabilities)
                    .map_err(preparation_error);
            }
            // SAFETY: raw_record_field_slot proved this live Value-sized slot;
            // the verified path carries an explicit unsafe source declaration.
            current = unsafe { slot.read_unaligned() };
        } else {
            let owner = match field.record {
                TypeRef::Concrete(id) | TypeRef::Instantiated { base: id, .. } => id,
                _ => return Err(invalid("retained counter lost its record declaration")),
            };
            let copy = object(state, place.value, owner)?;
            let count = state
                .module
                .get_type(owner)
                .ok_or_else(|| invalid("retained counter owner is absent"))?
                .fields
                .len();
            if copy.bytes != count.max(1) * 8 || offset + 8 > copy.bytes {
                return Err(invalid(
                    "retained counter owner has a different field extent",
                ));
            }
            if last {
                return state
                    .heap
                    .project_object_field(copy.source as usize, offset, place.capabilities)
                    .map_err(preparation_error);
            }
            // SAFETY: exact live owner and field extent were checked above.
            current = unsafe {
                copy.source
                    .add(heap::OBJECT_HEADER_SIZE + offset)
                    .cast::<Value>()
                    .read_unaligned()
            };
        }
        raw_allowed = matches!(
            field.field,
            TypeRef::Reference {
                tier: CbgrTier::Tier2,
                ..
            }
        );
    }
    Err(invalid("retained counter path is empty"))
}

fn inspect<'a>(
    state: &mut InterpreterState,
    source: Value,
    plan: &'a CopyPlan,
    mut node: NodeId,
    budget: &mut Budget,
) -> InterpreterResult<(Operation, Children<'a>, usize, bool)> {
    for _ in 0..plan.nodes.len() {
        match plan
            .nodes
            .get(node.0)
            .ok_or_else(|| invalid("typed copy plan edge is absent"))?
        {
            CopyNode::Alias(next) => {
                budget.step()?;
                node = *next;
            }
            CopyNode::Identity => {
                return Ok((Operation::Identity(source), Children::None, 0, false));
            }
            CopyNode::Record {
                owner,
                fields: declared,
            } => {
                let (operation, children, allocations) =
                    fields(state, source, *owner, declared, 0, budget)?;
                return Ok((operation, children, allocations, true));
            }
            CopyNode::Sum { owner, variants } => {
                let copy = object(state, source, *owner)?;
                if copy.bytes < 8 || !copy.flags.contains(heap::ObjectFlags::VARIANT) {
                    return Err(invalid("typed copy requires a declared sum representation"));
                }
                // SAFETY: the exact live owner includes its complete sum tag.
                let tag = unsafe { heap::variant_tag(copy.source.cast()) };
                let declared = variants
                    .iter()
                    .find(|(selected, _)| *selected == tag)
                    .map(|(_, fields)| fields.as_slice())
                    .ok_or_else(|| invalid("typed copy sum tag is not declared"))?;
                let (operation, children, allocations) =
                    fields(state, source, *owner, declared, 8, budget)?;
                return Ok((operation, children, allocations, true));
            }
            CopyNode::List { element, .. } | CopyNode::Array { element, .. } => {
                let shape = list_storage::CopyElements::resolve(state, source)
                    .map_err(preparation_error)?;
                if let CopyNode::Array { length, .. } = &plan.nodes[node.0] {
                    if u64::try_from(shape.len).ok() != Some(*length) {
                        return Err(invalid(
                            "typed copy array length differs from its declaration",
                        ));
                    }
                }
                let scalar = matches!(plan.nodes.get(element.0), Some(CopyNode::Identity));
                if shape.width != std::mem::size_of::<Value>() && !scalar {
                    return Err(invalid("packed storage cannot carry owned compound values"));
                }
                budget.allocation(
                    shape
                        .allocation_bytes()
                        .ok_or_else(|| invalid("typed copy allocation extent overflow"))?,
                )?;
                let children = if scalar {
                    Children::None
                } else {
                    Children::Elements {
                        source: shape.data.cast(),
                        length: shape.len,
                        node: *element,
                    }
                };
                let allocations = shape.allocation_count();
                return Ok((Operation::List(shape), children, allocations, true));
            }
            CopyNode::Shared {
                owner, contract, ..
            } => {
                let copy = object(state, source, *owner)?;
                let counter = counter(state, source, contract, budget)?;
                let layout =
                    crate::retain_copy::layout(&state.module, contract.owner()).map_err(invalid)?;
                if copy.bytes != layout.fields.max(1) * 8 {
                    return Err(invalid(
                        "retained copy handle extent differs from its declaration",
                    ));
                }
                budget.allocation(copy.bytes)?;
                return Ok((
                    Operation::Retained {
                        source,
                        counter,
                        layout,
                    },
                    Children::None,
                    1,
                    false,
                ));
            }
            CopyNode::Map { owner, key, value } => {
                let copy = object(state, source, *owner)?;
                if copy.bytes != 3 * 8 && copy.bytes != 4 * 8 {
                    return Err(invalid("typed Map copy has an invalid owner extent"));
                }
                // SAFETY: the live owner's complete three/four fields were checked.
                let fields = unsafe { copy.source.add(heap::OBJECT_HEADER_SIZE).cast::<Value>() };
                let (len, cap, backing) = unsafe {
                    (
                        nonnegative(fields.read_unaligned())?,
                        nonnegative(fields.add(1).read_unaligned())?,
                        fields.add(2).read_unaligned(),
                    )
                };
                if len > cap {
                    return Err(invalid("typed Map copy length exceeds capacity"));
                }
                let address = super::ffi_extended::value_as_addr(backing) as *mut u8;
                let managed = state.heap.backing_view(address);
                // SAFETY: backing_view only yields an owner from live heap membership.
                let flat = managed.owner.is_some_and(|owner| unsafe {
                    (*owner.cast::<heap::ObjectHeader>())
                        .flags
                        .contains(heap::ObjectFlags::HASH_BUCKETS)
                });
                let stride = if flat { 2 } else { 4 };
                let extent = cap
                    .checked_mul(stride * 8)
                    .ok_or_else(|| invalid("typed Map copy extent overflow"))?;
                let data = if cap == 0 {
                    if !address.is_null() {
                        return Err(invalid("empty Map has unexpected backing"));
                    }
                    std::ptr::null_mut()
                } else if flat {
                    if managed.payload_size != Some(extent) {
                        return Err(invalid("typed Map bucket extent differs from capacity"));
                    }
                    managed.data
                } else {
                    if state.cbgr_bridge_extents.get(&(address as usize)) != Some(&extent) {
                        return Err(invalid("typed Map copy requires its owned Slot allocation"));
                    }
                    address
                };
                budget.allocation(
                    copy.bytes
                        .checked_add(extent)
                        .ok_or_else(|| invalid("typed copy allocation extent overflow"))?,
                )?;
                let mut occupied = 0;
                for bucket in 0..cap {
                    budget.step()?;
                    // SAFETY: every complete bucket lies within the validated extent.
                    let entry = unsafe { data.cast::<Value>().add(bucket * stride) };
                    let used = if flat {
                        !unsafe { entry.read_unaligned() }.is_vacant_hash_key()
                    } else {
                        let hash = unsafe { entry.add(2).read_unaligned() }
                            .try_as_i64()
                            .ok_or_else(|| invalid("typed Map Slot hash lacks its Int carrier"))?;
                        if hash < -1 {
                            return Err(invalid("typed Map Slot hash is invalid"));
                        }
                        hash > 0
                    };
                    occupied += usize::from(used);
                }
                if occupied != len {
                    return Err(invalid("typed Map initialized count differs from length"));
                }
                let operation = Operation::Backed {
                    object: copy,
                    source: data,
                    extent,
                    field: 2,
                    backing: if flat {
                        Backing::HashBuckets { capacity: cap }
                    } else {
                        Backing::Raw
                    },
                };
                return Ok((
                    operation,
                    Children::Map {
                        source: data.cast(),
                        capacity: cap,
                        stride,
                        key: *key,
                        value: *value,
                    },
                    1 + usize::from(extent != 0),
                    true,
                ));
            }
            CopyNode::Deque { owner, element } => {
                let copy = object(state, source, *owner)?;
                if copy.bytes != 4 * 8 {
                    return Err(invalid("typed Deque copy has an invalid owner extent"));
                }
                // SAFETY: the exact owner includes the four declared ring fields.
                let fields = unsafe { copy.source.add(heap::OBJECT_HEADER_SIZE).cast::<Value>() };
                let (backing, head, len, cap) = unsafe {
                    (
                        fields.read_unaligned(),
                        nonnegative(fields.add(1).read_unaligned())?,
                        nonnegative(fields.add(2).read_unaligned())?,
                        nonnegative(fields.add(3).read_unaligned())?,
                    )
                };
                if len > cap || (cap == 0 && head != 0) || (cap != 0 && head >= cap) {
                    return Err(invalid("typed Deque ring bounds are invalid"));
                }
                let address = super::ffi_extended::value_as_addr(backing) as *mut u8;
                let extent = cap
                    .checked_mul(8)
                    .ok_or_else(|| invalid("typed Deque extent overflow"))?;
                let view = state.heap.backing_view(address);
                let data = if cap == 0 {
                    if !address.is_null() {
                        return Err(invalid("empty Deque has backing"));
                    }
                    std::ptr::null_mut()
                } else if view.owner.is_some() {
                    if view.payload_size != Some(extent) {
                        return Err(invalid("Deque backing extent differs from capacity"));
                    }
                    view.data
                } else {
                    if state.cbgr_bridge_extents.get(&(address as usize)) != Some(&extent) {
                        return Err(invalid("Deque copy requires its owned ring allocation"));
                    }
                    address
                };
                budget.allocation(
                    copy.bytes
                        .checked_add(extent)
                        .ok_or_else(|| invalid("typed copy allocation extent overflow"))?,
                )?;
                return Ok((
                    Operation::Backed {
                        object: copy,
                        source: data,
                        extent,
                        field: 0,
                        backing: Backing::Raw,
                    },
                    Children::Ring {
                        source: data.cast(),
                        capacity: cap,
                        length: len,
                        head,
                        node: *element,
                    },
                    1 + usize::from(extent != 0),
                    true,
                ));
            }
        }
    }
    Err(invalid("typed copy alias graph is cyclic"))
}
fn nonnegative(value: Value) -> InterpreterResult<usize> {
    value
        .try_as_i64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| invalid("typed copy storage index is not a nonnegative Int"))
}

/// Preflight retains a linear operation schedule, not copied storage. A common
/// subgraph may appear repeatedly; only an active owning ancestor is a cycle.
fn preflight(
    state: &mut InterpreterState,
    source: Value,
    plan: &CopyPlan,
    limits: CopyLimits,
) -> InterpreterResult<Schedule> {
    for node in &plan.nodes {
        if let CopyNode::Shared { contract, .. } = node {
            contract.validate(&state.module).map_err(invalid)?;
        }
    }
    let mut budget = Budget {
        work: limits.max_work,
        bytes: limits.max_allocation_bytes,
    };
    let mut stack: List<Frame<'_>> = List::new();
    let mut active = Set::new();
    let mut schedule = Schedule {
        steps: List::new(),
        allocations: 0,
    };
    let mut next = Some((source, NodeId(0), Destination::Root));
    loop {
        if let Some((source, node, destination)) = next.take() {
            budget.step()?;
            let (operation, children, allocations, owning) =
                inspect(state, source, plan, node, &mut budget)?;
            let owner = if owning {
                source.as_ptr::<u8>() as usize
            } else {
                0
            };
            if owning {
                active
                    .try_reserve(1)
                    .map_err(|_| failed(CopyFailure::AllocationFailure))?;
                if !active.insert(owner) {
                    return Err(failed(CopyFailure::OwnedCycle));
                }
            }
            let operation_index = schedule.steps.len();
            push(
                &mut schedule.steps,
                Step {
                    operation,
                    destination,
                },
            )?;
            schedule.allocations = schedule
                .allocations
                .checked_add(allocations)
                .ok_or_else(|| invalid("typed copy allocation count overflow"))?;
            push(
                &mut stack,
                Frame {
                    owner,
                    operation: operation_index,
                    next: 0,
                    children,
                },
            )?;
        }
        let Some(frame) = stack.last_mut() else {
            break;
        };
        while frame.children.vacant(frame.next) {
            budget.step()?;
            frame.next += 1;
        }
        if let Some(slot) = frame.children.at(frame.next) {
            frame.next += 1;
            let destination = if slot.element {
                Destination::Element {
                    parent: frame.operation,
                    index: slot.index,
                }
            } else {
                Destination::Field {
                    parent: frame.operation,
                    index: slot.index,
                }
            };
            // SAFETY: inspection established the initialized source slot.
            next = Some((
                unsafe { slot.source.read_unaligned() },
                slot.node,
                destination,
            ));
        } else {
            let owner = frame.owner;
            stack.pop();
            if owner != 0 {
                active.remove(&owner);
            }
        }
    }
    Ok(schedule)
}

/// The ledger never grows after preparation begins. Cleanup frees only newly
/// allocated storage and atomically subtracts exactly its own successful retains.
enum Entry {
    Managed(heap::Object),
    Raw(i64),
    Prepared(PreparedRetain),
    Retained(RetainedCopy),
    Vacant,
}
struct Transaction {
    entries: List<Entry>,
}
impl Transaction {
    fn new(capacity: usize) -> InterpreterResult<Self> {
        let mut entries = List::new();
        entries
            .try_reserve_exact(capacity)
            .map_err(|_| failed(CopyFailure::AllocationFailure))?;
        Ok(Self { entries })
    }
    fn room(&self) -> InterpreterResult<()> {
        if self.entries.len() < self.entries.capacity() {
            Ok(())
        } else {
            Err(invalid(
                "typed copy preparation exceeded its reserved journal",
            ))
        }
    }
    fn managed(
        &mut self,
        state: &mut InterpreterState,
        owner: TypeId,
        bytes: usize,
    ) -> InterpreterResult<Output> {
        self.room()?;
        let object = state.heap.alloc(owner, bytes).map_err(allocation_error)?;
        state.record_allocation();
        let output = Output {
            value: Value::from_ptr(object.as_ptr().cast::<u8>()),
            fields: object.data_ptr().cast(),
            elements: std::ptr::null_mut(),
        };
        self.entries.push(Entry::Managed(object));
        Ok(output)
    }
    fn object(
        &mut self,
        state: &mut InterpreterState,
        copy: ObjectCopy,
    ) -> InterpreterResult<Output> {
        let mut output = self.managed(state, copy.owner, copy.bytes)?;
        // SAFETY: preflight proved the live source extent, and the newly
        // journaled allocation has exactly the same extent. No callback ran.
        unsafe {
            (*output.value.as_ptr::<heap::ObjectHeader>()).flags |=
                copy.flags & heap::ObjectFlags::VARIANT;
            std::ptr::copy_nonoverlapping(
                copy.source.add(heap::OBJECT_HEADER_SIZE),
                output.fields.cast(),
                copy.bytes,
            );
            output.fields = output.fields.cast::<u8>().add(copy.field_offset).cast();
        }
        Ok(output)
    }
    fn raw(
        &mut self,
        state: &mut InterpreterState,
        bytes: usize,
    ) -> InterpreterResult<(Value, *mut u8)> {
        self.room()?;
        let bytes = i64::try_from(bytes)
            .map_err(|_| invalid("typed copy backing extent exceeds allocator range"))?;
        let address = super::ffi_extended::cbgr_user_allocate(state, bytes, 8);
        if address == 0 {
            return Err(failed(CopyFailure::AllocationFailure));
        }
        self.entries.push(Entry::Raw(address));
        Ok((Value::from_i64(address), address as *mut u8))
    }
    fn prepare(
        &mut self,
        state: &mut InterpreterState,
        operation: &Operation,
    ) -> InterpreterResult<Output> {
        match operation {
            Operation::Identity(value) => Ok(Output::scalar(*value)),
            Operation::Object(copy) => self.object(state, *copy),
            Operation::Retained {
                source,
                counter,
                layout,
            } => {
                self.room()?;
                let prepared = PreparedRetain::prepare(state, *source, *counter, *layout)
                    .map_err(preparation_error)?;
                let output = Output::scalar(prepared.prepared_value());
                self.entries.push(Entry::Prepared(prepared));
                Ok(output)
            }
            Operation::List(shape) => {
                let mut output = self.managed(state, shape.encoding, 3 * 8)?;
                let backing = if shape.capacity == 0 {
                    Value::from_i64(0)
                } else {
                    let buffer =
                        self.managed(state, shape.encoding, shape.capacity * shape.width)?;
                    output.elements = buffer.fields;
                    // SAFETY: checked initialized prefix, equal backing extent;
                    // unused capacity is neither inspected nor recursively copied.
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            shape.data,
                            buffer.fields.cast::<u8>(),
                            shape.len * shape.width,
                        );
                    }
                    buffer.value
                };
                // SAFETY: the journal owns the complete three-field wrapper.
                unsafe {
                    output.fields.write(Value::from_i64(shape.len as i64));
                    output
                        .fields
                        .add(1)
                        .write(Value::from_i64(shape.capacity as i64));
                    output.fields.add(2).write(backing);
                }
                Ok(output)
            }
            Operation::Backed {
                object,
                source,
                extent,
                field,
                backing,
            } => {
                let mut output = self.object(state, *object)?;
                if *extent != 0 {
                    let (value, target) = match backing {
                        Backing::Raw => self.raw(state, *extent)?,
                        Backing::HashBuckets { capacity } => {
                            self.room()?;
                            let allocation = state
                                .heap
                                .alloc_hash_buckets(*capacity)
                                .map_err(allocation_error)?;
                            state.record_allocation();
                            let pair = (
                                Value::from_ptr(allocation.as_ptr().cast::<u8>()),
                                allocation.data_ptr(),
                            );
                            self.entries.push(Entry::Managed(allocation));
                            pair
                        }
                    };
                    // SAFETY: both complete backing extents were established;
                    // only occupied slots are later interpreted as values.
                    unsafe {
                        std::ptr::copy_nonoverlapping(*source, target, *extent);
                        output.fields.add(*field).write(value);
                    }
                    output.elements = target.cast();
                }
                Ok(output)
            }
        }
    }
    fn commit(&mut self, state: &mut InterpreterState) -> InterpreterResult<()> {
        for index in 0..self.entries.len() {
            let entry = std::mem::replace(&mut self.entries[index], Entry::Vacant);
            self.entries[index] = match entry {
                Entry::Prepared(prepared) => match prepared.retain(state) {
                    Ok(retained) => Entry::Retained(retained),
                    Err((prepared, _error)) => {
                        self.entries[index] = Entry::Prepared(prepared);
                        return Err(failed(CopyFailure::RetainRefused));
                    }
                },
                other => other,
            };
        }
        Ok(())
    }
    fn abort(mut self, state: &mut InterpreterState) -> InterpreterResult<()> {
        let mut failure = None;
        while let Some(entry) = self.entries.pop() {
            match entry {
                // SAFETY: every receipt is a distinct unpublished allocation;
                // no user Drop or traversal of aliased fields is performed.
                Entry::Managed(object) => unsafe {
                    state.heap.free(object);
                },
                Entry::Raw(address) => super::ffi_extended::cbgr_user_deallocate(state, address),
                Entry::Prepared(prepared) => prepared.abort(state),
                Entry::Retained(retained) => {
                    if retained.abort(state).is_err() && failure.is_none() {
                        failure = Some(failed(CopyFailure::RollbackFailure));
                    }
                }
                Entry::Vacant => (),
            }
        }
        failure.map_or(Ok(()), Err)
    }
    fn publish(mut self) {
        while let Some(entry) = self.entries.pop() {
            if let Entry::Retained(retained) = entry {
                retained.publish();
            }
        }
    }
}

fn execute(
    state: &mut InterpreterState,
    source: Value,
    plan: &CopyPlan,
    limits: CopyLimits,
) -> InterpreterResult<Value> {
    let schedule = preflight(state, source, plan, limits)?;
    let mut outputs: List<Output> = List::new();
    outputs
        .try_reserve_exact(schedule.steps.len())
        .map_err(|_| failed(CopyFailure::AllocationFailure))?;
    let mut transaction = Transaction::new(schedule.allocations)?;
    let prepared = (|| {
        for step in &schedule.steps {
            let output = transaction.prepare(state, &step.operation)?;
            let destination = match step.destination {
                Destination::Root => None,
                Destination::Field { parent, index } => Some((outputs[parent].fields, index)),
                Destination::Element { parent, index } => Some((outputs[parent].elements, index)),
            };
            if let Some((pointer, index)) = destination {
                // SAFETY: the schedule contains only checked parent slots;
                // every parent was prepared earlier and remains unpublished.
                unsafe {
                    pointer.add(index).write_unaligned(output.value);
                }
            }
            outputs.push(output);
        }
        transaction.commit(state)?;
        Ok(outputs[0].value)
    })();
    match prepared {
        Ok(value) => {
            transaction.publish();
            Ok(value)
        }
        Err(error) => {
            transaction.abort(state)?;
            Err(error)
        }
    }
}

pub(in super::super) fn handle(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let declared = read_type_ref(state)?;
    let declared = state.resolve_generic_witness(&declared)?;
    let plan = copy_plan::build(&state.module, &declared).map_err(invalid)?;
    let source = state.get_reg(src);
    let result = execute(state, source, &plan, state.config.typed_copy_limits)?;
    state.set_reg(dst, result);
    Ok(DispatchResult::Continue)
}

#[cfg(all(test, feature = "codegen"))]
#[path = "../../../../tests/unit/typed_copy_transactions.rs"]
mod tests;
