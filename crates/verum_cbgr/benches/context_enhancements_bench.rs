//! Benchmarks for context-sensitive analysis enhancements
//!
//! Tests performance of:
//! 1. Flow-sensitive context tracking
//! 2. Adaptive context depth
//! 3. Context compression
//!
//! Performance target: 2-3x speedup vs fixed depth

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use verum_cbgr::analysis::{BlockId, FunctionId, RefId};
use verum_cbgr::call_graph::{CallGraph, FunctionSignature, RefFlow};
use verum_cbgr::context_enhancements::*;

// ============================================================================
// BENCHMARK 1: Flow-sensitive Context Operations
// ============================================================================

fn bench_dataflow_state_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("dataflow_state");

    group.bench_function("create", |b| {
        b.iter(|| {
            black_box(DataflowState::new(RefId(1), BlockId(0)));
        });
    });

    group.bench_function("with_predicate", |b| {
        b.iter(|| {
            let state = DataflowState::new(RefId(1), BlockId(0));
            black_box(state.with_predicate(Predicate::BlockTrue(BlockId(1))));
        });
    });

    group.bench_function("merge", |b| {
        let state1 = DataflowState::new(RefId(1), BlockId(0))
            .with_alias_state(AliasState::NoAlias)
            .with_predicate(Predicate::BlockTrue(BlockId(1)));

        let state2 = DataflowState::new(RefId(1), BlockId(0))
            .with_alias_state(AliasState::NoAlias)
            .with_predicate(Predicate::BlockTrue(BlockId(1)));

        b.iter(|| {
            black_box(state1.merge(&state2));
        });
    });

    group.bench_function("next_generation", |b| {
        let state = DataflowState::new(RefId(1), BlockId(0));
        b.iter(|| {
            black_box(state.clone().next_generation());
        });
    });

    group.finish();
}

fn bench_flow_sensitive_context_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("flow_sensitive_context");

    group.bench_function("create", |b| {
        b.iter(|| {
            black_box(FlowSensitiveContext::new(FunctionId(1)));
        });
    });

    group.bench_function("extend", |b| {
        let base = FlowSensitiveContext::new(FunctionId(1));
        b.iter(|| {
            black_box(base.extend(FunctionId(2)));
        });
    });

    group.bench_function("update_state", |b| {
        let mut context = FlowSensitiveContext::new(FunctionId(1));
        let state = DataflowState::new(RefId(1), BlockId(0));

        b.iter(|| {
            context.update_state(BlockId(0), state.clone());
        });
    });

    group.bench_function("merge_states", |b| {
        let mut context1 = FlowSensitiveContext::new(FunctionId(1));
        let context2 = FlowSensitiveContext::new(FunctionId(1));

        for i in 0..10 {
            let state = DataflowState::new(RefId(i), BlockId(i));
            context1.update_state(BlockId(i), state.clone());
        }

        for i in 0..10 {
            let state = DataflowState::new(RefId(i), BlockId(i));
            let mut ctx2 = context2.clone();
            ctx2.update_state(BlockId(i), state);
        }

        b.iter(|| {
            let mut ctx = context1.clone();
            ctx.merge_states(&context2);
            black_box(ctx);
        });
    });

    group.finish();
}

// ============================================================================
// BENCHMARK 2: Adaptive Depth Policy
// ============================================================================

fn bench_importance_metrics(c: &mut Criterion) {
    let mut group = c.benchmark_group("importance_metrics");

    group.bench_function("create", |b| {
        b.iter(|| {
            black_box(ImportanceMetrics::new());
        });
    });

    group.bench_function("importance_score", |b| {
        let mut metrics = ImportanceMetrics::new();
        metrics.call_frequency = 0.8;
        metrics.escape_probability = 0.6;
        metrics.code_complexity = 0.7;
        metrics.num_callers = 8;
        metrics.num_references = 15;

        b.iter(|| {
            black_box(metrics.importance_score());
        });
    });

    group.bench_function("depth_limit", |b| {
        let mut metrics = ImportanceMetrics::new();
        metrics.call_frequency = 0.8;

        b.iter(|| {
            black_box(metrics.depth_limit());
        });
    });

    group.finish();
}

fn bench_adaptive_depth_policy(c: &mut Criterion) {
    let mut group = c.benchmark_group("adaptive_depth_policy");

    // Benchmark with varying numbers of functions
    for num_funcs in [10, 50, 100, 500].iter() {
        group.throughput(Throughput::Elements(*num_funcs as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(num_funcs),
            num_funcs,
            |b, &num_funcs| {
                let mut call_graph = CallGraph::new();

                // Build call graph
                for i in 0..num_funcs {
                    let func = FunctionId(i as u64);
                    call_graph.add_function(func, FunctionSignature::new(format!("func{}", i), 0));
                }

                // Add some calls
                for i in 1..num_funcs {
                    call_graph.add_call(FunctionId(i as u64), FunctionId((i / 2) as u64), RefFlow::default());
                }

                b.iter(|| {
                    let mut policy = AdaptiveDepthPolicy::new(3, 10);
                    policy.compute_metrics(&call_graph);
                    black_box(policy);
                });
            },
        );
    }

    group.finish();
}

fn bench_depth_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("depth_lookup");

    let mut policy = AdaptiveDepthPolicy::new(3, 10);

    // Populate policy with many functions
    for i in 0..1000 {
        let mut metrics = ImportanceMetrics::new();
        metrics.call_frequency = (i as f64 / 1000.0).min(1.0);
        policy.set_metrics(FunctionId(i as u64), metrics);
    }

    group.bench_function("cold_lookup", |b| {
        b.iter(|| {
            for i in 0..100 {
                black_box(policy.depth_for_function(FunctionId(i * 10)));
            }
        });
    });

    group.bench_function("hot_lookup", |b| {
        // Warm up cache
        for _ in 0..100 {
            policy.depth_for_function(FunctionId(42));
        }

        b.iter(|| {
            black_box(policy.depth_for_function(FunctionId(42)));
        });
    });

    group.finish();
}

// ============================================================================
// BENCHMARK 3: Context Compression
// ============================================================================

fn bench_context_compression(c: &mut Criterion) {
    let mut group = c.benchmark_group("context_compression");

    // Benchmark with varying numbers of contexts
    for num_contexts in [10, 50, 100, 500].iter() {
        group.throughput(Throughput::Elements(*num_contexts as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(num_contexts),
            num_contexts,
            |b, &num_contexts| {
                let mut contexts = Vec::new();

                // Create contexts (50% identical, 50% unique)
                for i in 0..num_contexts {
                    let func_id = if i < num_contexts / 2 {
                        FunctionId(1) // Identical
                    } else {
                        FunctionId(i as u64) // Unique
                    };
                    contexts.push(FlowSensitiveContext::new(func_id));
                }

                b.iter(|| {
                    let mut compressor = ContextCompressor::new();
                    black_box(compressor.compress(contexts.clone()));
                });
            },
        );
    }

    group.finish();
}

fn bench_abstract_context_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("abstract_context");

    group.bench_function("create", |b| {
        b.iter(|| {
            black_box(AbstractContext::new(FunctionId(1)));
        });
    });

    group.bench_function("from_concrete", |b| {
        let concrete = FlowSensitiveContext::new(FunctionId(1));

        b.iter(|| {
            black_box(AbstractContext::from_concrete(&concrete));
        });
    });

    group.bench_function("is_mergeable", |b| {
        let ctx1 = AbstractContext::new(FunctionId(1));
        let ctx2 = AbstractContext::new(FunctionId(1));

        b.iter(|| {
            black_box(ctx1.is_mergeable_with(&ctx2));
        });
    });

    group.finish();
}

fn bench_call_pattern_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("call_pattern");

    group.bench_function("from_chain_entry", |b| {
        let chain = vec![];
        b.iter(|| {
            black_box(CallPattern::from_chain(&chain));
        });
    });

    group.bench_function("from_chain_direct", |b| {
        let chain = vec![FunctionId(1), FunctionId(2)];
        b.iter(|| {
            black_box(CallPattern::from_chain(&chain));
        });
    });

    group.bench_function("from_chain_recursive", |b| {
        let chain = vec![FunctionId(1), FunctionId(1)];
        b.iter(|| {
            black_box(CallPattern::from_chain(&chain));
        });
    });

    group.bench_function("from_chain_multiple", |b| {
        let chain = vec![FunctionId(1), FunctionId(2), FunctionId(3), FunctionId(4)];
        b.iter(|| {
            black_box(CallPattern::from_chain(&chain));
        });
    });

    group.finish();
}

// ============================================================================
// BENCHMARK 4: End-to-End Comparison
// ============================================================================

fn bench_end_to_end_analysis(c: &mut Criterion) {
    let mut group = c.benchmark_group("end_to_end");

    // Benchmark with varying call graph sizes
    for num_funcs in [10, 20, 50].iter() {
        group.throughput(Throughput::Elements(*num_funcs as u64));

        group.bench_with_input(
            BenchmarkId::new("baseline_fixed_depth", num_funcs),
            num_funcs,
            |b, &num_funcs| {
                let call_graph = build_test_call_graph(num_funcs);

                b.iter(|| {
                    // Fixed depth 3
                    let contexts = build_flow_sensitive_contexts(FunctionId(0), &call_graph, 3);
                    black_box(contexts);
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("adaptive_depth", num_funcs),
            num_funcs,
            |b, &num_funcs| {
                let call_graph = build_test_call_graph(num_funcs);
                let mut policy = AdaptiveDepthPolicy::new(3, 10);
                policy.compute_metrics(&call_graph);

                b.iter(|| {
                    let depth = policy.depth_for_function(FunctionId(0));
                    let contexts = build_flow_sensitive_contexts(FunctionId(0), &call_graph, depth);
                    black_box(contexts);
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("with_compression", num_funcs),
            num_funcs,
            |b, &num_funcs| {
                let call_graph = build_test_call_graph(num_funcs);
                let mut policy = AdaptiveDepthPolicy::new(3, 10);
                policy.compute_metrics(&call_graph);

                b.iter(|| {
                    let depth = policy.depth_for_function(FunctionId(0));
                    let contexts = build_flow_sensitive_contexts(FunctionId(0), &call_graph, depth);

                    let mut compressor = ContextCompressor::new();
                    let compressed = compressor.compress(contexts);
                    black_box(compressed);
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// BENCHMARK 5: Scalability Tests
// ============================================================================

fn bench_scalability(c: &mut Criterion) {
    let mut group = c.benchmark_group("scalability");

    // Test how well compression scales
    for num_contexts in [100, 500, 1000, 5000].iter() {
        group.throughput(Throughput::Elements(*num_contexts));

        group.bench_with_input(
            BenchmarkId::new("compression_scaling", num_contexts),
            num_contexts,
            |b, &num_contexts| {
                // Generate contexts with 90% duplicates (realistic scenario)
                let mut contexts = Vec::new();
                for i in 0..num_contexts {
                    let func_id = if i < num_contexts * 9 / 10 {
                        FunctionId(i % 10) // 90% duplicates
                    } else {
                        FunctionId(i) // 10% unique
                    };
                    contexts.push(FlowSensitiveContext::new(func_id));
                }

                b.iter(|| {
                    let mut compressor = ContextCompressor::new();
                    let compressed = compressor.compress(contexts.clone());
                    black_box(compressed);
                });
            },
        );
    }

    group.finish();
}

fn bench_memory_efficiency(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_efficiency");

    // Measure memory overhead of enhancements
    group.bench_function("dataflow_state_size", |b| {
        b.iter(|| {
            let states: Vec<_> = (0..1000)
                .map(|i| DataflowState::new(RefId(i), BlockId(i)))
                .collect();
            black_box(states);
        });
    });

    group.bench_function("flow_sensitive_context_size", |b| {
        b.iter(|| {
            let contexts: Vec<_> = (0..1000)
                .map(|i| FlowSensitiveContext::new(FunctionId(i as u64)))
                .collect();
            black_box(contexts);
        });
    });

    group.bench_function("importance_metrics_size", |b| {
        b.iter(|| {
            let metrics: Vec<_> = (0..1000).map(|_| ImportanceMetrics::new()).collect();
            black_box(metrics);
        });
    });

    group.finish();
}

// ============================================================================
// Helper Functions
// ============================================================================

fn build_test_call_graph(num_funcs: usize) -> CallGraph {
    let mut call_graph = CallGraph::new();

    // Build a tree-like call graph
    for i in 0..num_funcs {
        let func = FunctionId(i as u64);
        call_graph.add_function(func, FunctionSignature::new(format!("func{}", i), 0));
    }

    // Add parent-child relationships
    for i in 1..num_funcs {
        let parent = FunctionId((i / 2) as u64);
        let child = FunctionId(i as u64);
        call_graph.add_call(parent, child, RefFlow::default());
    }

    call_graph
}

// ============================================================================
// Criterion Configuration
// ============================================================================

// ============================================================================
// BENCHMARK 6: Parallel Analysis
// ============================================================================

fn bench_parallel_config_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("parallel_config");

    group.bench_function("create", |b| {
        b.iter(|| {
            black_box(ParallelConfig::new(10));
        });
    });

    group.bench_function("should_parallelize", |b| {
        let config = ParallelConfig::new(10);
        b.iter(|| {
            black_box(config.should_parallelize(50));
        });
    });

    group.finish();
}

fn bench_parallel_result_accumulator(c: &mut Criterion) {
    let mut group = c.benchmark_group("parallel_accumulator");

    group.bench_function("add_result", |b| {
        let accumulator = ParallelResultAccumulator::new();
        let mut counter = 0;

        b.iter(|| {
            accumulator.add_result(counter, counter * 2);
            counter += 1;
        });
    });

    group.bench_function("add_error", |b| {
        let accumulator: ParallelResultAccumulator<i32> = ParallelResultAccumulator::new();

        b.iter(|| {
            accumulator.add_error();
        });
    });

    group.bench_function("result_count", |b| {
        let accumulator = ParallelResultAccumulator::new();
        for i in 0..100 {
            accumulator.add_result(i, i);
        }

        b.iter(|| {
            black_box(accumulator.result_count());
        });
    });

    group.finish();
}

fn bench_parallel_context_analysis(c: &mut Criterion) {
    let mut group = c.benchmark_group("parallel_analysis");

    // Benchmark with varying numbers of contexts
    for num_contexts in [10, 50, 100, 500, 1000].iter() {
        group.throughput(Throughput::Elements(*num_contexts as u64));

        // Sequential (high threshold to force sequential)
        group.bench_with_input(
            BenchmarkId::new("sequential", num_contexts),
            num_contexts,
            |b, &num_contexts| {
                let analyzer = ParallelContextAnalyzer::with_threshold(10000);
                let contexts: Vec<_> = (0..num_contexts)
                    .map(|i| FlowSensitiveContext::new(FunctionId(i as u64)))
                    .collect();

                b.iter(|| {
                    black_box(analyzer.analyze_parallel(&contexts, |ctx| {
                        // Simulate light computation
                        ctx.function.0 * 2
                    }));
                });
            },
        );

        // Parallel (low threshold to force parallel)
        group.bench_with_input(
            BenchmarkId::new("parallel", num_contexts),
            num_contexts,
            |b, &num_contexts| {
                let analyzer = ParallelContextAnalyzer::with_threshold(5);
                let contexts: Vec<_> = (0..num_contexts)
                    .map(|i| FlowSensitiveContext::new(FunctionId(i as u64)))
                    .collect();

                b.iter(|| {
                    black_box(analyzer.analyze_parallel(&contexts, |ctx| {
                        // Simulate light computation
                        ctx.function.0 * 2
                    }));
                });
            },
        );
    }

    group.finish();
}

fn bench_parallel_vs_sequential_speedup(c: &mut Criterion) {
    let mut group = c.benchmark_group("parallel_speedup");

    // Test with different workload complexities
    for complexity in [1, 10, 100].iter() {
        group.throughput(Throughput::Elements(100));

        group.bench_with_input(
            BenchmarkId::new("sequential_complexity", complexity),
            complexity,
            |b, &complexity| {
                let analyzer = ParallelContextAnalyzer::with_threshold(10000);
                let contexts: Vec<_> = (0..100)
                    .map(|i| {
                        let mut ctx = FlowSensitiveContext::new(FunctionId(i as u64));
                        // Add states based on complexity
                        for j in 0..complexity {
                            let state = DataflowState::new(RefId(j), BlockId(j));
                            ctx.update_state(BlockId(j), state);
                        }
                        ctx
                    })
                    .collect();

                b.iter(|| {
                    black_box(analyzer.analyze_parallel(&contexts, |ctx| {
                        // Count states (simulate work)
                        ctx.dataflow_states.len()
                    }));
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("parallel_complexity", complexity),
            complexity,
            |b, &complexity| {
                let analyzer = ParallelContextAnalyzer::with_threshold(5);
                let contexts: Vec<_> = (0..100)
                    .map(|i| {
                        let mut ctx = FlowSensitiveContext::new(FunctionId(i as u64));
                        for j in 0..complexity {
                            let state = DataflowState::new(RefId(j), BlockId(j));
                            ctx.update_state(BlockId(j), state);
                        }
                        ctx
                    })
                    .collect();

                b.iter(|| {
                    black_box(
                        analyzer.analyze_parallel(&contexts, |ctx| ctx.dataflow_states.len()),
                    );
                });
            },
        );
    }

    group.finish();
}

fn bench_parallel_equivalence_classes(c: &mut Criterion) {
    let mut group = c.benchmark_group("parallel_equivalence_classes");

    for num_classes in [10, 50, 100, 200].iter() {
        group.throughput(Throughput::Elements(*num_classes as u64));

        group.bench_with_input(
            BenchmarkId::new("parallel", num_classes),
            num_classes,
            |b, &num_classes| {
                let analyzer = ParallelContextAnalyzer::with_threshold(5);

                let classes: Vec<_> = (0..num_classes)
                    .map(|i| {
                        let abstract_ctx = AbstractContext::new(FunctionId(i as u64));
                        let mut class = ContextEquivalenceClass::new(abstract_ctx);
                        // Add multiple members
                        for _ in 0..5 {
                            class.add_member(FlowSensitiveContext::new(FunctionId(i as u64)));
                        }
                        class
                    })
                    .collect();

                b.iter(|| {
                    black_box(
                        analyzer.analyze_equivalence_classes(&classes, |cls| cls.members.len()),
                    );
                });
            },
        );
    }

    group.finish();
}

fn bench_context_sensitive_analyzer_parallel(c: &mut Criterion) {
    let mut group = c.benchmark_group("context_sensitive_analyzer_parallel");

    for num_contexts in [20, 50, 100].iter() {
        group.throughput(Throughput::Elements(*num_contexts as u64));

        group.bench_with_input(
            BenchmarkId::new("without_parallel", num_contexts),
            num_contexts,
            |b, &num_contexts| {
                let analyzer = ContextSensitiveAnalyzer::new();
                let contexts: Vec<_> = (0..num_contexts)
                    .map(|i| FlowSensitiveContext::new(FunctionId(i as u64)))
                    .collect();

                b.iter(|| {
                    black_box(analyzer.analyze_contexts_parallel(&contexts, |ctx| ctx.function.0));
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("with_parallel", num_contexts),
            num_contexts,
            |b, &num_contexts| {
                let analyzer = ContextSensitiveAnalyzer::new().with_parallel(10);
                let contexts: Vec<_> = (0..num_contexts)
                    .map(|i| FlowSensitiveContext::new(FunctionId(i as u64)))
                    .collect();

                b.iter(|| {
                    black_box(analyzer.analyze_contexts_parallel(&contexts, |ctx| ctx.function.0));
                });
            },
        );
    }

    group.finish();
}

fn bench_parallel_thread_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("parallel_thread_scaling");

    // Benchmark with different parallelism thresholds
    for threshold in [5, 10, 20, 50, 100].iter() {
        group.throughput(Throughput::Elements(100));

        group.bench_with_input(
            BenchmarkId::new("threshold", threshold),
            threshold,
            |b, &threshold| {
                let analyzer = ParallelContextAnalyzer::with_threshold(threshold);
                let contexts: Vec<_> = (0..100)
                    .map(|i| FlowSensitiveContext::new(FunctionId(i as u64)))
                    .collect();

                b.iter(|| {
                    black_box(analyzer.analyze_parallel(&contexts, |ctx| {
                        // Light work
                        ctx.function.0 + ctx.depth as u64
                    }));
                });
            },
        );
    }

    group.finish();
}

fn bench_parallel_with_all_enhancements(c: &mut Criterion) {
    let mut group = c.benchmark_group("all_enhancements_with_parallel");

    for num_funcs in [20, 50, 100].iter() {
        group.throughput(Throughput::Elements(*num_funcs as u64));

        group.bench_with_input(
            BenchmarkId::new("full_workflow", num_funcs),
            num_funcs,
            |b, &num_funcs| {
                let call_graph = build_test_call_graph(num_funcs);
                let mut policy = AdaptiveDepthPolicy::new(3, 10);
                policy.compute_metrics(&call_graph);

                b.iter(|| {
                    // 1. Build contexts with adaptive depth
                    let depth = policy.depth_for_function(FunctionId(0));
                    let contexts = build_flow_sensitive_contexts(FunctionId(0), &call_graph, depth);

                    // 2. Compress
                    let mut compressor = ContextCompressor::new();
                    let compressed = compressor.compress(contexts);

                    // 3. Analyze in parallel
                    let analyzer = ParallelContextAnalyzer::with_threshold(5);
                    let results =
                        analyzer.analyze_equivalence_classes(&compressed, |cls| cls.members.len());

                    black_box(results);
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_dataflow_state_operations,
    bench_flow_sensitive_context_operations,
    bench_importance_metrics,
    bench_adaptive_depth_policy,
    bench_depth_lookup,
    bench_context_compression,
    bench_abstract_context_operations,
    bench_call_pattern_operations,
    bench_end_to_end_analysis,
    bench_scalability,
    bench_memory_efficiency,
    bench_parallel_config_operations,
    bench_parallel_result_accumulator,
    bench_parallel_context_analysis,
    bench_parallel_vs_sequential_speedup,
    bench_parallel_equivalence_classes,
    bench_context_sensitive_analyzer_parallel,
    bench_parallel_thread_scaling,
    bench_parallel_with_all_enhancements,
);

criterion_main!(benches);
