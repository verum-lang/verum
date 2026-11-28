//! Heap-local, revocable handles to typed elements and sequence ranges.
//! Distinct issuer tickets and allocation generations prevent address reuse
//! from reviving a borrow.
use super::{Heap, OBJECT_HEADER_SIZE, ObjectHeader};
use crate::interpreter::{CbgrViolationKind, InterpreterError, InterpreterResult};
use crate::value::{Capabilities, ThinRef, Value};
use verum_common::Map;

#[derive(Debug, Default)]
pub(super) struct ProjectionTable {
    owners: Map<usize, OwnerProjections>,
    slots: Map<u64, HeapSlotRef>,
    /// Physical addresses exported to unsafe code remain usable by the native
    /// ABI. Only the field producer can associate one with a managed place.
    exposed_fields: Map<usize, Value>,
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
    shape: ProjectionShape,
    container: Option<ContainerLink>,
}

/// Physical encoding comes from the existing runtime storage descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ProjectionEncoding {
    Value,
    Byte,
}
impl ProjectionEncoding {
    fn width(self) -> usize {
        match self {
            Self::Value => std::mem::size_of::<Value>(),
            Self::Byte => 1,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ProjectionShape {
    Element(ProjectionEncoding),
    Slice {
        len: usize,
        encoding: ProjectionEncoding,
    },
}
impl ProjectionShape {
    fn layout(self) -> (usize, ProjectionEncoding) {
        match self {
            Self::Element(encoding) => (1, encoding),
            Self::Slice { len, encoding } => (len, encoding),
        }
    }
}
/// The owned container is distinct from its replaceable allocation. Snapshot
/// both the owner incarnation and its exact backing field; a live old buffer
/// alone cannot keep a removed view valid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ContainerLink {
    owner: usize,
    generation: u32,
    epoch: u16,
    backing_field: usize,
    backing_value: Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum SlotStorage {
    Object {
        backing: usize,
        backing_field: Option<usize>,
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
            shape: ProjectionShape::Element(ProjectionEncoding::Value),
            container: None,
            storage: SlotStorage::Object {
                backing_field: Some(backing_field),
                backing,
                generation: header.generation,
                epoch: header.epoch,
            },
        };
        self.issue_projection(target)
    }

    /// Borrow a Value field of the exact managed object. Unlike a collection
    /// element, the storage has no replaceable backing allocation. Sibling
    /// field references coexist and retire together with their owner.
    pub(crate) fn project_object_field(
        &mut self,
        owner: usize,
        offset: usize,
        caps: Capabilities,
    ) -> InterpreterResult<Value> {
        if !self.contains(owner as *const ObjectHeader) {
            return Err(InterpreterError::CbgrViolation {
                kind: CbgrViolationKind::UseAfterFree,
                ptr: owner,
            });
        }
        // SAFETY: exact membership proves a live ObjectHeader.
        let header = unsafe { &*(owner as *const ObjectHeader) };
        let target = HeapSlotRef {
            owner,
            offset,
            caps,
            shape: ProjectionShape::Element(ProjectionEncoding::Value),
            container: None,
            storage: SlotStorage::Object {
                backing: owner,
                backing_field: None,
                generation: header.generation,
                epoch: header.epoch,
            },
        };
        self.resolve_slot(target)?;
        self.cache_projection(target)
    }

    /// Keep the exact owner incarnation alongside a declared record field's
    /// physical address. Repeated exports of the same field reuse its ticket.
    pub(crate) fn expose_object_field(
        &mut self,
        owner: usize,
        offset: usize,
    ) -> InterpreterResult<()> {
        // A physical field address supplies ordinary storage access. It does
        // not convey ownership, delegation, exclusivity or destruction rights.
        let access = Capabilities::new(Capabilities::READ | Capabilities::WRITE);
        let reference = self.project_object_field(owner, offset, access)?;
        let address = owner + OBJECT_HEADER_SIZE + offset;
        self.projections.exposed_fields.insert(address, reference);
        Ok(())
    }

    pub(crate) fn borrow_exposed_field(
        &mut self,
        address: usize,
        allowed: u16,
    ) -> InterpreterResult<Option<Value>> {
        let Some(reference) = self.projections.exposed_fields.get(&address).copied() else {
            return Ok(None);
        };
        self.attenuate_reference(reference, allowed).map(Some)
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
            shape: ProjectionShape::Element(ProjectionEncoding::Value),
            container: None,
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
        if let Some(id) = value.projection_id() {
            let mut target = self.projection_target(id)?;
            self.resolve_slot(target)?;
            target.caps = target.caps.attenuate(!allowed);
            self.cache_projection(target)
        } else {
            Ok(Value::from_thin_ref(
                self.thin_ref(value)?.attenuate(!allowed),
            ))
        }
    }

    pub(crate) fn revoke_projections(&mut self, owner: usize) {
        if let Some(slots) = self.projections.owners.remove(&owner) {
            for (target, id) in slots.slots {
                self.projections.slots.remove(&id);
                if let SlotStorage::Object { backing_field: None, backing, .. } = target.storage
                    && backing == owner
                    && !self.projections.exposed_fields.is_empty()
                {
                    let address = owner + OBJECT_HEADER_SIZE + target.offset;
                    self.projections.exposed_fields.remove(&address);
                }
            }
        }
    }

    /// Read a preserved reference snapshot without accessing its pointee.
    /// Register references, raw addresses and raw projection tickets do not
    /// carry an epoch snapshot; current heap state cannot reconstruct it.
    pub(crate) fn reference_epoch_caps(&self, value: Value) -> InterpreterResult<u32> {
        if let Some(id) = value.projection_id() {
            let slot = self
                .projections
                .slots
                .get(&id)
                .ok_or(InterpreterError::CbgrViolation {
                    kind: CbgrViolationKind::UseAfterFree,
                    ptr: 0,
                })?;
            if let SlotStorage::Object { epoch, .. } = slot.storage {
                return Ok((u32::from(epoch) << verum_common::layout::CAPS_BITS)
                    | u32::from(slot.caps.0));
            }
        } else if let Some(reference) = value.try_as_thin_ref() {
            return Ok(reference.epoch_caps);
        } else if let Some(reference) = value.try_as_fat_ref() {
            return Ok(reference.thin.epoch_caps);
        }
        Err(InterpreterError::InvalidOperand {
            message: "GetEpochCaps requires a preserved reference epoch/capability snapshot".into(),
        })
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
            if slot.shape != ProjectionShape::Element(ProjectionEncoding::Value) {
                return Err(invalid("typed sequence projection is not a Value slot"));
            }
            return self.resolve_slot(*slot);
        }
        value.raw_thin_ref()
    }

    fn projection_target(&self, id: u64) -> InterpreterResult<HeapSlotRef> {
        self.projections
            .slots
            .get(&id)
            .copied()
            .ok_or(InterpreterError::CbgrViolation {
                kind: CbgrViolationKind::UseAfterFree,
                ptr: 0,
            })
    }

    /// The caller supplies a checked storage view from the existing container
    /// resolver. Raw storage is registered under its allocator base so the
    /// existing deallocation path revokes every derived ticket before release.
    pub(crate) fn project_sequence(
        &mut self,
        owner: usize,
        backing_field: usize,
        backing_value: Value,
        data: *mut u8,
        raw_extent: Option<(usize, usize)>,
        len: usize,
        encoding: ProjectionEncoding,
        caps: Capabilities,
    ) -> InterpreterResult<Value> {
        if !self.contains(owner as *const ObjectHeader) {
            return Err(invalid("sequence owner is not a live managed object"));
        }
        // SAFETY: the existing storage resolver and membership prove the owner.
        let header = unsafe { &*(owner as *const ObjectHeader) };
        let link = ContainerLink {
            owner,
            generation: header.generation,
            epoch: header.epoch,
            backing_field,
            backing_value,
        };
        let backing = self.backing_view(data);
        let (root, offset, storage) = if let Some(base) = backing.owner {
            // SAFETY: backing_view established exact managed membership.
            let header = unsafe { &*base.cast::<ObjectHeader>() };
            (
                owner,
                0,
                SlotStorage::Object {
                    backing: base as usize,
                    backing_field: None,
                    generation: header.generation,
                    epoch: header.epoch,
                },
            )
        } else if let Some((base, extent)) = raw_extent {
            let offset = (data as usize)
                .checked_sub(base)
                .ok_or_else(|| invalid("sequence starts before its allocation"))?;
            (base, offset, SlotStorage::Raw { len: extent })
        } else if len == 0 && data.is_null() {
            // Empty containers have no backing. A zero-length span still has
            // its actual owner incarnation and cannot authorize an element.
            (
                owner,
                0,
                SlotStorage::Object {
                    backing: owner,
                    backing_field: None,
                    generation: header.generation,
                    epoch: header.epoch,
                },
            )
        } else {
            return Err(invalid("sequence backing has no live allocation authority"));
        };
        let target = HeapSlotRef {
            owner: root,
            offset,
            caps,
            storage,
            shape: ProjectionShape::Slice { len, encoding },
            container: Some(link),
        };
        self.issue_projection(target)
    }

    pub(crate) fn projection_capabilities(
        &self,
        value: Value,
    ) -> InterpreterResult<Option<Capabilities>> {
        value
            .projection_id()
            .map(|id| {
                self.resolve_slot(self.projection_target(id)?)
                    .map(|r| r.capabilities())
            })
            .transpose()
    }

    pub(crate) fn projection_len(&self, value: Value) -> InterpreterResult<Option<usize>> {
        let Some(id) = value.projection_id() else {
            return Ok(None);
        };
        let target = self.projection_target(id)?;
        require(
            self.resolve_slot(target)?.capabilities(),
            Capabilities::READ,
        )?;
        Ok(match target.shape {
            ProjectionShape::Slice { len, .. } => Some(len),
            _ => None,
        })
    }

    /// Derived projections share their parent's revocation domain. Creating a
    /// subview must not issue a new exclusive owner borrow and retire its parent.
    pub(crate) fn project_subsequence(
        &mut self,
        source: Value,
        start: usize,
        len: usize,
        allowed: Capabilities,
    ) -> InterpreterResult<Value> {
        let mut target = self.projection_target(
            source
                .projection_id()
                .ok_or_else(|| invalid("expected a sequence projection"))?,
        )?;
        let ProjectionShape::Slice {
            len: available,
            encoding,
        } = target.shape
        else {
            return Err(invalid("expected a sequence projection"));
        };
        let rights = self
            .resolve_slot(target)?
            .capabilities()
            .intersect(&allowed);
        require(rights, Capabilities::READ)?;
        if start > available || len > available - start {
            return Err(invalid("slice range exceeds its owner-backed view"));
        }
        target.offset = start
            .checked_mul(encoding.width())
            .and_then(|bytes| target.offset.checked_add(bytes))
            .ok_or_else(|| invalid("slice range byte offset overflow"))?;
        target.shape = ProjectionShape::Slice { len, encoding };
        target.caps = rights;
        self.resolve_slot(target)?;
        self.cache_projection(target)
    }

    pub(crate) fn project_sequence_element(
        &mut self,
        source: Value,
        index: usize,
        allowed: Capabilities,
    ) -> InterpreterResult<Value> {
        let mut target = self.sequence_element(source, index, allowed)?;
        let (_, encoding) = target.shape.layout();
        target.shape = ProjectionShape::Element(encoding);
        self.cache_projection(target)
    }

    fn sequence_element(
        &self,
        source: Value,
        index: usize,
        allowed: Capabilities,
    ) -> InterpreterResult<HeapSlotRef> {
        let mut target = self.projection_target(
            source
                .projection_id()
                .ok_or_else(|| invalid("expected a sequence projection"))?,
        )?;
        let ProjectionShape::Slice { len, encoding } = target.shape else {
            return Err(invalid("expected a sequence projection"));
        };
        let rights = self
            .resolve_slot(target)?
            .capabilities()
            .intersect(&allowed);
        require(rights, Capabilities::READ)?;
        if index >= len {
            return Err(InterpreterError::IndexOutOfBounds {
                index: index as i64,
                length: len,
            });
        }
        target.offset = index
            .checked_mul(encoding.width())
            .and_then(|n| target.offset.checked_add(n))
            .ok_or_else(|| invalid("sequence element offset overflow"))?;
        target.shape = ProjectionShape::Element(encoding);
        target.caps = rights;
        Ok(target)
    }

    /// Sized raw operations may access a managed element only in its complete
    /// physical encoding. A ticket never authorizes a partial Value or an
    /// unchecked address, even when its source reference uses the unsafe tier.
    fn scalar_projection(
        &self,
        value: Value,
        width: u8,
    ) -> InterpreterResult<Option<HeapSlotRef>> {
        let Some(id) = value.projection_id() else {
            return Ok(None);
        };
        let target = self.projection_target(id)?;
        let ProjectionShape::Element(encoding) = target.shape else {
            return Err(invalid("a sequence view is not a scalar element"));
        };
        if usize::from(width) != encoding.width() {
            return Err(invalid("scalar width does not match managed element encoding"));
        }
        Ok(Some(target))
    }

    pub(crate) fn read_projected_scalar(
        &self,
        value: Value,
        width: u8,
    ) -> InterpreterResult<Option<Value>> {
        self.scalar_projection(value, width)?
            .map(|target| self.read_projection(target))
            .transpose()
    }

    /// A typed dereference reads the whole VM value from a slot, or decodes
    /// the declared scalar from a packed element of the same physical width.
    pub(crate) fn read_projected_scalar_reference(
        &self,
        reference: Value,
        kind: crate::module::CType,
        width: u8,
    ) -> InterpreterResult<Option<Value>> {
        let Some(id) = reference.projection_id() else {
            return Ok(None);
        };
        let target = self.projection_target(id)?;
        if target.shape == ProjectionShape::Element(ProjectionEncoding::Byte) {
            if width != 1 {
                return Err(invalid("scalar width exceeds packed element storage"));
            }
            let byte = self.read_projection(target)?.as_i64() as u8;
            return crate::ffi::read_native_scalar(kind, &[byte])
                .map(Some)
                .ok_or_else(|| invalid("invalid packed scalar type"));
        }
        self.read_projection(target).map(Some)
    }

    pub(crate) fn write_projected_scalar(
        &self,
        target: Value,
        value: Value,
        width: u8,
    ) -> InterpreterResult<bool> {
        let Some(target) = self.scalar_projection(target, width)? else {
            return Ok(false);
        };
        self.write_projection(target, value)?;
        Ok(true)
    }

    /// Resolve an initialized, readable and writable element before an exchange.
    pub(crate) fn mutable_projection_storage(
        &self,
        reference: Value,
    ) -> InterpreterResult<Option<(*mut u8, ProjectionEncoding)>> {
        let Some(id) = reference.projection_id() else {
            return Ok(None);
        };
        let target = self.projection_target(id)?;
        let ProjectionShape::Element(encoding) = target.shape else {
            return Err(invalid("a sequence view is not an element slot"));
        };
        let slot = self.resolve_slot(target)?;
        let mut required = Capabilities::READ | Capabilities::WRITE;
        if target.container.is_some() {
            required |= Capabilities::EXCLUSIVE;
        }
        require(slot.capabilities(), required)?;
        Ok(Some((slot.ptr, encoding)))
    }

    pub(crate) fn read_projected_element(&self, value: Value) -> InterpreterResult<Value> {
        let target = self.projection_target(
            value
                .projection_id()
                .ok_or_else(|| invalid("expected an element projection"))?,
        )?;
        self.read_projection(target)
    }
    pub(crate) fn read_projected_index(
        &self,
        value: Value,
        index: usize,
    ) -> InterpreterResult<Value> {
        self.read_projection(self.sequence_element(value, index, Capabilities::FULL)?)
    }
    fn read_projection(&self, target: HeapSlotRef) -> InterpreterResult<Value> {
        let ProjectionShape::Element(encoding) = target.shape else {
            return Err(invalid("a sequence view is not an element slot"));
        };
        let reference = self.resolve_slot(target)?;
        require(reference.capabilities(), Capabilities::READ)?;
        // SAFETY: lifetime and the encoding-specific complete extent were checked.
        Ok(unsafe {
            match encoding {
                ProjectionEncoding::Value => reference.ptr.cast::<Value>().read(),
                ProjectionEncoding::Byte => Value::from_i64(i64::from(reference.ptr.read())),
            }
        })
    }
    pub(crate) fn write_projected_element(
        &self,
        target: Value,
        value: Value,
    ) -> InterpreterResult<()> {
        self.write_projection(
            self.projection_target(
                target
                    .projection_id()
                    .ok_or_else(|| invalid("expected an element projection"))?,
            )?,
            value,
        )
    }
    pub(crate) fn write_projected_index(
        &self,
        target: Value,
        index: usize,
        value: Value,
        allowed: Capabilities,
    ) -> InterpreterResult<()> {
        self.write_projection(self.sequence_element(target, index, allowed)?, value)
    }
    fn write_projection(&self, target: HeapSlotRef, value: Value) -> InterpreterResult<()> {
        let ProjectionShape::Element(encoding) = target.shape else {
            return Err(invalid("a sequence view is not an element slot"));
        };
        let reference = self.resolve_slot(target)?;
        require(
            reference.capabilities(),
            if target.container.is_some() {
                Capabilities::WRITE | Capabilities::EXCLUSIVE
            } else {
                Capabilities::WRITE
            },
        )?;
        match encoding {
            ProjectionEncoding::Value => {
                // SAFETY: the checked, writable extent is one complete Value slot.
                unsafe { reference.ptr.cast::<Value>().write(value) };
            }
            ProjectionEncoding::Byte => {
                let byte = value
                    .try_as_i64()
                    .and_then(|n| u8::try_from(n).ok())
                    .ok_or_else(|| invalid("packed Byte projection requires a value in 0..=255"))?;
                // SAFETY: the checked, writable extent is exactly one byte.
                unsafe { reference.ptr.write(byte) };
            }
        }
        Ok(())
    }

    fn resolve_slot(&self, slot: HeapSlotRef) -> InterpreterResult<ThinRef> {
        let stale = || InterpreterError::CbgrViolation {
            kind: CbgrViolationKind::UseAfterFree,
            ptr: slot.owner,
        };
        let (count, encoding) = slot.shape.layout();
        let bytes = count.checked_mul(encoding.width()).ok_or_else(stale)?;
        let alignment = encoding.width();
        let mut capabilities = slot.caps;
        if let Some(link) = slot.container {
            if !self.contains(link.owner as *const ObjectHeader) {
                return Err(stale());
            }
            // SAFETY: exact membership precedes the retained owner snapshot check.
            let owner = unsafe { &*(link.owner as *const ObjectHeader) };
            let field_offset = link
                .backing_field
                .checked_mul(std::mem::size_of::<Value>())
                .ok_or_else(stale)?;
            if !owner.is_valid()
                || owner.generation != link.generation
                || owner.epoch != link.epoch
                || field_offset
                    .checked_add(std::mem::size_of::<Value>())
                    .is_none_or(|end| end > owner.size as usize)
            {
                return Err(stale());
            }
            // SAFETY: the checked live owner extent contains this backing field.
            let current = unsafe {
                *((link.owner as *const u8)
                    .add(OBJECT_HEADER_SIZE + field_offset)
                    .cast::<Value>())
            };
            if current != link.backing_value {
                return Err(stale());
            }
            capabilities = restrict_storage_access(capabilities, owner.capabilities);
        }
        let (backing, backing_field, generation, epoch) = match slot.storage {
            SlotStorage::Raw { len } => {
                let address = slot.owner.checked_add(slot.offset).ok_or_else(stale)?;
                if !address.is_multiple_of(alignment)
                    || slot.offset.checked_add(bytes).is_none_or(|end| end > len)
                {
                    return Err(stale());
                }
                // Raw allocations use revocable tickets as their lifetime
                // authority. A freed allocation cannot reissue an old ticket.
                return Ok(ThinRef::new(address as *mut u8, 0, 0, capabilities));
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
        if !owner.is_valid() {
            return Err(stale());
        }
        if let Some(backing_field) = backing_field {
            let field_offset = backing_field
                .checked_mul(std::mem::size_of::<Value>())
                .ok_or_else(stale)?;
            if field_offset
                .checked_add(std::mem::size_of::<Value>())
                .is_none_or(|end| end > owner.size as usize)
            {
                return Err(stale());
            }
            // SAFETY: the live owner and checked field extent contain a Value.
            let current_backing = unsafe {
                *((slot.owner as *const u8)
                    .add(OBJECT_HEADER_SIZE + field_offset)
                    .cast::<Value>())
            };
            if !current_backing.is_regular_ptr()
                || current_backing.as_ptr::<u8>() as usize != backing
            {
                return Err(stale());
            }
        }
        let header = unsafe { &*(backing as *const ObjectHeader) };
        if header.generation != generation
            || header.epoch != epoch
            || !header.is_valid()
            || slot.offset % alignment != 0
            || slot
                .offset
                .checked_add(bytes)
                .is_none_or(|end| end > header.size as usize)
        {
            return Err(stale());
        }
        let ptr = unsafe { (backing as *mut u8).add(OBJECT_HEADER_SIZE + slot.offset) };
        Ok(ThinRef::new(
            ptr,
            header.generation,
            header.epoch,
            restrict_storage_access(capabilities, header.capabilities),
        ))
    }
}

fn invalid(message: &str) -> InterpreterError {
    InterpreterError::InvalidOperand {
        message: message.into(),
    }
}
fn require(actual: Capabilities, required: u16) -> InterpreterResult<()> {
    if actual.0 & required == required {
        Ok(())
    } else {
        Err(InterpreterError::CapabilityDenied {
            required: u64::from(required),
            actual: u64::from(actual.0),
        })
    }
}

/// Allocation headers restrict memory access. Exclusivity and delegation belong
/// to the incoming reference; a READ|WRITE allocation neither grants nor erases
/// that borrow authority.
fn restrict_storage_access(reference: Capabilities, allocation: u16) -> Capabilities {
    reference.attenuate((Capabilities::READ | Capabilities::WRITE) & !allocation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TypeId;

    #[test]
    fn exposed_record_field_views_restrict_rights_and_expire_with_the_owner() {
        let mut heap = Heap::new();
        let stride = std::mem::size_of::<Value>();
        let object = heap.alloc(TypeId(50_001), 2 * stride).unwrap();
        let first = object.data_ptr() as usize;
        let second = first + stride;
        let owner = object.as_ptr() as usize;
        heap.expose_object_field(owner, stride).unwrap();

        let read = heap.borrow_exposed_field(second, Capabilities::READ_ONLY.0).unwrap().unwrap();
        let write = heap.borrow_exposed_field(second, u16::MAX).unwrap().unwrap();
        let rights = |heap: &Heap, view| heap.projection_capabilities(view).unwrap().unwrap().0;
        assert_eq!(rights(&heap, read), Capabilities::READ);
        assert_eq!(rights(&heap, write), Capabilities::READ | Capabilities::WRITE);
        assert!(heap.borrow_exposed_field(first, u16::MAX).unwrap().is_none());
        heap.expose_object_field(owner, stride).unwrap();
        assert_eq!(heap.borrow_exposed_field(second, u16::MAX).unwrap().unwrap().bits(), write.bits());

        // SAFETY: the heap owns this live object. Only tickets are examined after free.
        unsafe { heap.free(object) };
        assert!(heap.borrow_exposed_field(second, u16::MAX).unwrap().is_none());
        for view in [read, write] {
            assert!(matches!(heap.projection_capabilities(view),
                Err(InterpreterError::CbgrViolation { kind: CbgrViolationKind::UseAfterFree, .. })));
        }
    }
}
