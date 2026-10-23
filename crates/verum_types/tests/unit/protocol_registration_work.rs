//! T1655: measure real coherence comparisons while preserving refusal semantics.
use super::*;
use crate::ty::TypeVar;
use verum_ast::ty::PathSegment;
use verum_common::span::FileId;

fn implementation(protocol: &str, target: Type) -> ProtocolImpl {
    ProtocolImpl {
        protocol: Path::new(
            protocol
                .split('.')
                .map(|part| PathSegment::Name(Ident::new(part, Span::default())))
                .collect(),
            Span::default(),
        ),
        protocol_args: List::new(),
        for_type: target,
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

#[test]
fn registration_does_not_compare_unrelated_protocols() {
    let mut checker = ProtocolChecker::new_empty();
    for index in 0..1024 {
        checker
            .register_impl(implementation(&format!("Protocol{index}"), Type::Int))
            .unwrap();
    }
    assert_eq!(checker.impls.len(), 1024);
    assert_eq!(
        checker.registration_overlap_comparisons, 0,
        "different protocol identities cannot overlap; these are actual check_overlap calls",
    );
}

#[test]
fn same_protocol_conflicts_are_still_refused() {
    let mut checker = ProtocolChecker::new_empty();
    checker
        .register_impl(implementation("Selected", Type::Int))
        .unwrap();
    let error = checker.register_impl(implementation("Selected", Type::Int));
    assert!(matches!(
        error,
        Err(CoherenceError::OverlappingImplementations { .. })
    ));
    assert_eq!(checker.registration_overlap_comparisons, 1);
    assert_eq!(checker.impls.len(), 1);
}

#[test]
fn qualified_protocols_with_the_same_leaf_are_distinct() {
    let mut checker = ProtocolChecker::new_empty();
    checker
        .register_impl(implementation("alpha.Selected", Type::Int))
        .unwrap();
    checker
        .register_impl(implementation("beta.Selected", Type::Int))
        .unwrap();
    assert_eq!(checker.registration_overlap_comparisons, 0);
    assert!(matches!(
        checker.register_impl(implementation("alpha.Selected", Type::Int)),
        Err(CoherenceError::OverlappingImplementations { .. }),
    ));
    assert_eq!(checker.registration_overlap_comparisons, 1);
    assert_eq!(checker.impls.len(), 2);
}

#[test]
fn protocol_arguments_remain_overlap_candidates() {
    let mut checker = ProtocolChecker::new_empty();
    for argument in [Type::Int, Type::Text] {
        let mut entry = implementation("Selected", Type::Bool);
        entry.protocol_args.push(argument);
        checker.register_impl(entry).unwrap();
    }
    assert_eq!(checker.registration_overlap_comparisons, 1);
    let mut generic = implementation("Selected", Type::Bool);
    generic.protocol_args.push(Type::Var(TypeVar::with_id(17)));
    assert!(matches!(
        checker.register_impl(generic),
        Err(CoherenceError::OverlappingImplementations { .. }),
    ));
    assert_eq!(checker.registration_overlap_comparisons, 2);
    assert_eq!(checker.impls.len(), 2);
}

fn partial_target(argument: Type) -> Type {
    Type::Tuple([argument, Type::Bool].into_iter().collect())
}

#[test]
fn partial_blankets_still_conflict_and_explicit_specialization_still_works() {
    let mut checker = ProtocolChecker::new_empty();
    let generic = implementation("Selected", partial_target(Type::Var(TypeVar::with_id(17))));
    checker.register_impl(generic).unwrap();
    let mut concrete = implementation("Selected", partial_target(Type::Int));
    assert!(matches!(
        checker.register_impl(concrete.clone()),
        Err(CoherenceError::OverlappingImplementations { .. }),
    ));
    concrete.specialization = Some(crate::advanced_protocols::SpecializationInfo::specialized(
        Path::single(Ident::new("GenericImpl", Span::default())),
        1,
    ));
    checker.register_impl(concrete).unwrap();
    assert_eq!(checker.registration_overlap_comparisons, 2);
    assert_eq!(checker.impls.len(), 2);
}

#[test]
fn disabled_coherence_and_cloning_preserve_future_candidates() {
    let mut checker = ProtocolChecker::new_empty();
    checker.set_coherence_mode(CoherenceMode::Off);
    checker
        .register_impl(implementation("Other", Type::Int))
        .unwrap();
    checker
        .register_impl(implementation("Selected", Type::Int))
        .unwrap();
    checker
        .register_impl(implementation("Selected", Type::Int))
        .unwrap();
    assert_eq!(checker.impls.len(), 2, "re-registration remains idempotent");
    assert_eq!(checker.registration_overlap_comparisons, 0);

    let mut cloned = checker.clone();
    cloned.set_coherence_mode(CoherenceMode::Strict);
    cloned
        .register_impl(implementation("Selected", Type::Text))
        .unwrap();
    assert_eq!(
        cloned.registration_overlap_comparisons, 1,
        "no duplicate candidate"
    );
    assert!(matches!(
        cloned.register_impl(implementation("Selected", Type::Int)),
        Err(CoherenceError::OverlappingImplementations { .. }),
    ));
    assert_eq!(cloned.registration_overlap_comparisons, 2);
    assert_eq!(cloned.impls.len(), 3);
    assert_eq!(checker.impls.len(), 2);
    assert_eq!(checker.registration_overlap_comparisons, 0);
}

#[test]
fn lenient_warnings_preserve_registration_order_and_duplicate_checks() {
    let mut checker = ProtocolChecker::new_empty();
    checker.set_coherence_mode(CoherenceMode::Off);
    checker
        .register_impl(implementation("Other", Type::Int))
        .unwrap();
    for offset in [10_u32, 20, 30] {
        // Alpha-renamed copies are deliberately idempotent. Use distinct
        // partial patterns that all overlap the concrete three-field tuple.
        let variable = Type::Var(TypeVar::with_id(offset as usize));
        let arguments = match offset {
            10 => [variable, Type::Bool, Type::Text],
            20 => [Type::Int, variable, Type::Text],
            _ => [Type::Int, Type::Bool, variable],
        };
        let mut entry = implementation("Selected", Type::Tuple(arguments.into_iter().collect()));
        entry.span = Span::new(offset, offset + 1, FileId::new(1));
        checker.register_impl(entry).unwrap();
    }
    checker.set_coherence_mode(CoherenceMode::Lenient);
    let mut concrete = implementation(
        "Selected",
        Type::Tuple([Type::Int, Type::Bool, Type::Text].into_iter().collect()),
    );
    concrete.span = Span::new(100, 101, FileId::new(1));
    checker.register_impl(concrete.clone()).unwrap();
    let warnings = checker.drain_coherence_warnings();
    assert_eq!(warnings.len(), 3);
    for (warning, offset) in warnings.iter().zip([10, 20, 30]) {
        match warning {
            CoherenceError::OverlappingImplementations {
                first_impl_location,
                second_impl_location,
                ..
            } => {
                assert_eq!(*first_impl_location, concrete.span);
                assert_eq!(second_impl_location.start, offset);
            }
            other => panic!("expected overlap warning, got {other:?}"),
        }
    }
    assert_eq!(checker.registration_overlap_comparisons, 3);
    checker.register_impl(concrete).unwrap();
    assert_eq!(checker.drain_coherence_warnings().len(), 4);
    assert_eq!(checker.registration_overlap_comparisons, 7);
    assert_eq!(
        checker.impls.len(),
        5,
        "lenient duplicate emits warnings without inserting"
    );
}

#[test]
fn orphan_rules_still_precede_overlap_checks_in_strict_and_lenient_modes() {
    let mut checker = ProtocolChecker::new_empty();
    checker.set_current_crate("consumer".into());
    assert!(matches!(
        checker.register_impl(implementation("Foreign", Type::Int)),
        Err(CoherenceError::OrphanRuleViolation { .. }),
    ));
    assert_eq!(checker.impls.len(), 0);
    checker.set_coherence_mode(CoherenceMode::Lenient);
    checker
        .register_impl(implementation("Foreign", Type::Int))
        .unwrap();
    assert!(matches!(
        checker.drain_coherence_warnings().as_slice(),
        [CoherenceError::OrphanRuleViolation { .. }]
    ));
    checker
        .register_impl(implementation("Foreign", Type::Int))
        .unwrap();
    assert!(matches!(
        checker.drain_coherence_warnings().as_slice(),
        [
            CoherenceError::OrphanRuleViolation { .. },
            CoherenceError::OverlappingImplementations { .. }
        ],
    ));
    assert_eq!(checker.registration_overlap_comparisons, 1);
    checker.set_coherence_mode(CoherenceMode::Strict);
    assert!(matches!(
        checker.register_impl(implementation("Foreign", Type::Int)),
        Err(CoherenceError::OrphanRuleViolation { .. }),
    ));
    assert_eq!(
        checker.registration_overlap_comparisons, 1,
        "orphan refusal happens first"
    );
    assert_eq!(checker.impls.len(), 1);
}

fn nominal_target(owner: &str, arguments: List<Type>, offset: u32) -> Type {
    let span = Span::new(offset, offset + 1, FileId::new(offset));
    Type::Named {
        path: Path::new(
            owner
                .split('.')
                .map(|part| PathSegment::Name(Ident::new(part, span)))
                .collect(),
            span,
        ),
        args: arguments,
    }
}

#[test]
fn nominal_duplicate_ignores_source_locations() {
    let mut checker = ProtocolChecker::new_empty();
    checker
        .register_impl(implementation(
            "Selected",
            nominal_target("package.Item", List::new(), 10),
        ))
        .unwrap();
    assert!(matches!(
        checker.register_impl(implementation(
            "Selected",
            nominal_target("package.Item", List::new(), 20),
        )),
        Err(CoherenceError::OverlappingImplementations { .. }),
    ));
    assert_eq!(checker.impls.len(), 1);
}

#[test]
fn nominal_blanket_overlap_ignores_source_locations() {
    let mut checker = ProtocolChecker::new_empty();
    checker
        .register_impl(implementation(
            "Selected",
            nominal_target("package.Item", List::from_iter([Type::Var(TypeVar::fresh())]), 10),
        ))
        .unwrap();
    assert!(matches!(
        checker.register_impl(implementation(
            "Selected",
            nominal_target("package.Item", List::from_iter([Type::Int]), 20),
        )),
        Err(CoherenceError::OverlappingImplementations { .. }),
    ));
    assert_eq!(checker.impls.len(), 1);
}

#[test]
fn nominal_protocol_arguments_ignore_source_locations() {
    let mut checker = ProtocolChecker::new_empty();
    let mut first = implementation("Selected", Type::Bool);
    first.protocol_args.push(nominal_target("package.Input", List::new(), 10));
    checker.register_impl(first).unwrap();
    let mut second = implementation("Selected", Type::Bool);
    second.protocol_args.push(nominal_target("package.Input", List::new(), 20));
    assert!(matches!(
        checker.register_impl(second),
        Err(CoherenceError::OverlappingImplementations { .. }),
    ));
    assert_eq!(checker.impls.len(), 1);
}

#[test]
fn nominal_coherence_keeps_owners_and_arguments_distinct() {
    let mut checker = ProtocolChecker::new_empty();
    for owner in ["alpha.Item", "beta.Item", "alpha.nested.Item", "Item"] {
        for argument in [Type::Int, Type::Text] {
            checker
                .register_impl(implementation(
                    "Selected",
                    nominal_target(owner, List::from_iter([argument]), 10),
                ))
                .unwrap();
        }
    }
    assert_eq!(checker.impls.len(), 8);
}

#[test]
fn disjoint_nominal_targets_share_a_protocol() {
    let mut checker = ProtocolChecker::new_empty();
    let started = std::time::Instant::now();
    for index in 0..256 {
        checker
            .register_impl(implementation(
                "Selected",
                nominal_target(&format!("package.Type{index}"), List::new(), index),
            ))
            .unwrap();
    }
    assert_eq!(checker.impls.len(), 256);
    assert_eq!(checker.registration_overlap_comparisons, 0);
    eprintln!("256 nominal targets, zero coherence comparisons: {:?}", started.elapsed());
}

#[test]
fn nominal_index_survives_disabled_coherence_and_cloning() {
    let mut checker = ProtocolChecker::new_empty();
    checker.set_coherence_mode(CoherenceMode::Off);
    let generic = nominal_target(
        "owner.Item",
        List::from_iter([Type::Var(TypeVar::fresh())]),
        1,
    );
    checker
        .register_impl(implementation("Selected", generic))
        .unwrap();
    let mut cloned = checker.clone();
    cloned.set_coherence_mode(CoherenceMode::Strict);
    cloned
        .register_impl(implementation(
            "Selected",
            nominal_target("other.Item", List::from_iter([Type::Int]), 2),
        ))
        .unwrap();
    assert_eq!(cloned.registration_overlap_comparisons, 0);
    assert!(matches!(
        cloned.register_impl(implementation(
            "Selected",
            nominal_target("owner.Item", List::from_iter([Type::Int]), 3)
        )),
        Err(CoherenceError::OverlappingImplementations { .. }),
    ));
    assert_eq!(cloned.registration_overlap_comparisons, 1);
    assert_eq!(checker.impls.len(), 1);
}

#[test]
fn nominal_candidates_match_full_scan_warnings_in_insertion_order() {
    let mut checker = ProtocolChecker::new_empty();
    checker.set_coherence_mode(CoherenceMode::Off);
    let variable = Type::Var(TypeVar::with_id(23));
    for (offset, target) in [
        nominal_target(
            "owner.Item",
            List::from_iter([variable.clone(), Type::Bool]),
            10,
        ),
        variable.clone(),
        nominal_target("other.Item", List::from_iter([Type::Int, Type::Bool]), 20),
        Type::Tuple(List::from_iter([variable.clone()])),
        nominal_target(
            "owner.Item",
            List::from_iter([Type::Int, variable.clone()]),
            30,
        ),
        nominal_target("owner.Item", List::from_iter([variable]), 40),
    ]
    .into_iter()
    .enumerate()
    {
        let mut entry = implementation("Selected", target);
        entry.span = Span::new(offset as u32, offset as u32 + 1, FileId::new(1));
        checker.register_impl(entry).unwrap();
    }
    checker.set_coherence_mode(CoherenceMode::Lenient);
    let entry = implementation(
        "Selected",
        nominal_target("owner.Item", List::from_iter([Type::Int, Type::Bool]), 50),
    );
    let expected: List<_> = checker
        .impls
        .iter()
        .filter_map(|existing| checker.check_overlap(&entry, existing).err())
        .collect();
    checker.register_impl(entry).unwrap();
    let actual = checker.drain_coherence_warnings();
    assert_eq!(format!("{actual:?}"), format!("{:?}", expected.as_slice()));
    assert_eq!(actual.len(), 2);
    assert_eq!(checker.registration_overlap_comparisons, 4);
}

#[test]
fn nonnominal_and_special_path_targets_keep_conservative_candidates() {
    let mut checker = ProtocolChecker::new_empty();
    let mut path = Path::single(Ident::new("Item", Span::default()));
    path.segments.insert(0, PathSegment::SelfValue);
    let special = Type::Named {
        path,
        args: List::new(),
    };
    checker
        .register_impl(implementation("Selected", special.clone()))
        .unwrap();
    checker
        .register_impl(implementation(
            "Selected",
            nominal_target("owner.Item", List::new(), 1),
        ))
        .unwrap();
    let before = checker.registration_overlap_comparisons;
    assert!(matches!(
        checker.register_impl(implementation("Selected", special)),
        Err(CoherenceError::OverlappingImplementations { .. })
    ));
    assert_eq!(checker.registration_overlap_comparisons, before + 1);
    // A new nonnominal target still visits both prior nominal candidates.
    checker
        .register_impl(implementation("Selected", Type::Var(TypeVar::fresh())))
        .unwrap();
    assert_eq!(checker.registration_overlap_comparisons, before + 3);
}
