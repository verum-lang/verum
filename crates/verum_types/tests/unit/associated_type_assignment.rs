use super::*;
use crate::Unifier;

fn value_substitution(id: ValueId, replacement: &Type) -> Substitution {
    let mut inference = replacement.clone();
    let Type::Meta { value, .. } = &mut inference else {
        panic!("value replacement")
    };
    *value = ValueTerm::Inference(id);
    let mut unifier = Unifier::new();
    let substitution = unifier
        .unify(&inference, replacement, verum_ast::Span::default())
        .expect("public value unification constructs the intended owner substitution");
    assert_eq!(
        inference.try_apply_subst(&substitution).unwrap(),
        *replacement
    );
    substitution
}

fn value_id(ty: &Type) -> ValueId {
    match ty {
        Type::Meta {
            value: ValueTerm::Parameter(id),
            ..
        } => *id,
        _ => panic!("rigid value parameter"),
    }
}

#[test]
fn unrelated_and_shadowed_substitutions_preserve_mixed_binder_identities() {
    let local_type = TypeVar::fresh();
    let local_value = Type::meta("N".into(), Type::Var(local_type), None);
    let assignment = AssociatedTypeAssignment::mixed_family(
        List::from_iter([
            GenericBinder::Type(local_type),
            GenericBinder::Value(local_value.clone()),
        ]),
        Type::Tuple(List::from_iter([
            Type::Var(local_type),
            local_value.clone(),
        ])),
    )
    .unwrap();
    assert_eq!(
        assignment
            .try_apply_owner_subst(&Substitution::new())
            .unwrap(),
        assignment
    );
    let mut unrelated = value_substitution(ValueId::fresh(), &local_value);
    unrelated.insert(TypeVar::fresh(), Type::Var(local_type));
    assert_eq!(
        assignment.try_apply_owner_subst(&unrelated).unwrap(),
        assignment
    );
    let mut shadowed = value_substitution(
        value_id(&local_value),
        &Type::meta_value(verum_common::ConstValue::Int(3), Type::Int),
    );
    shadowed.insert(local_type, Type::Bool);
    assert_eq!(
        assignment.try_apply_owner_subst(&shadowed).unwrap(),
        assignment
    );
}

#[test]
fn noncapturing_owner_substitution_preserves_value_identity() {
    let owner = TypeVar::fresh();
    let local = Type::meta("N".into(), Type::Int, None);
    let assignment = AssociatedTypeAssignment::mixed_family(
        List::from_iter([GenericBinder::Value(local.clone())]),
        Type::Tuple(List::from_iter([Type::Var(owner), local.clone()])),
    )
    .unwrap();
    let mut substitution = Substitution::new();
    substitution.insert(owner, Type::Bool);
    let transported = assignment.try_apply_owner_subst(&substitution).unwrap();
    assert_eq!(transported.parameters(), assignment.parameters());
    assert_eq!(
        transported
            .instantiate(std::slice::from_ref(&local), &Substitution::new())
            .unwrap(),
        Type::Tuple(List::from_iter([Type::Bool, local]))
    );
}

#[test]
fn type_capture_renames_a_dependent_value_sort_without_freshening_its_value() {
    let owner = TypeVar::fresh();
    let local_type = TypeVar::fresh();
    let local_value = Type::meta("N".into(), Type::Var(local_type), None);
    let assignment = AssociatedTypeAssignment::mixed_family(
        List::from_iter([
            GenericBinder::Type(local_type),
            GenericBinder::Value(local_value.clone()),
        ]),
        Type::Tuple(List::from_iter([
            Type::Var(owner),
            Type::Var(local_type),
            local_value.clone(),
        ])),
    )
    .unwrap();
    let mut substitution = Substitution::new();
    substitution.insert(owner, Type::Var(local_type));
    let transported = assignment.try_apply_owner_subst(&substitution).unwrap();
    let GenericBinder::Type(fresh_type) = &transported.parameters()[0] else {
        panic!("type local")
    };
    let GenericBinder::Value(Type::Meta {
        ty,
        value: ValueTerm::Parameter(id),
        ..
    }) = &transported.parameters()[1]
    else {
        panic!("value local")
    };
    assert_ne!(*fresh_type, local_type);
    assert_eq!(**ty, Type::Var(*fresh_type));
    assert_eq!(*id, value_id(&local_value));
    let truth = Type::meta_value(verum_common::ConstValue::Bool(true), Type::Bool);
    assert_eq!(
        transported
            .instantiate(&[Type::Bool, truth.clone()], &Substitution::new())
            .unwrap(),
        Type::Tuple(List::from_iter([Type::Var(local_type), Type::Bool, truth]))
    );
    assert_eq!(assignment.parameters()[0], GenericBinder::Type(local_type));
}

#[test]
fn a_type_owner_range_cannot_capture_a_value_local() {
    let owner = TypeVar::fresh();
    let local = Type::meta("N".into(), Type::Int, None);
    let range = Type::Generic {
        name: "Carrier".into(),
        args: List::from_iter([local.clone()]),
    };
    let assignment = AssociatedTypeAssignment::mixed_family(
        List::from_iter([GenericBinder::Value(local.clone())]),
        Type::Tuple(List::from_iter([Type::Var(owner), local.clone()])),
    )
    .unwrap();
    let mut substitution = Substitution::new();
    substitution.insert(owner, range.clone());
    let transported = assignment.try_apply_owner_subst(&substitution).unwrap();
    assert_ne!(transported.parameters()[0], GenericBinder::Value(local));
    let seven = Type::meta_value(verum_common::ConstValue::Int(7), Type::Int);
    assert_eq!(
        transported
            .instantiate(&[seven.clone()], &Substitution::new())
            .unwrap(),
        Type::Tuple(List::from_iter([range, seven]))
    );
}

#[test]
fn owner_substitution_cannot_capture_a_family_local() {
    let owner = TypeVar::fresh();
    let local = TypeVar::fresh();
    let assignment = AssociatedTypeAssignment::family(
        List::from_iter([local]),
        Type::Tuple(List::from_iter([Type::Var(owner), Type::Var(local)])),
    )
    .unwrap();
    let mut substitution = Substitution::new();
    substitution.insert(owner, Type::Var(local));
    let transported = assignment.try_apply_owner_subst(&substitution).unwrap();
    assert_ne!(transported.parameters()[0], GenericBinder::Type(local));
    assert!(transported.free_owner_variables().contains(&local));
    assert_eq!(
        transported
            .instantiate(&[Type::bool()], &Substitution::new())
            .unwrap(),
        Type::Tuple(List::from_iter([Type::Var(local), Type::bool()]))
    );
}
#[test]
fn family_body_requires_all_of_its_local_arguments() {
    let local = TypeVar::fresh();
    let family =
        AssociatedTypeAssignment::family(List::from_iter([local]), Type::Var(local)).unwrap();
    assert!(family.ordinary_body().is_none());
    assert!(family.instantiate(&[], &Substitution::new()).is_err());
    assert!(
        family
            .instantiate(&[Type::bool(), Type::bool()], &Substitution::new())
            .is_err()
    );
}

#[test]
fn owner_substitution_cannot_capture_a_value_local() {
    let owner = Type::meta("owner".into(), Type::Int, None);
    let local = Type::meta("local".into(), Type::Int, None);
    let owner_id = match &owner {
        Type::Meta {
            value: ValueTerm::Parameter(id),
            ..
        } => *id,
        _ => unreachable!(),
    };
    let assignment = AssociatedTypeAssignment::mixed_family(
        List::from_iter([GenericBinder::Value(local.clone())]),
        Type::Tuple(List::from_iter([owner, local.clone()])),
    )
    .unwrap();
    let substitution = value_substitution(owner_id, &local);
    let transported = assignment.try_apply_owner_subst(&substitution).unwrap();
    assert_ne!(
        transported.parameters()[0],
        GenericBinder::Value(local.clone())
    );
    let argument = Type::meta_value(verum_common::ConstValue::Int(7), Type::Int);
    assert_eq!(
        transported
            .instantiate(&[argument.clone()], &Substitution::new())
            .unwrap(),
        Type::Tuple(List::from_iter([local, argument]))
    );
    assert_eq!(assignment.parameters().len(), 1);
}

#[test]
fn value_family_rejects_type_wrong_sort_and_unresolved_value() {
    let local = Type::meta("N".into(), Type::Int, None);
    let family = AssociatedTypeAssignment::mixed_family(
        List::from_iter([GenericBinder::Value(local.clone())]),
        local,
    )
    .unwrap();
    assert!(
        family
            .instantiate(&[Type::Int], &Substitution::new())
            .is_err()
    );
    assert!(
        family
            .instantiate(
                &[Type::meta_value(
                    verum_common::ConstValue::Bool(true),
                    Type::Bool
                )],
                &Substitution::new()
            )
            .is_err()
    );
    let inference = Type::Meta {
        name: "N".into(),
        ty: Box::new(Type::Int),
        refinement: None,
        value: ValueTerm::Inference(ValueId::fresh()),
    };
    assert!(
        family
            .instantiate(&[inference], &Substitution::new())
            .is_err()
    );
    let zero = Type::meta_value(verum_common::ConstValue::Int(0), Type::Int);
    let one = Type::meta_value(verum_common::ConstValue::Int(1), Type::Int);
    assert_eq!(
        family
            .instantiate(&[zero.clone()], &Substitution::new())
            .unwrap(),
        zero
    );
    assert_eq!(
        family
            .instantiate(&[one.clone()], &Substitution::new())
            .unwrap(),
        one
    );
}
