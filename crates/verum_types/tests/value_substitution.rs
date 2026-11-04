//! Composition retains one exact value for every call-local inference identity.
use verum_ast::Span;
use verum_common::{ConstValue, List};
use verum_types::{Type, context::TypeScheme, ty::{SubstitutionExt, ValueId, ValueTerm}, unify::Unifier};

fn variable(id: ValueId) -> Type {
    Type::Meta { name: "N".into(), ty: Box::new(Type::Int), refinement: None,
        value: ValueTerm::Inference(id) }
}
fn number(value: i128) -> Type { Type::meta_value(ConstValue::Int(value), Type::Int) }

#[test]
fn conflicting_independently_produced_value_substitutions_are_refused() {
    let value = variable(ValueId::fresh());
    let left = Unifier::new().unify(&value, &number(1), Span::default()).unwrap();
    let right = Unifier::new().unify(&value, &number(2), Span::default()).unwrap();
    assert!(left.compose(&right).is_err());
}

#[test]
fn chained_value_substitution_preserves_the_composition_law() {
    let first = variable(ValueId::fresh());
    let second = variable(ValueId::fresh());
    let left = Unifier::new().unify(&first, &second, Span::default()).unwrap();
    let right = Unifier::new().unify(&second, &number(7), Span::default()).unwrap();
    let composed = left.compose(&right).unwrap();
    assert_eq!(first.try_apply_subst(&composed).unwrap(), number(7));
    assert_eq!(first.try_apply_subst(&left).unwrap().try_apply_subst(&right).unwrap(), number(7));
}

#[test]
fn independently_produced_cyclic_value_constraints_are_refused() {
    let first = variable(ValueId::fresh());
    let second = variable(ValueId::fresh());
    let left = Unifier::new().unify(&first, &second, Span::default()).unwrap();
    let right = Unifier::new().unify(&second, &first, Span::default()).unwrap();
    assert!(left.compose(&right).is_err());
}

#[test]
fn each_instance_connects_all_occurrences_to_its_own_value_binder() {
    let value = Type::meta("N".into(), Type::Int, None);
    let mut scheme = TypeScheme::mono(Type::function(List::from_single(value.clone()), value.clone()));
    scheme.value_parameters.push(value.clone());
    let first = scheme.try_instantiate().unwrap();
    let second = scheme.try_instantiate().unwrap();
    let Type::Function { params, return_type, .. } = first.ty else { panic!("callable") };
    assert_eq!(params[0], *return_type);
    assert_ne!(params[0], value);
    assert_ne!(first.value_parameters, second.value_parameters);
}

#[test]
fn explicit_value_slots_cannot_invent_an_unquantified_owner() {
    use verum_types::context::GenericBinder;
    let mut scheme = TypeScheme::mono(Type::function(List::new(), Type::Int));
    scheme.explicit_method_vars = Some(List::from_single(Some(GenericBinder::Value(
        Type::meta("N".into(), Type::Int, None)))));
    assert!(scheme.try_instantiate().is_err());
}

#[test]
fn explicit_value_slots_cannot_change_the_quantified_domain() {
    use verum_types::context::GenericBinder;
    let owner = Type::meta("N".into(), Type::Int, None);
    let mut false_slot = owner.clone();
    if let Type::Meta { ty, .. } = &mut false_slot { *ty = Box::new(Type::Bool); }
    let mut scheme = TypeScheme::mono(Type::function(List::new(), Type::Int));
    scheme.value_parameters.push(owner);
    scheme.explicit_method_vars = Some(List::from_single(Some(GenericBinder::Value(false_slot))));
    assert!(scheme.try_instantiate().is_err());
}
