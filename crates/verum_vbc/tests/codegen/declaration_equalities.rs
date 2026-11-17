use super::*;
use crate::types::{TypeId, TypeParamId};

fn generic(id: u16) -> TypeRef { TypeRef::Generic(TypeParamId(id)) }

fn projection(base: TypeRef) -> TypeRef {
    TypeRef::AssociatedProjection {
        base: Box::new(base), qualifier: None, assoc: "Member".into(), args: Default::default(),
    }
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
