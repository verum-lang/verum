use super::*;

#[test]
fn wildcard_array_accepts_fixed_members_without_proving_their_lengths_equal() {
    let family = TypeRef::Instantiated {
        base: TypeId::ARRAY,
        args: vec![TypeRef::Generic(crate::types::TypeParamId(7))],
    };
    let array = |length| TypeRef::Array {
        element: Box::new(TypeRef::Concrete(TypeId::INT)),
        length,
    };
    let mut bindings = TypeSubstitution::empty();
    assert!(match_declared_types(&family, &array(3), &mut bindings));
    assert_eq!(
        bindings.get(crate::types::TypeParamId(7)),
        Some(&TypeRef::Concrete(TypeId::INT))
    );
    assert!(!match_declared_types(
        &array(3),
        &array(4),
        &mut TypeSubstitution::empty()
    ));
    assert!(!match_declared_types(
        &array(3),
        &family,
        &mut TypeSubstitution::empty()
    ));
}
