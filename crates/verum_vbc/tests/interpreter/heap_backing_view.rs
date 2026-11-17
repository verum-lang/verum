//! Internal heap authority controls; included by interpreter::heap.
use super::*;
use verum_common::List;

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
    assert_eq!(heap.object_count(), 1);
    assert_eq!(heap.backing_view(unsafe { data.add(1) }).owner, None);
    unsafe { heap.free(first) };
    assert_eq!(heap.object_count(), 0);
    assert_eq!(heap.backing_view(base).owner, None);
    assert_eq!(heap.backing_view(data).owner, None);

    let second = heap.alloc(TypeId::BYTE_LIST, 16).unwrap();
    let second_base = second.as_ptr() as *mut u8;
    let second_data = second.data_ptr();
    assert_eq!(heap.backing_view(second_data).owner, Some(second_base));
    unsafe { heap.clear() };
    assert_eq!(heap.object_count(), 0);
    assert_eq!(heap.backing_view(second_base).owner, None);
    assert_eq!(heap.backing_view(second_data).owner, None);
    let third = heap.alloc(TypeId::BYTE_LIST, 32).unwrap();
    assert_eq!(heap.object_count(), 1);
    assert_eq!(
        heap.backing_view(third.data_ptr()).owner,
        Some(third.as_ptr() as *mut u8)
    );
}

fn assert_exact_membership(heap: &Heap) {
    for object in heap.objects.iter() {
        assert!(heap.contains(object.as_ptr()));
    }
}

#[test]
fn retiring_front_middle_and_tail_preserves_other_live_payloads() {
    let mut heap = Heap::new();
    let mut objects: List<_> = (0u64..17).map(|value| {
        Some(heap.alloc_with_init(TypeId::U64, 8, |bytes| {
            bytes.copy_from_slice(&value.to_ne_bytes());
        }).unwrap())
    }).collect();
    let mut retired = verum_common::Set::new();
    for index in [0, 8, 16, 15, 1, 7, 9] {
        let object = objects[index].take().unwrap();
        let address = object.as_ptr();
        // SAFETY: each selected object is live, belongs to this heap, and is
        // not dereferenced after retirement.
        unsafe { heap.free(object) };
        retired.insert(index);
        assert!(!heap.contains(address));
        assert_exact_membership(&heap);
        for (index, object) in objects.iter().enumerate() {
            let Some(object) = object else { continue; };
            assert!(heap.contains(object.as_ptr()));
            // SAFETY: membership establishes this original eight-byte payload.
            assert_eq!(unsafe { object.data_ptr().cast::<u64>().read_unaligned() }, index as u64);
        }
    }
    assert_eq!(heap.allocated(), (17 - retired.len()) * (OBJECT_HEADER_SIZE + 8));
    assert_eq!(heap.stats().total_frees, retired.len() as u64);
    // SAFETY: the test retains no references into the remaining allocations.
    unsafe { heap.clear() };
    assert_exact_membership(&heap);
    assert_eq!(heap.allocated(), 0);
    let next = heap.alloc(TypeId::I64, 8).unwrap();
    assert!(heap.contains(next.as_ptr()));
    assert_exact_membership(&heap);
}

#[test]
fn permuted_retirement_releases_every_owner_exactly_once() {
    let mut heap = Heap::new();
    const COUNT: usize = 2048;
    let mut objects: List<_> = (0..COUNT).map(|_| Some(heap.alloc(TypeId::I64, 8).unwrap())).collect();
    // An odd multiplier permutes the original allocation order modulo COUNT;
    // it is independent of the live allocation set's storage order.
    for step in 0..COUNT {
        let object = objects[(step * 4051) % COUNT].take().unwrap();
        let address = object.as_ptr();
        // SAFETY: the permutation visits each live owner once.
        unsafe { heap.free(object) };
        assert!(!heap.contains(address));
        assert_eq!(heap.object_count(), COUNT - step - 1);
        assert_exact_membership(&heap);
    }
    assert_eq!(heap.allocated(), 0);
    assert_eq!(heap.stats().total_allocs, COUNT as u64);
    assert_eq!(heap.stats().total_frees, COUNT as u64);
}
