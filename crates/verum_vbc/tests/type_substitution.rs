//! T1526/T1528: declaration identity and bound-variable substitution.

use verum_vbc::module::FunctionDescriptor;
use verum_vbc::mono::{SubstitutionError, TypeSubstitution, lift_free_parameters};
use verum_vbc::types::{TypeId, Variance};
use verum_vbc::types::{TypeParamDescriptor, TypeParamId, TypeRef};

#[test]
fn test_type_substitution() {
    let params = vec![TypeParamDescriptor {
        id: TypeParamId(0),
        name: verum_vbc::types::StringId(0),
        bounds: Default::default(),
        variance: Variance::Invariant,
        default: None,
        type_bounds: smallvec::SmallVec::new(),
    }];
    let args = vec![TypeRef::Concrete(TypeId::INT)];

    let subst = TypeSubstitution::new(&params, &args);

    let generic = TypeRef::Generic(TypeParamId(0));
    let substituted = subst.apply(&generic).unwrap();

    assert_eq!(substituted, TypeRef::Concrete(TypeId::INT));
}

#[test]
fn test_type_substitution_nested() {
    let params = vec![TypeParamDescriptor {
        id: TypeParamId(0),
        name: verum_vbc::types::StringId(0),
        bounds: Default::default(),
        variance: Variance::Invariant,
        default: None,
        type_bounds: smallvec::SmallVec::new(),
    }];
    let args = vec![TypeRef::Concrete(TypeId::INT)];

    let subst = TypeSubstitution::new(&params, &args);

    // List<T> where T = Int
    let generic = TypeRef::Instantiated {
        base: TypeId(20), // Assume List is type 20
        args: vec![TypeRef::Generic(TypeParamId(0))],
    };
    let substituted = subst.apply(&generic).unwrap();

    assert_eq!(
        substituted,
        TypeRef::Instantiated {
            base: TypeId(20),
            args: vec![TypeRef::Concrete(TypeId::INT)],
        }
    );
}

#[test]
fn test_empty_substitution() {
    let subst = TypeSubstitution::empty();
    assert!(subst.is_empty());

    let generic = TypeRef::Generic(TypeParamId(0));
    let result = subst.apply(&generic).unwrap();
    // Unbound generics remain unchanged
    assert_eq!(result, generic);
}

fn descriptor(ids: &[u16]) -> FunctionDescriptor {
    let mut function = FunctionDescriptor::default();
    function.type_params = ids
        .iter()
        .map(|id| TypeParamDescriptor {
            id: TypeParamId(*id),
            name: verum_vbc::types::StringId(0),
            bounds: Default::default(),
            variance: Variance::Invariant,
            default: None,
            type_bounds: Default::default(),
        })
        .collect();
    function
}

#[test]
fn compact_shadow_parameters_bind_the_declared_ids() {
    let subst = TypeSubstitution::from_function(
        &descriptor(&[0, 0x8000]),
        &[
            TypeRef::Concrete(TypeId::INT),
            TypeRef::Concrete(TypeId::TEXT),
        ],
    );
    assert_eq!(
        subst.apply(&TypeRef::Generic(TypeParamId(0))).unwrap(),
        TypeRef::Concrete(TypeId::INT)
    );
    assert_eq!(
        subst.apply(&TypeRef::Generic(TypeParamId(0x8000))).unwrap(),
        TypeRef::Concrete(TypeId::TEXT)
    );
    assert_eq!(
        subst.apply(&TypeRef::Generic(TypeParamId(1))).unwrap(),
        TypeRef::Generic(TypeParamId(1))
    );
}

#[test]
fn compact_reordered_and_partial_parameters_keep_identity() {
    let subst =
        TypeSubstitution::from_function(&descriptor(&[5, 2]), &[TypeRef::Concrete(TypeId::BOOL)]);
    assert_eq!(
        subst.get(TypeParamId(5)),
        Some(&TypeRef::Concrete(TypeId::BOOL))
    );
    assert_eq!(subst.get(TypeParamId(2)), None);
    assert_eq!(subst.get(TypeParamId(0)), None);
}

#[test]
fn rank2_local_parameters_mask_outer_bindings() {
    let mut subst = TypeSubstitution::empty();
    subst.bind(TypeParamId(0), TypeRef::Concrete(TypeId::INT));
    subst.bind(TypeParamId(0x8000), TypeRef::Concrete(TypeId::TEXT));
    let rank2 = TypeRef::Rank2Function {
        type_param_count: 1,
        params: vec![TypeRef::Generic(TypeParamId(0))],
        return_type: Box::new(TypeRef::Tuple(vec![
            TypeRef::Generic(TypeParamId(0)),
            TypeRef::Generic(TypeParamId(0x8001)),
        ])),
        contexts: Default::default(),
    };
    assert_eq!(
        subst.apply(&rank2).unwrap(),
        TypeRef::Rank2Function {
            type_param_count: 1,
            params: vec![TypeRef::Generic(TypeParamId(0))],
            return_type: Box::new(TypeRef::Tuple(vec![
                TypeRef::Generic(TypeParamId(0)),
                TypeRef::Concrete(TypeId::TEXT)
            ])),
            contexts: Default::default(),
        }
    );
}

fn generic(id: u16) -> TypeRef {
    TypeRef::Generic(TypeParamId(id))
}

fn quantified(count: u16, result: TypeRef) -> TypeRef {
    TypeRef::Rank2Function {
        type_param_count: count,
        params: vec![generic(0)],
        return_type: Box::new(result),
        contexts: Default::default(),
    }
}

#[test]
fn substitution_cannot_capture_a_replacement_free_parameter() {
    let mut subst = TypeSubstitution::empty();
    subst.bind(TypeParamId(0), generic(0));
    let ty = quantified(1, TypeRef::Tuple(vec![generic(0), generic(1)]));
    assert_eq!(subst.apply(&ty).unwrap(), ty);
}

#[test]
fn nested_binders_preserve_local_and_outer_identity() {
    let mut subst = TypeSubstitution::empty();
    subst.bind(TypeParamId(0), TypeRef::Concrete(TypeId::INT));
    subst.bind(TypeParamId(1), generic(3));
    let ty = quantified(
        2,
        quantified(1, TypeRef::Tuple((0..5).map(generic).collect())),
    );
    let expected = quantified(
        2,
        quantified(
            1,
            TypeRef::Tuple(vec![
                generic(0),
                generic(1),
                generic(2),
                TypeRef::Concrete(TypeId::INT),
                generic(6),
            ]),
        ),
    );
    assert_eq!(subst.apply(&ty).unwrap(), expected);
    assert_eq!(TypeSubstitution::empty().apply(&ty).unwrap(), ty);
}

#[test]
fn quantified_replacement_lifts_only_its_free_parameters() {
    let mut subst = TypeSubstitution::empty();
    subst.bind(
        TypeParamId(0),
        quantified(1, TypeRef::Tuple(vec![generic(0), generic(1)])),
    );
    assert_eq!(
        subst.apply(&quantified(2, generic(2))).unwrap(),
        quantified(
            2,
            quantified(1, TypeRef::Tuple(vec![generic(0), generic(3)]))
        )
    );
}

#[test]
fn substitution_is_simultaneous_not_recursive() {
    let mut subst = TypeSubstitution::empty();
    subst.bind(TypeParamId(0), generic(1));
    subst.bind(TypeParamId(1), TypeRef::Concrete(TypeId::TEXT));
    assert_eq!(subst.apply(&generic(0)).unwrap(), generic(1));
    assert_eq!(
        subst.apply(&quantified(1, generic(1))).unwrap(),
        quantified(1, generic(2))
    );
}

#[test]
fn unrepresentable_lifting_is_an_error_instead_of_index_wrap() {
    let mut subst = TypeSubstitution::empty();
    subst.bind(TypeParamId(0), generic(u16::MAX));
    assert_eq!(
        subst.apply(&quantified(1, generic(1))),
        Err(SubstitutionError::GenericIndexOverflow {
            index: u16::MAX,
            shift: 1
        })
    );
    assert_eq!(
        lift_free_parameters(&generic(u16::MAX), 0).unwrap(),
        generic(u16::MAX)
    );
    assert!(lift_free_parameters(&generic(u16::MAX), 1).is_err());
}

#[test]
fn large_binder_scopes_do_not_reject_closed_replacements() {
    let mut subst = TypeSubstitution::empty();
    subst.bind(TypeParamId(0), TypeRef::Concrete(TypeId::BOOL));
    assert_eq!(
        subst
            .apply(&quantified(u16::MAX, generic(u16::MAX)))
            .unwrap(),
        quantified(u16::MAX, TypeRef::Concrete(TypeId::BOOL))
    );
    let closed = quantified(u16::MAX, quantified(u16::MAX, generic(u16::MAX)));
    assert_eq!(subst.apply(&closed).unwrap(), closed);
}

#[test]
fn lifting_composes_without_capturing_nested_locals() {
    let ty = quantified(2, TypeRef::Tuple((0..8).map(generic).collect()));
    for first in 0..8 {
        for second in 0..8 {
            assert_eq!(
                lift_free_parameters(&lift_free_parameters(&ty, first).unwrap(), second).unwrap(),
                lift_free_parameters(&ty, first + second).unwrap()
            );
        }
    }
}

#[test]
fn failed_alias_substitution_cannot_authorize_unrestricted_ownership() {
    use verum_common::ResourceDiscipline;
    use verum_vbc::{VbcModule, types::{TypeDescriptor, TypeKind}};
    let mut module = VbcModule::new("scope_ownership".into());
    let mut declaration = TypeDescriptor::default();
    declaration.id = TypeId(7001);
    declaration.kind = TypeKind::Alias;
    declaration.resource_discipline = ResourceDiscipline::Unrestricted;
    declaration.type_params = descriptor(&[0]).type_params;
    declaration.alias_target = Some(quantified(1, generic(1)));
    module.types.push(declaration);
    let alias = |argument| TypeRef::Instantiated { base: TypeId(7001), args: vec![argument] };
    assert_eq!(
        module.resource_discipline(&alias(TypeRef::Concrete(TypeId::INT))),
        ResourceDiscipline::Unrestricted
    );
    assert_eq!(
        module.resource_discipline(&alias(generic(u16::MAX))),
        ResourceDiscipline::Unknown
    );
}
