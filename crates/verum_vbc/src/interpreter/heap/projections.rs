//! Heap-local, revocable handles to Value slots. Distinct issuer tickets and
//! allocation generations prevent a recycled address from reviving a borrow.
use super::{Heap, OBJECT_HEADER_SIZE, ObjectHeader};
use crate::interpreter::{CbgrViolationKind, InterpreterError, InterpreterResult};
use crate::value::{Capabilities, ThinRef, Value};
use std::collections::HashMap;

#[derive(Debug, Default)]
pub(super) struct ProjectionTable {
    owners: HashMap<usize, OwnerProjections>,
    slots: HashMap<u64, HeapSlotRef>,
}
#[derive(Debug, Default)]
struct OwnerProjections {
    slots: HashMap<HeapSlotRef, u64>,
    exclusive: bool,
}

/// A typed slot witness is owned by the heap, independent of the process-wide
/// tables used for direct ThinRefs. It carries no unchecked interior address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct HeapSlotRef {
    owner: usize,
    backing: usize,
    backing_field: usize,
    offset: usize,
    generation: u32,
    epoch: u16,
    caps: Capabilities,
}

impl Heap {
    /// Project a checked Value slot. The producer has proved the allocation
    /// path and passes owner/backing bases and a byte offset, never an
    /// unchecked interior pointer in the returned handle.
    pub(crate) fn project_slot(
        &mut self,
        owner: usize,
        backing_field: usize,
        backing: usize,
        offset: usize,
        caps: Capabilities,
    ) -> InterpreterResult<Value> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT_TICKET: AtomicU64 = AtomicU64::new(1);
        if !self.contains(backing as *const ObjectHeader) {
            return Err(InterpreterError::CbgrViolation {
                kind: CbgrViolationKind::UseAfterFree,
                ptr: backing,
            });
        }
        // SAFETY: membership above proves an exact live allocation base.
        let header = unsafe { &*(backing as *const ObjectHeader) };
        let target = HeapSlotRef {
            owner,
            backing_field,
            backing,
            offset,
            caps,
            generation: header.generation,
            epoch: header.epoch,
        };
        self.resolve_slot(target)?;
        let exclusive = caps.has(Capabilities::WRITE);
        if exclusive
            || self
                .projections
                .owners
                .get(&owner)
                .is_some_and(|previous| previous.exclusive)
        {
            self.revoke_projections(owner);
        }
        let owner_slots = self.projections.owners.entry(owner).or_default();
        owner_slots.exclusive = exclusive;
        let id = match owner_slots.slots.entry(target) {
            std::collections::hash_map::Entry::Occupied(previous) => *previous.get(),
            std::collections::hash_map::Entry::Vacant(vacant) => {
                let id = NEXT_TICKET
                    .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| {
                        id.checked_add(1)
                            .filter(|next| Value::projected_slot(*next).is_some())
                    })
                    .map_err(|_| InterpreterError::InvalidOperand {
                        message: "managed projection tickets exhausted".into(),
                    })?;
                self.projections.slots.insert(id, target);
                *vacant.insert(id)
            }
        };
        Ok(Value::projected_slot(id).expect("ticket bounded by the handle format"))
    }

    pub(crate) fn revoke_projections(&mut self, owner: usize) {
        if let Some(slots) = self.projections.owners.remove(&owner) {
            for id in slots.slots.into_values() {
                self.projections.slots.remove(&id);
            }
        }
    }

    /// Resolve direct and managed thin references at the memory boundary.
    pub(crate) fn thin_ref(&self, value: Value) -> InterpreterResult<ThinRef> {
        if let Some(id) = value.projection_id() {
            let slot = self
                .projections
                .slots
                .get(&id)
                .ok_or(InterpreterError::CbgrViolation {
                    kind: CbgrViolationKind::UseAfterFree,
                    ptr: 0,
                })?;
            return self.resolve_slot(*slot);
        }
        value.raw_thin_ref()
    }

    fn resolve_slot(&self, slot: HeapSlotRef) -> InterpreterResult<ThinRef> {
        let stale = || InterpreterError::CbgrViolation {
            kind: CbgrViolationKind::UseAfterFree,
            ptr: slot.owner,
        };
        if !self.contains(slot.owner as *const ObjectHeader)
            || !self.contains(slot.backing as *const ObjectHeader)
        {
            return Err(stale());
        }
        // SAFETY: both bases belong to this heap. No heap mutation or callback
        // occurs between this extent check and the caller's immediate access.
        let owner = unsafe { &*(slot.owner as *const ObjectHeader) };
        let field_offset = slot
            .backing_field
            .checked_mul(std::mem::size_of::<Value>())
            .ok_or_else(stale)?;
        if !owner.is_valid()
            || field_offset
                .checked_add(std::mem::size_of::<Value>())
                .is_none_or(|end| end > owner.size as usize)
        {
            return Err(stale());
        }
        let current_backing = unsafe {
            *((slot.owner as *const u8)
                .add(OBJECT_HEADER_SIZE + field_offset)
                .cast::<Value>())
        };
        if !current_backing.is_regular_ptr()
            || current_backing.as_ptr::<u8>() as usize != slot.backing
        {
            return Err(stale());
        }
        let header = unsafe { &*(slot.backing as *const ObjectHeader) };
        if header.generation != slot.generation
            || header.epoch != slot.epoch
            || !header.is_valid()
            || slot.offset % std::mem::align_of::<Value>() != 0
            || slot
                .offset
                .checked_add(std::mem::size_of::<Value>())
                .is_none_or(|end| end > header.size as usize)
        {
            return Err(stale());
        }
        let ptr = unsafe { (slot.backing as *mut u8).add(OBJECT_HEADER_SIZE + slot.offset) };
        Ok(ThinRef::new(
            ptr,
            header.generation,
            header.epoch,
            slot.caps,
        ))
    }
}
