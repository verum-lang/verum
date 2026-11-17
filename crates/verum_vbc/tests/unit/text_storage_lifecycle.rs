use super::*;
use crate::{VbcModule, interpreter::Interpreter};

fn vm() -> Interpreter { Interpreter::new(VbcModule::new("text_lifecycle".into()).into()) }

fn owned(vm: &mut Interpreter, bytes: &[u8], capacity: usize) -> Value {
    assert!(bytes.len() <= capacity);
    let address = super::super::ffi_extended::cbgr_user_allocate(&mut vm.state, capacity as i64 + 1, 1);
    assert_ne!(address, 0);
    // SAFETY: this real allocator supplied capacity+1 writable bytes.
    unsafe {
        std::ptr::write_bytes(address as *mut u8, 0x7f, capacity + 1);
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), address as *mut u8, bytes.len());
    }
    let value = adopt_owned(&mut vm.state, Value::from_i64(address), Value::from_i64(bytes.len() as i64),
        Value::from_i64(capacity as i64)).unwrap();
    // SAFETY: adoption retains the complete live byte allocation in value.
    assert_eq!(unsafe { *((address as *const u8).add(bytes.len())) }, 0);
    value
}

#[test]
fn owned_buffer_and_descriptor_are_retired_exactly_once() {
    let mut vm = vm();
    let initial = (vm.state.heap.object_count(), vm.state.cbgr_bridge_extents.len());
    for bytes in [b"".as_slice(), b"retained bytes"] {
        let value = owned(&mut vm, bytes, bytes.len());
        let storage = TextStorage::checked(&vm.state, value).unwrap();
        let data = storage.data as usize;
        storage.destroy(&mut vm.state).unwrap();
        assert!(!vm.state.heap.contains(value.as_ptr()));
        assert!(!vm.state.cbgr_bridge_extents.contains_key(&data));
        assert!(TextStorage::checked(&vm.state, value).is_err());
        assert_eq!((vm.state.heap.object_count(), vm.state.cbgr_bridge_extents.len()), initial);
    }
}

#[test]
fn explicit_release_then_automatic_storage_cleanup_does_not_double_free() {
    let mut vm = vm();
    let value = owned(&mut vm, b"payload", 64);
    TextStorage::checked(&vm.state, value).unwrap().release_contents(&mut vm.state).unwrap();
    assert!(vm.state.heap.contains(value.as_ptr()));
    assert!(vm.state.cbgr_bridge_extents.is_empty());
    let empty = TextStorage::checked(&vm.state, value).unwrap();
    assert_eq!((empty.len, empty.capacity), (0, 0));
    empty.destroy(&mut vm.state).unwrap();
    assert_eq!(vm.state.heap.object_count(), 0);
    assert_eq!(vm.state.heap.stats().total_frees, 1);
}

#[test]
fn inline_storage_releases_its_record_without_treating_interior_bytes_as_cbgr() {
    let mut vm = vm();
    for capacity in [None, Some(64)] {
        let object = match capacity {
            Some(capacity) => vm.state.heap.alloc_text_with_capacity(b"owned inline", capacity),
            None => vm.state.heap.alloc_text(b"owned inline"),
        }.unwrap();
        let value = Value::from_ptr(object.as_ptr().cast::<u8>());
        TextStorage::checked(&vm.state, value).unwrap().destroy(&mut vm.state).unwrap();
        assert!(vm.state.cbgr_bridge_extents.is_empty());
        assert_eq!(vm.state.heap.object_count(), 0);
    }
}

#[test]
fn cached_literal_ownership_survives_local_cleanup() {
    let mut vm = vm();
    let object = vm.state.heap.alloc_text(b"module literal").unwrap();
    // SAFETY: this fixture acts as the module-cache producer on its own object.
    unsafe { (*object.as_ptr()).flags.insert(heap::ObjectFlags::MODULE_LITERAL); }
    let value = Value::from_ptr(object.as_ptr().cast::<u8>());
    TextStorage::checked(&vm.state, value).unwrap().destroy(&mut vm.state).unwrap();
    let live = TextStorage::checked(&vm.state, value).unwrap();
    assert_eq!(live.len, 14);
    assert!(live.release_contents(&mut vm.state).is_err());
    assert_eq!(vm.state.heap.stats().total_frees, 0);
}

#[test]
fn admission_refuses_forged_interior_or_mismatched_allocation_without_consuming_it() {
    let mut vm = vm();
    let address = super::super::ffi_extended::cbgr_user_allocate(&mut vm.state, 17, 1);
    for (pointer, len, cap) in [(address + 1, 9, 15), (address, 9, 15),
        (address, 17, 16), (address, -1, 16), (address, 0, -1), (4096, 0, 0)]
    {
        assert!(adopt_owned(&mut vm.state, Value::from_i64(pointer), Value::from_i64(len),
            Value::from_i64(cap)).is_err());
        assert_eq!(vm.state.heap.object_count(), 0);
        assert_eq!(vm.state.cbgr_bridge_extents.len(), 1);
    }
    super::super::ffi_extended::cbgr_user_deallocate(&mut vm.state, address);
}

#[test]
fn a_text_tag_and_capacity_do_not_authorize_destruction() {
    let mut vm = vm();
    let object = vm.state.heap.alloc(TypeId::TEXT, heap::TEXT_RECORD_SIZE).unwrap();
    // SAFETY: a complete initialized but unreceipted Text-shaped record.
    unsafe {
        let fields = object.data_ptr().cast::<Value>();
        fields.write(Value::nil());
        fields.add(1).write(Value::from_i64(0));
        fields.add(2).write(Value::from_i64(0));
    }
    let value = Value::from_ptr(object.as_ptr().cast::<u8>());
    let storage = TextStorage::checked(&vm.state, value).unwrap();
    assert!(storage.destroy(&mut vm.state).is_err());
    assert!(vm.state.heap.contains(value.as_ptr()));
}

#[test]
fn refused_replacement_preserves_the_owner_and_reclaims_the_unpublished_buffer() {
    use crate::value::{Capabilities, ThinRef};
    let mut vm = vm();
    let previous = owned(&mut vm, b"previous", 16);
    let before = (vm.state.heap.object_count(), vm.state.cbgr_bridge_extents.len());
    let replacement = owned(&mut vm, b"replacement", 32);
    let mut slot = previous;
    let readonly = Value::from_thin_ref(ThinRef::new(
        (&mut slot as *mut Value).cast(), 1, 0, Capabilities::READ_ONLY,
    ));
    assert!(matches!(replace(&mut vm.state, previous, readonly, replacement),
        Err(InterpreterError::CapabilityDenied { .. })));
    assert_eq!(slot, previous);
    assert_eq!((vm.state.heap.object_count(), vm.state.cbgr_bridge_extents.len()), before);
    assert!(!vm.state.heap.contains(replacement.as_ptr()));
    let storage = TextStorage::checked(&vm.state, previous).unwrap();
    // SAFETY: the preserved descriptor still owns the complete checked prefix.
    assert_eq!(unsafe { std::slice::from_raw_parts(storage.data, storage.len) }, b"previous");
    storage.destroy(&mut vm.state).unwrap();
}

#[test]
fn replacement_publishes_before_retiring_old_storage_and_allows_identity() {
    use crate::value::{Capabilities, ThinRef};
    let mut vm = vm();
    let previous = owned(&mut vm, b"previous", 16);
    let replacement = owned(&mut vm, b"replacement", 32);
    let mut slot = previous;
    let writable = Value::from_thin_ref(ThinRef::new(
        (&mut slot as *mut Value).cast(), 1, 0, Capabilities::MUT_EXCLUSIVE,
    ));
    assert!(replace(&mut vm.state, previous, writable, replacement).unwrap());
    assert_eq!(slot, replacement);
    assert!(!vm.state.heap.contains(previous.as_ptr()));
    assert_eq!(vm.state.cbgr_bridge_extents.len(), 1);
    assert!(replace(&mut vm.state, replacement, writable, replacement).unwrap());
    assert_eq!(vm.state.heap.object_count(), 1);
    TextStorage::checked(&vm.state, replacement).unwrap().destroy(&mut vm.state).unwrap();
    assert!(vm.state.cbgr_bridge_extents.is_empty());
}
