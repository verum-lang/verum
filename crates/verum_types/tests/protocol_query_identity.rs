//! Protocol routing keys select candidates; exact query witnesses decide reuse.
use verum_ast::{
    Span,
    ty::{Ident, Path},
};
use verum_common::{ConstValue, List, Map, Maybe};
use verum_types::{
    protocol::{
        AssociatedTypeAssignment, CoherenceMode, Protocol, ProtocolChecker, ProtocolImpl,
        ProtocolKind,
    },
    specialization_selection::{SpecializationError, SpecializationSelector},
    ty::{Type, TypeVar},
    unify::Unifier,
};

fn path(name: &str) -> Path {
    Path::single(Ident::new(name, Span::default()))
}
fn indexed(value: Type) -> Type {
    Type::Named {
        path: path("Cell"),
        args: [value].into_iter().collect(),
    }
}
fn integer(value: i128) -> Type {
    Type::meta_value(ConstValue::Int(value), Type::Int)
}
fn nan(payload: u64) -> Type {
    Type::meta_value(
        ConstValue::Float(f64::from_bits(0x7ff8_0000_0000_0000 | payload)),
        Type::Float,
    )
}
fn protocol() -> Protocol {
    Protocol {
        source_parameters: None,
        constraints: Default::default(),
        name: "Evidence".into(),
        kind: ProtocolKind::Constraint,
        type_params: List::new(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::None,
        span: Span::default(),
    }
}
fn implementation(receiver: Type) -> ProtocolImpl {
    ProtocolImpl {
        protocol: path("Evidence"),
        protocol_args: List::new(),
        for_type: receiver,
        where_clauses: List::new(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,
        impl_crate: Maybe::None,
        span: Span::default(),
        type_param_fn_bounds: Map::new(),
    }
}
fn checker() -> ProtocolChecker {
    let mut checker = ProtocolChecker::new();
    checker.register_protocol(protocol()).expect("protocol");
    checker
}

#[test]
fn cached_protocol_queries_preserve_same_spelled_value_identity() {
    let first = indexed(Type::meta("N".into(), Type::Int, None));
    let second = indexed(Type::meta("N".into(), Type::Int, None));
    let mut checker = checker();
    checker
        .register_impl(implementation(first.clone()))
        .expect("implementation");
    let evidence = path("Evidence");
    assert!(checker.implements_cached(&first, &evidence));
    assert!(!checker.implements_cached(&second, &evidence));
    assert!(!checker.implements(&second, &evidence));
    assert!(checker.implements_cached(&first, &evidence));
}

#[test]
fn cached_protocol_queries_distinguish_ieee_payloads() {
    let first = indexed(nan(1));
    let second = indexed(nan(2));
    let mut checker = checker();
    checker
        .register_impl(implementation(first.clone()))
        .expect("implementation");
    let evidence = path("Evidence");
    assert!(checker.implements_cached(&first, &evidence));
    assert!(!checker.implements_cached(&second, &evidence));
    assert!(!checker.implements(&second, &evidence));
    assert!(checker.implements_cached(&first, &evidence));
}

#[test]
fn colliding_index_heads_are_retained_and_resolve_their_own_assignments() {
    let first = indexed(nan(1));
    let second = indexed(nan(2));
    let mut checker = checker();
    // Coherence is deliberately disabled here: this control isolates index
    // admission from overlap policy. It must retain both supplied declarations.
    checker.set_coherence_mode(CoherenceMode::Off);
    let mut a = implementation(first.clone());
    a.associated_types.insert(
        "Output".into(),
        AssociatedTypeAssignment::ordinary(Type::Int),
    );
    let mut b = implementation(second.clone());
    b.associated_types.insert(
        "Output".into(),
        AssociatedTypeAssignment::ordinary(Type::Bool),
    );
    checker.register_impl(a.clone()).expect("first");
    checker.register_impl(b).expect("second");
    checker.register_impl(a).expect("idempotent repeat");
    let evidence = path("Evidence");
    for (receiver, expected) in [(&first, Type::Int), (&second, Type::Bool)] {
        assert!(checker.implements_instantiation(receiver, &evidence, &[]));
        let implementation = checker
            .find_impl(receiver, &evidence)
            .expect("exact implementation");
        assert_eq!(&implementation.for_type, receiver);
        assert_eq!(
            implementation.associated_types[&"Output".into()].ordinary_body(),
            Some(&expected)
        );
    }
    let own = checker
        .all_implementations()
        .iter()
        .filter(|implementation| implementation.protocol == evidence)
        .count();
    assert_eq!(
        own, 2,
        "distinct collided heads survive; duplicate does not accumulate"
    );
}

#[test]
fn protocol_instantiation_index_checks_exact_value_arguments() {
    let mut checker = checker();
    checker.set_coherence_mode(CoherenceMode::Off);
    let receiver = Type::Named {
        path: path("Owner"),
        args: List::new(),
    };
    let mut implementation = implementation(receiver.clone());
    implementation.protocol_args.push(nan(1));
    checker
        .register_impl(implementation)
        .expect("indexed implementation");
    let evidence = path("Evidence");
    assert!(checker.implements_instantiation(&receiver, &evidence, &[nan(1)]));
    assert!(!checker.implements_instantiation(&receiver, &evidence, &[nan(2)]));
    assert!(
        checker
            .find_impl_for_instantiation(&receiver, &evidence, &[nan(2)])
            .is_none()
    );
}

#[test]
fn specialization_rechecks_value_queries_with_the_same_routing_key() {
    for (first, second) in [
        (indexed(integer(7)), indexed(integer(8))),
        (indexed(nan(1)), indexed(nan(2))),
        (
            indexed(Type::meta("N".into(), Type::Int, None)),
            indexed(Type::meta("N".into(), Type::Int, None)),
        ),
    ] {
        let mut checker = checker();
        checker
            .register_impl(implementation(first.clone()))
            .expect("implementation");
        let mut selector = SpecializationSelector::new();
        let mut unifier = Unifier::new();
        assert_eq!(
            selector.type_to_cache_key(&first),
            selector.type_to_cache_key(&second)
        );
        assert_eq!(
            selector
                .select_implementation(&protocol(), &first, &checker, &mut unifier)
                .expect("first query"),
            0
        );
        assert!(matches!(
            selector.select_implementation(&protocol(), &second, &checker, &mut unifier),
            Err(SpecializationError::NoApplicableImpl { .. })
        ));
        assert_eq!(
            selector
                .select_implementation(&protocol(), &first, &checker, &mut unifier)
                .expect("warm first"),
            0
        );
        assert_eq!(selector.stats().cache_hits, 1);
        assert_eq!(selector.cache_len(), 1);
    }
}

#[test]
fn specialization_repeated_type_parameters_are_one_binding() {
    let variable = Type::Var(TypeVar::fresh());
    let pair = |first, second| Type::Named {
        path: path("Pair"),
        args: [first, second].into_iter().collect(),
    };
    let pattern = pair(variable.clone(), variable);
    let mut checker = checker();
    checker
        .register_impl(implementation(pattern))
        .expect("generic implementation");
    let mut selector = SpecializationSelector::new();
    let mut unifier = Unifier::new();
    assert!(
        selector
            .select_implementation(
                &protocol(),
                &pair(Type::Int, Type::Int),
                &checker,
                &mut unifier
            )
            .is_ok()
    );
    assert!(matches!(
        selector.select_implementation(
            &protocol(),
            &pair(Type::Int, Type::Bool),
            &checker,
            &mut unifier
        ),
        Err(SpecializationError::NoApplicableImpl { .. })
    ));
}

#[test]
fn specialization_cache_follows_registry_identity_and_mutation() {
    let receiver = indexed(integer(7));
    let mut populated = checker();
    populated
        .register_impl(implementation(receiver.clone()))
        .expect("implementation");
    let mut selector = SpecializationSelector::new();
    let mut unifier = Unifier::new();
    assert!(
        selector
            .select_implementation(&protocol(), &receiver, &populated, &mut unifier)
            .is_ok()
    );
    let empty = checker();
    assert!(matches!(
        selector.select_implementation(&protocol(), &receiver, &empty, &mut unifier),
        Err(SpecializationError::NoApplicableImpl { .. })
    ));

    let mut left = empty.clone();
    let mut right = empty.clone();
    left.register_impl(implementation(receiver.clone()))
        .expect("left implementation");
    right
        .register_impl(implementation(indexed(integer(8))))
        .expect("right implementation");
    assert!(
        selector
            .select_implementation(&protocol(), &receiver, &left, &mut unifier)
            .is_ok()
    );
    assert!(matches!(
        selector.select_implementation(&protocol(), &receiver, &right, &mut unifier),
        Err(SpecializationError::NoApplicableImpl { .. })
    ));
}

#[test]
fn associated_assignment_requires_the_actual_indexed_receiver() {
    let mut checker = checker();
    let first = indexed(integer(7));
    let second = indexed(integer(8));
    let mut implementation = implementation(first.clone());
    implementation.associated_types.insert(
        "Output".into(),
        AssociatedTypeAssignment::ordinary(Type::Int),
    );
    checker
        .register_impl(implementation)
        .expect("implementation");
    assert_eq!(
        checker.try_find_associated_type(&first, &"Output".into()),
        Some(Type::Int)
    );
    assert_eq!(
        checker.try_find_associated_type(&second, &"Output".into()),
        None
    );
}

#[test]
fn method_cache_does_not_reuse_a_colliding_receiver() {
    let mut checker = checker();
    let first = indexed(nan(1));
    let second = indexed(nan(2));
    let mut implementation = implementation(first.clone());
    implementation
        .methods
        .insert("read".into(), Type::function(List::new(), Type::Int));
    checker
        .register_impl(implementation)
        .expect("implementation");
    let evidence = path("Evidence");
    let method = "read".into();
    assert!(
        checker
            .resolve_method_cached(&first, &evidence, &method)
            .is_ok()
    );
    assert!(checker.resolve_method(&second, &evidence, &method).is_err());
    assert!(
        checker
            .resolve_method_cached(&second, &evidence, &method)
            .is_err()
    );
    assert!(
        checker
            .resolve_method_cached(&first, &evidence, &method)
            .is_ok()
    );
}

fn segmented(name: &str) -> Path {
    Path::new(
        name.split('.')
            .map(|part| verum_ast::ty::PathSegment::Name(Ident::new(part, Span::default())))
            .collect(),
        Span::default(),
    )
}

#[test]
fn canonical_full_owner_survives_metadata_and_source_path_representations() {
    let mut checker = checker();
    let receiver = Type::Named {
        path: path("demo.model.Cell"),
        args: List::new(),
    };
    let mut implementation = implementation(receiver);
    implementation.protocol = path("demo.contract.Evidence");
    checker
        .register_impl(implementation)
        .expect("metadata representation");
    let receiver = Type::Named {
        path: segmented("demo.model.Cell"),
        args: List::new(),
    };
    let protocol = segmented("demo.contract.Evidence");
    assert!(checker.find_impl(&receiver, &protocol).is_some());
    assert!(
        checker
            .check_bounds(
                &receiver,
                &[verum_types::protocol::ProtocolBound::simple(
                    protocol.clone()
                )]
            )
            .is_ok()
    );
    let foreign = Type::Named {
        path: segmented("other.model.Cell"),
        args: List::new(),
    };
    assert!(checker.find_impl(&foreign, &protocol).is_none());
    assert!(
        checker
            .find_impl(&receiver, &segmented("other.contract.Evidence"))
            .is_none()
    );
    let mut relative = protocol;
    relative
        .segments
        .insert(0, verum_ast::ty::PathSegment::Relative);
    assert!(
        checker.find_impl(&receiver, &relative).is_none(),
        "relative routing collision is not absolute identity"
    );
}

#[test]
fn primitive_metadata_spelling_preserves_carrier_identity_without_erasing_values() {
    let mut checker = checker();
    let mut declared = protocol();
    declared.associated_types.insert(
        "EvidenceCarrier".into(),
        verum_types::protocol::AssociatedType::simple("EvidenceCarrier".into(), List::new()),
    );
    checker
        .register_protocol(declared)
        .expect("declared associated member");
    let mut implementation = implementation(Type::Named {
        path: path("Int"),
        args: List::new(),
    });
    implementation.associated_types.insert(
        "EvidenceCarrier".into(),
        AssociatedTypeAssignment::ordinary(Type::Bool),
    );
    checker
        .register_impl(implementation)
        .expect("primitive metadata representation");
    assert!(checker.find_impl(&Type::Int, &path("Evidence")).is_some());
    assert_eq!(
        checker.try_find_associated_type(&Type::Int, &"EvidenceCarrier".into()),
        Some(Type::Bool)
    );
    assert_eq!(
        checker
            .resolve_associated_type(&Type::Int, &path("Evidence"), &"EvidenceCarrier".into())
            .expect("qualified associated result"),
        Type::Bool
    );
    for unrelated in [
        integer(7),
        Type::Named {
            path: segmented("foreign.Int"),
            args: List::new(),
        },
        Type::Named {
            path: path("Int"),
            args: [Type::Bool].into_iter().collect(),
        },
    ] {
        assert!(checker.find_impl(&unrelated, &path("Evidence")).is_none());
        assert_eq!(
            checker.try_find_associated_type(&unrelated, &"EvidenceCarrier".into()),
            None
        );
    }
}

#[test]
fn named_unknown_is_not_the_unknown_top_type() {
    let mut checker = checker();
    let nominal = Type::Named {
        path: path("unknown"),
        args: List::new(),
    };
    checker
        .register_impl(implementation(nominal.clone()))
        .expect("nominal implementation");
    assert!(checker.find_impl(&nominal, &path("Evidence")).is_some());
    assert!(
        checker
            .find_impl(&Type::Unknown, &path("Evidence"))
            .is_none()
    );
}
