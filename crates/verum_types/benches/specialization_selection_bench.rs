//! Performance Benchmarks for Specialization Selection
//!
//! Measures the performance of specialization selection under various scenarios.
//!
//! Run with: cargo bench --bench specialization_selection_bench

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_common::{List, Map, Maybe, Set};

use verum_types::advanced_protocols::{SpecializationInfo, SpecializationLattice};
use verum_types::protocol::{Protocol, ProtocolChecker, ProtocolImpl, ProtocolKind};
use verum_types::specialization_selection::{CoherenceChecker, SpecializationSelector};
use verum_types::ty::Type;
use verum_types::unify::Unifier;

// ==================== Helper Functions ====================

fn make_impl(protocol: &str, for_type: Type, rank: usize) -> ProtocolImpl {
    let path = Path::single(Ident::new(protocol, Span::default()));

    ProtocolImpl {
        parameters: for_type.free_vars().into_iter().map(verum_types::context::GenericBinder::Type).collect(),
        protocol: path,
        protocol_args: List::new(),
        for_type,
        where_clauses: List::new(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: if rank > 0 {
            Maybe::Some(SpecializationInfo {
                is_specialized: true,
                specializes: Maybe::None,
                specificity_rank: rank,
                is_default: false,
                span: Span::default(),
            })
        } else {
            Maybe::None
        },
        impl_crate: Maybe::None,
        span: Span::default(),
        type_param_fn_bounds: Map::new(),
    }
}

fn make_type(name: &str) -> Type {
    Type::Named {
        path: Path::single(Ident::new(name, Span::default())),
        args: List::new(),
    }
}

fn make_type_var(id: usize) -> Type {
    use verum_types::ty::TypeVar;
    Type::Var(TypeVar::with_id(id))
}

fn selection_registry(count: usize) -> (Protocol, ProtocolChecker) {
    let protocol = Protocol {
        source_parameters: None,
        constraints: Default::default(),
        kind: ProtocolKind::Constraint,
        name: "Render".into(),
        type_params: List::new(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: None,
        defining_crate: None,
        span: Span::default(),
    };
    let mut registry = ProtocolChecker::new();
    registry.register_protocol(protocol.clone()).unwrap();
    for i in 0..count {
        registry
            .register_impl(make_impl("Render", make_type(&format!("Surface{i}")), 0))
            .unwrap();
    }
    (protocol, registry)
}

// ==================== Benchmarks ====================

fn bench_lattice_construction(c: &mut Criterion) {
    let mut group = c.benchmark_group("lattice_construction");

    for size in [5, 10, 20, 50].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.iter(|| {
                let mut lattice = SpecializationLattice::new();

                // Add implementations
                for i in 0..size {
                    lattice.add_impl(i);
                }

                // Build linear chain
                for i in 1..size {
                    lattice.ordering.insert((i, i - 1), true);
                }

                black_box(lattice)
            });
        });
    }

    group.finish();
}

fn bench_selection_uncached(c: &mut Criterion) {
    let mut group = c.benchmark_group("selection_uncached");

    for num_impls in [2, 5, 10].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(num_impls),
            num_impls,
            |b, &num_impls| {
                b.iter(|| {
                    let mut lattice = SpecializationLattice::new();

                    // Add implementations
                    for i in 0..num_impls {
                        lattice.add_impl(i);
                    }

                    // Build ordering (linear chain)
                    for i in 1..num_impls {
                        lattice.ordering.insert((i, i - 1), true);
                    }

                    // Select from all implementations
                    let mut applicable = Set::new();
                    for i in 0..num_impls {
                        applicable.insert(i);
                    }

                    black_box(lattice.select_most_specific(&applicable))
                });
            },
        );
    }

    group.finish();
}

fn bench_selection_cached(c: &mut Criterion) {
    let mut group = c.benchmark_group("selection_cached");
    let (protocol, registry) = selection_registry(100);
    let mut selector = SpecializationSelector::new();
    let mut unifier = Unifier::new();
    for i in 0..100 {
        assert_eq!(
            selector
                .select_implementation(
                    &protocol,
                    &make_type(&format!("Surface{i}")),
                    &registry,
                    &mut unifier
                )
                .unwrap(),
            i
        );
    }
    let target = make_type("Surface50");
    group.bench_function("validated_cached_selection", |b| {
        b.iter(|| {
            black_box(
                selector
                    .select_implementation(&protocol, &target, &registry, &mut unifier)
                    .unwrap(),
            )
        });
    });
    group.finish();
}

fn bench_type_matching(c: &mut Criterion) {
    let mut group = c.benchmark_group("type_matching");

    let selector = SpecializationSelector::new();
    let mut unifier = Unifier::new();

    group.bench_function("concrete_match", |b| {
        let concrete = make_type("Int");
        let pattern = make_type("Int");

        b.iter(|| black_box(selector.matches_impl_pattern(&concrete, &pattern, &mut unifier)));
    });

    group.bench_function("generic_match", |b| {
        let concrete = make_type("Int");
        let pattern = make_type_var(0);

        b.iter(|| black_box(selector.matches_impl_pattern(&concrete, &pattern, &mut unifier)));
    });

    group.bench_function("compound_match", |b| {
        let concrete = Type::Named {
            path: Path::single(Ident::new("List", Span::default())),
            args: vec![make_type("Int")].into(),
        };
        let pattern = Type::Named {
            path: Path::single(Ident::new("List", Span::default())),
            args: vec![make_type_var(0)].into(),
        };

        b.iter(|| black_box(selector.matches_impl_pattern(&concrete, &pattern, &mut unifier)));
    });

    group.finish();
}

fn bench_coherence_checking(c: &mut Criterion) {
    let mut group = c.benchmark_group("coherence_checking");

    for num_impls in [5, 10, 20].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(num_impls),
            num_impls,
            |b, &num_impls| {
                let mut impls = List::new();

                // Create non-overlapping implementations
                for i in 0..num_impls {
                    let impl_info = make_impl("Display", make_type(&format!("Type{}", i)), 0);
                    impls.push(impl_info);
                }

                b.iter(|| {
                    let checker = CoherenceChecker::new();

                    // Check all pairs for overlap
                    for i in 0..impls.len() {
                        for j in (i + 1)..impls.len() {
                            black_box(checker.overlaps(&impls[i], &impls[j]));
                        }
                    }

                    black_box(checker)
                });
            },
        );
    }

    group.finish();
}

fn bench_lattice_selection_chain(c: &mut Criterion) {
    let mut group = c.benchmark_group("lattice_selection_chain");

    for chain_length in [3, 5, 10].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(chain_length),
            chain_length,
            |b, &chain_length| {
                let mut lattice = SpecializationLattice::new();

                // Build chain
                for i in 0..chain_length {
                    lattice.add_impl(i);
                }

                for i in 1..chain_length {
                    lattice.ordering.insert((i, i - 1), true);
                }

                b.iter(|| {
                    let mut applicable = Set::new();
                    for i in 0..chain_length {
                        applicable.insert(i);
                    }

                    black_box(lattice.select_most_specific(&applicable))
                });
            },
        );
    }

    group.finish();
}

fn bench_lattice_selection_diamond(c: &mut Criterion) {
    let mut group = c.benchmark_group("lattice_selection_diamond");

    group.bench_function("diamond_4_nodes", |b| {
        let mut lattice = SpecializationLattice::new();

        // Diamond:
        //  0
        //  / \
        //  1 2
        //  \ /
        //  3
        lattice.add_impl(0);
        lattice.add_impl(1);
        lattice.add_impl(2);
        lattice.add_impl(3);

        lattice.ordering.insert((1, 0), true);
        lattice.ordering.insert((2, 0), true);
        lattice.ordering.insert((3, 1), true);
        lattice.ordering.insert((3, 2), true);

        b.iter(|| {
            let mut applicable = Set::new();
            applicable.insert(0);
            applicable.insert(1);
            applicable.insert(2);
            applicable.insert(3);

            black_box(lattice.select_most_specific(&applicable))
        });
    });

    group.finish();
}

fn bench_cache_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_operations");
    let (protocol, registry) = selection_registry(1);
    let target = make_type("Surface0");
    group.bench_function("cold_selection", |b| {
        b.iter(|| {
            let mut selector = SpecializationSelector::new();
            black_box(
                selector
                    .select_implementation(&protocol, &target, &registry, &mut Unifier::new())
                    .unwrap(),
            )
        });
    });
    group.bench_function("warm_selection", |b| {
        let mut selector = SpecializationSelector::new();
        let mut unifier = Unifier::new();
        assert_eq!(
            selector
                .select_implementation(&protocol, &target, &registry, &mut unifier)
                .unwrap(),
            0
        );
        b.iter(|| {
            black_box(
                selector
                    .select_implementation(&protocol, &target, &registry, &mut unifier)
                    .unwrap(),
            )
        });
    });
    group.bench_function("missing_implementation", |b| {
        let mut selector = SpecializationSelector::new();
        let mut unifier = Unifier::new();
        let missing = make_type("Missing");
        b.iter(|| {
            black_box(
                selector
                    .select_implementation(&protocol, &missing, &registry, &mut unifier)
                    .unwrap_err(),
            )
        });
    });
    group.finish();
}

// NOTE: bench_specificity_comparison is disabled because is_more_specific_type is private
// If needed for performance testing, the method would need to be made pub(crate) or pub
/*

fn bench_specificity_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("specificity_comparison");

    let selector = SpecializationSelector::new();

    group.bench_function("concrete_vs_generic", |b| {
        let concrete = make_type("Int");
        let generic = make_type_var(0);

        b.iter(|| black_box(selector.is_more_specific_type(&concrete, &generic)));
    });

    group.bench_function("compound_types", |b| {
        let t1 = Type::Named {
            path: Path::single(Ident::new("List", Span::default())),
            args: vec![make_type("Int")].into(),
        };
        let t2 = Type::Named {
            path: Path::single(Ident::new("List", Span::default())),
            args: vec![make_type_var(0)].into(),
        };

        b.iter(|| black_box(selector.is_more_specific_type(&t1, &t2)));
    });

    group.finish();
}
*/

criterion_group!(
    benches,
    bench_lattice_construction,
    bench_selection_uncached,
    bench_selection_cached,
    bench_type_matching,
    bench_coherence_checking,
    bench_lattice_selection_chain,
    bench_lattice_selection_diamond,
    bench_cache_operations,
    // bench_specificity_comparison, // Disabled: uses private method
);

criterion_main!(benches);
