//! Red-team Round 2 §6.2 — refinement-witness purity invariant.
//!
//! Adversarial scenario: a function whose body emits side effects
//! (IO, Mutates, Async) is used as a refinement witness. Without
//! a purity guard, the SMT translation would either lose the
//! effects (silently accepting an unsound proof) or panic on the
//! impure body (no fail-closed behaviour).
//!
//! Defense: Verum's computational-properties system tracks
//! Pure/IO/Async/Fallible/Mutates per `ComputationalProperty`.
//! `PropertySet::is_pure()` is the canonical predicate for "may
//! be used as a refinement witness". The purity invariant must
//! hold structurally:
//!
//!   1. `pure()` is pure; `single(IO)` is not.
//!   2. Adding any non-Pure property to a Pure set demotes it
//!      out of pure.
//!   3. Combining two Pure sets stays Pure.
//!   4. Combining a Pure set with an effectful set demotes the
//!      union out of Pure.
//!   5. The default is Pure (matches the spec's
//!      "absence-of-properties = Pure" convention).
//!
//! These tests pin the algebra programmatically so any future
//! refactor of `PropertySet` cannot silently break the
//! refinement-witness purity contract.

use verum_types::computational_properties::{
    ComputationalProperty, PropertySet,
};

#[test]
fn pure_constructor_is_pure() {
    assert!(PropertySet::pure().is_pure());
}

#[test]
fn single_io_is_not_pure() {
    let p = PropertySet::single(ComputationalProperty::IO);
    assert!(!p.is_pure());
    assert!(p.has_io());
}

#[test]
fn single_async_is_not_pure() {
    let p = PropertySet::single(ComputationalProperty::Async);
    assert!(!p.is_pure());
    assert!(p.is_async());
}

#[test]
fn single_fallible_is_not_pure() {
    let p = PropertySet::single(ComputationalProperty::Fallible);
    assert!(!p.is_pure());
    assert!(p.is_fallible());
}

#[test]
fn single_mutates_is_not_pure() {
    let p = PropertySet::single(ComputationalProperty::Mutates);
    assert!(!p.is_pure());
    // No `is_mutates()` accessor; `contains` is the canonical query.
    assert!(p.contains(&ComputationalProperty::Mutates));
}

#[test]
fn single_divergent_is_not_pure() {
    let p = PropertySet::single(ComputationalProperty::Divergent);
    assert!(!p.is_pure());
    assert!(p.is_divergent());
}

#[test]
fn from_properties_drops_pure_when_effectful_added() {
    // Pin: `from_properties([Pure, IO])` must not retain Pure
    // alongside IO; the result is `{IO}` not `{Pure, IO}`.
    // This is the fundamental invariant of the property
    // algebra: Pure is the bottom element, dominated by any
    // other property.
    let p = PropertySet::from_properties([
        ComputationalProperty::Pure,
        ComputationalProperty::IO,
    ]);
    assert!(!p.is_pure());
    assert!(p.has_io());
    assert!(!p.contains(&ComputationalProperty::Pure));
}

#[test]
fn from_properties_empty_is_pure() {
    // Pin: empty input means "no documented effects" = Pure
    // by absence-of-evidence convention.
    let p = PropertySet::from_properties(std::iter::empty());
    assert!(p.is_pure());
}

#[test]
fn union_of_two_pure_stays_pure() {
    let a = PropertySet::pure();
    let b = PropertySet::pure();
    let combined = a.union(&b);
    assert!(combined.is_pure());
}

#[test]
fn union_pure_with_io_demotes() {
    // Pin: `Pure ∪ {IO}` = `{IO}`, not `{Pure, IO}`. This is
    // the load-bearing rule for refinement-witness purity:
    // composing a pure caller with an effectful callee
    // *demotes* the call site out of pure.
    let pure = PropertySet::pure();
    let io = PropertySet::single(ComputationalProperty::IO);
    let combined = pure.union(&io);
    assert!(!combined.is_pure());
    assert!(combined.has_io());
    assert!(!combined.contains(&ComputationalProperty::Pure));
}

#[test]
fn union_io_with_async_combines_both() {
    // Pin: distinct effects union into a multi-property set;
    // Pure is not auto-injected.
    let io = PropertySet::single(ComputationalProperty::IO);
    let r#async = PropertySet::single(ComputationalProperty::Async);
    let combined = io.union(&r#async);
    assert!(!combined.is_pure());
    assert!(combined.has_io());
    assert!(combined.is_async());
    assert!(!combined.contains(&ComputationalProperty::Pure));
}

#[test]
fn default_is_pure() {
    // Pin: `PropertySet::default()` matches `pure()` so a
    // freshly-constructed Function (with no annotated
    // properties) is treated as a viable refinement witness
    // until proven otherwise. This is the spec convention.
    let d: PropertySet = Default::default();
    assert!(d.is_pure());
}

/// Pin: every effectful property surfaces a sibling accessor
/// (`has_io`, `is_async`, `is_fallible`, `is_divergent`)
/// AND a generic `contains` query. The contract is that the
/// canonical `is_pure()` query returns `true` IFF the set is
/// exactly `{Pure}` and `false` for any other shape — a
/// regression that admits IO into the "pure" predicate would
/// be a soundness hole on the refinement-witness path.
#[test]
fn is_pure_is_strict_singleton_pure() {
    // The single-IO case (already pinned above) plus the
    // combined cases must all read as not-pure.
    for prop in [
        ComputationalProperty::IO,
        ComputationalProperty::Async,
        ComputationalProperty::Fallible,
        ComputationalProperty::Mutates,
        ComputationalProperty::Divergent,
    ] {
        let p = PropertySet::single(prop.clone());
        assert!(
            !p.is_pure(),
            "{:?}-only set must not register as pure",
            prop
        );
    }
}
