use super::*;
use crate::{TypeId, types::TypeParamId};

fn payload(value: i64) -> OwnedPayload {
    OwnedPayload {
        value: Value::from_i64(value),
        declaration: TypeRef::Concrete(TypeId::I64),
    }
}

#[test]
fn packet_ticket_retains_the_exact_declaration_and_transfers_once() {
    let mut heap = Heap::new();
    let ticket = heap.alloc_owned_packet(payload(73)).unwrap();
    let (value, declaration) = heap.borrow_owned_packet(ticket).unwrap();
    assert_eq!(value.try_as_i64(), Some(73));
    assert_eq!(*declaration, TypeRef::Concrete(TypeId::I64));
    assert_eq!(heap.stats().total_frees, 0);
    let owner = heap.take_owned_packet(ticket).unwrap();
    assert_eq!(owner.value.try_as_i64(), Some(73));
    assert_eq!(owner.declaration, TypeRef::Concrete(TypeId::I64));
    assert!(heap.borrow_owned_packet(ticket).is_err());
    assert!(heap.take_owned_packet(ticket).is_err());
    assert_eq!(heap.stats().total_allocs, heap.stats().total_frees);
}

#[test]
fn retired_packet_ticket_does_not_authorize_a_later_owner_or_another_heap() {
    let mut first = Heap::new();
    let retired = first.alloc_owned_packet(payload(11)).unwrap();
    first.take_owned_packet(retired).unwrap();
    let current = first.alloc_owned_packet(payload(73)).unwrap();
    assert_ne!(retired, current);
    assert!(first.take_owned_packet(retired).is_err());
    let mut other = Heap::new();
    let other_ticket = other.alloc_owned_packet(payload(97)).unwrap();
    assert!(other.borrow_owned_packet(current).is_err());
    assert!(first.borrow_owned_packet(other_ticket).is_err());
    assert_eq!(
        first.take_owned_packet(current).unwrap().value.try_as_i64(),
        Some(73)
    );
    assert_eq!(
        other
            .take_owned_packet(other_ticket)
            .unwrap()
            .value
            .try_as_i64(),
        Some(97)
    );
}

#[test]
fn heap_reset_retires_packet_receipts_and_does_not_revive_their_tickets() {
    let mut heap = Heap::new();
    let retired = heap.alloc_owned_packet(payload(11)).unwrap();
    assert_eq!(heap.owned_packets.len(), 1);
    // SAFETY: this test holds no live dereference while resetting its heap.
    unsafe {
        heap.clear();
    }
    assert!(heap.owned_packets.is_empty());
    assert!(heap.take_owned_packet(retired).is_err());
    let current = heap.alloc_owned_packet(payload(73)).unwrap();
    assert_ne!(retired, current);
    assert!(heap.borrow_owned_packet(retired).is_err());
    assert_eq!(
        heap.take_owned_packet(current).unwrap().value.try_as_i64(),
        Some(73)
    );
}

#[test]
fn raw_pointer_and_ordinary_projection_do_not_issue_packet_ownership() {
    let mut heap = Heap::new();
    let object = heap
        .alloc(
            TypeId(verum_common::layout::SYNTHETIC_RECORD_TYPE_ID),
            std::mem::size_of::<Value>(),
        )
        .unwrap();
    let raw = Value::from_ptr(object.as_ptr().cast::<u8>());
    let borrowed = heap
        .project_object_field(object.as_ptr() as usize, 0, Capabilities::READ_ONLY)
        .unwrap();
    assert!(heap.take_owned_packet(raw).is_err());
    assert!(heap.take_owned_packet(borrowed).is_err());
    assert!(heap.contains(object.as_ptr()));
    // SAFETY: this test still owns the exact ordinary allocation.
    unsafe {
        heap.free(object);
    }
    assert_eq!(heap.stats().total_allocs, heap.stats().total_frees);
}

#[test]
fn unresolved_packet_declaration_refuses_before_allocation() {
    let mut heap = Heap::new();
    let before = heap.stats().total_allocs;
    let result = heap.alloc_owned_packet(OwnedPayload {
        value: Value::from_i64(73),
        declaration: TypeRef::Generic(TypeParamId(19)),
    });
    assert!(result.is_err());
    assert_eq!(heap.stats().total_allocs, before);
}

#[test]
fn panic_record_handoff_requires_the_text_declaration_and_retires_its_ticket() {
    let mut heap = Heap::new();
    let integer = heap.alloc_owned_packet(payload(73)).unwrap();
    assert!(heap.publish_panic_record(integer).is_err());
    assert_eq!(
        heap.take_owned_packet(integer).unwrap().value.try_as_i64(),
        Some(73)
    );

    let message = Value::from_small_string("panic").unwrap();
    let owner = heap
        .alloc_owned_packet(OwnedPayload {
            value: message,
            declaration: TypeRef::Concrete(TypeId::TEXT),
        })
        .unwrap();
    let record = heap.publish_panic_record(owner).unwrap();
    assert!(heap.take_owned_packet(owner).is_err());
    assert!(heap.publish_panic_record(owner).is_err());
    assert!(heap.contains(record.as_ptr()));
    // SAFETY: publishing transferred precisely this live one-Value record.
    let object = unsafe { Object::from_raw(record.as_ptr()) }.unwrap();
    assert_eq!(unsafe { object.data_ptr().cast::<Value>().read() }, message);
    unsafe {
        heap.free(object);
    }
    assert_eq!(heap.stats().total_allocs, heap.stats().total_frees);
}

#[test]
fn taking_the_carrier_preserves_the_payload_allocation() {
    let mut heap = Heap::new();
    let object = heap
        .alloc(TypeId::TUPLE, std::mem::size_of::<Value>())
        .unwrap();
    // SAFETY: the producer owns one complete Value slot.
    unsafe {
        object.data_ptr().cast::<Value>().write(Value::from_i64(73));
    }
    let value = Value::from_ptr(object.as_ptr().cast::<u8>());
    let ticket = heap
        .alloc_owned_packet(OwnedPayload {
            value,
            declaration: TypeRef::Tuple(vec![TypeRef::Concrete(TypeId::I64)]),
        })
        .unwrap();
    assert_eq!(heap.take_owned_packet(ticket).unwrap().value, value);
    assert!(heap.contains(object.as_ptr()));
    assert_eq!(heap.stats().total_frees, 1);
    // SAFETY: take transferred this exact payload to the test; no other owner exists.
    unsafe {
        heap.free(object);
    }
    assert_eq!(heap.stats().total_allocs, heap.stats().total_frees);
}
