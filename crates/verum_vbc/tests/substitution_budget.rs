//! Substituted shapes are bounded before their output is allocated.
use verum_vbc::{
    mono::{SubstitutionError, TypeSubstitution, lift_free_parameters},
    types::{MAX_TYPE_REF_DEPTH, MAX_TYPE_REF_NODES, TypeId, TypeParamId, TypeRef},
};

fn scalar() -> TypeRef {
    TypeRef::Concrete(TypeId::INT)
}
fn generic() -> TypeRef {
    TypeRef::Generic(TypeParamId(0))
}
fn repeated(ty: TypeRef, count: usize) -> TypeRef {
    TypeRef::Tuple(std::iter::repeat_n(ty, count).collect())
}
fn nested(mut ty: TypeRef, levels: usize) -> TypeRef {
    for _ in 0..levels {
        ty = TypeRef::Tuple(std::iter::once(ty).collect());
    }
    ty
}

#[test]
fn repeated_replacements_share_one_output_budget() {
    let mut substitution = TypeSubstitution::empty();
    substitution.bind(TypeParamId(0), repeated(scalar(), 256));
    let source = repeated(generic(), 256);
    // Each input is shallow and small. The result would have 65,793 nodes.
    assert!(
        substitution.apply(&source).is_err(),
        "expanded type escaped the output budget"
    );
}

#[test]
fn substitution_accepts_the_node_boundary_and_refuses_the_next_node() {
    let mut substitution = TypeSubstitution::empty();
    substitution.bind(TypeParamId(0), repeated(scalar(), MAX_TYPE_REF_NODES - 2));
    let source = TypeRef::Tuple(std::iter::once(generic()).collect());
    let output = substitution.apply(&source).unwrap();
    let TypeRef::Tuple(parts) = output else {
        panic!("outer tuple missing");
    };
    let TypeRef::Tuple(elements) = &parts[0] else {
        panic!("replacement tuple missing");
    };
    assert_eq!(elements.len(), MAX_TYPE_REF_NODES - 2);
    substitution.bind(TypeParamId(0), repeated(scalar(), MAX_TYPE_REF_NODES - 1));
    assert!(
        substitution.apply(&source).is_err(),
        "one extra node was admitted"
    );
}

#[test]
fn substitution_counts_depth_across_the_replacement_boundary() {
    let mut substitution = TypeSubstitution::empty();
    substitution.bind(TypeParamId(0), nested(scalar(), MAX_TYPE_REF_DEPTH / 2));
    let accepted = nested(generic(), MAX_TYPE_REF_DEPTH / 2 - 1);
    assert!(substitution.apply(&accepted).is_ok());
    let rejected = nested(generic(), MAX_TYPE_REF_DEPTH / 2);
    assert!(
        substitution.apply(&rejected).is_err(),
        "combined depth escaped its budget"
    );
}

#[test]
fn lifting_uses_the_same_shape_boundary() {
    assert!(lift_free_parameters(&nested(generic(), MAX_TYPE_REF_DEPTH - 1), 1).is_ok());
    assert!(lift_free_parameters(&nested(generic(), MAX_TYPE_REF_DEPTH), 1).is_err());
    assert!(lift_free_parameters(&repeated(generic(), MAX_TYPE_REF_NODES), 1).is_err());
}

#[test]
fn closed_sequence_payloads_keep_their_own_resource_boundary() {
    use std::sync::Arc;
    use verum_common::const_value::{MAX_SEQUENCE_DEPTH, MAX_SEQUENCE_ELEMENTS};
    use verum_vbc::const_sequence::ConstSequence;
    let valid = TypeRef::ConstSequence(ConstSequence::Integers(Arc::from([2, 7, 9])));
    assert_eq!(TypeSubstitution::empty().apply(&valid).unwrap(), valid);
    assert_eq!(lift_free_parameters(&valid, u32::MAX).unwrap(), valid);
    let oversized = TypeRef::ConstSequence(ConstSequence::Integers(
        std::iter::repeat_n(0, MAX_SEQUENCE_ELEMENTS + 1).collect(),
    ));
    let mut substitution = TypeSubstitution::empty();
    substitution.bind(TypeParamId(0), oversized);
    assert_eq!(
        substitution.apply(&generic()),
        Err(SubstitutionError::ConstSequenceLimit)
    );
    let mut deep = ConstSequence::Integers(Arc::from([73]));
    for _ in 0..MAX_SEQUENCE_DEPTH {
        deep = ConstSequence::Arrays(Arc::from([deep]));
    }
    assert_eq!(
        lift_free_parameters(&TypeRef::ConstSequence(deep), 0),
        Err(SubstitutionError::ConstSequenceLimit)
    );
}
