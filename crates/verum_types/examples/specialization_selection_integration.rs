//! Resolve real implementations, reuse validated queries and invalidate stale decisions.
//!
//! Run: cargo run -p verum_types --example specialization_selection_integration

use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_common::{List, Map};
use verum_types::advanced_protocols::SpecializationInfo;
use verum_types::protocol::{Protocol, ProtocolChecker, ProtocolImpl, ProtocolKind};
use verum_types::specialization_selection::{SpecializationError, SpecializationSelector};
use verum_types::ty::{Type, TypeVar};
use verum_types::unify::Unifier;

fn implementation(target: Type, rank: usize) -> ProtocolImpl {
    ProtocolImpl {
        parameters: target.free_vars().into_iter().map(verum_types::context::GenericBinder::Type).collect(),
        protocol: Path::single(Ident::new("Render", Span::default())),
        protocol_args: List::new(),
        for_type: target,
        where_clauses: List::new(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: (rank > 0).then(|| SpecializationInfo {
            is_specialized: true,
            specializes: None,
            specificity_rank: rank,
            is_default: false,
            span: Span::default(),
        }),
        impl_crate: None,
        span: Span::default(),
        type_param_fn_bounds: Map::new(),
    }
}

fn main() {
    let protocol = Protocol {
        source_parameters: None,
        constraints: Default::default(),
        name: "Render".into(),
        kind: ProtocolKind::Constraint,
        type_params: List::new(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: None,
        defining_crate: None,
        span: Span::default(),
    };
    let target = Type::Named {
        path: Path::single(Ident::new("Canvas", Span::default())),
        args: List::new(),
    };
    let mut registry = ProtocolChecker::new();
    registry.register_protocol(protocol.clone()).unwrap();
    registry
        .register_impl(implementation(Type::Var(TypeVar::fresh()), 0))
        .unwrap();
    let mut selector = SpecializationSelector::new();
    let mut unifier = Unifier::new();
    for _ in 0..2 {
        assert_eq!(
            selector
                .select_implementation(&protocol, &target, &registry, &mut unifier)
                .unwrap(),
            0
        );
    }
    assert_eq!(selector.stats().cache_hits, 1);
    println!("Repeated query reused its actual generic implementation.");

    registry
        .register_impl(implementation(target.clone(), 1))
        .unwrap();
    assert_eq!(
        selector
            .select_implementation(&protocol, &target, &registry, &mut unifier)
            .unwrap(),
        1
    );
    println!("After registration, Canvas selects its more specific implementation.");

    let mut empty_registry = ProtocolChecker::new();
    empty_registry.register_protocol(protocol.clone()).unwrap();
    assert!(matches!(
        selector.select_implementation(&protocol, &target, &empty_registry, &mut unifier),
        Err(SpecializationError::NoApplicableImpl { .. })
    ));
    assert_eq!(selector.cache_len(), 0);
    println!("The same query fails in a registry with no implementation.");
    println!(
        "Observed selections: {}; cache hits: {}; cache misses: {}",
        selector.stats().selections,
        selector.stats().cache_hits,
        selector.stats().cache_misses
    );
}
