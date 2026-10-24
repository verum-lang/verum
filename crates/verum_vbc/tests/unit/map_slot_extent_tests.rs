//! T1196: validate the actual runtime Map extent before reading or borrowing it.
//!
//! These are direct lookup-helper controls. They do not establish arbitrary raw
//! receiver safety for the earlier generic CallM dispatch probes.
use super::{map_value_slot, value_hash};
use crate::interpreter::{error::InterpreterError, heap, state::InterpreterState};
use crate::{module::VbcModule, types::TypeId, value::Value};
use std::sync::Arc;

const VALUE_SIZE: usize = std::mem::size_of::<Value>();

fn state() -> InterpreterState {
    InterpreterState::new(Arc::new(VbcModule::new("map_extent".into())))
}

fn backing(state: &mut InterpreterState, id: TypeId, slots: usize) -> Value {
    let object = state
        .heap
        .alloc_array(id, slots)
        .expect("small owned backing");
    // SAFETY: alloc_array reserves slots aligned, writable Value cells.
    let cells = unsafe {
        (object.as_ptr() as *mut u8)
            .add(heap::OBJECT_HEADER_SIZE)
            .cast::<Value>()
    };
    for index in 0..slots {
        // SAFETY: index is within the allocated array.
        unsafe {
            *cells.add(index) = Value::unit();
        }
    }
    Value::from_ptr(object.as_ptr())
}

fn header(
    state: &mut InterpreterState,
    slots: usize,
    length: Value,
    capacity: Value,
    entries: Value,
) -> Value {
    let object = state
        .heap
        .alloc(TypeId::MAP, slots * VALUE_SIZE)
        .expect("small Map header");
    // SAFETY: alloc supplies slots writable Value cells.
    let cells = unsafe {
        (object.as_ptr() as *mut u8)
            .add(heap::OBJECT_HEADER_SIZE)
            .cast::<Value>()
    };
    for (index, value) in [length, capacity, entries, Value::from_i64(0)]
        .into_iter()
        .enumerate()
        .take(slots)
    {
        // SAFETY: take(slots) keeps the intentionally short-header fixtures in bounds.
        unsafe {
            *cells.add(index) = value;
        }
    }
    Value::from_ptr(object.as_ptr())
}

fn pair(entries: Value, index: usize, key: i64, value: i64) -> *mut Value {
    // SAFETY: every caller allocated and initialized the indexed pair first.
    unsafe {
        let cells = entries
            .as_ptr::<u8>()
            .add(heap::OBJECT_HEADER_SIZE)
            .cast::<Value>();
        *cells.add(index * 2) = Value::from_i64(key);
        let slot = cells.add(index * 2 + 1);
        *slot = Value::from_i64(value);
        slot
    }
}

fn refuse(state: &InterpreterState, receiver: Value, reason: &str) {
    match map_value_slot(state, receiver, Value::from_i64(1)) {
        Err(InterpreterError::Panic { message }) => assert!(message.contains(reason), "{message}"),
        outcome => panic!("expected storage refusal {reason:?}, got {outcome:?}"),
    }
    assert!(
        state.cbgr_mutable_ptrs.is_empty(),
        "refusal must not register an address"
    );
}

#[test]
fn all_existing_flat_header_and_backing_forms_return_the_actual_slot() {
    for slots in [3, 4] {
        for id in [TypeId::UNIT, TypeId::MAP] {
            let mut state = state();
            let entries = backing(&mut state, id, 32);
            let index = value_hash(Value::from_i64(1), &state) % 16;
            let expected = pair(entries, index, 1, 520);
            let map = header(
                &mut state,
                slots,
                Value::from_i64(1),
                Value::from_i64(16),
                entries,
            );
            let slot = map_value_slot(&state, map, Value::from_i64(1))
                .unwrap()
                .unwrap();
            assert_eq!(slot, expected);
            // SAFETY: the helper proved the exact owned Value slot; write-through
            // distinguishes a slot address from either a copied or loaded value.
            unsafe {
                *slot = Value::from_i64(43);
            }
            assert_eq!(unsafe { *expected }.as_i64(), 43);
        }
    }
}

#[test]
fn empty_source_constructor_encodings_return_none_without_a_backing_read() {
    for absent in [Value::from_i64(0), Value::nil()] {
        let mut state = state();
        let map = header(
            &mut state,
            4,
            Value::from_i64(0),
            Value::from_i64(0),
            absent,
        );
        assert!(
            map_value_slot(&state, map, Value::from_i64(1))
                .unwrap()
                .is_none()
        );
        let copy = super::super::memory_collections::value_copy(&mut state, map).unwrap();
        assert!(map_value_slot(&state, copy, Value::from_i64(1)).unwrap().is_none());
    }
    let mut state = state();
    let entries = backing(&mut state, TypeId::UNIT, 0);
    let map = header(
        &mut state,
        4,
        Value::from_i64(0),
        Value::from_i64(0),
        entries,
    );
    refuse(&state, map, "zero capacity has a backing allocation");
}

#[test]
fn invalid_receiver_provenance_and_header_extent_are_refused_by_the_helper() {
    let mut state = state();
    let mut raw = [0_u64; 8];
    for receiver in [
        Value::nil(),
        Value::unit(),
        Value::from_i64(513),
        Value::from_ptr(raw.as_mut_ptr()),
    ] {
        refuse(&state, receiver, "receiver is not an owned object");
    }
    for slots in [0, 1, 2, 5] {
        let map = header(
            &mut state,
            slots,
            Value::from_i64(0),
            Value::from_i64(0),
            Value::nil(),
        );
        refuse(&state, map, "flat Map header");
    }
    let wrong_type = state.heap.alloc(TypeId::LIST, 3 * VALUE_SIZE).unwrap();
    refuse(
        &state,
        Value::from_ptr(wrong_type.as_ptr()),
        "flat Map header",
    );
    let variant = state.heap.alloc_variant(TypeId::MAP, 0, 3).unwrap();
    refuse(&state, Value::from_ptr(variant.as_ptr()), "flat Map header");
}

#[test]
fn invalid_counts_and_overflowing_capacity_are_refused_before_backing_access() {
    for (length, capacity, reason) in [
        (
            Value::from_i64(-1),
            Value::from_i64(16),
            "nonnegative integers",
        ),
        (
            Value::from_i64(0),
            Value::from_i64(-1),
            "nonnegative integers",
        ),
        (
            Value::from_i64(0),
            Value::from_bool(true),
            "nonnegative integers",
        ),
        (
            Value::from_i64(17),
            Value::from_i64(16),
            "length exceeds capacity",
        ),
        (
            Value::from_i64(0),
            Value::from_i64(i64::MAX),
            "capacity byte size overflows",
        ),
    ] {
        let mut state = state();
        let map = header(&mut state, 4, length, capacity, Value::nil());
        refuse(&state, map, reason);
    }
}

#[test]
fn raw_foreign_and_interior_backing_pointers_are_refused() {
    let mut foreign = state();
    let mut state = state();
    let foreign_entries = backing(&mut foreign, TypeId::UNIT, 32);
    let owned = backing(&mut state, TypeId::UNIT, 32);
    let mut raw = [0_u64; 32];
    // SAFETY: this creates a pointer within a live array, deliberately not its
    // allocation base. The lookup must reject it without inspecting a header.
    let interior = unsafe { Value::from_ptr(owned.as_ptr::<u8>().add(heap::OBJECT_HEADER_SIZE)) };
    for entries in [
        Value::nil(),
        Value::from_i64(7),
        foreign_entries,
        Value::from_ptr(raw.as_mut_ptr()),
        interior,
    ] {
        let map = header(
            &mut state,
            4,
            Value::from_i64(1),
            Value::from_i64(16),
            entries,
        );
        refuse(&state, map, "backing is not an owned object");
    }
}

#[test]
fn incompatible_backing_type_and_exact_extent_are_refused() {
    let mut state = state();
    for (id, slots) in [
        (TypeId::LIST, 32),
        (TypeId::UNIT, 31),
        (TypeId::UNIT, 33),
        (TypeId::UNIT, 64),
    ] {
        let entries = backing(&mut state, id, slots);
        let map = header(
            &mut state,
            4,
            Value::from_i64(1),
            Value::from_i64(16),
            entries,
        );
        refuse(&state, map, "flat key/value extent");
    }
    let variant = state.heap.alloc_variant(TypeId::UNIT, 0, 31).unwrap();
    let map = header(
        &mut state,
        4,
        Value::from_i64(1),
        Value::from_i64(16),
        Value::from_ptr(variant.as_ptr()),
    );
    refuse(&state, map, "flat key/value extent");
}

#[test]
fn collision_fixtures_pin_real_hash_buckets_and_the_last_value_cell() {
    let mut state = state();
    let key_bucket = |key| value_hash(Value::from_i64(key), &state) % 16;
    assert_eq!(key_bucket(1), key_bucket(17));
    assert_eq!(key_bucket(1), key_bucket(33));
    assert_eq!(key_bucket(10), 15);
    let entries = backing(&mut state, TypeId::MAP, 32);
    let last = pair(entries, 15, 10, 520);
    let map = header(
        &mut state,
        3,
        Value::from_i64(1),
        Value::from_i64(16),
        entries,
    );
    assert_eq!(
        map_value_slot(&state, map, Value::from_i64(10)).unwrap(),
        Some(last)
    );
}

#[test]
fn a_full_table_missing_key_terminates_without_mutation() {
    let mut state = state();
    let entries = backing(&mut state, TypeId::UNIT, 4);
    let first = pair(entries, 0, 1, 7);
    let second = pair(entries, 1, 17, 19);
    let map = header(
        &mut state,
        4,
        Value::from_i64(2),
        Value::from_i64(2),
        entries,
    );
    assert!(
        map_value_slot(&state, map, Value::from_i64(33))
            .unwrap()
            .is_none()
    );
    // SAFETY: both pairs remain live and the lookup cannot mutate them.
    assert_eq!(unsafe { *first }.as_i64(), 7);
    assert_eq!(unsafe { *second }.as_i64(), 19);
}
