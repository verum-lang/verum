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
//! Tests for the computational properties system.
//!
//! Covers ComputationalProperty enum, PropertySet operations,
//! and property inference rules.

use verum_types::computational_properties::{ComputationalProperty, PropertySet};

// ============================================================================
// ComputationalProperty Enum Tests
// ============================================================================

#[test]
fn test_property_equality() {
    assert_eq!(ComputationalProperty::Pure, ComputationalProperty::Pure);
    assert_eq!(ComputationalProperty::IO, ComputationalProperty::IO);
    assert_ne!(ComputationalProperty::Pure, ComputationalProperty::IO);
}

#[test]
fn test_property_clone() {
    let p = ComputationalProperty::Async;
    let p2 = p.clone();
    assert_eq!(p, p2);
}

#[test]
fn test_custom_property() {
    let p = ComputationalProperty::Custom("MyEffect".into());
    assert_eq!(p, ComputationalProperty::Custom("MyEffect".into()));
    assert_ne!(p, ComputationalProperty::Custom("OtherEffect".into()));
}

#[test]
fn test_all_properties_distinct() {
    let properties = vec![
        ComputationalProperty::Pure,
        ComputationalProperty::IO,
        ComputationalProperty::Async,
        ComputationalProperty::Fallible,
        ComputationalProperty::Divergent,
        ComputationalProperty::Allocates,
        ComputationalProperty::Deallocates,
        ComputationalProperty::Mutates,
        ComputationalProperty::ReadsExternal,
        ComputationalProperty::WritesExternal,
        ComputationalProperty::FFI,
        ComputationalProperty::Spawns,
    ];
    // All should be distinct
    for (i, a) in properties.iter().enumerate() {
        for (j, b) in properties.iter().enumerate() {
            if i != j {
                assert_ne!(
                    a, b,
                    "Properties at index {} and {} should be different",
                    i, j
                );
            }
        }
    }
}

// ============================================================================
// PropertySet Creation Tests
// ============================================================================

#[test]
fn test_property_set_pure() {
    let ps = PropertySet::pure();
    assert!(ps.is_pure());
    assert!(!ps.contains(&ComputationalProperty::IO));
}

#[test]
fn test_property_set_single() {
    let ps = PropertySet::single(ComputationalProperty::IO);
    assert!(ps.contains(&ComputationalProperty::IO));
    assert!(!ps.contains(&ComputationalProperty::Pure));
}

#[test]
fn test_property_set_from_properties() {
    let ps = PropertySet::from_properties(vec![
        ComputationalProperty::IO,
        ComputationalProperty::Async,
    ]);
    assert!(ps.contains(&ComputationalProperty::IO));
    assert!(ps.contains(&ComputationalProperty::Async));
    assert!(!ps.contains(&ComputationalProperty::Pure)); // Pure is removed when other props exist
}

#[test]
fn test_property_set_empty_defaults_to_pure() {
    let ps = PropertySet::from_properties(vec![]);
    assert!(ps.is_pure());
}

#[test]
fn test_property_set_pure_removed_with_others() {
    // When Pure is combined with IO, Pure should be removed
    let ps =
        PropertySet::from_properties(vec![ComputationalProperty::Pure, ComputationalProperty::IO]);
    assert!(!ps.contains(&ComputationalProperty::Pure));
    assert!(ps.contains(&ComputationalProperty::IO));
}

// ============================================================================
// PropertySet Operations Tests
// ============================================================================

#[test]
fn test_property_set_union() {
    let a = PropertySet::single(ComputationalProperty::IO);
    let b = PropertySet::single(ComputationalProperty::Async);
    let combined = a.union(&b);
    assert!(combined.contains(&ComputationalProperty::IO));
    assert!(combined.contains(&ComputationalProperty::Async));
}

#[test]
fn test_property_set_union_with_pure() {
    let a = PropertySet::pure();
    let b = PropertySet::single(ComputationalProperty::IO);
    let combined = a.union(&b);
    // IO subsumes Pure
    assert!(combined.contains(&ComputationalProperty::IO));
}

#[test]
fn test_property_set_is_subset() {
    let subset = PropertySet::single(ComputationalProperty::IO);
    let superset = PropertySet::from_properties(vec![
        ComputationalProperty::IO,
        ComputationalProperty::Async,
    ]);
    assert!(subset.is_subset_of(&superset));
    assert!(!superset.is_subset_of(&subset));
}

#[test]
fn test_property_set_is_pure() {
    assert!(PropertySet::pure().is_pure());
    assert!(!PropertySet::single(ComputationalProperty::IO).is_pure());
    assert!(!PropertySet::single(ComputationalProperty::Async).is_pure());
}

// ============================================================================
// PropertySet Specific Properties Tests
// ============================================================================

#[test]
fn test_fallible_property() {
    let ps = PropertySet::single(ComputationalProperty::Fallible);
    assert!(ps.contains(&ComputationalProperty::Fallible));
    assert!(!ps.is_pure());
}

#[test]
fn test_divergent_property() {
    let ps = PropertySet::single(ComputationalProperty::Divergent);
    assert!(ps.contains(&ComputationalProperty::Divergent));
}

#[test]
fn test_mutates_property() {
    let ps = PropertySet::single(ComputationalProperty::Mutates);
    assert!(ps.contains(&ComputationalProperty::Mutates));
}

#[test]
fn test_ffi_property() {
    let ps = PropertySet::single(ComputationalProperty::FFI);
    assert!(ps.contains(&ComputationalProperty::FFI));
}

#[test]
fn test_spawns_property() {
    let ps = PropertySet::single(ComputationalProperty::Spawns);
    assert!(ps.contains(&ComputationalProperty::Spawns));
}

#[test]
fn test_complex_property_combination() {
    let ps = PropertySet::from_properties(vec![
        ComputationalProperty::IO,
        ComputationalProperty::Async,
        ComputationalProperty::Fallible,
        ComputationalProperty::Mutates,
    ]);
    assert!(ps.contains(&ComputationalProperty::IO));
    assert!(ps.contains(&ComputationalProperty::Async));
    assert!(ps.contains(&ComputationalProperty::Fallible));
    assert!(ps.contains(&ComputationalProperty::Mutates));
    assert!(!ps.contains(&ComputationalProperty::Pure));
    assert!(!ps.contains(&ComputationalProperty::Divergent));
}

// ---------------------------------------------------------------------------
// T1493 — fallibility stops where the error cannot escape
// ---------------------------------------------------------------------------

/// `Fallible` means "this may hand an error to its caller". A function whose
/// own return type is not `Result`/`Maybe` has no channel to do that, so the
/// property must not survive an exhaustive `match`.
///
/// Found blocking `internal/registry-next`: `verum check` reported exactly two
/// errors, both of this shape —
///
/// ```text
/// error: Function 'round_trip_preserves' declared `pure`
///        but has impure properties: Fallible
/// ```
///
/// — on a function that decodes, matches both arms and answers `Bool`.
#[test]
fn without_fallible_drops_the_property_and_restores_purity() {
    let props = PropertySet::single(ComputationalProperty::Fallible);
    assert!(!props.is_pure(), "a Fallible set is not pure to begin with");

    let handled = props.without_fallible();
    assert!(
        !handled.iter().any(|p| *p == ComputationalProperty::Fallible),
        "the property must be gone: {:?}",
        handled
    );
    assert!(
        handled.is_pure(),
        "nothing impure is left, so the set is pure again: {:?}",
        handled
    );
}

/// ONLY `Fallible` is dropped. `IO`, `Mutates` and `Spawns` describe what the
/// call DID on the way to its result, and handling that result does not undo
/// any of it — a control, because the cheap mistake here is to clear the whole
/// set once the error is caught.
#[test]
fn without_fallible_leaves_every_other_property_alone() {
    let props = PropertySet::from_properties(vec![
        ComputationalProperty::Fallible,
        ComputationalProperty::IO,
        ComputationalProperty::Mutates,
    ]);

    let handled = props.without_fallible();
    assert!(
        handled.iter().any(|p| *p == ComputationalProperty::IO),
        "IO happened and still did: {:?}",
        handled
    );
    assert!(
        handled.iter().any(|p| *p == ComputationalProperty::Mutates),
        "the mutation happened and still did: {:?}",
        handled
    );
    assert!(
        !handled.is_pure(),
        "a set with IO is not pure, caught error or not: {:?}",
        handled
    );
}

/// A set that never carried `Fallible` comes back unchanged — including a
/// `Pure` one, which must not gain or lose anything.
#[test]
fn without_fallible_is_identity_when_there_was_none() {
    let pure_set = PropertySet::pure();
    assert_eq!(pure_set.without_fallible(), pure_set);

    let io_set = PropertySet::single(ComputationalProperty::IO);
    assert_eq!(io_set.without_fallible(), io_set);
}
