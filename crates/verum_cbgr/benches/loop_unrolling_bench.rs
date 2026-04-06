//! Benchmarks for Loop Unrolling
//!
//! Benchmarks for CBGR loop unrolling escape analysis. Validates that loop
//! unrolling (detecting loops, unrolling up to configurable bound, per-iteration
//! escape analysis) completes within performance targets for practical code sizes.
//!
//! These benchmarks measure the performance of loop unrolling for escape analysis.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use verum_cbgr::analysis::{
    BasicBlock, BlockId, ControlFlowGraph, DefSite, EscapeAnalyzer, RefId, UseeSite,
};
use verum_cbgr::loop_unrolling::{LoopUnroller, UnrollConfig};
use verum_common::{List, Set};

// ==================================================================================
// Benchmark Utilities
// ==================================================================================

/// Helper to create a BasicBlock with default flags
fn make_block(
    id: BlockId,
    predecessors: Set<BlockId>,
    successors: Set<BlockId>,
    definitions: List<DefSite>,
    uses: List<UseeSite>,
) -> BasicBlock {
    BasicBlock {
        id,
        predecessors,
        successors,
        definitions,
        uses,
        call_sites: List::new(),
        has_await_point: false,
        is_exception_handler: false,
        is_cleanup_handler: false,
        may_throw: false,
    }
}

/// Create a simple loop CFG for benchmarking
fn create_loop_cfg(num_blocks_in_body: usize) -> ControlFlowGraph {
    let entry = BlockId(0);
    let header = BlockId(1);
    let exit = BlockId(1000);

    let mut cfg = ControlFlowGraph::new(entry, exit);

    // Entry block
    cfg.add_block(make_block(
        entry,
        Set::new(),
        {
            let mut s = Set::new();
            s.insert(header);
            s
        },
        List::new(),
        List::new(),
    ));

    // Header block with allocation
    cfg.add_block(make_block(
        header,
        {
            let mut s = Set::new();
            s.insert(entry);
            if num_blocks_in_body > 0 {
                s.insert(BlockId(2 + num_blocks_in_body as u64 - 1)); // Back edge
            }
            s
        },
        {
            let mut s = Set::new();
            if num_blocks_in_body > 0 {
                s.insert(BlockId(2));
            } else {
                s.insert(exit);
            }
            s.insert(exit);
            s
        },
        {
            let mut defs = List::new();
            defs.push(DefSite {
                block: header,
                reference: RefId(1),
                is_stack_allocated: true,
                span: None,
            });
            defs
        },
        List::new(),
    ));

    // Body blocks
    for i in 0..num_blocks_in_body {
        let block_id = BlockId(2 + i as u64);
        let prev_block = if i == 0 {
            header
        } else {
            BlockId(1 + i as u64)
        };
        let next_block = if i == num_blocks_in_body - 1 {
            header // Back edge
        } else {
            BlockId(3 + i as u64)
        };

        cfg.add_block(make_block(
            block_id,
            {
                let mut s = Set::new();
                s.insert(prev_block);
                s
            },
            {
                let mut s = Set::new();
                s.insert(next_block);
                s
            },
            {
                let mut defs = List::new();
                defs.push(DefSite {
                    block: block_id,
                    reference: RefId(10 + i as u64),
                    is_stack_allocated: true,
                    span: None,
                });
                defs
            },
            {
                let mut uses = List::new();
                uses.push(UseeSite {
                    block: block_id,
                    reference: RefId(1),
                    is_mutable: false,
                    span: None,
                });
                uses
            },
        ));
    }

    // Exit block
    cfg.add_block(make_block(
        exit,
        {
            let mut s = Set::new();
            s.insert(header);
            s
        },
        Set::new(),
        List::new(),
        List::new(),
    ));

    cfg
}

/// Create CFG with multiple nested loops
fn create_nested_loop_cfg(depth: usize) -> ControlFlowGraph {
    let entry = BlockId(0);
    let exit = BlockId(1000);

    let mut cfg = ControlFlowGraph::new(entry, exit);

    // Entry
    cfg.add_block(make_block(
        entry,
        Set::new(),
        {
            let mut s = Set::new();
            s.insert(BlockId(1));
            s
        },
        List::new(),
        List::new(),
    ));

    // Create nested loop headers
    for i in 0..depth {
        let header = BlockId(1 + i as u64);
        let body = BlockId(100 + i as u64);
        let next_level = if i < depth - 1 {
            BlockId(2 + i as u64)
        } else {
            body
        };

        // Header
        cfg.add_block(make_block(
            header,
            {
                let mut s = Set::new();
                if i == 0 {
                    s.insert(entry);
                } else {
                    s.insert(BlockId(i as u64));
                }
                s.insert(body);
                s
            },
            {
                let mut s = Set::new();
                s.insert(next_level);
                s.insert(exit);
                s
            },
            {
                let mut defs = List::new();
                defs.push(DefSite {
                    block: header,
                    reference: RefId(i as u64),
                    is_stack_allocated: true,
                    span: None,
                });
                defs
            },
            List::new(),
        ));

        // Body
        cfg.add_block(make_block(
            body,
            {
                let mut s = Set::new();
                s.insert(next_level);
                s
            },
            {
                let mut s = Set::new();
                s.insert(header);
                s
            },
            List::new(),
            {
                let mut uses = List::new();
                uses.push(UseeSite {
                    block: body,
                    reference: RefId(i as u64),
                    is_mutable: false,
                    span: None,
                });
                uses
            },
        ));
    }

    // Exit
    cfg.add_block(make_block(
        exit,
        Set::new(),
        Set::new(),
        List::new(),
        List::new(),
    ));

    cfg
}

// ==================================================================================
// Benchmark 1: Loop Detection
// ==================================================================================

fn bench_loop_detection(c: &mut Criterion) {
    let mut group = c.benchmark_group("loop_detection");

    for body_size in [1, 5, 10, 20, 50] {
        group.bench_with_input(
            BenchmarkId::from_parameter(body_size),
            &body_size,
            |b, &size| {
                let cfg = create_loop_cfg(size);
                b.iter(|| {
                    let mut unroller = LoopUnroller::new();
                    let loops = unroller.detect_loops(black_box(&cfg));
                    black_box(loops);
                });
            },
        );
    }

    group.finish();
}

// ==================================================================================
// Benchmark 2: Loop Unrolling with Different Bounds
// ==================================================================================

fn bench_loop_unrolling_bounds(c: &mut Criterion) {
    let mut group = c.benchmark_group("loop_unrolling_bounds");

    let cfg = create_loop_cfg(10);

    for bound in [1, 2, 4, 8, 16] {
        group.bench_with_input(BenchmarkId::from_parameter(bound), &bound, |b, &bound| {
            let config = UnrollConfig::with_bound(bound);
            b.iter(|| {
                let mut unroller = LoopUnroller::with_config(config.clone());
                let loops = unroller.detect_loops(black_box(&cfg));
                if !loops.is_empty() {
                    let unrolled = unroller.unroll_loop(&loops[0], black_box(&cfg));
                    black_box(unrolled);
                }
            });
        });
    }

    group.finish();
}

// ==================================================================================
// Benchmark 3: Loop Unrolling with Different Body Sizes
// ==================================================================================

fn bench_loop_unrolling_body_sizes(c: &mut Criterion) {
    let mut group = c.benchmark_group("loop_unrolling_body_sizes");

    let config = UnrollConfig::default();

    for body_size in [1, 5, 10, 20, 50] {
        group.bench_with_input(
            BenchmarkId::from_parameter(body_size),
            &body_size,
            |b, &size| {
                let cfg = create_loop_cfg(size);
                b.iter(|| {
                    let mut unroller = LoopUnroller::with_config(config.clone());
                    let loops = unroller.detect_loops(black_box(&cfg));
                    if !loops.is_empty() {
                        let unrolled = unroller.unroll_loop(&loops[0], black_box(&cfg));
                        black_box(unrolled);
                    }
                });
            },
        );
    }

    group.finish();
}

// ==================================================================================
// Benchmark 4: Escape Analysis with Unrolling
// ==================================================================================

fn bench_escape_analysis_with_unrolling(c: &mut Criterion) {
    let mut group = c.benchmark_group("escape_analysis_with_unrolling");

    for body_size in [1, 5, 10, 20] {
        group.bench_with_input(
            BenchmarkId::from_parameter(body_size),
            &body_size,
            |b, &size| {
                let cfg = create_loop_cfg(size);
                let analyzer = EscapeAnalyzer::new(cfg);
                let config = UnrollConfig::default();
                b.iter(|| {
                    let result = analyzer
                        .analyze_with_unrolling(black_box(RefId(1)), black_box(config.clone()));
                    black_box(result);
                });
            },
        );
    }

    group.finish();
}

// ==================================================================================
// Benchmark 5: Loop Invariant Detection
// ==================================================================================

fn bench_loop_invariant_detection(c: &mut Criterion) {
    let mut group = c.benchmark_group("loop_invariant_detection");

    for body_size in [1, 5, 10, 20, 50] {
        group.bench_with_input(
            BenchmarkId::from_parameter(body_size),
            &body_size,
            |b, &size| {
                let cfg = create_loop_cfg(size);
                let analyzer = EscapeAnalyzer::new(cfg);
                let config = UnrollConfig::default();
                b.iter(|| {
                    let invariants = analyzer.detect_loop_invariants(black_box(config.clone()));
                    black_box(invariants);
                });
            },
        );
    }

    group.finish();
}

// ==================================================================================
// Benchmark 6: Nested Loop Handling
// ==================================================================================

fn bench_nested_loops(c: &mut Criterion) {
    let mut group = c.benchmark_group("nested_loops");

    for depth in [1, 2, 3] {
        group.bench_with_input(BenchmarkId::from_parameter(depth), &depth, |b, &depth| {
            let cfg = create_nested_loop_cfg(depth);
            b.iter(|| {
                let mut unroller = LoopUnroller::new();
                let loops = unroller.detect_loops(black_box(&cfg));
                for loop_info in loops {
                    let unrolled = unroller.unroll_loop(&loop_info, black_box(&cfg));
                    black_box(unrolled);
                }
            });
        });
    }

    group.finish();
}

// ==================================================================================
// Benchmark 7: Configuration Overhead
// ==================================================================================

fn bench_config_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("config_overhead");

    let configs = [
        ("default", UnrollConfig::default()),
        ("aggressive", UnrollConfig::aggressive()),
        ("conservative", UnrollConfig::conservative()),
    ];

    let cfg = create_loop_cfg(10);

    for (name, config) in configs.iter() {
        group.bench_with_input(BenchmarkId::from_parameter(name), config, |b, config| {
            b.iter(|| {
                let analyzer = EscapeAnalyzer::new(cfg.clone());
                let result = analyzer.loop_unrolling_stats(black_box(config.clone()));
                black_box(result);
            });
        });
    }

    group.finish();
}

// ==================================================================================
// Benchmark 8: Complete Analysis Pipeline
// ==================================================================================

fn bench_complete_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("complete_pipeline");

    for body_size in [1, 5, 10, 20] {
        group.bench_with_input(
            BenchmarkId::from_parameter(body_size),
            &body_size,
            |b, &size| {
                let cfg = create_loop_cfg(size);
                let config = UnrollConfig::default();
                b.iter(|| {
                    let analyzer = EscapeAnalyzer::new(cfg.clone());

                    // Unroll loops
                    let unrolled = analyzer.unroll_loops(black_box(config.clone()));

                    // Analyze each reference
                    for i in 1..=size {
                        let result = analyzer.analyze_with_unrolling(
                            black_box(RefId(i as u64)),
                            black_box(config.clone()),
                        );
                        black_box(result);
                    }

                    // Detect invariants
                    let invariants = analyzer.detect_loop_invariants(black_box(config.clone()));
                    black_box(invariants);

                    black_box(unrolled);
                });
            },
        );
    }

    group.finish();
}

// ==================================================================================
// Criterion Configuration
// ==================================================================================

criterion_group!(
    benches,
    bench_loop_detection,
    bench_loop_unrolling_bounds,
    bench_loop_unrolling_body_sizes,
    bench_escape_analysis_with_unrolling,
    bench_loop_invariant_detection,
    bench_nested_loops,
    bench_config_overhead,
    bench_complete_pipeline,
);

criterion_main!(benches);
