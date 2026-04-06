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
// Tests for types module
// Migrated from src/types.rs per CLAUDE.md standards

use verum_common::types::*;

#[test]
fn test_text_type() {
    let text: Text = "hello".into();
    assert_eq!(text, "hello");
}

#[test]
fn test_maybe_type() {
    let some: Maybe<Int> = Some(42);
    let none: Maybe<Int> = None;
    assert_eq!(some, Some(42));
    assert_eq!(none, None);
}

#[test]
fn test_result_type() {
    let ok: VerumResult<Int> = Ok(42);
    let err: VerumResult<Int> = Err("error".into());
    assert!(ok.is_ok());
    assert!(err.is_err());
}
