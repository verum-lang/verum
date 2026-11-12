//! Storage views for declared native products. Array layout identity is
//! retained in VBC; the live allocation supplies its element representation.
use crate::{
    interpreter::{OBJECT_HEADER_SIZE, ObjectHeader, typed_array_element_spec},
    module::{CType, FfiStructLayout},
    types::TypeId,
    value::Value,
};
use verum_common::List;

pub(super) struct Slots {
    data: *mut u8,
    stride: usize,
    packed: bool,
}

impl Slots {
    /// The caller owns a live object and any backing storage it references.
    pub(super) unsafe fn new(layout: &FfiStructLayout, object: *const u8) -> Option<Self> {
        // SAFETY: the caller supplies a live object. Every payload read is
        // checked against its allocation header before pointer arithmetic.
        unsafe {
            let header = ObjectHeader::try_from_ptr(object)?;
            let mut data = object.add(OBJECT_HEADER_SIZE) as *mut u8;
            let mut available = header.size as usize;
            let mut stride = size_of::<Value>();
            let mut packed = false;
            let mut fixed = false;
            if layout.verum_type == Some(TypeId::ARRAY) {
                fixed = true;
                if let Some((width, _)) = typed_array_element_spec(header.type_id) {
                    stride = width;
                    packed = true;
                } else if header.type_id == TypeId::LIST {
                    if available < 3 * size_of::<Value>() {
                        return None;
                    }
                    let values = data as *const Value;
                    let length = usize::try_from((*values).try_as_i64()?).ok()?;
                    if length != layout.fields.len() {
                        return None;
                    }
                    let backing = *values.add(2);
                    if !backing.is_regular_ptr() {
                        return None;
                    }
                    let base = backing.as_ptr::<u8>();
                    let backing_header = ObjectHeader::try_from_ptr(base)?;
                    available = backing_header.size as usize;
                    data = base.add(OBJECT_HEADER_SIZE);
                    fixed = false; // List capacity may exceed its checked length.
                } else if header.type_id != TypeId::ARRAY {
                    return None;
                }
            }
            let required = layout.fields.len().checked_mul(stride)?;
            if required > available || (fixed && required != available) {
                return None;
            }
            Some(Self {
                data,
                stride,
                packed,
            })
        }
    }

    /// `index` is a declared slot in the layout used to create this view.
    pub(super) unsafe fn read(&self, index: usize, ty: CType) -> Option<Value> {
        // SAFETY: new checked all declared slots; the caller stays in that range.
        unsafe {
            let at = self.data.add(index * self.stride);
            if self.packed {
                super::read_native_scalar(ty, std::slice::from_raw_parts(at, self.stride))
            } else {
                Some(*(at as *const Value))
            }
        }
    }

    /// The allocation is writable, with the same slot precondition as read.
    pub(super) unsafe fn write(&self, index: usize, ty: CType, bytes: &[u8]) {
        // SAFETY: new checked every destination slot; packed writes also
        // require the exact native element width.
        unsafe {
            let at = self.data.add(index * self.stride);
            if self.packed {
                if bytes.len() == self.stride {
                    std::ptr::copy_nonoverlapping(bytes.as_ptr(), at, bytes.len());
                }
            } else if let Some(value) = super::read_native_scalar(ty, bytes) {
                *(at as *mut Value) = value;
            }
        }
    }
}

/// Validate all carriers before a foreign call can observe a partial record.
/// The caller owns each live object in the aggregate.
pub(super) unsafe fn validate(
    root: &FfiStructLayout,
    layouts: &[FfiStructLayout],
    object: *const u8,
) -> Result<(), super::runtime::FfiError> {
    let invalid =
        || super::runtime::FfiError::CallFailed("native aggregate has incompatible storage".into());
    let mut pending = List::new();
    pending.push((root, object, 0));
    while let Some((layout, object, depth)) = pending.pop() {
        if depth >= super::runtime::MAX_FFI_STRUCT_NESTING {
            return Err(invalid());
        }
        // SAFETY: root ownership and each checked nested pointer establish
        // the caller's live aggregate contract; slots enforce allocation bounds.
        let slots = unsafe { Slots::new(layout, object) }.ok_or_else(invalid)?;
        for (index, field) in layout.fields.iter().enumerate() {
            if field
                .offset
                .checked_add(u32::from(field.size))
                .is_none_or(|end| end > layout.size)
            {
                return Err(invalid());
            }
            // SAFETY: index enumerates the slots checked by Slots::new.
            let value = unsafe { slots.read(index, field.c_type) }.ok_or_else(invalid)?;
            if field.c_type == CType::StructValue {
                let child = field
                    .nested_layout
                    .and_then(|index| layouts.get(index as usize))
                    .ok_or_else(invalid)?;
                if child.size != u32::from(field.size) {
                    return Err(invalid());
                }
                if !value.is_regular_ptr() {
                    return Err(invalid());
                }
                pending.push((child, value.as_ptr(), depth + 1));
            }
        }
    }
    Ok(())
}
