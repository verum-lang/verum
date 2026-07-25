//! Benchmarks for Flow Functions - Per-Field Interprocedural Analysis
//!
//! Benchmarks for CBGR flow functions (per-field interprocedural escape analysis).
//! Flow functions model escape propagation through CFG edges on a per-field basis,
//! enabling independent CBGR tier decisions for individual struct fields.
//!
//! Performance targets:
//! - Per-edge flow function: < 100ns
//! - Per-call interprocedural: < 500ns
//! - Whole-function analysis: < 5ms
//! - Overall complexity: O(edges × fields)

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use verum_cbgr::analysis::{BasicBlock, BlockId, ControlFlowGraph, EscapeAnalyzer, RefId};
use verum_cbgr::flow_functions::*;
use verum_common::{List, Maybe, Set, Text};

// ==================================================================================
// Benchmark Utilities
// ==================================================================================

fn create_cfg_with_blocks(num_blocks: usize) -> ControlFlowGraph {
    let mut cfg = ControlFlowGraph::new(BlockId(0), BlockId(num_blocks as u64 - 1));

    for i in 0..num_blocks {
        let id = BlockId(i as u64);
        let mut predecessors = Set::new();
        let mut successors = Set::new();

        if i > 0 {
            predecessors.insert(BlockId((i - 1) as u64));
        }
        if i < num_blocks - 1 {
            successors.insert(BlockId((i + 1) as u64));
        }

        let block = BasicBlock {
            id,
            predecessors,
            successors,
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

    cfg
}

fn create_flow_state_with_fields(num_refs: usize, num_fields: usize) -> FlowState {
    let mut state = FlowState::new();

    for ref_id in 0..num_refs {
        for field_id in 0..num_fields {
            let path = FieldPath::from_field(Text::from(format!("field_{}", field_id)));
            state.set_field_safe(RefId(ref_id as u64), path, true);
        }
    }

    state
}

// ==================================================================================
// Benchmark 1: FlowFunction Application (Baseline)
// ==================================================================================

fn bench_flow_function_apply(c: &mut Criterion) {
    let mut group = c.benchmark_group("flow_function_apply");

    for num_fields in [1, 5, 10, 20, 50].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(num_fields),
            num_fields,
            |b, &num_fields| {
                let op = IrOperation::Copy {
                    dest: SsaId(1),
                    src: SsaId(2),
                };
                let func = FlowFunction::new(op);
                let state = create_flow_state_with_fields(1, num_fields);

                b.iter(|| {
                    let output = func.apply(black_box(&state));
                    black_box(output);
                });
            },
        );
    }

    group.finish();
}

// ==================================================================================
// Benchmark 2: FlowFunctionCompiler - Compilation
// ==================================================================================

fn bench_flow_function_compilation(c: &mut Criterion) {
    let mut group = c.benchmark_group("flow_function_compilation");

    for num_blocks in [5, 10, 20, 50, 100].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(num_blocks),
            num_blocks,
            |b, &num_blocks| {
                let cfg = create_cfg_with_blocks(num_blocks);

                b.iter(|| {
                    let compiler = FlowFunctionCompiler::new(black_box(cfg.clone()));
                    let compiled = compiler.compile_all();
                    black_box(compiled);
                });
            },
        );
    }

    group.finish();
}

// ==================================================================================
// Benchmark 3: Edge Flow Function Application
// ==================================================================================

fn bench_edge_flow_function(c: &mut Criterion) {
    let mut group = c.benchmark_group("edge_flow_function");

    for num_fields in [1, 5, 10, 20, 50].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(num_fields),
            num_fields,
            |b, &num_fields| {
                let cfg = create_cfg_with_blocks(10);
                let compiler = FlowFunctionCompiler::new(cfg).compile_all();
                let state = create_flow_state_with_fields(1, num_fields);

                b.iter(|| {
                    let output = compiler.apply_edge(
                        black_box(BlockId(0)),
                        black_box(BlockId(1)),
                        black_box(&state),
                    );
                    black_box(output);
                });
            },
        );
    }

    group.finish();
}

// ==================================================================================
// Benchmark 4: FlowState Merge Operation
// ==================================================================================

fn bench_flow_state_merge(c: &mut Criterion) {
    let mut group = c.benchmark_group("flow_state_merge");

    for num_fields in [1, 10, 50, 100, 200].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(num_fields),
            num_fields,
            |b, &num_fields| {
                let state1 = create_flow_state_with_fields(5, num_fields);
                let state2 = create_flow_state_with_fields(5, num_fields);

                b.iter(|| {
                    let merged = black_box(&state1).merge(black_box(&state2));
                    black_box(merged);
                });
            },
        );
    }

    group.finish();
}

// ==================================================================================
// Benchmark 5: Interprocedural Field Flow
// ==================================================================================

fn bench_interprocedural_field_flow(c: &mut Criterion) {
    let mut group = c.benchmark_group("interprocedural_field_flow");

    for num_args in [1, 2, 5, 10, 20].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(num_args),
            num_args,
            |b, &num_args| {
                let mut flow = InterproceduralFieldFlow::new();
                let mut args = List::new();

                for i in 0..num_args {
                    let mut info = FieldFlowInfo::new(RefId(i as u64));
                    for j in 0..5 {
                        let path = FieldPath::from_field(Text::from(format!("f{}", j)));
                        info.set_field(path, true);
                    }
                    args.push(info);
                }

                b.iter(|| {
                    let output = flow.track_call(
                        black_box(BlockId(0)),
                        black_box(Text::from("test_func")),
                        black_box(args.clone()),
                    );
                    black_box(output);
                });
            },
        );
    }

    group.finish();
}

// ==================================================================================
// Benchmark 6: Field Path Operations
// ==================================================================================

fn bench_field_path_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("field_path_operations");

    // Test 1: Field path creation
    group.bench_function("create", |b| {
        b.iter(|| {
            let path = FieldPath::from_field(black_box(Text::from("field_name")));
            black_box(path);
        });
    });

    // Test 2: Field path extension
    group.bench_function("extend", |b| {
        let path = FieldPath::from_field(Text::from("x"));
        b.iter(|| {
            let extended = black_box(&path).extend(black_box(Text::from("y")));
            black_box(extended);
        });
    });

    // Test 3: Field path prefix check
    group.bench_function("is_prefix", |b| {
        let short = FieldPath::from_field(Text::from("x"));
        let long = short.clone().extend(Text::from("y"));
        b.iter(|| {
            let result = black_box(&short).is_prefix_of(black_box(&long));
            black_box(result);
        });
    });

    // Test 4: Deep nesting (10 levels)
    group.bench_function("deep_nesting", |b| {
        b.iter(|| {
            let mut path = FieldPath::from_field(Text::from("root"));
            for i in 0..10 {
                path = path.extend(Text::from(format!("level_{}", i)));
            }
            black_box(path);
        });
    });

    group.finish();
}

// ==================================================================================
// Benchmark 7: FieldFlowInfo Operations
// ==================================================================================

fn bench_field_flow_info(c: &mut Criterion) {
    let mut group = c.benchmark_group("field_flow_info");

    // Test 1: Set field
    group.bench_function("set_field", |b| {
        let mut info = FieldFlowInfo::new(RefId(1));
        let path = FieldPath::from_field(Text::from("x"));
        b.iter(|| {
            info.set_field(black_box(path.clone()), black_box(true));
        });
    });

    // Test 2: Check field safety
    group.bench_function("is_field_safe", |b| {
        let mut info = FieldFlowInfo::new(RefId(1));
        let path = FieldPath::from_field(Text::from("x"));
        info.set_field(path.clone(), true);

        b.iter(|| {
            let result = info.is_field_safe(black_box(&path));
            black_box(result);
        });
    });

    // Test 3: Merge field flow info
    group.bench_function("merge", |b| {
        let mut info1 = FieldFlowInfo::new(RefId(1));
        let mut info2 = FieldFlowInfo::new(RefId(1));

        for i in 0..10 {
            let path = FieldPath::from_field(Text::from(format!("f{}", i)));
            info1.set_field(path.clone(), true);
            info2.set_field(path, i % 2 == 0);
        }

        b.iter(|| {
            let merged = black_box(&info1).merge(black_box(&info2));
            black_box(merged);
        });
    });

    // Test 4: Safe fields enumeration
    group.bench_function("safe_fields", |b| {
        let mut info = FieldFlowInfo::new(RefId(1));

        for i in 0..20 {
            let path = FieldPath::from_field(Text::from(format!("f{}", i)));
            info.set_field(path, i % 2 == 0);
        }

        b.iter(|| {
            let fields = black_box(&info).safe_fields();
            black_box(fields);
        });
    });

    group.finish();
}

// ==================================================================================
// Benchmark 8: End-to-End Analysis
// ==================================================================================

fn bench_end_to_end_analysis(c: &mut Criterion) {
    let mut group = c.benchmark_group("end_to_end_analysis");

    for num_blocks in [5, 10, 20, 50].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(num_blocks),
            num_blocks,
            |b, &num_blocks| {
                let cfg = create_cfg_with_blocks(num_blocks);

                b.iter(|| {
                    // Compile flow functions
                    let analyzer = EscapeAnalyzer::new(black_box(cfg.clone()));
                    let compiler = analyzer.compute_flow_functions();

                    // Apply flow functions across all edges
                    let mut state = FlowState::new();
                    state.set_field_safe(RefId(1), FieldPath::from_field(Text::from("x")), true);

                    for i in 0..(num_blocks - 1) {
                        state =
                            compiler.apply_edge(BlockId(i as u64), BlockId((i + 1) as u64), &state);
                    }

                    black_box(state);
                });
            },
        );
    }

    group.finish();
}

// ==================================================================================
// Benchmark 9: Scalability Test
// ==================================================================================

fn bench_scalability(c: &mut Criterion) {
    let mut group = c.benchmark_group("scalability");

    for params in [(10, 5), (20, 10), (50, 20), (100, 50)].iter() {
        let (num_blocks, num_fields) = params;
        group.bench_with_input(
            BenchmarkId::new("blocks_fields", format!("{}_{}", num_blocks, num_fields)),
            params,
            |b, &(num_blocks, num_fields)| {
                let cfg = create_cfg_with_blocks(num_blocks);
                let compiler = FlowFunctionCompiler::new(cfg).compile_all();
                let state = create_flow_state_with_fields(5, num_fields);

                b.iter(|| {
                    let mut current = state.clone();
                    for i in 0..(num_blocks - 1) {
                        current = compiler.apply_edge(
                            BlockId(i as u64),
                            BlockId((i + 1) as u64),
                            &current,
                        );
                    }
                    black_box(current);
                });
            },
        );
    }

    group.finish();
}

// ==================================================================================
// Benchmark 10: IR Operation Processing
// ==================================================================================

fn bench_ir_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("ir_operations");

    // Test 1: Load operation
    group.bench_function("load", |b| {
        let op = IrOperation::Load {
            dest: SsaId(1),
            src: SsaId(2),
            field: Maybe::None,
        };
        let func = FlowFunction::new(op);
        let state = FlowState::new();

        b.iter(|| {
            let output = func.apply(black_box(&state));
            black_box(output);
        });
    });

    // Test 2: Store operation
    group.bench_function("store", |b| {
        let op = IrOperation::Store {
            dest: SsaId(1),
            src: SsaId(2),
            field: Maybe::None,
        };
        let func = FlowFunction::new(op);
        let state = FlowState::new();

        b.iter(|| {
            let output = func.apply(black_box(&state));
            black_box(output);
        });
    });

    // Test 3: Call operation
    group.bench_function("call", |b| {
        let mut args = List::new();
        args.push(SsaId(1));
        args.push(SsaId(2));

        let op = IrOperation::Call {
            result: Maybe::Some(SsaId(3)),
            function: Text::from("foo"),
            args,
        };
        let func = FlowFunction::new(op);
        let state = FlowState::new();

        b.iter(|| {
            let output = func.apply(black_box(&state));
            black_box(output);
        });
    });

    // Test 4: Phi operation
    group.bench_function("phi", |b| {
        let mut incoming = List::new();
        incoming.push((BlockId(0), SsaId(1)));
        incoming.push((BlockId(1), SsaId(2)));

        let op = IrOperation::Phi {
            dest: SsaId(3),
            incoming,
        };
        let func = FlowFunction::new(op);
        let state = FlowState::new();

        b.iter(|| {
            let output = func.apply(black_box(&state));
            black_box(output);
        });
    });

    // Test 5: Field access operation
    group.bench_function("field_access", |b| {
        let op = IrOperation::FieldAccess {
            dest: SsaId(1),
            src: SsaId(2),
            field: FieldPath::from_field(Text::from("x")),
        };
        let func = FlowFunction::new(op);
        let state = FlowState::new();

        b.iter(|| {
            let output = func.apply(black_box(&state));
            black_box(output);
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_flow_function_apply,
    bench_flow_function_compilation,
    bench_edge_flow_function,
    bench_flow_state_merge,
    bench_interprocedural_field_flow,
    bench_field_path_operations,
    bench_field_flow_info,
    bench_end_to_end_analysis,
    bench_scalability,
    bench_ir_operations
);

criterion_main!(benches);
