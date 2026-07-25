//! Benchmarks for array index analysis performance
//!
//! Measures performance of key operations:
//! - Index extraction from CFG
//! - Range analysis computation
//! - Aliasing queries (constant, symbolic, complex)
//! - Integration overhead
//! - Z3 solver usage (when applicable)

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use verum_cbgr::analysis::{BasicBlock, BlockId, ControlFlowGraph, FieldPath, RefId, UseeSite};
use verum_cbgr::array_analysis::{
    ArrayAccess, ArrayIndexAnalyzer, BinOp, IndexRange, InductionVariable, SymbolicIndex, VarId,
};
use verum_common::{List, Maybe, Set, Text};

// ==================================================================================
// Benchmark 1: Index Extraction
// ==================================================================================

fn bench_index_extraction(c: &mut Criterion) {
    let mut group = c.benchmark_group("index_extraction");

    // Constant index
    group.bench_function("constant", |b| {
        b.iter(|| black_box(SymbolicIndex::Constant(42)));
    });

    // Variable index
    group.bench_function("variable", |b| {
        b.iter(|| black_box(SymbolicIndex::Variable(VarId(0))));
    });

    // Binary operation (i+1)
    group.bench_function("binary_op_simple", |b| {
        b.iter(|| {
            black_box(SymbolicIndex::BinaryOp(
                BinOp::Add,
                Box::new(SymbolicIndex::Variable(VarId(0))),
                Box::new(SymbolicIndex::Constant(1)),
            ))
        });
    });

    // Complex binary operation ((i*2)+1)
    group.bench_function("binary_op_complex", |b| {
        b.iter(|| {
            let mul = SymbolicIndex::BinaryOp(
                BinOp::Mul,
                Box::new(SymbolicIndex::Variable(VarId(0))),
                Box::new(SymbolicIndex::Constant(2)),
            );
            black_box(SymbolicIndex::BinaryOp(
                BinOp::Add,
                Box::new(mul),
                Box::new(SymbolicIndex::Constant(1)),
            ))
        });
    });

    group.finish();
}

// ==================================================================================
// Benchmark 2: Range Analysis
// ==================================================================================

fn bench_range_analysis(c: &mut Criterion) {
    let mut group = c.benchmark_group("range_analysis");

    // Constant range
    group.bench_function("constant_range", |b| {
        b.iter(|| black_box(IndexRange::from_constant(42)));
    });

    // Bounds range
    group.bench_function("bounds_range", |b| {
        b.iter(|| black_box(IndexRange::from_bounds(0, 100)));
    });

    // Range intersection
    group.bench_function("intersection", |b| {
        let r1 = IndexRange::from_bounds(0, 50);
        let r2 = IndexRange::from_bounds(25, 75);
        b.iter(|| black_box(r1.intersect(&r2)));
    });

    // Range overlap check
    group.bench_function("may_overlap", |b| {
        let r1 = IndexRange::from_bounds(0, 50);
        let r2 = IndexRange::from_bounds(25, 75);
        b.iter(|| black_box(r1.may_overlap(&r2)));
    });

    // Induction variable range
    group.bench_function("induction_var_range", |b| {
        let var = InductionVariable::new(VarId(0), 0, 1, 100);
        b.iter(|| black_box(var.range()));
    });

    group.finish();
}

// ==================================================================================
// Benchmark 3: Aliasing Queries (Constant)
// ==================================================================================

fn bench_aliasing_constant(c: &mut Criterion) {
    let mut group = c.benchmark_group("aliasing_constant");

    let analyzer = ArrayIndexAnalyzer::new();

    // Same index
    group.bench_function("same_index", |b| {
        let access1 = ArrayAccess::new(RefId(1), SymbolicIndex::Constant(0), Maybe::None);
        let access2 = ArrayAccess::new(RefId(1), SymbolicIndex::Constant(0), Maybe::None);
        b.iter(|| black_box(analyzer.may_alias(&access1, &access2)));
    });

    // Different index
    group.bench_function("different_index", |b| {
        let access1 = ArrayAccess::new(RefId(1), SymbolicIndex::Constant(0), Maybe::None);
        let access2 = ArrayAccess::new(RefId(1), SymbolicIndex::Constant(1), Maybe::None);
        b.iter(|| black_box(analyzer.may_alias(&access1, &access2)));
    });

    // Different base
    group.bench_function("different_base", |b| {
        let access1 = ArrayAccess::new(RefId(1), SymbolicIndex::Constant(0), Maybe::None);
        let access2 = ArrayAccess::new(RefId(2), SymbolicIndex::Constant(0), Maybe::None);
        b.iter(|| black_box(analyzer.may_alias(&access1, &access2)));
    });

    group.finish();
}

// ==================================================================================
// Benchmark 4: Aliasing Queries (Symbolic)
// ==================================================================================

fn bench_aliasing_symbolic(c: &mut Criterion) {
    let mut group = c.benchmark_group("aliasing_symbolic");

    let analyzer = ArrayIndexAnalyzer::new();

    // Same variable
    group.bench_function("same_variable", |b| {
        let idx = SymbolicIndex::Variable(VarId(0));
        let access1 = ArrayAccess::new(RefId(1), idx.clone(), Maybe::None);
        let access2 = ArrayAccess::new(RefId(1), idx.clone(), Maybe::None);
        b.iter(|| black_box(analyzer.may_alias(&access1, &access2)));
    });

    // Different variables
    group.bench_function("different_variables", |b| {
        let idx1 = SymbolicIndex::Variable(VarId(0));
        let idx2 = SymbolicIndex::Variable(VarId(1));
        let access1 = ArrayAccess::new(RefId(1), idx1, Maybe::None);
        let access2 = ArrayAccess::new(RefId(1), idx2, Maybe::None);
        b.iter(|| black_box(analyzer.may_alias(&access1, &access2)));
    });

    // Variable with offset
    group.bench_function("variable_offset", |b| {
        let var = SymbolicIndex::Variable(VarId(0));
        let var_plus_1 = SymbolicIndex::BinaryOp(
            BinOp::Add,
            Box::new(var.clone()),
            Box::new(SymbolicIndex::Constant(1)),
        );
        let access1 = ArrayAccess::new(RefId(1), var, Maybe::None);
        let access2 = ArrayAccess::new(RefId(1), var_plus_1, Maybe::None);
        b.iter(|| black_box(analyzer.may_alias(&access1, &access2)));
    });

    group.finish();
}

// ==================================================================================
// Benchmark 5: Aliasing Queries (Complex)
// ==================================================================================

fn bench_aliasing_complex(c: &mut Criterion) {
    let mut group = c.benchmark_group("aliasing_complex");

    let mut analyzer = ArrayIndexAnalyzer::new();

    // With bounds checking
    group.bench_function("with_bounds", |b| {
        let access1 = ArrayAccess::new(
            RefId(1),
            SymbolicIndex::Variable(VarId(0)),
            Maybe::Some((0, 10)),
        );
        let access2 = ArrayAccess::new(RefId(1), SymbolicIndex::Constant(20), Maybe::None);
        b.iter(|| black_box(analyzer.may_alias(&access1, &access2)));
    });

    // With induction variable
    group.bench_function("with_induction_var", |b| {
        analyzer.add_induction_var(InductionVariable::new(VarId(0), 0, 1, 10));
        let access1 = ArrayAccess::new(
            RefId(1),
            SymbolicIndex::Variable(VarId(0)),
            Maybe::Some((0, 9)),
        );
        let access2 = ArrayAccess::new(RefId(1), SymbolicIndex::Constant(20), Maybe::None);
        b.iter(|| black_box(analyzer.may_alias(&access1, &access2)));
    });

    // Complex expression
    group.bench_function("complex_expression", |b| {
        let expr1 = SymbolicIndex::BinaryOp(
            BinOp::Add,
            Box::new(SymbolicIndex::BinaryOp(
                BinOp::Mul,
                Box::new(SymbolicIndex::Variable(VarId(0))),
                Box::new(SymbolicIndex::Constant(2)),
            )),
            Box::new(SymbolicIndex::Constant(1)),
        );
        let expr2 = SymbolicIndex::Variable(VarId(1));
        let access1 = ArrayAccess::new(RefId(1), expr1, Maybe::None);
        let access2 = ArrayAccess::new(RefId(1), expr2, Maybe::None);
        b.iter(|| black_box(analyzer.may_alias(&access1, &access2)));
    });

    group.finish();
}

// ==================================================================================
// Benchmark 6: Simplification
// ==================================================================================

fn bench_simplification(c: &mut Criterion) {
    let mut group = c.benchmark_group("simplification");

    // Identity: i + 0
    group.bench_function("identity_add", |b| {
        let expr = SymbolicIndex::BinaryOp(
            BinOp::Add,
            Box::new(SymbolicIndex::Variable(VarId(0))),
            Box::new(SymbolicIndex::Constant(0)),
        );
        b.iter(|| black_box(expr.simplify()));
    });

    // Constant folding: 2 + 3
    group.bench_function("constant_folding", |b| {
        let expr = SymbolicIndex::BinaryOp(
            BinOp::Add,
            Box::new(SymbolicIndex::Constant(2)),
            Box::new(SymbolicIndex::Constant(3)),
        );
        b.iter(|| black_box(expr.simplify()));
    });

    // Nested: (i + 0) * 1
    group.bench_function("nested_simplification", |b| {
        let inner = SymbolicIndex::BinaryOp(
            BinOp::Add,
            Box::new(SymbolicIndex::Variable(VarId(0))),
            Box::new(SymbolicIndex::Constant(0)),
        );
        let expr = SymbolicIndex::BinaryOp(
            BinOp::Mul,
            Box::new(inner),
            Box::new(SymbolicIndex::Constant(1)),
        );
        b.iter(|| black_box(expr.simplify()));
    });

    group.finish();
}

// ==================================================================================
// Benchmark 7: CFG Extraction
// ==================================================================================

fn bench_cfg_extraction(c: &mut Criterion) {
    let mut group = c.benchmark_group("cfg_extraction");
    group.throughput(Throughput::Elements(1));

    for size in [10, 50, 100, 500].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let cfg = create_cfg_with_array_accesses(size);
            b.iter(|| {
                let mut analyzer = ArrayIndexAnalyzer::new();
                black_box(analyzer.extract_array_accesses(&cfg))
            });
        });
    }

    group.finish();
}

fn create_cfg_with_array_accesses(num_accesses: usize) -> ControlFlowGraph {
    let mut cfg = ControlFlowGraph::new(BlockId(0), BlockId(1));

    let uses: List<UseeSite> = (0..num_accesses)
        .map(|i| UseeSite {
            block: BlockId(0),
            reference: RefId(i as u64),
            is_mutable: false,
            span: None,
        })
        .collect();

    let block = BasicBlock {
        id: BlockId(0),
        predecessors: Set::new(),
        successors: Set::new(),
        definitions: List::new(),
        uses,
        call_sites: List::new(),
        has_await_point: false,
        is_exception_handler: false,
        is_cleanup_handler: false,
        may_throw: false,
    };

    cfg.add_block(block);
    cfg
}

// ==================================================================================
// Benchmark 8: Integration Overhead
// ==================================================================================

fn bench_integration_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("integration_overhead");

    // Field path with array index
    group.bench_function("field_path_array_index", |b| {
        let base = FieldPath::named(Text::from("data"));
        b.iter(|| black_box(base.with_array_index(SymbolicIndex::Constant(0))));
    });

    // Field path aliasing check
    group.bench_function("field_path_aliasing", |b| {
        let path1 =
            FieldPath::named(Text::from("arr")).with_array_index(SymbolicIndex::Constant(0));
        let path2 =
            FieldPath::named(Text::from("arr")).with_array_index(SymbolicIndex::Constant(1));
        b.iter(|| black_box(path1.may_alias_with_array(&path2)));
    });

    group.finish();
}

// ==================================================================================
// Benchmark 9: Statistics Generation
// ==================================================================================

fn bench_statistics(c: &mut Criterion) {
    let mut group = c.benchmark_group("statistics");

    for size in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let mut analyzer = ArrayIndexAnalyzer::new();
            let mut accesses = Vec::new();
            for i in 0..size {
                accesses.push(ArrayAccess::new(
                    RefId(1),
                    SymbolicIndex::Constant(i as i64),
                    Maybe::None,
                ));
            }
            analyzer.insert_accesses(RefId(1), accesses);

            b.iter(|| black_box(analyzer.statistics()));
        });
    }

    group.finish();
}

// ==================================================================================
// Benchmark 10: May Equal Checks
// ==================================================================================

fn bench_may_equal(c: &mut Criterion) {
    let mut group = c.benchmark_group("may_equal");

    // Constant vs Constant
    group.bench_function("constant_vs_constant", |b| {
        let idx1 = SymbolicIndex::Constant(5);
        let idx2 = SymbolicIndex::Constant(10);
        b.iter(|| black_box(idx1.may_equal(&idx2)));
    });

    // Variable vs Variable
    group.bench_function("variable_vs_variable", |b| {
        let idx1 = SymbolicIndex::Variable(VarId(0));
        let idx2 = SymbolicIndex::Variable(VarId(1));
        b.iter(|| black_box(idx1.may_equal(&idx2)));
    });

    // Complex vs Complex
    group.bench_function("complex_vs_complex", |b| {
        let idx1 = SymbolicIndex::BinaryOp(
            BinOp::Add,
            Box::new(SymbolicIndex::Variable(VarId(0))),
            Box::new(SymbolicIndex::Constant(1)),
        );
        let idx2 = SymbolicIndex::BinaryOp(
            BinOp::Mul,
            Box::new(SymbolicIndex::Variable(VarId(1))),
            Box::new(SymbolicIndex::Constant(2)),
        );
        b.iter(|| black_box(idx1.may_equal(&idx2)));
    });

    group.finish();
}

// ==================================================================================
// Benchmark 11: Batch Aliasing Queries
// ==================================================================================

fn bench_batch_aliasing(c: &mut Criterion) {
    let mut group = c.benchmark_group("batch_aliasing");
    group.throughput(Throughput::Elements(1));

    for num_accesses in [10, 50, 100].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(num_accesses),
            num_accesses,
            |b, &num_accesses| {
                let analyzer = ArrayIndexAnalyzer::new();
                let accesses: Vec<_> = (0..num_accesses)
                    .map(|i| {
                        ArrayAccess::new(RefId(1), SymbolicIndex::Constant(i as i64), Maybe::None)
                    })
                    .collect();

                b.iter(|| {
                    let mut count = 0;
                    for i in 0..accesses.len() {
                        for j in i + 1..accesses.len() {
                            if analyzer.may_alias(&accesses[i], &accesses[j]) {
                                count += 1;
                            }
                        }
                    }
                    black_box(count)
                });
            },
        );
    }

    group.finish();
}

// ==================================================================================
// Benchmark 12: Range Inference
// ==================================================================================

fn bench_range_inference(c: &mut Criterion) {
    let mut group = c.benchmark_group("range_inference");

    let analyzer = ArrayIndexAnalyzer::new();
    let cfg = ControlFlowGraph::new(BlockId(0), BlockId(1));

    // Constant
    group.bench_function("constant", |b| {
        let idx = SymbolicIndex::Constant(42);
        b.iter(|| black_box(analyzer.infer_range(&idx, BlockId(0), &cfg)));
    });

    // Variable (no induction)
    group.bench_function("variable_no_induction", |b| {
        let idx = SymbolicIndex::Variable(VarId(0));
        b.iter(|| black_box(analyzer.infer_range(&idx, BlockId(0), &cfg)));
    });

    // Binary operation
    group.bench_function("binary_op", |b| {
        let idx = SymbolicIndex::BinaryOp(
            BinOp::Add,
            Box::new(SymbolicIndex::Constant(5)),
            Box::new(SymbolicIndex::Constant(10)),
        );
        b.iter(|| black_box(analyzer.infer_range(&idx, BlockId(0), &cfg)));
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_index_extraction,
    bench_range_analysis,
    bench_aliasing_constant,
    bench_aliasing_symbolic,
    bench_aliasing_complex,
    bench_simplification,
    bench_cfg_extraction,
    bench_integration_overhead,
    bench_statistics,
    bench_may_equal,
    bench_batch_aliasing,
    bench_range_inference,
);

criterion_main!(benches);
