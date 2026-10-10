//! Exact descriptor authority and bounded work for receiver probes (T1743).

use super::is_record_typed_as;
use crate::interpreter::state::InterpreterState;
use crate::module::VbcModule;
use crate::types::{TypeDescriptor, TypeId, TypeKind};
use crate::value::Value;
use std::cell::Cell;
use std::sync::Arc;

thread_local! {
    static DESCRIPTOR_VISITS: Cell<usize> = const { Cell::new(0) };
}

pub(super) fn record_descriptor_visit() {
    DESCRIPTOR_VISITS.with(|count| count.set(count.get() + 1));
}

fn take_descriptor_visits() -> usize {
    DESCRIPTOR_VISITS.with(|count| count.replace(0))
}

fn add_descriptor(module: &mut VbcModule, id: u32, name: &str) {
    let name = module.intern_string(name);
    module.add_type(TypeDescriptor {
        id: TypeId(id),
        name,
        kind: TypeKind::Record,
        ..TypeDescriptor::default()
    });
}

fn module(entries: &[(u32, &str)]) -> VbcModule {
    let mut module = VbcModule::new("record_probe".into());
    for &(id, name) in entries {
        add_descriptor(&mut module, id, name);
    }
    module
}

fn state(module: VbcModule) -> InterpreterState {
    InterpreterState::new(Arc::new(module))
}

fn record(state: &mut InterpreterState, id: u32) -> Value {
    let object = state.heap.alloc(TypeId(id), 8).expect("owned record");
    // Only the allocator-initialized header is inspected by this probe.
    Value::from_ptr(object.as_ptr() as *mut u8)
}

#[test]
fn exact_names_follow_sparse_ids_in_both_descriptor_orders() {
    for entries in [
        [(901_001, "alpha.Packet"), (37, "beta.Packet")],
        [(37, "beta.Packet"), (901_001, "alpha.Packet")],
    ] {
        let mut state = state(module(&entries));
        let alpha = record(&mut state, 901_001);
        let beta = record(&mut state, 37);
        assert!(is_record_typed_as(&state, alpha, "alpha.Packet"));
        assert!(is_record_typed_as(&state, beta, "beta.Packet"));
        assert!(!is_record_typed_as(&state, alpha, "beta.Packet"));
        assert!(!is_record_typed_as(&state, beta, "alpha.Packet"));
        assert!(!is_record_typed_as(&state, alpha, "Packet"));
        assert!(!is_record_typed_as(&state, beta, "Packet"));
    }
}

#[test]
fn an_absent_type_id_does_not_borrow_another_descriptor_name() {
    let mut state = state(module(&[(91, "TcpStream")]));
    let absent = record(&mut state, 92);
    assert!(!is_record_typed_as(&state, absent, "TcpStream"));
}

#[test]
fn nil_and_non_pointer_receivers_never_visit_descriptors() {
    let state = state(module(&[(91, "TcpStream")]));
    take_descriptor_visits();
    for value in [
        Value::nil(),
        Value::unit(),
        Value::from_i64(91),
        Value::from_bool(true),
        Value::from_ptr(std::ptr::null_mut::<u8>()),
    ] {
        assert!(!is_record_typed_as(&state, value, "TcpStream"));
    }
    assert_eq!(take_descriptor_visits(), 0);
}

#[test]
fn a_later_duplicate_id_cannot_authorize_a_different_name() {
    for entries in [
        [(91, "application.Credentials"), (91, "TcpStream")],
        [(91, "TcpStream"), (91, "application.Credentials")],
    ] {
        let mut state = state(module(&entries));
        let value = record(&mut state, 91);
        assert!(is_record_typed_as(&state, value, entries[0].1));
        assert!(
            !is_record_typed_as(&state, value, entries[1].1),
            "the runtime's first descriptor owns this TypeId"
        );
    }
}

#[test]
fn independent_modules_may_reuse_ids_without_reusing_authority() {
    let mut first = state(module(&[(91, "alpha.Packet")]));
    let mut second = state(module(&[(91, "beta.Packet")]));
    let a = record(&mut first, 91);
    let b = record(&mut second, 91);
    for _ in 0..3 {
        assert!(is_record_typed_as(&first, a, "alpha.Packet"));
        assert!(!is_record_typed_as(&first, a, "beta.Packet"));
        assert!(is_record_typed_as(&second, b, "beta.Packet"));
        assert!(!is_record_typed_as(&second, b, "alpha.Packet"));
    }
}

#[test]
fn cloned_warm_module_keeps_first_authority_and_accepts_new_descriptors() {
    let original = module(&[(91, "alpha.Packet")]);
    assert_eq!(original.type_index_by_id(TypeId(91)), Some(0));
    assert_eq!(original.type_index_by_id(TypeId(92)), None);
    let mut cloned = original.clone();
    add_descriptor(&mut cloned, 91, "shadow.Packet");
    add_descriptor(&mut cloned, 92, "beta.Packet");
    let mut first = state(original);
    let mut second = state(cloned);
    let a = record(&mut second, 91);
    let b = record(&mut second, 92);
    let absent = record(&mut first, 92);
    assert!(is_record_typed_as(&second, a, "alpha.Packet"));
    assert!(!is_record_typed_as(&second, a, "shadow.Packet"));
    assert!(is_record_typed_as(&second, b, "beta.Packet"));
    assert!(!is_record_typed_as(&first, absent, "beta.Packet"));
}

#[test]
fn decoded_module_rebuilds_the_same_exact_descriptor_authority() {
    let original = module(&[(901_001, "alpha.Packet"), (37, "beta.Packet")]);
    // Warm caches are process-local; the wire carries descriptors, not caches.
    assert_eq!(original.type_index_by_id(TypeId(37)), Some(1));
    let bytes = crate::serialize::serialize_module(&original).expect("serialize descriptors");
    let decoded = crate::deserialize::deserialize_module(&bytes).expect("decode descriptors");
    for module in [original, decoded] {
        let mut state = state(module);
        let value = record(&mut state, 37);
        assert!(is_record_typed_as(&state, value, "beta.Packet"));
        assert!(!is_record_typed_as(&state, value, "alpha.Packet"));
    }
}

#[test]
fn repeated_unrelated_intercepts_do_not_rescan_all_descriptors() {
    const TYPE_COUNT: u32 = 16_384;
    const CALL_COUNT: usize = 128;
    let mut module = module(&[]);
    for id in 0..TYPE_COUNT {
        add_descriptor(&mut module, 1_000_000 + id, "application.Credentials");
    }
    let mut state = state(module);
    let receiver = record(&mut state, 1_000_000 + TYPE_COUNT - 1);
    let mut intercept = || {
        super::super::net_runtime::try_intercept_tcp_method(
            &mut state,
            "application.Credentials.inspect",
            "inspect",
            receiver,
            0,
            0,
            0,
        )
        .expect("unrelated interceptor probe")
    };
    assert!(intercept().is_none()); // Exclude one possible lazy index build.
    take_descriptor_visits();
    for _ in 0..CALL_COUNT {
        assert!(intercept().is_none());
    }
    let visits = take_descriptor_visits();
    assert_eq!(
        visits,
        CALL_COUNT * 3,
        "three exact receiver probes should each inspect one selected descriptor, not all {TYPE_COUNT}"
    );
}
