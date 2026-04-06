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
//! Integration test to verify verum_common conversions work correctly in verum_smt
//!
//! This test verifies that the centralized conversion utilities from verum_common
//! are properly accessible and functional within verum_smt.

// Import the re-exported conversion functions from lib.rs
use verum_common::Maybe;
use verum_smt::{maybe_to_option, option_to_maybe};

#[test]
fn test_conversion_functions_accessible() {
    // Test option_to_maybe
    let opt = Some(42);
    let maybe = option_to_maybe(opt);
    assert_eq!(maybe, Maybe::Some(42));

    // Test maybe_to_option
    let maybe = Maybe::Some(42);
    let opt = maybe_to_option(maybe);
    assert_eq!(opt, Some(42));
}

#[test]
fn test_conversions_with_none() {
    let opt: Option<i32> = None;
    let maybe = option_to_maybe(opt);
    assert_eq!(maybe, Maybe::None);

    let maybe: Maybe<i32> = Maybe::None;
    let opt = maybe_to_option(maybe);
    assert_eq!(opt, None);
}

#[test]
fn test_conversions_roundtrip() {
    let opt = Some("test");
    let maybe = option_to_maybe(opt);
    let opt2 = maybe_to_option(maybe);
    assert_eq!(opt, opt2);
}
