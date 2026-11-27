use super::*;
use crate::types::{TypeId, TypeParamId};

fn generic(id: u16) -> TypeRef { TypeRef::Generic(TypeParamId(id)) }

fn projection(base: TypeRef) -> TypeRef {
    TypeRef::AssociatedProjection {
        base: Box::new(base), qualifier: None, assoc: "Member".into(), args: Default::default(),
    }
}

#[test]
fn selected_member_definitions_reject_conflicting_premises_and_cycles() {
    let cg = VbcCodegen::new();
    let member = projection(TypeRef::Concrete(TypeId::U8));
    let int = TypeRef::Concrete(TypeId::INT);
    let text = TypeRef::Concrete(TypeId::TEXT);
    let mut selected = DeclarationEqualities::with_members(
        &cg, List::from_iter([(member.clone(), int.clone())]),
    ).unwrap();
    assert!(selected.add(&cg, member.clone(), text.clone()).is_err());
    assert_eq!(selected.normalize(&cg, &member).unwrap(), int);
    let other = DeclarationEqualities::with_members(
        &cg, List::from_iter([(member.clone(), text.clone())]),
    ).unwrap();
    assert_eq!(other.normalize(&cg, &member).unwrap(), text);
    assert!(DeclarationEqualities::with_members(&cg, List::from_iter([
        (member.clone(), int), (member.clone(), text)
    ])).is_err());
    assert!(DeclarationEqualities::with_members(&cg, List::from_iter([
        (member.clone(), TypeRef::Slice(Box::new(member)))
    ])).is_err());
}

#[test]
fn equality_substitution_lifts_outer_parameters_under_nested_binders() {
    let codegen = VbcCodegen::new();
    let mut equalities = DeclarationEqualities::default();
    equalities.add(&codegen, generic(0), TypeRef::Concrete(TypeId::INT)).unwrap();
    let value = TypeRef::Rank2Function {
        is_async: false, type_param_count: 1,
        params: [generic(0), generic(1)].into(),
        return_type: Box::new(TypeRef::Rank2Function {
            is_async: false, type_param_count: 2,
            params: [generic(0), generic(1), generic(2), generic(3)].into(),
            return_type: Box::new(generic(2)), contexts: Default::default(),
        }), contexts: Default::default(),
    };
    let TypeRef::Rank2Function { params, return_type, .. } = equalities.normalize(&codegen, &value).unwrap()
        else { panic!("rank-2 function remains quantified") };
    assert_eq!(params, [generic(0), TypeRef::Concrete(TypeId::INT)]);
    let TypeRef::Rank2Function { params, return_type, .. } = *return_type else { panic!("nested binder") };
    assert_eq!(params, [generic(0), generic(1), generic(2), TypeRef::Concrete(TypeId::INT)]);
    assert_eq!(*return_type, generic(2));

    let mut equalities = DeclarationEqualities::default();
    equalities.add(&codegen, projection(generic(0)), TypeRef::Concrete(TypeId::BOOL)).unwrap();
    let value = TypeRef::Rank2Function {
        is_async: false, type_param_count: 1,
        params: [projection(generic(0)), projection(generic(1))].into(),
        return_type: Box::new(TypeRef::Concrete(TypeId::UNIT)), contexts: Default::default(),
    };
    let TypeRef::Rank2Function { params, .. } = equalities.normalize(&codegen, &value).unwrap()
        else { panic!("rank-2 projection") };
    assert_eq!(params, [projection(generic(0)), TypeRef::Concrete(TypeId::BOOL)]);
}

#[test]
fn equality_normalization_is_bounded_on_the_ordinary_test_stack() {
    let codegen = VbcCodegen::new();
    let equalities = DeclarationEqualities::default();
    let nested = |depth| (0..depth).fold(TypeRef::Concrete(TypeId::INT), |ty, _| TypeRef::Slice(Box::new(ty)));
    let valid = nested(24);
    assert_eq!(equalities.normalize(&codegen, &valid).unwrap(), valid);
    let error = equalities.normalize(&codegen, &nested(MAX_TYPE_REF_DEPTH)).unwrap_err().to_string();
    assert!(error.contains("declared type equality exceeds the depth limit"), "{error}");
    let wide = TypeRef::Tuple((0..MAX_TYPE_REF_NODES).map(|_| TypeRef::Concrete(TypeId::INT)).collect());
    let error = equalities.normalize(&codegen, &wide).unwrap_err().to_string();
    assert!(error.contains("declared type equality exceeds the component limit"), "{error}");
}

#[test]
fn protocol_member_substitution_uses_the_formal_receiver_before_instantiation() {
    use crate::mono::TypeSubstitution;
    let mut substitution = TypeSubstitution::empty();
    for id in [0, 1] {
        substitution.bind(TypeParamId(id), TypeRef::Concrete(TypeId::INT));
    }
    let members = [("Member".to_owned(), generic(0))].into();
    let receiver = TypeParamId(1);
    // Both outer parameters become Int. Only the second declared the equation.
    // The local binder keeps its own projection. The caller's replacement
    // becomes slot 1 underneath it and does not undergo the Int substitution.
    let ty = TypeRef::Rank2Function {
        is_async: false,
        type_param_count: 1,
        params: vec![projection(generic(0)), projection(generic(1)), projection(generic(2))],
        return_type: Box::new(TypeRef::Tuple(vec![projection(generic(2))])),
        contexts: Default::default(),
    };
    let TypeRef::Rank2Function { params, return_type, .. } =
        substitution.apply_with_members(&ty, Some((receiver, &members))).unwrap()
    else { panic!("rank-2 signature"); };
    assert_eq!(params, [projection(generic(0)), projection(TypeRef::Concrete(TypeId::INT)), generic(1)]);
    assert_eq!(*return_type, TypeRef::Tuple(vec![generic(1)]));
}
