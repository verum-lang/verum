//! T1655: measure real coherence comparisons while preserving refusal semantics.
use super::*;

fn implementation(protocol: &str, target: Type) -> ProtocolImpl {
    ProtocolImpl {
        protocol: Path::single(Ident::new(protocol, Span::default())),
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
        checker.register_impl(implementation(&format!("Protocol{index}"), Type::Int)).unwrap();
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
    checker.register_impl(implementation("Selected", Type::Int)).unwrap();
    let error = checker.register_impl(implementation("Selected", Type::Int));
    assert!(matches!(error, Err(CoherenceError::OverlappingImplementations { .. })));
    assert_eq!(checker.registration_overlap_comparisons, 1);
    assert_eq!(checker.impls.len(), 1);
}
