//! Internal heap authority controls; included by interpreter::heap.
use super::*;

#[test]
fn base_and_exact_data_share_one_tracked_payload() {
    let mut heap = Heap::new();
    let object = heap.alloc(TypeId::BYTE_LIST, 16).unwrap();
    let base = object.as_ptr() as *mut u8;
    let data = unsafe { base.add(OBJECT_HEADER_SIZE) };
    for address in [base, data] {
        let backing = heap.backing_view(address);
        assert_eq!(backing.data, data);
        assert_eq!(backing.owner, Some(base));
        assert_eq!(backing.payload_size, Some(16));
    }
}
#[test]
fn raw_empty_and_foreign_heap_addresses_do_not_invent_headers() {
    let heap = Heap::new();
    let mut raw = [0u8; 64];
    let mut foreign = Heap::new();
    let object = foreign.alloc(TypeId::BYTE_LIST, 16).unwrap();
    for address in [
        std::ptr::null_mut(),
        raw.as_mut_ptr(),
        object.as_ptr() as *mut u8,
    ] {
        let backing = heap.backing_view(address);
        assert_eq!(backing.data, address);
        assert_eq!(backing.owner, None);
        assert_eq!(backing.payload_size, None);
    }
}
#[test]
fn managed_copy_never_exceeds_payload_even_when_generic_size_is_erased() {
    let mut heap = Heap::new();
    let object = heap.alloc(TypeId::BYTE_LIST, 16).unwrap();
    let backing = heap.backing_view(object.as_ptr() as *mut u8);
    assert_eq!(backing.copy_len(128, 24), 16);
    assert_eq!(backing.copy_len(128, 4), 4);
    assert_eq!(backing.copy_len(2, 24), 2);
}

#[test]
fn backing_membership_tracks_free_clear_and_new_allocations() {
    let mut heap = Heap::new();
    let first = heap.alloc(TypeId::BYTE_LIST, 16).unwrap();
    let base = first.as_ptr() as *mut u8;
    let data = first.data_ptr();
    assert_eq!(heap.object_addresses.len(), heap.object_count());
    assert_eq!(heap.backing_view(unsafe { data.add(1) }).owner, None);
    unsafe { heap.free(first) };
    assert_eq!(heap.object_count(), 0);
    assert_eq!(heap.object_addresses.len(), 0);
    assert_eq!(heap.backing_view(base).owner, None);
    assert_eq!(heap.backing_view(data).owner, None);

    let second = heap.alloc(TypeId::BYTE_LIST, 16).unwrap();
    let second_base = second.as_ptr() as *mut u8;
    let second_data = second.data_ptr();
    assert_eq!(heap.backing_view(second_data).owner, Some(second_base));
    unsafe { heap.clear() };
    assert_eq!(heap.object_addresses.len(), 0);
    assert_eq!(heap.backing_view(second_base).owner, None);
    assert_eq!(heap.backing_view(second_data).owner, None);
    let third = heap.alloc(TypeId::BYTE_LIST, 32).unwrap();
    assert_eq!(heap.object_addresses.len(), heap.object_count());
    assert_eq!(
        heap.backing_view(third.data_ptr()).owner,
        Some(third.as_ptr() as *mut u8)
    );
}
