//! Bounded native correspondence controls; the generic algebra is proved in Lean.
use verum_common::List;
use verum_types::computational_properties::{
    ComputationalProperty as P, PropertyRequirement as R, PropertySet as S,
};

fn property_sets() -> List<S> {
    let hazards = [
        P::Async,
        P::IO,
        P::Mutates,
        P::Allocates,
        P::Fallible,
        P::Custom("external-domain".into()),
    ];
    let mut sets: List<S> = (0..(1 << hazards.len()))
        .map(|bits| {
            S::from_properties(
                hazards
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| bits & (1 << i) != 0)
                    .map(|(_, p)| p.clone()),
            )
        })
        .collect();
    sets.push(S::single(P::Divergent));
    // The decoder does not normalize these lists. Pure must remain the empty
    // hazard marker even when the representation is not in constructor normal form.
    sets.push(serde_json::from_str::<S>("[]").unwrap());
    sets.push(serde_json::from_str::<S>(r#"["Pure", "IO"]"#).unwrap());
    sets
}

#[test]
fn native_join_permits_exactly_the_union_of_operand_hazards() {
    let sets = property_sets();
    let mut requirements = List::from_iter([R::Unrestricted]);
    for set in &sets {
        requirements.push(R::AtMost(set.clone()));
        requirements.push(R::Excludes(set.clone()));
    }
    for left in &requirements {
        for right in &requirements {
            let joined = left.join(right);
            for property in [
                P::Async,
                P::IO,
                P::Mutates,
                P::Allocates,
                P::Fallible,
                P::Divergent,
                P::Custom("external-domain".into()),
                P::Custom("unmentioned-domain".into()),
            ] {
                let singleton = S::single(property);
                assert_eq!(
                    joined.accepts(&singleton),
                    left.accepts(&singleton) || right.accepts(&singleton),
                    "{left:?} join {right:?} on {singleton:?}"
                );
            }
            assert!(joined.accepts(&S::pure()));
            for footprint in &sets {
                if left.implies(right) && left.accepts(footprint) {
                    assert!(
                        right.accepts(footprint),
                        "{left:?} implies {right:?} but loses {footprint:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn missing_inference_never_manufactures_a_finite_pure_bound() {
    let pure = R::AtMost(S::pure());
    assert!(!pure.satisfied_by(&R::Unrestricted, None));
    assert!(!pure.satisfied_by(&R::source_callable(false), None));
    for property in [P::Divergent, P::Fallible, P::Async, P::IO, P::Mutates] {
        assert!(!pure.accepts(&S::single(property)));
    }
    assert!(pure.accepts(&S::pure()));
}

#[test]
fn construction_bound_preserves_allocation_failure_and_operand_hazards() {
    let construction = R::value_construction();
    let pure = R::declared_function(true, false);
    assert!(construction.implies(&pure));
    assert!(!construction.implies(&R::AtMost(S::pure())));
    for property in [P::Allocates, P::Fallible] {
        assert!(construction.accepts(&S::single(property)));
    }
    for property in [
        P::IO,
        P::Mutates,
        P::Async,
        P::FFI,
        P::ReadsExternal,
        P::WritesExternal,
        P::Spawns,
    ] {
        let operand = S::single(property);
        assert!(!construction.accepts(&operand));
        assert!(!construction.join(&R::AtMost(operand)).implies(&pure));
    }
}
