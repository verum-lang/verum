//! Performance Benchmarks for Specialization Selection
//!
//! Measures the performance of specialization selection under various scenarios.
//!
//! Run with: cargo bench --bench specialization_selection_bench

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_common::{List, Map, Maybe, Set, Text};

use verum_types::advanced_protocols::{SpecializationInfo, SpecializationLattice};
use verum_types::protocol::{Protocol, ProtocolChecker, ProtocolImpl};
use verum_types::specialization_selection::{CoherenceChecker, SpecializationSelector};
use verum_types::ty::Type;
use verum_types::unify::Unifier;

// ==================== Helper Functions ====================

fn make_protocol(name: &str) -> Protocol {
    Protocol {
        name: name.into(),
        kind: verum_types::protocol::ProtocolKind::Constraint,
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

fn make_impl(protocol: &str, for_type: Type, rank: usize) -> ProtocolImpl {
    let path = Path::single(Ident::new(protocol, Span::default()));

    ProtocolImpl {
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

    let mut selector = SpecializationSelector::new();

    // Pre-populate cache
    for i in 0..100 {
        selector.cache_selection("Display".into(), format!("Type{}", i).into(), i % 10);
    }

    group.bench_function("cached_lookup", |b| {
        b.iter(|| black_box(selector.cache.get(&("Display".into(), "Type50".into()))));
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
                    let mut checker = CoherenceChecker::new();

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
        //       0
        //      / \
        //     1   2
        //      \ /
        //       3
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

    group.bench_function("cache_insert", |b| {
        b.iter(|| {
            let mut selector = SpecializationSelector::new();
            selector.cache_selection("Display".into(), "Int".into(), 42);
            black_box(selector.cache.len())
        });
    });

    group.bench_function("cache_lookup_hit", |b| {
        let mut selector = SpecializationSelector::new();
        selector.cache_selection("Display".into(), "Int".into(), 42);

        b.iter(|| black_box(selector.cache.get(&("Display".into(), "Int".into()))));
    });

    group.bench_function("cache_lookup_miss", |b| {
        let selector = SpecializationSelector::new();

        b.iter(|| black_box(selector.cache.get(&("Display".into(), "Int".into()))));
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
