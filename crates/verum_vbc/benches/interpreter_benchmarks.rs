//! Interpreter dispatch benchmarks for VBC.
//!
//! Benchmarks the function table dispatch performance which provides:
//! - O(1) opcode lookup via array indexing
//! - Better branch prediction for indirect calls
//! - Reduced code size improving instruction cache utilization
//!
//! Run with: cargo bench -p verum_vbc -- interpreter

use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId, Throughput};
use std::hint::black_box;
use std::sync::Arc;

use verum_vbc::bytecode::encode_instructions_with_fixup;
use verum_vbc::instruction::{BinaryIntOp, CompareOp, Instruction, Reg, UnaryIntOp};
use verum_vbc::interpreter::{execute_table, InterpreterState};
use verum_vbc::module::{CallingConvention, FunctionDescriptor, FunctionId, OptimizationHints, VbcModule};
use verum_vbc::types::{PropertySet, TypeId, TypeRef, Visibility};

// ============================================================================
// Test Module Builder
// ============================================================================

/// Creates a simple test module with a counting loop function.
///
/// The function is equivalent to:
/// ```
/// fn count_loop(n: i64) -> i64 {
///     let mut sum = 0;
///     let mut i = 0;
///     while i < n {
///         sum = sum + i;
///         i = i + 1;
///     }
///     return sum;
/// }
/// ```
fn create_loop_module(iterations: i64) -> Arc<VbcModule> {
    let mut module = VbcModule::new("bench_loop".to_string());
    let func_name = module.intern_string("count_loop");

    // Build instruction list for the loop function
    // r0 = n (input)
    // r1 = sum = 0
    // r2 = i = 0
    // r3 = temp for comparison
    // loop (index 3):
    //   r3 = i < n
    //   if !r3 goto end (index 8)
    //   sum = sum + i
    //   i = i + 1
    //   goto loop (index 3)
    // end (index 8):
    //   return sum

    let instructions = vec![
        // 0: Load iteration count into r0
        Instruction::LoadI { dst: Reg(0), value: iterations },
        // 1: r1 = sum = 0
        Instruction::LoadI { dst: Reg(1), value: 0 },
        // 2: r2 = i = 0
        Instruction::LoadI { dst: Reg(2), value: 0 },
        // 3: loop: r3 = i < n (CmpI)
        Instruction::CmpI {
            op: CompareOp::Lt,
            dst: Reg(3),
            a: Reg(2),
            b: Reg(0),
        },
        // 4: JmpNot r3, end (will jump to index 8)
        Instruction::JmpNot {
            cond: Reg(3),
            offset: 4, // Relative instruction index offset: from 4 to 8
        },
        // 5: sum = sum + i
        Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(1),
            a: Reg(1),
            b: Reg(2)
        },
        // 6: i = i + 1 (UnaryI with Inc)
        Instruction::UnaryI {
            op: UnaryIntOp::Inc,
            dst: Reg(2),
            src: Reg(2)
        },
        // 7: goto loop (jump to index 3)
        Instruction::Jmp { offset: -4 }, // Relative: from 7 to 3
        // 8: return sum (r1)
        Instruction::Ret { value: Reg(1) },
    ];

    // Encode with jump fixup
    let mut bytecode = Vec::new();
    encode_instructions_with_fixup(&instructions, &mut bytecode);

    let bytecode_len = bytecode.len() as u32;
    module.bytecode = bytecode;

    // Create function descriptor
    let func_desc = FunctionDescriptor {
        id: FunctionId(0),
        name: func_name,
        parent_type: None,
        type_params: smallvec::smallvec![],
        params: smallvec::smallvec![],
        return_type: TypeRef::Concrete(TypeId::INT),
        contexts: smallvec::smallvec![],
        properties: PropertySet::PURE,
        bytecode_offset: 0,
        bytecode_length: bytecode_len,
        locals_count: 0,
        register_count: 4,
        max_stack: 0,
        is_inline_candidate: false,
        is_generic: false,
        visibility: Visibility::Public,
        is_generator: false,
        yield_type: None,
        suspend_point_count: 0,
        calling_convention: CallingConvention::C,
        optimization_hints: OptimizationHints::default(),
        instructions: Some(instructions),
        func_id_base: 0,
        debug_variables: Vec::new(),
        is_test: false,
    };

    module.functions.push(func_desc);

    Arc::new(module)
}

/// Creates a simple arithmetic benchmark module.
///
/// Performs N iterations of simple arithmetic operations.
fn create_arith_module(iterations: i64) -> Arc<VbcModule> {
    let mut module = VbcModule::new("bench_arith".to_string());
    let func_name = module.intern_string("arith_bench");

    // Build instruction list:
    // r0 = iterations
    // r1 = counter = 0
    // r2 = a = 1
    // r3 = b = 2
    // r4 = result
    // loop:
    //   r4 = a + b
    //   r2 = r4
    //   r4 = a + b (simulating more work)
    //   r3 = r4
    //   counter++
    //   if counter < iterations goto loop
    // return r4

    let instructions = vec![
        // 0: r0 = iterations
        Instruction::LoadI { dst: Reg(0), value: iterations },
        // 1: r1 = counter = 0
        Instruction::LoadI { dst: Reg(1), value: 0 },
        // 2: r2 = a = 1
        Instruction::LoadI { dst: Reg(2), value: 1 },
        // 3: r3 = b = 2
        Instruction::LoadI { dst: Reg(3), value: 2 },
        // 4: loop: r4 = a + b
        Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(4),
            a: Reg(2),
            b: Reg(3)
        },
        // 5: r2 = r4
        Instruction::Mov { dst: Reg(2), src: Reg(4) },
        // 6: r4 = a + b (again, simulating more work)
        Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(4),
            a: Reg(2),
            b: Reg(3)
        },
        // 7: r3 = r4
        Instruction::Mov { dst: Reg(3), src: Reg(4) },
        // 8: counter++
        Instruction::UnaryI {
            op: UnaryIntOp::Inc,
            dst: Reg(1),
            src: Reg(1)
        },
        // 9: r5 = counter < iterations
        Instruction::CmpI {
            op: CompareOp::Lt,
            dst: Reg(5),
            a: Reg(1),
            b: Reg(0),
        },
        // 10: JmpIf r5, loop (jump to index 4)
        Instruction::JmpIf {
            cond: Reg(5),
            offset: -6, // Relative: from 10 to 4
        },
        // 11: return r4
        Instruction::Ret { value: Reg(4) },
    ];

    // Encode with jump fixup
    let mut bytecode = Vec::new();
    encode_instructions_with_fixup(&instructions, &mut bytecode);

    let bytecode_len = bytecode.len() as u32;
    module.bytecode = bytecode;

    let func_desc = FunctionDescriptor {
        id: FunctionId(0),
        name: func_name,
        parent_type: None,
        type_params: smallvec::smallvec![],
        params: smallvec::smallvec![],
        return_type: TypeRef::Concrete(TypeId::INT),
        contexts: smallvec::smallvec![],
        properties: PropertySet::PURE,
        bytecode_offset: 0,
        bytecode_length: bytecode_len,
        locals_count: 0,
        register_count: 6,
        max_stack: 0,
        is_inline_candidate: false,
        is_generic: false,
        visibility: Visibility::Public,
        is_generator: false,
        yield_type: None,
        suspend_point_count: 0,
        calling_convention: CallingConvention::C,
        optimization_hints: OptimizationHints::default(),
        instructions: Some(instructions),
        func_id_base: 0,
        debug_variables: Vec::new(),
        is_test: false,
    };

    module.functions.push(func_desc);

    Arc::new(module)
}

// ============================================================================
// Benchmarks
// ============================================================================

fn bench_dispatch_loop(c: &mut Criterion) {
    let mut group = c.benchmark_group("dispatch_loop");

    for iterations in [100, 1000, 10000, 100000].iter() {
        let module = create_loop_module(*iterations);

        group.throughput(Throughput::Elements(*iterations as u64));
        group.bench_with_input(
            BenchmarkId::new("loop", iterations),
            iterations,
            |b, _| {
                b.iter(|| {
                    let mut state = InterpreterState::new(Arc::clone(&module));
                    let result = execute_table(&mut state, FunctionId(0));
                    black_box(result)
                });
            },
        );
    }

    group.finish();
}

fn bench_arith(c: &mut Criterion) {
    let mut group = c.benchmark_group("arith_dispatch");

    for iterations in [1000, 10000, 100000].iter() {
        let module = create_arith_module(*iterations);

        group.throughput(Throughput::Elements(*iterations as u64));
        group.bench_with_input(
            BenchmarkId::new("arith", iterations),
            iterations,
            |b, _| {
                b.iter(|| {
                    let mut state = InterpreterState::new(Arc::clone(&module));
                    let result = execute_table(&mut state, FunctionId(0));
                    black_box(result)
                });
            },
        );
    }

    group.finish();
}

/// Comprehensive dispatch benchmark with varying workloads.
fn bench_dispatch_comprehensive(c: &mut Criterion) {
    let mut group = c.benchmark_group("dispatch_comprehensive");

    // Test with different workload sizes
    for iterations in [1000i64, 10000, 50000].iter() {
        let loop_module = create_loop_module(*iterations);
        let arith_module = create_arith_module(*iterations);

        group.throughput(Throughput::Elements(*iterations as u64));

        group.bench_with_input(
            BenchmarkId::new("loop", iterations),
            iterations,
            |b, _| {
                b.iter(|| {
                    let mut state = InterpreterState::new(Arc::clone(&loop_module));
                    let result = execute_table(&mut state, FunctionId(0));
                    black_box(result)
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("arith", iterations),
            iterations,
            |b, _| {
                b.iter(|| {
                    let mut state = InterpreterState::new(Arc::clone(&arith_module));
                    let result = execute_table(&mut state, FunctionId(0));
                    black_box(result)
                });
            },
        );
    }

    group.finish();
}

// Table dispatch benchmarks (the only dispatch method)
criterion_group!(
    benches,
    bench_dispatch_loop,
    bench_arith,
    bench_dispatch_comprehensive,
);

criterion_main!(benches);
