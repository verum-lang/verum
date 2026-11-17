//! Opaque runtime owners retain the declaration supplied by their producer.
//! The payload's object tag and a pointer-shaped word never supply that witness.
use super::{Heap, OBJECT_HEADER_SIZE, Object, ObjectHeader};
use crate::{
    TypeRef, Value,
    interpreter::{InterpreterError, InterpreterResult},
    value::Capabilities,
};

pub(super) struct PacketReceipt {
    generation: u32,
    ticket: Value,
    declaration: TypeRef,
}

/// A consuming operation takes the payload and its declaration together.
/// Dropping this Rust carrier does not execute Verum destruction: the caller
/// must run the shared typed plan or transfer both parts to another owner.
pub(crate) struct OwnedPayload {
    pub value: Value,
    pub declaration: TypeRef,
}

fn invalid(message: &str) -> InterpreterError {
    InterpreterError::InvalidOperand {
        message: message.into(),
    }
}

impl Heap {
    /// The executing producer has resolved the declaration and validated its
    /// ordinary owned-value plan before handing its payload to this operation.
    /// Allocation failure leaves the input payload with that producer.
    pub(crate) fn alloc_owned_packet(&mut self, payload: OwnedPayload) -> InterpreterResult<Value> {
        if !crate::mono::is_concrete_type(&payload.declaration) {
            return Err(invalid(
                "owned packet requires a resolved producer declaration",
            ));
        }
        let object = self.alloc(
            crate::TypeId(verum_common::layout::SYNTHETIC_RECORD_TYPE_ID),
            std::mem::size_of::<Value>(),
        )?;
        // SAFETY: this producer owns the exact one-Value allocation. The header
        // is only a storage tag; the separate receipt carries its declaration.
        unsafe {
            object.data_ptr().cast::<Value>().write(payload.value);
        }
        let ticket =
            match self.project_object_field(object.as_ptr() as usize, 0, Capabilities::READ_ONLY) {
                Ok(ticket) => ticket,
                Err(error) => {
                    // SAFETY: only the unpublished carrier was allocated here;
                    // no lifecycle operation has consumed its payload on failure.
                    unsafe {
                        self.free(object);
                    }
                    return Err(error);
                }
            };
        self.owned_packets.insert(
            object.as_ptr() as usize,
            PacketReceipt {
                generation: object.generation(),
                ticket,
                declaration: payload.declaration,
            },
        );
        Ok(ticket)
    }

    fn owned_packet_receipt(
        &self,
        packet: Value,
    ) -> InterpreterResult<(*mut ObjectHeader, &PacketReceipt)> {
        if packet.projection_id().is_none() {
            return Err(invalid("owned packet requires its producer-issued carrier"));
        }
        // CBGR validates the globally unique ticket and allocation generation
        // before yielding an address. Removing the carrier revokes this ticket;
        // reallocating the same address cannot revive a retired packet owner.
        let reference = self.thin_ref(packet)?;
        let address = (reference.ptr as usize)
            .checked_sub(OBJECT_HEADER_SIZE)
            .ok_or_else(|| invalid("owned packet has an invalid payload address"))?;
        let pointer = address as *mut ObjectHeader;
        let receipt = self
            .owned_packets
            .get(&(pointer as usize))
            .filter(|receipt| receipt.ticket == packet && self.contains(pointer))
            .ok_or_else(|| invalid("owned packet requires its producer-issued carrier"))?;
        // SAFETY: exact managed membership is checked before reading a header.
        let header = unsafe { &*pointer };
        if !header.is_valid()
            || header.generation != receipt.generation
            || header.size as usize != std::mem::size_of::<Value>()
        {
            return Err(invalid("owned packet carrier disagrees with its producer"));
        }
        Ok((pointer, receipt))
    }

    /// Borrowing observes the payload. It does not create a second owner.
    pub(crate) fn borrow_owned_packet(
        &self,
        packet: Value,
    ) -> InterpreterResult<(Value, &TypeRef)> {
        let (pointer, receipt) = self.owned_packet_receipt(packet)?;
        // SAFETY: the exact receipt proves the complete one-Value payload.
        let value = unsafe {
            pointer
                .cast::<u8>()
                .add(OBJECT_HEADER_SIZE)
                .cast::<Value>()
                .read()
        };
        Ok((value, &receipt.declaration))
    }

    /// GetException transfers the complete v64 one-Text-field panic record.
    /// The allocation and its message move together. The private ticket ends here.
    pub(crate) fn publish_panic_record(&mut self, packet: Value) -> InterpreterResult<Value> {
        let (pointer, receipt) = self.owned_packet_receipt(packet)?;
        if receipt.declaration != TypeRef::Concrete(crate::TypeId::TEXT) {
            return Err(invalid(
                "runtime panic record requires its declared Text message",
            ));
        }
        self.owned_packets.remove(&(pointer as usize));
        self.revoke_projections(pointer as usize);
        Ok(Value::from_ptr(pointer))
    }

    /// Detach before running user code or publishing another owner. Retiring
    /// the carrier also removes its producer receipt from the allocation table.
    pub(crate) fn take_owned_packet(&mut self, packet: Value) -> InterpreterResult<OwnedPayload> {
        let (pointer, _) = self.owned_packet_receipt(packet)?;
        let receipt = self
            .owned_packets
            .remove(&(pointer as usize))
            .expect("validated owned packet");
        // SAFETY: the receipt proves a live exact allocation; only this owner
        // can consume it, and the payload has moved to the returned carrier.
        let object = unsafe { Object::from_raw(pointer) }.expect("validated owned packet");
        let value = unsafe { object.data_ptr().cast::<Value>().read() };
        unsafe {
            self.free(object);
        }
        Ok(OwnedPayload {
            value,
            declaration: receipt.declaration,
        })
    }
}

#[cfg(test)]
#[path = "../../../tests/interpreter/owned_packets.rs"]
mod tests;
