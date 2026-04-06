//! Benchmarks for value tracking performance
//!
//! Measures performance of key operations:
//! - Concrete value propagation
//! - Range analysis computation
//! - Symbolic expression evaluation
//! - Path predicate evaluation
//! - Full dataflow analysis
//!
//! Performance target: < 200μs for typical functions

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use verum_cbgr::analysis::{BasicBlock, BlockId, ControlFlowGraph, EscapeAnalyzer};
use verum_cbgr::value_tracking::{
    BinaryOp as ValueBinaryOp, ConcreteValue, PathPredicate, SymbolicValue, ValuePropagator,
    ValueRange, ValueState,
};
use verum_common::{List, Map, Set};

// ==================================================================================
// Benchmark 1: Concrete Value Operations
// ==================================================================================

fn bench_concrete_value_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("concrete_value_operations");

    // Constant creation
    group.bench_function("create_integer", |b| {
        b.iter(|| black_box(ConcreteValue::Integer(42)));
    });

    // Value merging
    group.bench_function("merge_same", |b| {
        let v1 = ConcreteValue::Integer(42);
        let v2 = ConcreteValue::Integer(42);
        b.iter(|| black_box(v1.merge(&v2)));
    });

    group.bench_function("merge_different", |b| {
        let v1 = ConcreteValue::Integer(42);
        let v2 = ConcreteValue::Integer(10);
        b.iter(|| black_box(v1.merge(&v2)));
    });

    // Binary operations
    group.bench_function("add_integers", |b| {
        let v1 = ConcreteValue::Integer(10);
        let v2 = ConcreteValue::Integer(5);
        b.iter(|| black_box(v1.eval_binop(ValueBinaryOp::Add, &v2)));
    });

    group.bench_function("compare_integers", |b| {
        let v1 = ConcreteValue::Integer(10);
        let v2 = ConcreteValue::Integer(5);
        b.iter(|| black_box(v1.eval_binop(ValueBinaryOp::Lt, &v2)));
    });

    group.finish();
}

// ==================================================================================
// Benchmark 2: Range Analysis Operations
// ==================================================================================

fn bench_range_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("range_operations");

    // Range creation
    group.bench_function("create_constant_range", |b| {
        b.iter(|| black_box(ValueRange::from_constant(42)));
    });

    group.bench_function("create_bounded_range", |b| {
        b.iter(|| black_box(ValueRange::from_bounds(0, 100)));
    });

    // Range operations
    group.bench_function("intersect_ranges", |b| {
        let r1 = ValueRange::from_bounds(0, 50);
        let r2 = ValueRange::from_bounds(25, 75);
        b.iter(|| black_box(r1.intersect(&r2)));
    });

    group.bench_function("union_ranges", |b| {
        let r1 = ValueRange::from_bounds(0, 50);
        let r2 = ValueRange::from_bounds(25, 75);
        b.iter(|| black_box(r1.union(&r2)));
    });

    // Range arithmetic
    group.bench_function("range_add", |b| {
        let r1 = ValueRange::from_bounds(0, 10);
        let r2 = ValueRange::from_bounds(5, 15);
        b.iter(|| black_box(r1.eval_binop(ValueBinaryOp::Add, &r2)));
    });

    group.bench_function("range_multiply", |b| {
        let r1 = ValueRange::from_bounds(0, 10);
        let r2 = ValueRange::from_bounds(5, 15);
        b.iter(|| black_box(r1.eval_binop(ValueBinaryOp::Mul, &r2)));
    });

    group.finish();
}

// ==================================================================================
// Benchmark 3: Symbolic Expression Evaluation
// ==================================================================================

fn bench_symbolic_evaluation(c: &mut Criterion) {
    let mut group = c.benchmark_group("symbolic_evaluation");

    // Simple expressions
    group.bench_function("evaluate_constant", |b| {
        let sym = SymbolicValue::from_concrete(ConcreteValue::Integer(42));
        let env = Map::new();
        b.iter(|| black_box(sym.evaluate(&env)));
    });

    group.bench_function("evaluate_variable", |b| {
        let sym = SymbolicValue::variable(0);
        let mut env = Map::new();
        env.insert(0, ConcreteValue::Integer(10));
        b.iter(|| black_box(sym.evaluate(&env)));
    });

    // Binary operations
    group.bench_function("evaluate_add", |b| {
        let sym = SymbolicValue::binop(
            ValueBinaryOp::Add,
            SymbolicValue::variable(0),
            SymbolicValue::from_concrete(ConcreteValue::Integer(10)),
        );
        let mut env = Map::new();
        env.insert(0, ConcreteValue::Integer(32));
        b.iter(|| black_box(sym.evaluate(&env)));
    });

    // Complex expressions
    group.bench_function("evaluate_complex", |b| {
        // (x * 2) + 10
        let x = SymbolicValue::variable(0);
        let two = SymbolicValue::from_concrete(ConcreteValue::Integer(2));
        let ten = SymbolicValue::from_concrete(ConcreteValue::Integer(10));
        let mul = SymbolicValue::binop(ValueBinaryOp::Mul, x, two);
        let result = SymbolicValue::binop(ValueBinaryOp::Add, mul, ten);

        let mut env = Map::new();
        env.insert(0, ConcreteValue::Integer(5));

        b.iter(|| black_box(result.evaluate(&env)));
    });

    group.finish();
}

// ==================================================================================
// Benchmark 4: Value State Operations
// ==================================================================================

fn bench_value_state_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("value_state_operations");

    // Set/get operations
    group.bench_function("set_concrete", |b| {
        let mut state = ValueState::new();
        b.iter(|| {
            state.set_concrete(0, ConcreteValue::Integer(42));
        });
    });

    group.bench_function("get_concrete", |b| {
        let mut state = ValueState::new();
        state.set_concrete(0, ConcreteValue::Integer(42));
        b.iter(|| black_box(state.get_concrete(0)));
    });

    // State merging
    group.bench_function("merge_states_small", |b| {
        let mut state1 = ValueState::new();
        let mut state2 = ValueState::new();
        for i in 0..10 {
            state1.set_concrete(i, ConcreteValue::Integer(i as i64));
            state2.set_concrete(i, ConcreteValue::Integer(i as i64));
        }
        b.iter(|| black_box(state1.merge(&state2)));
    });

    group.bench_function("merge_states_large", |b| {
        let mut state1 = ValueState::new();
        let mut state2 = ValueState::new();
        for i in 0..100 {
            state1.set_concrete(i, ConcreteValue::Integer(i as i64));
            state2.set_concrete(i, ConcreteValue::Integer(i as i64));
        }
        b.iter(|| black_box(state1.merge(&state2)));
    });

    // Condition refinement
    group.bench_function("refine_with_condition", |b| {
        let mut state = ValueState::new();
        state.set_range(0, ValueRange::from_bounds(0, 100));

        let predicate = SymbolicValue::binop(
            ValueBinaryOp::Lt,
            SymbolicValue::variable(0),
            SymbolicValue::from_concrete(ConcreteValue::Integer(50)),
        );

        b.iter(|| black_box(state.refine_with_condition(&predicate, true)));
    });

    group.finish();
}

// ==================================================================================
// Benchmark 5: Value Propagation
// ==================================================================================

fn bench_value_propagation(c: &mut Criterion) {
    let mut group = c.benchmark_group("value_propagation");

    // Constant propagation
    group.bench_function("propagate_constant", |b| {
        let mut propagator = ValuePropagator::new();
        let mut state = ValueState::new();
        b.iter(|| {
            propagator.propagate_constant(&mut state, 0, ConcreteValue::Integer(42));
        });
    });

    // Binary operation propagation
    group.bench_function("propagate_binop", |b| {
        let mut propagator = ValuePropagator::new();
        let mut state = ValueState::new();
        state.set_concrete(1, ConcreteValue::Integer(10));
        state.set_concrete(2, ConcreteValue::Integer(5));

        b.iter(|| {
            propagator.propagate_binop(&mut state, 0, ValueBinaryOp::Add, 1, 2);
        });
    });

    // Phi node propagation
    group.bench_function("propagate_phi", |b| {
        let mut propagator = ValuePropagator::new();
        let mut state = ValueState::new();

        let incoming: List<(BlockId, u32)> = vec![(BlockId(0), 1u32), (BlockId(1), 2u32)].into();

        b.iter(|| {
            propagator.propagate_phi(&mut state, 0, BlockId(2), &incoming);
        });
    });

    // Chain propagation (varying length)
    for chain_len in [10, 50, 100].iter() {
        group.bench_with_input(
            BenchmarkId::new("propagate_chain", chain_len),
            chain_len,
            |b, &len| {
                b.iter(|| {
                    let mut propagator = ValuePropagator::new();
                    let mut state = ValueState::new();

                    // x0 = 1
                    propagator.propagate_constant(&mut state, 0, ConcreteValue::Integer(1));

                    // xi = xi-1 + 1
                    for i in 1..len {
                        state.set_concrete(i, ConcreteValue::Integer(1));
                        propagator.propagate_binop(&mut state, i, ValueBinaryOp::Add, i - 1, i);
                    }
                });
            },
        );
    }

    group.finish();
}

// ==================================================================================
// Benchmark 6: Path Predicate Evaluation
// ==================================================================================

fn bench_path_predicate_evaluation(c: &mut Criterion) {
    let mut group = c.benchmark_group("path_predicate_evaluation");

    // Simple predicate
    group.bench_function("evaluate_constant_predicate", |b| {
        let condition = SymbolicValue::from_concrete(ConcreteValue::Boolean(true));
        let predicate = PathPredicate::new(condition, true, BlockId(0));
        let state = ValueState::new();

        b.iter(|| black_box(predicate.evaluate(&state)));
    });

    // Variable predicate
    group.bench_function("evaluate_variable_predicate", |b| {
        let condition = SymbolicValue::binop(
            ValueBinaryOp::Lt,
            SymbolicValue::variable(0),
            SymbolicValue::from_concrete(ConcreteValue::Integer(100)),
        );
        let predicate = PathPredicate::new(condition, true, BlockId(0));

        let mut state = ValueState::new();
        state.set_concrete(0, ConcreteValue::Integer(50));

        b.iter(|| black_box(predicate.evaluate(&state)));
    });

    // State refinement
    group.bench_function("refine_state_with_predicate", |b| {
        let condition = SymbolicValue::binop(
            ValueBinaryOp::Lt,
            SymbolicValue::variable(0),
            SymbolicValue::from_concrete(ConcreteValue::Integer(100)),
        );
        let predicate = PathPredicate::new(condition, true, BlockId(0));

        let mut state = ValueState::new();
        state.set_range(0, ValueRange::unbounded());

        b.iter(|| black_box(predicate.refine_state(&state)));
    });

    group.finish();
}

// ==================================================================================
// Benchmark 7: Full Value Tracking (Integration)
// ==================================================================================

fn bench_full_value_tracking(c: &mut Criterion) {
    let mut group = c.benchmark_group("full_value_tracking");
    group.sample_size(50); // Fewer samples for expensive benchmarks

    // Create CFG with varying sizes
    for block_count in [5, 10, 20, 50].iter() {
        group.bench_with_input(
            BenchmarkId::new("track_values", block_count),
            block_count,
            |b, &num_blocks| {
                // Create CFG
                let mut cfg = ControlFlowGraph::new(BlockId(0), BlockId(num_blocks - 1));

                for i in 0..num_blocks {
                    let mut preds = Set::new();
                    let mut succs = Set::new();

                    if i > 0 {
                        preds.insert(BlockId(i - 1));
                    }
                    if i < num_blocks - 1 {
                        succs.insert(BlockId(i + 1));
                    }

                    let block = BasicBlock {
                        id: BlockId(i),
                        predecessors: preds,
                        successors: succs,
                        definitions: List::new(),
                        uses: List::new(),
                        call_sites: List::new(),
                        has_await_point: false,
                        is_exception_handler: false,
                        is_cleanup_handler: false,
                        may_throw: false,
                    };
                    cfg.add_block(block);
                }

                let analyzer = EscapeAnalyzer::new(cfg);

                b.iter(|| black_box(analyzer.track_concrete_values()));
            },
        );
    }

    group.finish();
}

// ==================================================================================
// Benchmark 8: Scalability Tests
// ==================================================================================

fn bench_scalability(c: &mut Criterion) {
    let mut group = c.benchmark_group("scalability");
    group.sample_size(20);

    // Many concurrent values
    for value_count in [10, 50, 100, 500].iter() {
        group.bench_with_input(
            BenchmarkId::new("many_values", value_count),
            value_count,
            |b, &count| {
                b.iter(|| {
                    let mut state = ValueState::new();
                    for i in 0..count {
                        state.set_concrete(i, ConcreteValue::Integer(i as i64));
                    }
                    black_box(state)
                });
            },
        );
    }

    // Complex expression depth
    for depth in [5, 10, 15].iter() {
        group.bench_with_input(
            BenchmarkId::new("expression_depth", depth),
            depth,
            |b, &d| {
                b.iter(|| {
                    let mut expr = SymbolicValue::variable(0);
                    for i in 1..d {
                        expr = SymbolicValue::binop(
                            ValueBinaryOp::Add,
                            expr,
                            SymbolicValue::from_concrete(ConcreteValue::Integer(i as i64)),
                        );
                    }

                    let mut env = Map::new();
                    env.insert(0, ConcreteValue::Integer(10));
                    black_box(expr.evaluate(&env))
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_concrete_value_operations,
    bench_range_operations,
    bench_symbolic_evaluation,
    bench_value_state_operations,
    bench_value_propagation,
    bench_path_predicate_evaluation,
    bench_full_value_tracking,
    bench_scalability,
);
criterion_main!(benches);
