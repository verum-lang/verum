//! Production Target Benchmarks for VBC Interpreter
//!
//! Targets:
//! - Runtime: 0.85-0.95x native C
//! - Memory overhead: <5%
//!
//! Benchmarks VBC interpreter execution on compute-intensive workloads
//! (fibonacci, sum loops) and compares throughput characteristics.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

use verum_vbc::bytecode::encode_instructions_with_fixup;
use verum_vbc::instruction::{BinaryIntOp, CompareOp, Instruction, Reg, UnaryIntOp};
use verum_vbc::interpreter::{execute_table, InterpreterState};
use verum_vbc::module::{
    CallingConvention, FunctionDescriptor, FunctionId, OptimizationHints, VbcModule,
};
use verum_vbc::types::{PropertySet, TypeId, TypeRef, Visibility};

// ============================================================================
// Module Builders
// ============================================================================

fn build_module(name: &str, instructions: Vec<Instruction>, reg_count: u8) -> Arc<VbcModule> {
    let mut module = VbcModule::new(name.to_string());
    let func_name = module.intern_string(name);

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
        register_count: reg_count as u16,
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

/// Sum loop: sum = 0; for i in 0..n { sum += i; } return sum
fn create_sum_module(n: i64) -> Arc<VbcModule> {
    let instructions = vec![
        Instruction::LoadI { dst: Reg(0), value: n },
        Instruction::LoadI { dst: Reg(1), value: 0 }, // sum
        Instruction::LoadI { dst: Reg(2), value: 0 }, // i
        // loop:
        Instruction::CmpI { op: CompareOp::Lt, dst: Reg(3), a: Reg(2), b: Reg(0) },
        Instruction::JmpNot { cond: Reg(3), offset: 4 },
        Instruction::BinaryI { op: BinaryIntOp::Add, dst: Reg(1), a: Reg(1), b: Reg(2) },
        Instruction::UnaryI { op: UnaryIntOp::Inc, dst: Reg(2), src: Reg(2) },
        Instruction::Jmp { offset: -4 },
        // end:
        Instruction::Ret { value: Reg(1) },
    ];
    build_module("sum_loop", instructions, 4)
}

/// Nested loop: sum = 0; for i in 0..n { for j in 0..n { sum += 1; } } return sum
fn create_nested_loop_module(n: i64) -> Arc<VbcModule> {
    let instructions = vec![
        // r0 = n, r1 = sum, r2 = i, r3 = j, r4 = cmp_i, r5 = cmp_j
        Instruction::LoadI { dst: Reg(0), value: n },
        Instruction::LoadI { dst: Reg(1), value: 0 }, // sum
        Instruction::LoadI { dst: Reg(2), value: 0 }, // i = 0
        // outer loop:
        Instruction::CmpI { op: CompareOp::Lt, dst: Reg(4), a: Reg(2), b: Reg(0) },
        Instruction::JmpNot { cond: Reg(4), offset: 8 }, // to end
        Instruction::LoadI { dst: Reg(3), value: 0 }, // j = 0
        // inner loop:
        Instruction::CmpI { op: CompareOp::Lt, dst: Reg(5), a: Reg(3), b: Reg(0) },
        Instruction::JmpNot { cond: Reg(5), offset: 4 }, // to outer_inc
        Instruction::UnaryI { op: UnaryIntOp::Inc, dst: Reg(1), src: Reg(1) }, // sum++
        Instruction::UnaryI { op: UnaryIntOp::Inc, dst: Reg(3), src: Reg(3) }, // j++
        Instruction::Jmp { offset: -4 }, // to inner loop
        // outer_inc:
        Instruction::UnaryI { op: UnaryIntOp::Inc, dst: Reg(2), src: Reg(2) }, // i++
        Instruction::Jmp { offset: -9 }, // to outer loop
        // end:
        Instruction::Ret { value: Reg(1) },
    ];
    build_module("nested_loop", instructions, 6)
}

/// Arithmetic-heavy: repeated multiply-add operations
fn create_arith_heavy_module(n: i64) -> Arc<VbcModule> {
    let instructions = vec![
        // r0 = n, r1 = counter, r2 = a, r3 = b, r4 = tmp
        Instruction::LoadI { dst: Reg(0), value: n },
        Instruction::LoadI { dst: Reg(1), value: 0 },
        Instruction::LoadI { dst: Reg(2), value: 1 },
        Instruction::LoadI { dst: Reg(3), value: 2 },
        // loop:
        Instruction::CmpI { op: CompareOp::Lt, dst: Reg(5), a: Reg(1), b: Reg(0) },
        Instruction::JmpNot { cond: Reg(5), offset: 8 },
        // a = a * 3 + b
        Instruction::LoadI { dst: Reg(4), value: 3 },
        Instruction::BinaryI { op: BinaryIntOp::Mul, dst: Reg(4), a: Reg(2), b: Reg(4) },
        Instruction::BinaryI { op: BinaryIntOp::Add, dst: Reg(2), a: Reg(4), b: Reg(3) },
        // b = b + a % 7
        Instruction::LoadI { dst: Reg(4), value: 7 },
        Instruction::BinaryI { op: BinaryIntOp::Mod, dst: Reg(4), a: Reg(2), b: Reg(4) },
        Instruction::BinaryI { op: BinaryIntOp::Add, dst: Reg(3), a: Reg(3), b: Reg(4) },
        Instruction::UnaryI { op: UnaryIntOp::Inc, dst: Reg(1), src: Reg(1) },
        Instruction::Jmp { offset: -9 },
        // end:
        Instruction::Ret { value: Reg(2) },
    ];
    build_module("arith_heavy", instructions, 6)
}

// ============================================================================
// Native C equivalents (for comparison baseline)
// ============================================================================

#[inline(never)]
fn native_sum_loop(n: i64) -> i64 {
    let mut sum: i64 = 0;
    let mut i: i64 = 0;
    while i < n {
        sum = sum.wrapping_add(i);
        i += 1;
    }
    sum
}

#[inline(never)]
fn native_nested_loop(n: i64) -> i64 {
    let mut sum: i64 = 0;
    let mut i: i64 = 0;
    while i < n {
        let mut j: i64 = 0;
        while j < n {
            sum += 1;
            j += 1;
        }
        i += 1;
    }
    sum
}

// ============================================================================
// Benchmarks
// ============================================================================

fn bench_vbc_vs_native(c: &mut Criterion) {
    let mut group = c.benchmark_group("production_target_runtime");
    group.warm_up_time(Duration::from_secs(2));
    group.measurement_time(Duration::from_secs(5));

    // Sum loop comparison
    for n in [1000i64, 10000, 100000] {
        group.throughput(Throughput::Elements(n as u64));

        let module = create_sum_module(n);
        group.bench_with_input(
            BenchmarkId::new("vbc_sum", n),
            &n,
            |b, _| {
                b.iter(|| {
                    let mut state = InterpreterState::new(Arc::clone(&module));
                    black_box(execute_table(&mut state, FunctionId(0)))
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("native_sum", n),
            &n,
            |b, &n| {
                b.iter(|| black_box(native_sum_loop(black_box(n))))
            },
        );
    }

    group.finish();
}

fn bench_nested_loop(c: &mut Criterion) {
    let mut group = c.benchmark_group("production_target_nested");
    group.warm_up_time(Duration::from_secs(2));

    for n in [50i64, 100, 200] {
        let total_ops = n * n;
        group.throughput(Throughput::Elements(total_ops as u64));

        let module = create_nested_loop_module(n);
        group.bench_with_input(
            BenchmarkId::new("vbc_nested", n),
            &n,
            |b, _| {
                b.iter(|| {
                    let mut state = InterpreterState::new(Arc::clone(&module));
                    black_box(execute_table(&mut state, FunctionId(0)))
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("native_nested", n),
            &n,
            |b, &n| {
                b.iter(|| black_box(native_nested_loop(black_box(n))))
            },
        );
    }

    group.finish();
}

fn bench_arith_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("production_target_arith");

    for n in [1000i64, 10000, 50000] {
        group.throughput(Throughput::Elements(n as u64));

        let module = create_arith_heavy_module(n);
        group.bench_with_input(
            BenchmarkId::new("vbc_arith", n),
            &n,
            |b, _| {
                b.iter(|| {
                    let mut state = InterpreterState::new(Arc::clone(&module));
                    black_box(execute_table(&mut state, FunctionId(0)))
                });
            },
        );
    }

    group.finish();
}

fn bench_interpreter_memory_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("production_target_memory");

    // Measure InterpreterState creation overhead
    group.bench_function("state_creation", |b| {
        let module = create_sum_module(100);
        b.iter(|| {
            let state = InterpreterState::new(Arc::clone(&module));
            black_box(std::mem::size_of_val(&state))
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_vbc_vs_native,
    bench_nested_loop,
    bench_arith_throughput,
    bench_interpreter_memory_overhead,
);
criterion_main!(benches);
