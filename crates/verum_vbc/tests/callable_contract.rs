//! Lifecycle validation uses the declared callable allowance, not type equality.
use verum_vbc::{TypeId, TypeRef, parameter_convention::validate_callable_contract};

fn function(is_async: bool, parameter: TypeRef, result: TypeRef) -> TypeRef {
    TypeRef::Function {
        is_async, params: vec![parameter], return_type: Box::new(result),
        contexts: Default::default(),
    }
}

fn scalar(is_async: bool) -> TypeRef {
    function(is_async, TypeRef::Concrete(TypeId::INT), TypeRef::Concrete(TypeId::INT))
}

#[test]
fn async_allowance_is_directional_and_keeps_exact_scalar_identity() {
    assert!(validate_callable_contract(&scalar(false), &scalar(true)).is_ok());
    assert!(validate_callable_contract(&scalar(true), &scalar(false)).is_err());
    for actual in [
        function(false, TypeRef::Concrete(TypeId::BOOL), TypeRef::Concrete(TypeId::INT)),
        function(false, TypeRef::Concrete(TypeId::INT), TypeRef::Concrete(TypeId::I32)),
    ] {
        assert!(validate_callable_contract(&actual, &scalar(true)).is_err());
    }
}

#[test]
fn nested_callable_parameters_and_results_have_opposite_variance() {
    let unit = TypeRef::Concrete(TypeId::UNIT);
    let accepts_sync = function(false, scalar(false), unit.clone());
    let accepts_async = function(false, scalar(true), unit.clone());
    assert!(validate_callable_contract(&accepts_async, &accepts_sync).is_ok());
    assert!(validate_callable_contract(&accepts_sync, &accepts_async).is_err());
    let returns_sync = function(false, unit.clone(), scalar(false));
    let returns_async = function(false, unit, scalar(true));
    assert!(validate_callable_contract(&returns_sync, &returns_async).is_ok());
    assert!(validate_callable_contract(&returns_async, &returns_sync).is_err());
}

#[test]
fn nominal_context_and_reference_components_remain_invariant() {
    use verum_vbc::types::{CbgrTier, ContextRef, Mutability};
    let reference = |inner| TypeRef::Reference {
        inner: Box::new(inner), mutability: Mutability::Mutable, tier: CbgrTier::Tier0,
    };
    let unit = TypeRef::Concrete(TypeId::UNIT);
    let left = function(false, reference(scalar(false)), unit.clone());
    let right = function(false, reference(scalar(true)), unit.clone());
    assert!(validate_callable_contract(&left, &right).is_err());
    assert!(validate_callable_contract(&right, &left).is_err());
    let left = function(false, TypeRef::Concrete(TypeId(9001)), unit.clone());
    let right = function(false, TypeRef::Concrete(TypeId(9002)), unit);
    assert!(validate_callable_contract(&left, &right).is_err());
    let mut contextual = scalar(false);
    let TypeRef::Function { contexts, .. } = &mut contextual else { unreachable!() };
    contexts.push(ContextRef(3));
    assert!(validate_callable_contract(&contextual, &scalar(false)).is_err());
    assert!(validate_callable_contract(&scalar(false), &contextual).is_err());
}

#[test]
fn unresolved_and_over_budget_signatures_do_not_gain_a_contract() {
    use verum_vbc::types::{MAX_TYPE_REF_DEPTH, MAX_TYPE_REF_NODES, TypeParamId};
    let unresolved = function(false, TypeRef::Generic(TypeParamId(0)), TypeRef::Concrete(TypeId::INT));
    assert!(validate_callable_contract(&unresolved, &unresolved).is_err());
    let mut deep = TypeRef::Concrete(TypeId::INT);
    for _ in 0..MAX_TYPE_REF_DEPTH { deep = TypeRef::Slice(Box::new(deep)); }
    let deep = function(false, deep, TypeRef::Concrete(TypeId::UNIT));
    assert!(validate_callable_contract(&deep, &deep).is_err());
    let wide = TypeRef::Function {
        is_async: false,
        params: vec![TypeRef::Concrete(TypeId::INT); MAX_TYPE_REF_NODES],
        return_type: Box::new(TypeRef::Concrete(TypeId::UNIT)),
        contexts: Default::default(),
    };
    assert!(validate_callable_contract(&wide, &wide).is_err());
}
