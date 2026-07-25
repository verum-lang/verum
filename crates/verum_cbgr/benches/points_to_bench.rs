//! Performance benchmarks for points-to analysis
//!
//! Validates that Andersen-style points-to analysis meets performance targets:
//! - O(n³) worst-case complexity
//! - O(n) to O(n²) typical complexity
//! - Sub-millisecond analysis for typical functions

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use verum_cbgr::analysis::{BasicBlock, BlockId, ControlFlowGraph, DefSite, RefId};
use verum_cbgr::points_to_analysis::*;
use verum_common::{List, Set};

// ==================================================================================
// Benchmark Utilities
// ==================================================================================

/// Create a CFG with N allocations
fn create_cfg_with_allocations(n: usize) -> ControlFlowGraph {
    let entry = BlockId(0);
    let exit = BlockId(n as u64 + 1);

    let mut cfg = ControlFlowGraph::new(entry, exit);

    // Entry block with allocations
    let mut definitions = List::new();
    for i in 0..n {
        definitions.push(DefSite {
            block: entry,
            reference: RefId(i as u64),
            is_stack_allocated: true,
            span: None,
        });
    }

    let entry_block = BasicBlock {
        id: entry,
        predecessors: Set::new(),
        successors: {
            let mut s = Set::new();
            s.insert(exit);
            s
        },
        definitions,
        uses: List::new(),
        call_sites: List::new(),
        has_await_point: false,
        is_exception_handler: false,
        is_cleanup_handler: false,
        may_throw: false,
    };

    // Exit block
    let exit_block = BasicBlock {
        id: exit,
        predecessors: {
            let mut p = Set::new();
            p.insert(entry);
            p
        },
        successors: Set::new(),
        definitions: List::new(),
        uses: List::new(),
        call_sites: List::new(),
        has_await_point: false,
        is_exception_handler: false,
        is_cleanup_handler: false,
        may_throw: false,
    };

    cfg.add_block(entry_block);
    cfg.add_block(exit_block);

    cfg
}

/// Create a chain of copy constraints
fn create_copy_chain(analyzer: &mut PointsToAnalyzer, length: usize) {
    let mut vars = vec![];
    for _ in 0..length {
        vars.push(analyzer.allocate_variable());
    }

    let loc = analyzer.allocate_location();

    // First variable points to location
    analyzer.add_constraint(PointsToConstraint::AddressOf {
        variable: vars[0],
        location: loc,
    });

    // Chain of copies
    for i in 0..length - 1 {
        analyzer.add_constraint(PointsToConstraint::Copy {
            dest: vars[i + 1],
            src: vars[i],
        });
    }
}

// ==================================================================================
// Benchmark 1: Constraint Generation from CFG
// ==================================================================================

fn bench_constraint_generation(c: &mut Criterion) {
    let mut group = c.benchmark_group("points_to_constraint_generation");

    for size in [10, 50, 100, 200, 500].iter() {
        let cfg = create_cfg_with_allocations(*size);

        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, _| {
            b.iter(|| {
                let mut analyzer = PointsToAnalyzer::new();
                let result = analyzer.generate_constraints_from_cfg(black_box(&cfg));
                black_box(result);
            });
        });
    }

    group.finish();
}

// ==================================================================================
// Benchmark 2: Constraint Solving (Fixpoint Iteration)
// ==================================================================================

fn bench_constraint_solving(c: &mut Criterion) {
    let mut group = c.benchmark_group("points_to_constraint_solving");

    for chain_length in [10, 20, 50, 100, 200].iter() {
        group.throughput(Throughput::Elements(*chain_length as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(chain_length),
            chain_length,
            |b, &length| {
                b.iter_batched(
                    || {
                        let mut analyzer = PointsToAnalyzer::new();
                        create_copy_chain(&mut analyzer, length);
                        analyzer
                    },
                    |mut analyzer| {
                        let result = analyzer.solve();
                        black_box(result);
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

// ==================================================================================
// Benchmark 3: Points-to Graph Queries
// ==================================================================================

fn bench_graph_queries(c: &mut Criterion) {
    let mut group = c.benchmark_group("points_to_graph_queries");

    // Build a graph with many variables
    let mut graph = PointsToGraph::new();
    for i in 0..1000 {
        let var = VarId(i);
        let loc = LocationId(i / 10); // Multiple vars point to same locs
        graph.add_points_to(var, loc);
    }

    group.bench_function("get_points_to_set", |b| {
        b.iter(|| {
            for i in 0..100 {
                let pts = graph.get_points_to_set(black_box(VarId(i)));
                black_box(pts);
            }
        });
    });

    group.bench_function("may_alias", |b| {
        b.iter(|| {
            for i in 0..50 {
                let result = graph.may_alias(black_box(VarId(i * 2)), black_box(VarId(i * 2 + 1)));
                black_box(result);
            }
        });
    });

    group.bench_function("must_alias", |b| {
        b.iter(|| {
            for i in 0..50 {
                let result = graph.must_alias(black_box(VarId(i * 2)), black_box(VarId(i * 2 + 1)));
                black_box(result);
            }
        });
    });

    group.bench_function("points_to_heap", |b| {
        b.iter(|| {
            for i in 0..100 {
                let result = graph.points_to_heap(black_box(VarId(i)));
                black_box(result);
            }
        });
    });

    group.finish();
}

// ==================================================================================
// Benchmark 4: End-to-End Analysis
// ==================================================================================

fn bench_end_to_end_analysis(c: &mut Criterion) {
    let mut group = c.benchmark_group("points_to_end_to_end");

    for size in [10, 50, 100, 200].iter() {
        let cfg = create_cfg_with_allocations(*size);

        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, _| {
            b.iter(|| {
                let result = PointsToAnalyzerBuilder::new()
                    .with_cfg(black_box(&cfg))
                    .build();
                black_box(result);
            });
        });
    }

    group.finish();
}

// ==================================================================================
// Benchmark 5: Scalability Test (Complexity Analysis)
// ==================================================================================

fn bench_scalability(c: &mut Criterion) {
    let mut group = c.benchmark_group("points_to_scalability");

    // Test scaling from 10 to 500 variables
    for size in [10, 25, 50, 100, 200, 500].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &n| {
            b.iter_batched(
                || {
                    let mut analyzer = PointsToAnalyzer::new();

                    // Create complex graph
                    let mut vars = vec![];
                    let mut locs = vec![];

                    for _ in 0..n {
                        vars.push(analyzer.allocate_variable());
                    }

                    for _ in 0..n / 2 {
                        locs.push(analyzer.allocate_location());
                    }

                    // Address-of constraints
                    for i in 0..n / 2 {
                        analyzer.add_constraint(PointsToConstraint::AddressOf {
                            variable: vars[i],
                            location: locs[i],
                        });
                    }

                    // Copy constraints (creates dependencies)
                    for i in 0..n - 1 {
                        analyzer.add_constraint(PointsToConstraint::Copy {
                            dest: vars[i + 1],
                            src: vars[i],
                        });
                    }

                    analyzer
                },
                |mut analyzer| {
                    let result = analyzer.solve();
                    black_box(result);
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

// ==================================================================================
// Benchmark 6: Constraint Type Performance
// ==================================================================================

fn bench_constraint_types(c: &mut Criterion) {
    let mut group = c.benchmark_group("points_to_constraint_types");

    // Address-of constraints
    group.bench_function("address_of", |b| {
        b.iter_batched(
            || {
                let mut analyzer = PointsToAnalyzer::new();
                let var = analyzer.allocate_variable();
                let loc = analyzer.allocate_location();
                analyzer.add_constraint(PointsToConstraint::AddressOf {
                    variable: var,
                    location: loc,
                });
                analyzer
            },
            |mut analyzer| {
                let result = analyzer.solve();
                black_box(result);
            },
            criterion::BatchSize::SmallInput,
        );
    });

    // Copy constraints
    group.bench_function("copy", |b| {
        b.iter_batched(
            || {
                let mut analyzer = PointsToAnalyzer::new();
                let var1 = analyzer.allocate_variable();
                let var2 = analyzer.allocate_variable();
                let loc = analyzer.allocate_location();

                analyzer.add_constraint(PointsToConstraint::AddressOf {
                    variable: var1,
                    location: loc,
                });
                analyzer.add_constraint(PointsToConstraint::Copy {
                    dest: var2,
                    src: var1,
                });
                analyzer
            },
            |mut analyzer| {
                let result = analyzer.solve();
                black_box(result);
            },
            criterion::BatchSize::SmallInput,
        );
    });

    // Load constraints
    group.bench_function("load", |b| {
        b.iter_batched(
            || {
                let mut analyzer = PointsToAnalyzer::new();
                let ptr = analyzer.allocate_variable();
                let dest = analyzer.allocate_variable();
                let loc = analyzer.allocate_location();

                analyzer.add_constraint(PointsToConstraint::AddressOf {
                    variable: ptr,
                    location: loc,
                });
                analyzer.add_constraint(PointsToConstraint::Load { dest, ptr });
                analyzer
            },
            |mut analyzer| {
                let result = analyzer.solve();
                black_box(result);
            },
            criterion::BatchSize::SmallInput,
        );
    });

    // Store constraints
    group.bench_function("store", |b| {
        b.iter_batched(
            || {
                let mut analyzer = PointsToAnalyzer::new();
                let ptr = analyzer.allocate_variable();
                let value = analyzer.allocate_variable();
                let loc = analyzer.allocate_location();

                analyzer.add_constraint(PointsToConstraint::AddressOf {
                    variable: ptr,
                    location: loc,
                });
                analyzer.add_constraint(PointsToConstraint::AddressOf {
                    variable: value,
                    location: loc,
                });
                analyzer.add_constraint(PointsToConstraint::Store { ptr, value });
                analyzer
            },
            |mut analyzer| {
                let result = analyzer.solve();
                black_box(result);
            },
            criterion::BatchSize::SmallInput,
        );
    });

    group.finish();
}

// ==================================================================================
// Criterion Configuration
// ==================================================================================

criterion_group!(
    benches,
    bench_constraint_generation,
    bench_constraint_solving,
    bench_graph_queries,
    bench_end_to_end_analysis,
    bench_scalability,
    bench_constraint_types,
);

criterion_main!(benches);
