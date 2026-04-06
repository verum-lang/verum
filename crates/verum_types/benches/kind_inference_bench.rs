//! Performance Benchmarks for Kind Inference
//!
//! Higher-kinded type (HKT) kind inference: infers kinds for type constructors
//! (e.g., List has kind Type -> Type, Map has kind Type -> Type -> Type).
//! Uses constraint-based kind inference with unification.
//!
//! Performance Targets:
//! - Kind inference: <5ms for typical protocols
//! - Kind checking: <1ms per type application
//! - Constraint solving: <10ms for complex kinds

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use verum_ast::{
    span::Span,
    ty::{Ident, Path},
};
use verum_common::{List, Map, Maybe};
use verum_types::{
    TypeChecker,
    advanced_protocols::{GATTypeParam, Variance},
    kind_inference::KindInference,
    kind_inference::{Kind, KindConstraint, KindInferer},
    protocol::{AssociatedType, Protocol},
    ty::Type,
};

// Helper to create a simple path
fn simple_path(name: &str) -> Path {
    Path::single(Ident::new(name, Span::default()))
}

// Helper to create a named type
fn named_type(name: &str, args: List<Type>) -> Type {
    Type::Named {
        path: simple_path(name),
        args,
    }
}

// ==================== Basic Kind Inference Benchmarks ====================

fn bench_infer_primitive(c: &mut Criterion) {
    c.bench_function("kind_infer_primitive", |b| {
        b.iter(|| {
            let mut inferer = KindInferer::new();
            black_box(inferer.infer_kind(&Type::Int))
        });
    });
}

fn bench_infer_simple_type_constructor(c: &mut Criterion) {
    c.bench_function("kind_infer_list_int", |b| {
        b.iter(|| {
            let mut inferer = KindInferer::new();
            let ty = named_type("List", List::from(vec![Type::Int]));
            black_box(inferer.infer_kind(&ty))
        });
    });
}

fn bench_infer_nested_types(c: &mut Criterion) {
    let mut group = c.benchmark_group("kind_infer_nested");

    for depth in [1, 3, 5, 10, 20] {
        group.bench_with_input(BenchmarkId::from_parameter(depth), &depth, |b, &depth| {
            // Create nested List<List<...<Int>...>>
            let mut ty = Type::Int;
            for _ in 0..depth {
                ty = named_type("List", List::from(vec![ty]));
            }

            b.iter(|| {
                let mut inferer = KindInferer::new();
                black_box(inferer.infer_kind(&ty))
            });
        });
    }

    group.finish();
}

fn bench_infer_complex_types(c: &mut Criterion) {
    c.bench_function("kind_infer_map_text_list_int", |b| {
        b.iter(|| {
            let mut inferer = KindInferer::new();
            let inner = named_type("List", List::from(vec![Type::Int]));
            let ty = named_type("Map", List::from(vec![Type::Text, inner]));
            black_box(inferer.infer_kind(&ty))
        });
    });
}

// ==================== Kind Unification Benchmarks ====================

fn bench_unify_concrete_kinds(c: &mut Criterion) {
    c.bench_function("kind_unify_concrete", |b| {
        b.iter(|| {
            let mut inferer = KindInferer::new();
            black_box(inferer.unify(
                &Kind::unary_constructor(),
                &Kind::unary_constructor(),
                Span::default(),
                "bench".into(),
            ))
        });
    });
}

fn bench_unify_kind_variables(c: &mut Criterion) {
    c.bench_function("kind_unify_vars", |b| {
        b.iter(|| {
            let mut inferer = KindInferer::new();
            black_box(inferer.unify(
                &Kind::KindVar(0),
                &Kind::Type,
                Span::default(),
                "bench".into(),
            ))
        });
    });
}

fn bench_unify_arrow_kinds(c: &mut Criterion) {
    c.bench_function("kind_unify_arrows", |b| {
        b.iter(|| {
            let mut inferer = KindInferer::new();
            let k1 = Kind::arrow(Kind::KindVar(0), Kind::KindVar(1));
            let k2 = Kind::arrow(Kind::Type, Kind::Type);
            black_box(inferer.unify(&k1, &k2, Span::default(), "bench".into()))
        });
    });
}

// ==================== Protocol Kind Checking Benchmarks ====================

fn bench_check_simple_protocol(c: &mut Criterion) {
    c.bench_function("kind_check_simple_protocol", |b| {
        // Create simple protocol with regular associated type
        let mut associated_types = Map::new();
        associated_types.insert(
            "Item".into(),
            AssociatedType::simple("Item".into(), List::new()),
        );

        let protocol = Protocol {
            kind: verum_types::protocol::ProtocolKind::Constraint,
            name: "Iterator".into(),
            type_params: List::new(),
            methods: Map::new(),
            associated_types,
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::None,
            defining_crate: Maybe::Some("bench".into()),
            span: Span::default(),
        };

        b.iter(|| {
            let mut inferer = KindInferer::new();
            black_box(inferer.check_protocol_kinds(&protocol))
        });
    });
}

fn bench_check_gat_protocol(c: &mut Criterion) {
    c.bench_function("kind_check_gat_protocol", |b| {
        // Create protocol with GAT
        let type_params = List::from(vec![GATTypeParam {
            name: "T".into(),
            bounds: List::new(),
            default: Maybe::None,
            variance: Variance::Covariant,
        }]);

        let mut associated_types = Map::new();
        associated_types.insert(
            "Item".into(),
            AssociatedType::generic("Item".into(), type_params, List::new(), List::new()),
        );

        let protocol = Protocol {
            kind: verum_types::protocol::ProtocolKind::Constraint,
            name: "Collection".into(),
            type_params: List::new(),
            methods: Map::new(),
            associated_types,
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::None,
            defining_crate: Maybe::Some("bench".into()),
            span: Span::default(),
        };

        b.iter(|| {
            let mut inferer = KindInferer::new();
            black_box(inferer.check_protocol_kinds(&protocol))
        });
    });
}

// ==================== Constraint Solving Benchmarks ====================

fn bench_solve_constraints(c: &mut Criterion) {
    let mut group = c.benchmark_group("kind_solve_constraints");

    for n_constraints in [10, 50, 100, 200] {
        group.bench_with_input(
            BenchmarkId::from_parameter(n_constraints),
            &n_constraints,
            |b, &n_constraints| {
                b.iter(|| {
                    let mut inferer = KindInferer::new();

                    // Add constraints
                    for i in 0..n_constraints {
                        inferer.add_constraint(KindConstraint::equal(
                            Kind::KindVar(i as u32),
                            if i % 2 == 0 {
                                Kind::Type
                            } else {
                                Kind::unary_constructor()
                            },
                            Span::default(),
                            format!("constraint {}", i),
                        ));
                    }

                    black_box(inferer.solve())
                });
            },
        );
    }

    group.finish();
}

// ==================== TypeChecker Integration Benchmarks ====================

fn bench_typechecker_infer_kind(c: &mut Criterion) {
    c.bench_function("typechecker_infer_kind", |b| {
        let ty = named_type("List", List::from(vec![Type::Int]));

        b.iter(|| {
            let mut checker = TypeChecker::new();
            black_box(checker.infer_kind(&ty))
        });
    });
}

fn bench_typechecker_check_kind(c: &mut Criterion) {
    c.bench_function("typechecker_check_kind", |b| {
        let ty = named_type("Map", List::from(vec![Type::Text, Type::Int]));

        b.iter(|| {
            let mut checker = TypeChecker::new();
            black_box(checker.check_kind(&ty, &Kind::Type))
        });
    });
}

// ==================== Real-World Scenario Benchmarks ====================

fn bench_functor_protocol(c: &mut Criterion) {
    c.bench_function("kind_check_functor_protocol", |b| {
        // Simulate Functor protocol:
        // protocol Functor {
        //     type F<_>  // Kind: * -> *
        //     fn fmap<A, B>(f: fn(A) -> B, fa: F<A>) -> F<B>
        // }

        let type_params = List::from(vec![GATTypeParam {
            name: "T".into(),
            bounds: List::new(),
            default: Maybe::None,
            variance: Variance::Covariant,
        }]);

        let mut associated_types = Map::new();
        associated_types.insert(
            "F".into(),
            AssociatedType::generic("F".into(), type_params, List::new(), List::new()),
        );

        let protocol = Protocol {
            kind: verum_types::protocol::ProtocolKind::Constraint,
            name: "Functor".into(),
            type_params: List::new(),
            methods: Map::new(),
            associated_types,
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::None,
            defining_crate: Maybe::Some("bench".into()),
            span: Span::default(),
        };

        b.iter(|| {
            let mut inferer = KindInferer::new();
            black_box(inferer.check_protocol_kinds(&protocol))
        });
    });
}

fn bench_monad_protocol(c: &mut Criterion) {
    c.bench_function("kind_check_monad_protocol", |b| {
        // Simulate Monad protocol:
        // protocol Monad {
        //     type M<_>  // Kind: * -> *
        //     fn pure<T>(value: T) -> M<T>
        //     fn flat_map<A, B>(ma: M<A>, f: fn(A) -> M<B>) -> M<B>
        // }

        let type_params = List::from(vec![GATTypeParam {
            name: "T".into(),
            bounds: List::new(),
            default: Maybe::None,
            variance: Variance::Covariant,
        }]);

        let mut associated_types = Map::new();
        associated_types.insert(
            "M".into(),
            AssociatedType::generic("M".into(), type_params, List::new(), List::new()),
        );

        let protocol = Protocol {
            kind: verum_types::protocol::ProtocolKind::Constraint,
            name: "Monad".into(),
            type_params: List::new(),
            methods: Map::new(),
            associated_types,
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::None,
            defining_crate: Maybe::Some("bench".into()),
            span: Span::default(),
        };

        b.iter(|| {
            let mut inferer = KindInferer::new();
            black_box(inferer.check_protocol_kinds(&protocol))
        });
    });
}

// ==================== Benchmark Groups ====================

criterion_group!(
    basic_benchmarks,
    bench_infer_primitive,
    bench_infer_simple_type_constructor,
    bench_infer_nested_types,
    bench_infer_complex_types
);

criterion_group!(
    unification_benchmarks,
    bench_unify_concrete_kinds,
    bench_unify_kind_variables,
    bench_unify_arrow_kinds
);

criterion_group!(
    protocol_benchmarks,
    bench_check_simple_protocol,
    bench_check_gat_protocol
);

criterion_group!(constraint_benchmarks, bench_solve_constraints);

criterion_group!(
    integration_benchmarks,
    bench_typechecker_infer_kind,
    bench_typechecker_check_kind
);

criterion_group!(
    scenario_benchmarks,
    bench_functor_protocol,
    bench_monad_protocol
);

criterion_main!(
    basic_benchmarks,
    unification_benchmarks,
    protocol_benchmarks,
    constraint_benchmarks,
    integration_benchmarks,
    scenario_benchmarks
);
