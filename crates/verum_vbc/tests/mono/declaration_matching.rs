use super::*;
use crate::types::{CbgrTier, Mutability, TypeParamId};

fn generic(id: u16) -> TypeRef {
    TypeRef::Generic(TypeParamId(id))
}

#[test]
fn sparse_actual_generics_preserve_their_caller_scope() {
    let pattern = TypeRef::Tuple(vec![generic(0), generic(7), generic(0)]);
    let actual = TypeRef::Tuple(vec![generic(0x8000), generic(2), generic(0x8000)]);
    let mut bindings = TypeSubstitution::empty();
    assert!(match_declared_types(&pattern, &actual, &mut bindings));
    assert_eq!(bindings.get(TypeParamId(0)), Some(&generic(0x8000)));
    assert_eq!(bindings.get(TypeParamId(7)), Some(&generic(2)));
    assert_eq!(bindings.apply(&pattern), actual);
    assert!(!match_declared_types(
        &generic(0),
        &generic(2),
        &mut bindings
    ));
}

#[test]
fn repeated_bindings_and_structural_qualifiers_must_agree() {
    let repeated = TypeRef::Tuple(vec![generic(0), generic(0)]);
    assert!(!match_declared_types(
        &repeated,
        &TypeRef::Tuple(vec![generic(7), generic(8)]),
        &mut TypeSubstitution::empty()
    ));
    assert!(!match_declared_types(
        &repeated,
        &TypeRef::Tuple(vec![generic(7)]),
        &mut TypeSubstitution::empty()
    ));
    let reference = |mutability, tier, inner| TypeRef::Reference {
        inner: Box::new(inner),
        mutability,
        tier,
    };
    let expected = reference(Mutability::Mutable, CbgrTier::Tier0, generic(0));
    assert!(match_declared_types(
        &expected,
        &reference(Mutability::Mutable, CbgrTier::Tier0, generic(0x8000)),
        &mut TypeSubstitution::empty()
    ));
    for actual in [
        reference(Mutability::Immutable, CbgrTier::Tier0, generic(0x8000)),
        reference(Mutability::Mutable, CbgrTier::Tier2, generic(0x8000)),
    ] {
        assert!(!match_declared_types(
            &expected,
            &actual,
            &mut TypeSubstitution::empty()
        ));
    }
    let array = |element, length| TypeRef::Array {
        element: Box::new(element),
        length,
    };
    assert!(match_declared_types(
        &array(generic(0), 3),
        &array(generic(7), 3),
        &mut TypeSubstitution::empty()
    ));
    assert!(!match_declared_types(
        &array(generic(0), 3),
        &array(generic(7), 4),
        &mut TypeSubstitution::empty()
    ));
}

#[test]
fn concrete_unifier_still_refuses_unresolved_actuals() {
    let mut bindings = TypeSubstitution::empty();
    assert!(!unify(&generic(0), &generic(7), &mut bindings));
    assert!(bindings.get(TypeParamId(0)).is_none());
    assert!(!unify(
        &TypeRef::Tuple(vec![generic(0)]),
        &TypeRef::Tuple(vec![generic(0x8000)]),
        &mut bindings
    ));
    assert!(unify(
        &generic(0),
        &TypeRef::Concrete(TypeId::INT),
        &mut bindings
    ));
}
