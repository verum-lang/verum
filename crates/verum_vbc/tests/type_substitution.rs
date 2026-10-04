//! T1526/T1528: declaration identity and bound-variable substitution.

use verum_vbc::module::FunctionDescriptor;
use verum_vbc::mono::TypeSubstitution;
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
    let substituted = subst.apply(&generic);

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
    let substituted = subst.apply(&generic);

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
    let result = subst.apply(&generic);
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
        subst.apply(&TypeRef::Generic(TypeParamId(0))),
        TypeRef::Concrete(TypeId::INT)
    );
    assert_eq!(
        subst.apply(&TypeRef::Generic(TypeParamId(0x8000))),
        TypeRef::Concrete(TypeId::TEXT)
    );
    assert_eq!(
        subst.apply(&TypeRef::Generic(TypeParamId(1))),
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
            TypeRef::Generic(TypeParamId(0x8000)),
        ])),
        contexts: Default::default(),
    };
    assert_eq!(
        subst.apply(&rank2),
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
