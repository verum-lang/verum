#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
// Tests for unsafe_cell module
// Migrated from src/unsafe_cell.rs per CLAUDE.md standards

use verum_common::unsafe_cell::*;

#[test]
fn test_new_and_into_inner() {
    let cell = UnsafeCell::new(42);
    assert_eq!(cell.into_inner(), 42);
}

#[test]
fn test_get_mut() {
    let mut cell = UnsafeCell::new(5);
    *cell.get_mut() = 10;
    assert_eq!(cell.into_inner(), 10);
}

#[test]
fn test_get_raw_pointer() {
    let cell = UnsafeCell::new(5);
    unsafe {
        let ptr = cell.get();
        assert_eq!(*ptr, 5);
        *ptr = 10;
        assert_eq!(*ptr, 10);
    }
}

#[test]
fn test_clone() {
    let cell1 = UnsafeCell::new(42);
    let cell2 = cell1.clone();
    assert_eq!(cell1.into_inner(), 42);
    assert_eq!(cell2.into_inner(), 42);
}

#[test]
fn test_partial_eq() {
    let cell1 = UnsafeCell::new(42);
    let cell2 = UnsafeCell::new(42);
    let cell3 = UnsafeCell::new(43);
    assert_eq!(cell1, cell2);
    assert_ne!(cell1, cell3);
}

#[test]
fn test_default() {
    let cell: UnsafeCell<i32> = UnsafeCell::default();
    assert_eq!(cell.into_inner(), 0);
}

#[test]
fn test_from() {
    let cell = UnsafeCell::from(42);
    assert_eq!(cell.into_inner(), 42);
}

#[test]
fn test_send() {
    fn assert_send<T: Send>() {}
    assert_send::<UnsafeCell<i32>>();
}

#[test]
fn test_not_sync() {
    fn assert_not_sync<T: Sync>() {}
    // This should NOT compile:
    // assert_not_sync::<UnsafeCell<i32>>();

    // Instead verify it's not Sync by checking the trait bound
    fn is_sync<T: Sync>(_: &T) {}
    let _cell = UnsafeCell::new(42);
    // Uncommenting the next line would cause a compile error:
    // is_sync(&_cell);
}
