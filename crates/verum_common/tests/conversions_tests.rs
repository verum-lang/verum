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
// Tests for conversions module
// Migrated from src/conversions.rs per CLAUDE.md standards

use verum_common::Maybe;
use verum_common::conversions::*;

#[test]
fn test_option_maybe_conversions() {
    // Option to Maybe
    assert_eq!(option_to_maybe(Some(42)), Maybe::Some(42));
    assert_eq!(option_to_maybe::<i32>(None), Maybe::None);

    // Maybe to Option
    assert_eq!(maybe_to_option(Maybe::Some(42)), Some(42));
    assert_eq!(maybe_to_option::<i32>(Maybe::None), None);

    // Round-trip
    let opt = Some(42);
    assert_eq!(maybe_to_option(option_to_maybe(opt)), opt);

    let maybe = Maybe::Some(42);
    assert_eq!(option_to_maybe(maybe_to_option(maybe)), maybe);
}

#[test]
fn test_traits() {
    // ToMaybe
    let opt = Some(42);
    assert_eq!(opt.to_maybe(), Maybe::Some(42));

    // ToOption
    let maybe = Maybe::Some(42);
    assert_eq!(maybe.to_option(), Some(42));
}

#[test]
fn test_trait_conversions() {
    // Test ToMaybe trait
    let opt = Some(42);
    let maybe: Maybe<i32> = opt.to_maybe();
    assert_eq!(maybe, Maybe::Some(42));

    // Test ToOption trait
    let maybe = Maybe::Some(42);
    let opt: Option<i32> = maybe.to_option();
    assert_eq!(opt, Some(42));
}

#[test]
fn test_batch_conversions() {
    let opts = vec![Some(1), None, Some(3)];
    let maybes: Vec<_> = options_to_maybes(opts.into_iter()).collect();
    assert_eq!(maybes, vec![Maybe::Some(1), Maybe::None, Maybe::Some(3)]);

    let maybes = vec![Maybe::Some(1), Maybe::None, Maybe::Some(3)];
    let opts: Vec<_> = maybes_to_options(maybes.into_iter()).collect();
    assert_eq!(opts, vec![Some(1), None, Some(3)]);
}
