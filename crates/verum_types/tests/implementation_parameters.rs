//! Only a declaration's explicit telescope may be instantiated during selection.
use verum_ast::{
    Span,
    ty::{Ident, Path},
};
use verum_common::{ConstValue, List, Map};
use verum_types::{
    context::{GenericBinder, TypeScheme},
    protocol::{AssociatedTypeAssignment, ProtocolChecker, ProtocolImpl},
    ty::{Type, TypeVar, UniverseLevel, ValueId, ValueTerm},
};

#[test]
fn value_domains_body_and_explicit_slots_share_the_owned_universe_instance() {
    let owned = UniverseLevel::Variable(17);
    let foreign = UniverseLevel::Variable(18);
    let parameter = Type::meta("T".into(), Type::universe(owned.clone()), None);
    let captured = Type::meta("Captured".into(), Type::universe(foreign.clone()), None);
    let mut scheme = TypeScheme::mono(Type::tuple(
        [
            parameter.clone(),
            Type::universe(owned.clone()),
            captured.clone(),
        ]
        .into_iter()
        .collect(),
    ))
    .with_universe_parameters([17].into_iter().collect())
    .unwrap();
    scheme.value_parameters = [parameter.clone()].into_iter().collect();
    scheme.explicit_method_vars = Some(
        [Some(GenericBinder::Value(parameter))]
            .into_iter()
            .collect(),
    );

    let instance = scheme.try_instantiate().unwrap();
    let fresh = instance.universe_instantiation().apply(&owned).unwrap();
    assert!(matches!(fresh, UniverseLevel::Meta(_)));
    let Type::Tuple(body) = &instance.ty else {
        panic!("tuple body");
    };
    let Type::Meta { ty: domain, .. } = &instance.value_parameters[0] else {
        panic!("value parameter");
    };
    assert_eq!(domain.as_ref(), &Type::universe(fresh));
    assert_eq!(domain.as_ref(), &body[1]);
    assert_eq!(body[0], instance.value_parameters[0]);
    assert_eq!(body[2], captured);
    assert_eq!(
        instance.universe_instantiation().apply(&foreign).unwrap(),
        foreign
    );
    assert_eq!(
        instance.explicit_method_vars.as_ref().unwrap()[0],
        Some(GenericBinder::Value(instance.value_parameters[0].clone())),
    );
    let second = scheme.try_instantiate().unwrap();
    assert_ne!(instance.value_parameters[0], second.value_parameters[0]);
    assert_ne!(
        instance.universe_instantiation().apply(&owned).unwrap(),
        second.universe_instantiation().apply(&owned).unwrap(),
    );
}

fn protocol() -> Path {
    Path::single(Ident::new("OwnedEvidence", Span::default()))
}
fn cell(argument: Type) -> Type {
    Type::Named {
        path: Path::single(Ident::new("Cell", Span::default())),
        args: [argument].into_iter().collect(),
    }
}
fn index(value: i128) -> Type {
    Type::meta_value(ConstValue::Int(value), Type::Int)
}
fn implementation(parameters: List<GenericBinder>, receiver: Type) -> ProtocolImpl {
    ProtocolImpl {
        parameters,
        protocol: protocol(),
        protocol_args: List::new(),
        for_type: receiver,
        where_clauses: List::new(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: None,
        impl_crate: None,
        span: Span::default(),
        type_param_fn_bounds: Map::new(),
    }
}

#[test]
fn unlisted_type_variables_are_captures_not_implicit_quantifiers() {
    let captured = Type::Var(TypeVar::fresh());
    let mut checker = ProtocolChecker::new_empty();
    checker
        .register_impl(implementation(List::new(), cell(captured.clone())))
        .unwrap();
    assert!(checker.find_impl(&cell(captured), &protocol()).is_some());
    assert!(checker.find_impl(&cell(Type::Int), &protocol()).is_none());
    assert!(checker.get_implementations(&cell(Type::Int)).is_empty());
}

#[test]
fn declared_type_variables_are_instantiated_and_repeated_positions_agree() {
    let parameter = TypeVar::fresh();
    let pair = |a, b| Type::Tuple([a, b].into_iter().collect());
    let mut checker = ProtocolChecker::new_empty();
    checker
        .register_impl(implementation(
            [GenericBinder::Type(parameter)].into_iter().collect(),
            pair(Type::Var(parameter), Type::Var(parameter)),
        ))
        .unwrap();
    assert!(
        checker
            .find_impl(&pair(Type::Int, Type::Int), &protocol())
            .is_some()
    );
    assert!(
        checker
            .find_impl(&pair(Type::Int, Type::Bool), &protocol())
            .is_none()
    );
    let first = Type::Var(TypeVar::fresh());
    let second = Type::Var(TypeVar::fresh());
    assert!(
        checker
            .find_impl(&pair(first.clone(), first), &protocol())
            .is_some()
    );
    assert!(
        checker
            .find_impl(&pair(Type::Var(TypeVar::fresh()), second), &protocol())
            .is_none()
    );
}

#[test]
fn declared_value_identity_reaches_associated_and_method_results() {
    let parameter = Type::meta("N".into(), Type::Int, None);
    let receiver = cell(parameter.clone());
    let mut declaration = implementation(
        [GenericBinder::Value(parameter)].into_iter().collect(),
        receiver.clone(),
    );
    declaration.associated_types.insert(
        "Carrier".into(),
        AssociatedTypeAssignment::ordinary(receiver.clone()),
    );
    declaration
        .methods
        .insert("retain".into(), Type::function(List::new(), receiver));
    let mut checker = ProtocolChecker::new_empty();
    checker.register_impl(declaration).unwrap();
    let actual = cell(index(7));
    assert_eq!(
        checker
            .resolve_associated_type(&actual, &protocol(), &"Carrier".into())
            .unwrap(),
        actual
    );
    let expected = Type::function(List::new(), actual.clone());
    assert_eq!(
        checker
            .resolve_method(&actual, &protocol(), &"retain".into())
            .unwrap()
            .ty,
        expected
    );
    assert_eq!(
        checker
            .lookup_protocol_method(&actual, &"retain".into())
            .unwrap(),
        Some(expected.clone())
    );
    assert_eq!(
        checker
            .lookup_all_protocol_methods(&actual, &"retain".into())
            .unwrap()
            .as_slice(),
        &[expected]
    );
}

#[test]
fn unlisted_value_identity_is_not_freshened_by_selection() {
    let first = Type::meta("N".into(), Type::Int, None);
    let second = Type::meta("N".into(), Type::Int, None);
    let mut checker = ProtocolChecker::new_empty();
    checker
        .register_impl(implementation(List::new(), cell(first.clone())))
        .unwrap();
    assert!(checker.find_impl(&cell(first), &protocol()).is_some());
    assert!(checker.find_impl(&cell(second), &protocol()).is_none());
    assert!(checker.find_impl(&cell(index(7)), &protocol()).is_none());
}

#[test]
fn unresolved_value_inference_is_not_a_proved_implementation_argument() {
    let parameter = Type::meta("N".into(), Type::Int, None);
    let mut unresolved = Type::meta("N".into(), Type::Int, None);
    if let Type::Meta { value, .. } = &mut unresolved {
        *value = ValueTerm::Inference(ValueId::fresh());
    }
    let mut checker = ProtocolChecker::new_empty();
    checker
        .register_impl(implementation(
            [GenericBinder::Value(parameter.clone())]
                .into_iter()
                .collect(),
            cell(parameter),
        ))
        .unwrap();
    assert!(checker.find_impl(&cell(index(7)), &protocol()).is_some());
    assert!(checker.find_impl(&cell(unresolved), &protocol()).is_none());
}

#[test]
fn value_and_type_parameters_share_one_substitution_without_aliasing() {
    let left = TypeVar::fresh();
    let right = TypeVar::fresh();
    let count = Type::meta("N".into(), Type::Int, None);
    let target = Type::Tuple(
        [Type::Var(left), count.clone(), Type::Var(right)]
            .into_iter()
            .collect(),
    );
    let mut declaration = implementation(
        [
            GenericBinder::Type(left),
            GenericBinder::Value(count),
            GenericBinder::Type(right),
        ]
        .into_iter()
        .collect(),
        target.clone(),
    );
    declaration
        .associated_types
        .insert("Carrier".into(), AssociatedTypeAssignment::ordinary(target));
    let mut checker = ProtocolChecker::new_empty();
    checker.register_impl(declaration).unwrap();
    let actual = Type::Tuple([Type::Bool, index(7), Type::Text].into_iter().collect());
    assert_eq!(
        checker
            .resolve_associated_type(&actual, &protocol(), &"Carrier".into())
            .unwrap(),
        actual
    );
}

#[test]
fn unowned_inference_value_is_not_a_quantified_implementation_pattern() {
    let mut unresolved = Type::meta("N".into(), Type::Int, None);
    if let Type::Meta { value, .. } = &mut unresolved {
        *value = ValueTerm::Inference(ValueId::fresh());
    }
    let mut checker = ProtocolChecker::new_empty();
    checker
        .register_impl(implementation(List::new(), cell(unresolved)))
        .unwrap();
    assert!(
        checker.find_impl(&cell(index(7)), &protocol()).is_none(),
        "an unowned inference hole must not grant universal implementation authority"
    );
}

#[test]
fn unresolved_owned_type_does_not_escape_as_an_associated_result() {
    let hidden = TypeVar::fresh();
    let mut declaration = implementation(
        [GenericBinder::Type(hidden)].into_iter().collect(),
        cell(Type::Int),
    );
    declaration.associated_types.insert(
        "Carrier".into(),
        AssociatedTypeAssignment::ordinary(Type::Var(hidden)),
    );
    let mut checker = ProtocolChecker::new_empty();
    checker.register_impl(declaration).unwrap();
    assert!(
        checker
            .resolve_associated_type(&cell(Type::Int), &protocol(), &"Carrier".into())
            .is_err(),
        "an implementation parameter needs an actual argument before it can escape"
    );
}

#[test]
fn inference_query_does_not_choose_a_value_from_a_concrete_implementation() {
    let mut unresolved = Type::meta("N".into(), Type::Int, None);
    if let Type::Meta { value, .. } = &mut unresolved {
        *value = ValueTerm::Inference(ValueId::fresh());
    }
    let mut checker = ProtocolChecker::new_empty();
    checker
        .register_impl(implementation(List::new(), cell(index(7))))
        .unwrap();
    assert!(
        checker.find_impl(&cell(unresolved), &protocol()).is_none(),
        "proof selection must not guess the query's unresolved value"
    );
}

#[test]
fn nested_receiver_patterns_use_the_same_applicability_for_associated_lookup() {
    let parameter = TypeVar::fresh();
    let borrowed = |inner| Type::Reference {
        mutable: false,
        inner: Box::new(inner),
    };
    let mut declaration = implementation(
        [GenericBinder::Type(parameter)].into_iter().collect(),
        borrowed(cell(Type::Var(parameter))),
    );
    declaration.associated_types.insert(
        "Carrier".into(),
        AssociatedTypeAssignment::ordinary(Type::Var(parameter)),
    );
    let mut checker = ProtocolChecker::new_empty();
    checker.register_impl(declaration).unwrap();
    let actual = borrowed(cell(Type::Int));
    assert!(checker.find_impl(&actual, &protocol()).is_some());
    assert_eq!(
        checker.try_find_associated_type(&actual, &"Carrier".into()),
        Some(Type::Int)
    );
    assert!(checker.get_implementations(&cell(Type::Int)).is_empty());
}

fn nominal_receiver(representation: usize, owner: &str, args: List<Type>) -> Type {
    let name = format!("{owner}.Cell");
    match representation {
        0 => Type::Generic {
            name: name.into(),
            args,
        },
        1 => Type::Named {
            path: Path::single(Ident::new(name, Span::default())),
            args,
        },
        2 => Type::Named {
            path: Path {
                segments: [owner, "Cell"]
                    .into_iter()
                    .map(|part| verum_ast::ty::PathSegment::Name(Ident::new(part, Span::default())))
                    .collect(),
                span: Span::default(),
            },
            args,
        },
        _ => unreachable!(),
    }
}

#[test]
fn generic_and_named_receiver_representations_do_not_hide_coherence_overlap() {
    use verum_types::protocol::CoherenceMode;
    for first in 0..3 {
        for second in 0..3 {
            let parameter = TypeVar::fresh();
            let mut checker = ProtocolChecker::new_empty();
            checker.set_coherence_mode(CoherenceMode::Strict);
            checker
                .register_impl(implementation(
                    [GenericBinder::Type(parameter)].into_iter().collect(),
                    nominal_receiver(first, "owner", [Type::Var(parameter)].into_iter().collect()),
                ))
                .unwrap();
            assert!(
                checker
                    .register_impl(implementation(
                        List::new(),
                        nominal_receiver(second, "owner", [Type::Int].into_iter().collect())
                    ))
                    .is_err(),
                "same declared constructor overlaps, first={first} second={second}"
            );
        }
    }
}

#[test]
fn coherence_keeps_distinct_nominal_owners_and_arities_disjoint() {
    use verum_types::protocol::CoherenceMode;
    for first in 0..3 {
        for second in 0..3 {
            for change_owner in [false, true] {
                let parameter = TypeVar::fresh();
                let mut checker = ProtocolChecker::new_empty();
                checker.set_coherence_mode(CoherenceMode::Strict);
                checker
                    .register_impl(implementation(
                        [GenericBinder::Type(parameter)].into_iter().collect(),
                        nominal_receiver(
                            first,
                            "owner",
                            [Type::Var(parameter)].into_iter().collect(),
                        ),
                    ))
                    .unwrap();
                let (owner, args) = if change_owner {
                    ("foreign", List::from_single(Type::Int))
                } else {
                    ("owner", [Type::Int, Type::Bool].into_iter().collect())
                };
                assert!(
                    checker
                        .register_impl(implementation(
                            List::new(),
                            nominal_receiver(second, owner, args)
                        ))
                        .is_ok(),
                    "distinct owner/arity is disjoint, first={first} second={second} change_owner={change_owner}"
                );
            }
        }
    }
}
