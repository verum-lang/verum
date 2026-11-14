//! Heap-local, revocable handles to Value slots. Distinct issuer tickets and
//! allocation generations prevent a recycled address from reviving a borrow.
use super::{Heap, OBJECT_HEADER_SIZE, ObjectHeader};
use crate::interpreter::{CbgrViolationKind, InterpreterError, InterpreterResult};
use crate::value::{Capabilities, ThinRef, Value};
use verum_common::Map;

#[derive(Debug, Default)]
pub(super) struct ProjectionTable {
    owners: Map<usize, OwnerProjections>,
    slots: Map<u64, HeapSlotRef>,
}
#[derive(Debug, Default)]
struct OwnerProjections {
    slots: Map<HeapSlotRef, u64>,
    exclusive: bool,
}

/// A typed slot witness is owned by the heap, independent of the process-wide
/// tables used for direct ThinRefs. It carries no unchecked interior address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct HeapSlotRef {
    owner: usize,
    offset: usize,
    caps: Capabilities,
    storage: SlotStorage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum SlotStorage {
    Object {
        backing: usize,
        backing_field: usize,
        generation: u32,
        epoch: u16,
    },
    Raw {
        len: usize,
    },
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
            offset,
            caps,
            storage: SlotStorage::Object {
                backing_field,
                backing,
                generation: header.generation,
                epoch: header.epoch,
            },
        };
        self.issue_projection(target)
    }

    /// The caller proves the live raw allocation's base and extent. Its
    /// deallocator revokes the owner's tickets before releasing that storage.
    pub(crate) fn project_raw_slot(
        &mut self,
        owner: usize,
        len: usize,
        offset: usize,
        caps: Capabilities,
    ) -> InterpreterResult<Value> {
        let target = HeapSlotRef {
            owner,
            offset,
            caps,
            storage: SlotStorage::Raw { len },
        };
        self.resolve_slot(target)?;
        // A field projection keeps the allocation's storage fixed. Other
        // fields and shared views retain their tickets until owner retirement.
        self.cache_projection(target)
    }

    fn issue_projection(&mut self, target: HeapSlotRef) -> InterpreterResult<Value> {
        self.resolve_slot(target)?;
        let owner = target.owner;
        let exclusive = target.caps.has(Capabilities::WRITE);
        if exclusive
            || self
                .projections
                .owners
                .get(&owner)
                .is_some_and(|previous| previous.exclusive)
        {
            self.revoke_projections(owner);
        }
        self.cache_projection(target)
    }

    fn cache_projection(&mut self, target: HeapSlotRef) -> InterpreterResult<Value> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT_TICKET: AtomicU64 = AtomicU64::new(1);
        let owner_slots = self.projections.owners.entry(target.owner).or_default();
        owner_slots.exclusive |= target.caps.has(Capabilities::WRITE);
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

    /// Derive a restricted view while retaining the same revocable owner.
    pub(crate) fn attenuate_reference(
        &mut self,
        value: Value,
        allowed: u16,
    ) -> InterpreterResult<Value> {
        let reference = self.thin_ref(value)?;
        if let Some(id) = value.projection_id() {
            let mut target = self.projections.slots[&id];
            target.caps = target.caps.attenuate(!allowed);
            self.cache_projection(target)
        } else {
            Ok(Value::from_thin_ref(reference.attenuate(!allowed)))
        }
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
        let (backing, backing_field, generation, epoch) = match slot.storage {
            SlotStorage::Raw { len } => {
                let address = slot.owner.checked_add(slot.offset).ok_or_else(stale)?;
                if !address.is_multiple_of(std::mem::align_of::<Value>())
                    || slot
                        .offset
                        .checked_add(std::mem::size_of::<Value>())
                        .is_none_or(|end| end > len)
                {
                    return Err(stale());
                }
                // Raw allocations use revocable tickets as their lifetime
                // authority. A freed allocation cannot reissue an old ticket.
                return Ok(ThinRef::new(address as *mut u8, 0, 0, slot.caps));
            }
            SlotStorage::Object {
                backing,
                backing_field,
                generation,
                epoch,
            } => (backing, backing_field, generation, epoch),
        };
        if !self.contains(slot.owner as *const ObjectHeader)
            || !self.contains(backing as *const ObjectHeader)
        {
            return Err(stale());
        }
        // SAFETY: both bases belong to this heap. No heap mutation or callback
        // occurs between this extent check and the caller's immediate access.
        let owner = unsafe { &*(slot.owner as *const ObjectHeader) };
        let field_offset = backing_field
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
        if !current_backing.is_regular_ptr() || current_backing.as_ptr::<u8>() as usize != backing {
            return Err(stale());
        }
        let header = unsafe { &*(backing as *const ObjectHeader) };
        if header.generation != generation
            || header.epoch != epoch
            || !header.is_valid()
            || slot.offset % std::mem::align_of::<Value>() != 0
            || slot
                .offset
                .checked_add(std::mem::size_of::<Value>())
                .is_none_or(|end| end > header.size as usize)
        {
            return Err(stale());
        }
        let ptr = unsafe { (backing as *mut u8).add(OBJECT_HEADER_SIZE + slot.offset) };
        Ok(ThinRef::new(
            ptr,
            header.generation,
            header.epoch,
            slot.caps,
        ))
    }
}
