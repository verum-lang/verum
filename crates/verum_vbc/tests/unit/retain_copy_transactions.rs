use super::*;
use crate::{VbcModule, interpreter::Interpreter, types::TypeId, value::Capabilities};

fn fixture(
    count: u32,
) -> (
    Interpreter,
    Value,
    Value,
    crate::retain_copy::RetainLayout,
    *mut Value,
) {
    let mut vm = Interpreter::new(VbcModule::new("retained-copy-transaction".into()).into());
    let owner = TypeId(1000);
    let handle = vm.state.heap.alloc(owner, 8).unwrap();
    let counter = vm.state.heap.alloc(TypeId(1001), 8).unwrap();
    // SAFETY: these exact fresh allocations each contain one initialized slot.
    unsafe {
        handle.data_ptr().cast::<Value>().write(Value::from_i64(73));
        counter
            .data_ptr()
            .cast::<Value>()
            .write(Value::from_i64(i64::from(count)));
    }
    let reference = vm
        .state
        .heap
        .project_object_field(counter.as_ptr() as usize, 0, Capabilities::MUT_EXCLUSIVE)
        .unwrap();
    let source = Value::from_ptr(handle.as_ptr().cast::<u8>());
    (
        vm,
        source,
        reference,
        crate::retain_copy::RetainLayout { owner, fields: 1 },
        counter.data_ptr().cast(),
    )
}

#[test]
fn later_overflow_rolls_back_only_successful_local_increments() {
    let (mut vm, source, counter, layout, slot) = fixture(u32::MAX - 1);
    let mut receipts = verum_common::List::new();
    receipts.try_reserve_exact(2).unwrap();
    for _ in 0..2 {
        receipts.push(PreparedRetain::prepare(&mut vm.state, source, counter, layout).unwrap());
    }
    let second = receipts.pop().unwrap();
    let first = receipts
        .pop()
        .unwrap()
        .retain(&mut vm.state)
        .unwrap_or_else(|_| panic!("first retain"));
    let (second, _) = second.retain(&mut vm.state).err().expect("late overflow");
    second.abort(&mut vm.state);
    first.abort(&mut vm.state).unwrap();
    // SAFETY: both original allocations are still live; no concurrent writer.
    unsafe {
        assert_eq!(slot.read().try_as_i64(), Some(i64::from(u32::MAX - 1)));
        assert_eq!(
            source
                .as_ptr::<u8>()
                .add(heap::OBJECT_HEADER_SIZE)
                .cast::<Value>()
                .read()
                .try_as_i64(),
            Some(73)
        );
    }
    assert_eq!(vm.state.heap.stats().total_allocs, 4);
    assert_eq!(vm.state.heap.stats().total_frees, 2);
}

#[test]
fn rollback_does_not_overwrite_an_independent_retain() {
    let (mut vm, source, counter, layout, slot) = fixture(1);
    let own = PreparedRetain::prepare(&mut vm.state, source, counter, layout).unwrap();
    let other = PreparedRetain::prepare(&mut vm.state, source, counter, layout).unwrap();
    let own = own
        .retain(&mut vm.state)
        .unwrap_or_else(|_| panic!("own retain"));
    let other = other
        .retain(&mut vm.state)
        .unwrap_or_else(|_| panic!("independent retain"));
    own.abort(&mut vm.state).unwrap();
    let independent = other.publish();
    assert!(vm.state.heap.contains(independent.as_ptr()));
    // SAFETY: still-live count with no concurrent accesses in this control.
    assert_eq!(unsafe { slot.read() }.try_as_i64(), Some(2));
    assert_eq!(vm.state.heap.stats().total_frees, 1);
}

#[test]
fn failure_during_preparation_never_acquires_ownership() {
    let (mut vm, source, counter, layout, slot) = fixture(1);
    let first = PreparedRetain::prepare(&mut vm.state, source, counter, layout).unwrap();
    let wrong = crate::retain_copy::RetainLayout {
        owner: TypeId(1002),
        ..layout
    };
    assert!(PreparedRetain::prepare(&mut vm.state, source, counter, wrong).is_err());
    first.abort(&mut vm.state);
    // SAFETY: this original counter was not retired.
    assert_eq!(unsafe { slot.read() }.try_as_i64(), Some(1));
    assert_eq!(vm.state.heap.stats().total_frees, 1);
}
