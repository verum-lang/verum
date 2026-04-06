//! Benchmarks for IR-based call site extraction
//!
//! Validates O(instructions) linear performance target

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use verum_cbgr::analysis::{BlockId, FunctionId, RefId};
use verum_cbgr::ir_call_extraction::{IrCallExtractor, IrFunction, IrInstruction, IrOperand};
use verum_common::Maybe;

// ==================================================================================
// Helper Functions
// ==================================================================================

/// Create a function with N call instructions
fn create_function_with_calls(num_calls: usize) -> IrFunction {
    let mut func = IrFunction::new(FunctionId(1), "benchmark_func");
    func.add_parameter(0, "data");

    for i in 0..num_calls {
        func.add_instruction(
            BlockId(0),
            i,
            IrInstruction::Call {
                target: format!("func_{}", i).into(),
                args: vec![IrOperand::Argument(0)],
                result: Maybe::Some((i + 1) as u32),
            },
        );
    }

    func
}

/// Create a function with N call instructions that pass references
fn create_function_with_ref_calls(num_calls: usize) -> IrFunction {
    let mut func = IrFunction::new(FunctionId(1), "ref_benchmark");
    func.add_parameter(0, "data");

    for i in 0..num_calls {
        func.add_instruction(
            BlockId(0),
            i,
            IrInstruction::Call {
                target: format!("func_{}", i).into(),
                args: vec![IrOperand::Reference(RefId(i as u64))],
                result: Maybe::Some((i + 1) as u32),
            },
        );
    }

    func
}

/// Create a complex function with mixed instruction types
fn create_complex_function(num_instructions: usize) -> IrFunction {
    let mut func = IrFunction::new(FunctionId(1), "complex_func");
    func.add_parameter(0, "input");

    let calls_per_block = num_instructions / 4;

    for i in 0..calls_per_block {
        // Call instruction
        func.add_instruction(
            BlockId(0),
            i * 4,
            IrInstruction::Call {
                target: format!("func_{}", i).into(),
                args: vec![IrOperand::Reference(RefId(i as u64))],
                result: Maybe::Some((i + 1) as u32),
            },
        );

        // Assign instruction
        func.add_instruction(
            BlockId(0),
            i * 4 + 1,
            IrInstruction::Assign {
                dest: (i + 100) as u32,
                src: IrOperand::LocalVar(i as u32),
            },
        );

        // Load instruction
        func.add_instruction(
            BlockId(0),
            i * 4 + 2,
            IrInstruction::Load {
                result: (i + 200) as u32,
                ptr: IrOperand::LocalVar(i as u32),
            },
        );

        // Store instruction
        func.add_instruction(
            BlockId(0),
            i * 4 + 3,
            IrInstruction::Store {
                ptr: IrOperand::LocalVar((i + 100) as u32),
                value: IrOperand::LocalVar((i + 200) as u32),
            },
        );
    }

    func
}

/// Create multi-block function
fn create_multi_block_function(blocks: usize, calls_per_block: usize) -> IrFunction {
    let mut func = IrFunction::new(FunctionId(1), "multi_block");

    for block_idx in 0..blocks {
        for call_idx in 0..calls_per_block {
            func.add_instruction(
                BlockId(block_idx as u64),
                call_idx,
                IrInstruction::Call {
                    target: format!("func_{}_{}", block_idx, call_idx).into(),
                    args: vec![IrOperand::Reference(RefId(
                        (block_idx * calls_per_block + call_idx) as u64,
                    ))],
                    result: Maybe::Some((block_idx * calls_per_block + call_idx + 1) as u32),
                },
            );
        }
    }

    func
}

// ==================================================================================
// Benchmark 1: Basic Call Extraction Scalability
// ==================================================================================

fn bench_call_extraction_scalability(c: &mut Criterion) {
    let mut group = c.benchmark_group("call_extraction_scalability");

    for num_calls in [10, 50, 100, 500, 1000].iter() {
        group.throughput(Throughput::Elements(*num_calls as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(num_calls),
            num_calls,
            |b, &num_calls| {
                let func = create_function_with_calls(num_calls);
                let extractor = IrCallExtractor::new();

                b.iter(|| {
                    let sites = extractor.extract_from_function(black_box(&func));
                    black_box(sites);
                });
            },
        );
    }

    group.finish();
}

// ==================================================================================
// Benchmark 2: Reference Filtering Performance
// ==================================================================================

fn bench_reference_filtering(c: &mut Criterion) {
    let mut group = c.benchmark_group("reference_filtering");

    for num_calls in [100, 500, 1000].iter() {
        group.throughput(Throughput::Elements(*num_calls as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(num_calls),
            num_calls,
            |b, &num_calls| {
                let func = create_function_with_ref_calls(num_calls);
                let extractor = IrCallExtractor::new();
                let target_ref = RefId(42);

                b.iter(|| {
                    let sites = extractor
                        .extract_calls_with_reference(black_box(&func), black_box(target_ref));
                    black_box(sites);
                });
            },
        );
    }

    group.finish();
}

// ==================================================================================
// Benchmark 3: Call Info Extraction (with classification)
// ==================================================================================

fn bench_call_info_extraction(c: &mut Criterion) {
    let mut group = c.benchmark_group("call_info_extraction");

    for num_calls in [50, 200, 500].iter() {
        group.throughput(Throughput::Elements(*num_calls as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(num_calls),
            num_calls,
            |b, &num_calls| {
                let func = create_function_with_ref_calls(num_calls);
                let extractor = IrCallExtractor::new();

                b.iter(|| {
                    let infos = extractor.extract_with_info(black_box(&func));
                    black_box(infos);
                });
            },
        );
    }

    group.finish();
}

// ==================================================================================
// Benchmark 4: Complex Function Analysis
// ==================================================================================

fn bench_complex_function_analysis(c: &mut Criterion) {
    let mut group = c.benchmark_group("complex_function_analysis");

    for num_instructions in [100, 500, 1000, 2000].iter() {
        group.throughput(Throughput::Elements(*num_instructions as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(num_instructions),
            num_instructions,
            |b, &num_instructions| {
                let func = create_complex_function(num_instructions);
                let extractor = IrCallExtractor::new();

                b.iter(|| {
                    let sites = extractor.extract_from_function(black_box(&func));
                    let infos = extractor.extract_with_info(black_box(&func));
                    let returns = extractor.extract_return_sites(black_box(&func));
                    black_box((sites, infos, returns));
                });
            },
        );
    }

    group.finish();
}

// ==================================================================================
// Benchmark 5: Multi-Block Function Performance
// ==================================================================================

fn bench_multi_block_extraction(c: &mut Criterion) {
    let mut group = c.benchmark_group("multi_block_extraction");

    for blocks in [5, 10, 20].iter() {
        let calls_per_block = 10;
        let total_calls = blocks * calls_per_block;

        group.throughput(Throughput::Elements(total_calls as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}blocks", blocks)),
            blocks,
            |b, &blocks| {
                let func = create_multi_block_function(blocks, calls_per_block);
                let extractor = IrCallExtractor::new();

                b.iter(|| {
                    let sites = extractor.extract_from_function(black_box(&func));
                    black_box(sites);
                });
            },
        );
    }

    group.finish();
}

// ==================================================================================
// Benchmark 6: Return Site Extraction
// ==================================================================================

fn bench_return_site_extraction(c: &mut Criterion) {
    let mut group = c.benchmark_group("return_site_extraction");

    for num_blocks in [10, 50, 100].iter() {
        group.throughput(Throughput::Elements(*num_blocks as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(num_blocks),
            num_blocks,
            |b, &num_blocks| {
                let mut func = IrFunction::new(FunctionId(1), "returns");

                // Add return in each block
                for i in 0..num_blocks {
                    func.add_instruction(
                        BlockId(i as u64),
                        0,
                        IrInstruction::Return {
                            value: if i % 2 == 0 {
                                Maybe::Some(IrOperand::Reference(RefId(i as u64)))
                            } else {
                                Maybe::None
                            },
                        },
                    );
                }

                let extractor = IrCallExtractor::new();

                b.iter(|| {
                    let returns = extractor.extract_return_sites(black_box(&func));
                    black_box(returns);
                });
            },
        );
    }

    group.finish();
}

// ==================================================================================
// Benchmark 7: Flows-to-Return Analysis
// ==================================================================================

fn bench_flows_to_return_analysis(c: &mut Criterion) {
    let mut group = c.benchmark_group("flows_to_return_analysis");

    for num_blocks in [10, 50, 100].iter() {
        group.throughput(Throughput::Elements(*num_blocks as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(num_blocks),
            num_blocks,
            |b, &num_blocks| {
                let mut func = IrFunction::new(FunctionId(1), "flows");

                // Add returns with various references
                for i in 0..num_blocks {
                    func.add_instruction(
                        BlockId(i as u64),
                        0,
                        IrInstruction::Return {
                            value: Maybe::Some(IrOperand::Reference(RefId(i as u64))),
                        },
                    );
                }

                let extractor = IrCallExtractor::new();
                let target_ref = RefId(42);

                b.iter(|| {
                    let flows = extractor.flows_to_return(black_box(&func), black_box(target_ref));
                    black_box(flows);
                });
            },
        );
    }

    group.finish();
}

// ==================================================================================
// Benchmark 8: End-to-End Extraction Statistics
// ==================================================================================

fn bench_extraction_statistics(c: &mut Criterion) {
    let mut group = c.benchmark_group("extraction_statistics");

    for num_calls in [100, 500, 1000].iter() {
        group.throughput(Throughput::Elements(*num_calls as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(num_calls),
            num_calls,
            |b, &num_calls| {
                let func = create_function_with_ref_calls(num_calls);
                let extractor = IrCallExtractor::new();

                b.iter(|| {
                    let call_infos = extractor.extract_with_info(black_box(&func));
                    let returns = extractor.extract_return_sites(black_box(&func));
                    let stats = verum_cbgr::ir_call_extraction::ExtractionStats::from_extraction(
                        black_box(&func),
                        black_box(&call_infos),
                        black_box(returns.len()),
                    );
                    black_box(stats);
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
    bench_call_extraction_scalability,
    bench_reference_filtering,
    bench_call_info_extraction,
    bench_complex_function_analysis,
    bench_multi_block_extraction,
    bench_return_site_extraction,
    bench_flows_to_return_analysis,
    bench_extraction_statistics,
);

criterion_main!(benches);
