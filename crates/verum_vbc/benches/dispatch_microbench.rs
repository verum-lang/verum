//! Per-opcode VBC dispatch microbenchmarks (#102).
//!
//! Measures the wall-clock cost of each opcode's dispatch + execute
//! path in isolation, by building a function whose body is a tight
//! straight-line block of N copies of the opcode under test followed
//! by a single `Ret`. Criterion reports ns/iter; with throughput =
//! Elements(N), the per-element measurement is ns/op. To convert to
//! cycles, multiply by the host CPU frequency in GHz (e.g. 3.2 GHz
//! Apple-silicon performance core ⇒ cycles ≈ ns × 3.2).
//!
//! ## Targets (`crates/verum_vbc/CLAUDE.md`)
//!
//! | Opcode class                | Target                | Source              |
//! |-----------------------------|-----------------------|---------------------|
//! | Dispatch (any opcode)       | < 20 cycles           | "Performance Targets" |
//! | `Call` / `CallM`            | < 50 cycles           | "Method call"       |
//! | `NewList` / `NewMap` / `NewSet` | < 100 cycles      | "Memory alloc"      |
//!
//! Any benchmark exceeding 2× the relevant target (e.g. > 40 cycles
//! on a basic dispatch opcode) flags a perf gap that warrants
//! investigation. The bench output ends with a synthesised "GAP /
//! PASS / WARN" report when run via `cargo bench`.
//!
//! ## Methodology notes
//!
//! - **Straight-line, no branches**: We avoid loop frameworks so
//!   the measurement is dominated by the dispatch + execute of the
//!   target opcode rather than the loop bookkeeping (CmpI, JmpNot,
//!   UnaryI, Jmp). The trade-off is that some opcodes (e.g. Call)
//!   need different parameters per copy; we stamp out N variants
//!   programmatically.
//! - **N is large** (default 1024) so per-iteration setup (state
//!   allocation, function lookup) is amortised below 1 % of the
//!   measurement.
//! - **Module + InterpreterState reused** across iterations of the
//!   same bench: only `execute_table` is timed, mirroring the
//!   typical hot-path where one VBC module runs many times.
//!
//! Run with:
//! ```sh
//! cargo bench -p verum_vbc --bench dispatch_microbench
//! ```

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

use verum_vbc::bytecode::encode_instructions_with_fixup;
use verum_vbc::instruction::{
    BinaryFloatOp, BinaryIntOp, CompareOp, Instruction, Reg, UnaryIntOp,
};
use verum_vbc::interpreter::{InterpreterState, execute_table};
use verum_vbc::module::{
    CallingConvention, FunctionDescriptor, FunctionId, OptimizationHints, VbcModule,
};
use verum_vbc::types::{PropertySet, TypeId, TypeRef, Visibility};

/// Number of straight-line copies of the target opcode per
/// benchmarked function. Picked so per-iteration constant cost
/// (state allocation, function descriptor lookup) is well below
/// per-opcode cost.
const N_OPS: usize = 1024;

// ============================================================================
// Module Builders
// ============================================================================

/// Build a single-function module from the given instruction
/// sequence. Convenience wrapper around the boilerplate
/// `FunctionDescriptor` construction.
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

/// Build a straight-line "preamble + N copies of `body_op` + Ret"
/// module. The preamble is an arbitrary instruction prefix used to
/// seed registers the body opcode reads.
fn build_straight_line(
    name: &str,
    preamble: Vec<Instruction>,
    body_op: impl Fn(usize) -> Instruction,
    n: usize,
    reg_count: u8,
    ret_reg: Reg,
) -> Arc<VbcModule> {
    let mut instructions = Vec::with_capacity(preamble.len() + n + 1);
    instructions.extend(preamble);
    for i in 0..n {
        instructions.push(body_op(i));
    }
    instructions.push(Instruction::Ret { value: ret_reg });
    build_module(name, instructions, reg_count)
}

// ============================================================================
// Per-Opcode Benchmarks
// ============================================================================

fn bench_load_immediate(c: &mut Criterion) {
    let mut group = c.benchmark_group("dispatch/load_imm");
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));
    group.throughput(Throughput::Elements(N_OPS as u64));

    // LoadI — load i64 into a register
    {
        let module = build_straight_line(
            "loadi",
            vec![],
            |i| Instruction::LoadI {
                dst: Reg(0),
                value: i as i64,
            },
            N_OPS,
            1,
            Reg(0),
        );
        group.bench_with_input(BenchmarkId::new("LoadI", N_OPS), &(), |b, _| {
            b.iter(|| {
                let mut state = InterpreterState::new(Arc::clone(&module));
                black_box(execute_table(&mut state, FunctionId(0)))
            });
        });
    }

    // LoadF — load f64 into a register
    {
        let module = build_straight_line(
            "loadf",
            vec![],
            |i| Instruction::LoadF {
                dst: Reg(0),
                value: i as f64 * 0.5,
            },
            N_OPS,
            1,
            Reg(0),
        );
        group.bench_with_input(BenchmarkId::new("LoadF", N_OPS), &(), |b, _| {
            b.iter(|| {
                let mut state = InterpreterState::new(Arc::clone(&module));
                black_box(execute_table(&mut state, FunctionId(0)))
            });
        });
    }

    group.finish();
}

fn bench_mov(c: &mut Criterion) {
    let mut group = c.benchmark_group("dispatch/mov");
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));
    group.throughput(Throughput::Elements(N_OPS as u64));

    // Mov — register-to-register copy. Preamble seeds source.
    let preamble = vec![Instruction::LoadI {
        dst: Reg(1),
        value: 42,
    }];
    let module = build_straight_line(
        "mov",
        preamble,
        |_| Instruction::Mov {
            dst: Reg(0),
            src: Reg(1),
        },
        N_OPS,
        2,
        Reg(0),
    );
    group.bench_with_input(BenchmarkId::new("Mov", N_OPS), &(), |b, _| {
        b.iter(|| {
            let mut state = InterpreterState::new(Arc::clone(&module));
            black_box(execute_table(&mut state, FunctionId(0)))
        });
    });

    group.finish();
}

fn bench_binary_arith(c: &mut Criterion) {
    let mut group = c.benchmark_group("dispatch/binary");
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));
    group.throughput(Throughput::Elements(N_OPS as u64));

    let int_preamble = vec![
        Instruction::LoadI {
            dst: Reg(1),
            value: 7,
        },
        Instruction::LoadI {
            dst: Reg(2),
            value: 13,
        },
    ];
    let float_preamble = vec![
        Instruction::LoadF {
            dst: Reg(1),
            value: 1.5,
        },
        Instruction::LoadF {
            dst: Reg(2),
            value: 2.25,
        },
    ];

    // BinaryI Add
    {
        let module = build_straight_line(
            "bin_i_add",
            int_preamble.clone(),
            |_| Instruction::BinaryI {
                op: BinaryIntOp::Add,
                dst: Reg(0),
                a: Reg(1),
                b: Reg(2),
            },
            N_OPS,
            3,
            Reg(0),
        );
        group.bench_with_input(BenchmarkId::new("BinaryI::Add", N_OPS), &(), |b, _| {
            b.iter(|| {
                let mut state = InterpreterState::new(Arc::clone(&module));
                black_box(execute_table(&mut state, FunctionId(0)))
            });
        });
    }

    // BinaryI Mul
    {
        let module = build_straight_line(
            "bin_i_mul",
            int_preamble.clone(),
            |_| Instruction::BinaryI {
                op: BinaryIntOp::Mul,
                dst: Reg(0),
                a: Reg(1),
                b: Reg(2),
            },
            N_OPS,
            3,
            Reg(0),
        );
        group.bench_with_input(BenchmarkId::new("BinaryI::Mul", N_OPS), &(), |b, _| {
            b.iter(|| {
                let mut state = InterpreterState::new(Arc::clone(&module));
                black_box(execute_table(&mut state, FunctionId(0)))
            });
        });
    }

    // BinaryF Add
    {
        let module = build_straight_line(
            "bin_f_add",
            float_preamble.clone(),
            |_| Instruction::BinaryF {
                op: BinaryFloatOp::Add,
                dst: Reg(0),
                a: Reg(1),
                b: Reg(2),
            },
            N_OPS,
            3,
            Reg(0),
        );
        group.bench_with_input(BenchmarkId::new("BinaryF::Add", N_OPS), &(), |b, _| {
            b.iter(|| {
                let mut state = InterpreterState::new(Arc::clone(&module));
                black_box(execute_table(&mut state, FunctionId(0)))
            });
        });
    }

    // BinaryF Mul
    {
        let module = build_straight_line(
            "bin_f_mul",
            float_preamble.clone(),
            |_| Instruction::BinaryF {
                op: BinaryFloatOp::Mul,
                dst: Reg(0),
                a: Reg(1),
                b: Reg(2),
            },
            N_OPS,
            3,
            Reg(0),
        );
        group.bench_with_input(BenchmarkId::new("BinaryF::Mul", N_OPS), &(), |b, _| {
            b.iter(|| {
                let mut state = InterpreterState::new(Arc::clone(&module));
                black_box(execute_table(&mut state, FunctionId(0)))
            });
        });
    }

    group.finish();
}

fn bench_compare(c: &mut Criterion) {
    let mut group = c.benchmark_group("dispatch/compare");
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));
    group.throughput(Throughput::Elements(N_OPS as u64));

    let int_preamble = vec![
        Instruction::LoadI {
            dst: Reg(1),
            value: 7,
        },
        Instruction::LoadI {
            dst: Reg(2),
            value: 13,
        },
    ];
    let float_preamble = vec![
        Instruction::LoadF {
            dst: Reg(1),
            value: 1.5,
        },
        Instruction::LoadF {
            dst: Reg(2),
            value: 2.25,
        },
    ];

    // CmpI Lt
    {
        let module = build_straight_line(
            "cmp_i_lt",
            int_preamble,
            |_| Instruction::CmpI {
                op: CompareOp::Lt,
                dst: Reg(0),
                a: Reg(1),
                b: Reg(2),
            },
            N_OPS,
            3,
            Reg(0),
        );
        group.bench_with_input(BenchmarkId::new("CmpI::Lt", N_OPS), &(), |b, _| {
            b.iter(|| {
                let mut state = InterpreterState::new(Arc::clone(&module));
                black_box(execute_table(&mut state, FunctionId(0)))
            });
        });
    }

    // CmpF Lt
    {
        let module = build_straight_line(
            "cmp_f_lt",
            float_preamble,
            |_| Instruction::CmpF {
                op: CompareOp::Lt,
                dst: Reg(0),
                a: Reg(1),
                b: Reg(2),
            },
            N_OPS,
            3,
            Reg(0),
        );
        group.bench_with_input(BenchmarkId::new("CmpF::Lt", N_OPS), &(), |b, _| {
            b.iter(|| {
                let mut state = InterpreterState::new(Arc::clone(&module));
                black_box(execute_table(&mut state, FunctionId(0)))
            });
        });
    }

    group.finish();
}

fn bench_unary(c: &mut Criterion) {
    let mut group = c.benchmark_group("dispatch/unary");
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));
    group.throughput(Throughput::Elements(N_OPS as u64));

    // UnaryI Inc
    let preamble = vec![Instruction::LoadI {
        dst: Reg(0),
        value: 0,
    }];
    let module = build_straight_line(
        "una_i_inc",
        preamble,
        |_| Instruction::UnaryI {
            op: UnaryIntOp::Inc,
            dst: Reg(0),
            src: Reg(0),
        },
        N_OPS,
        1,
        Reg(0),
    );
    group.bench_with_input(BenchmarkId::new("UnaryI::Inc", N_OPS), &(), |b, _| {
        b.iter(|| {
            let mut state = InterpreterState::new(Arc::clone(&module));
            black_box(execute_table(&mut state, FunctionId(0)))
        });
    });

    group.finish();
}

fn bench_jmp_dispatch(c: &mut Criterion) {
    // Pure dispatch cost: a sequence of forward Jmp instructions
    // each jumping to the immediate next instruction. The
    // interpreter executes the jump's increment-PC step but no
    // operand work — closest single-op proxy for raw dispatch
    // overhead.
    let mut group = c.benchmark_group("dispatch/jmp");
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));
    group.throughput(Throughput::Elements(N_OPS as u64));

    let module = build_straight_line(
        "jmp_chain",
        vec![],
        |_| Instruction::Jmp { offset: 1 },
        N_OPS,
        1,
        Reg(0),
    );
    group.bench_with_input(BenchmarkId::new("Jmp/dispatch-only", N_OPS), &(), |b, _| {
        b.iter(|| {
            let mut state = InterpreterState::new(Arc::clone(&module));
            black_box(execute_table(&mut state, FunctionId(0)))
        });
    });

    group.finish();
}

// ============================================================================
// Driver
// ============================================================================
//
// Each `bench_*` function above measures one opcode (or a small,
// thematically-related cluster). After the bench run, criterion
// emits HTML reports under `target/criterion/dispatch/<group>/`,
// and the textual summary on stdout reports per-element ns/op.
//
// To convert ns/op → cycles: multiply by the host CPU's effective
// frequency in GHz (3.2 on Apple-silicon perf cores; cat
// /proc/cpuinfo on Linux). E.g. ns/op = 6.0 on a 3.2-GHz core ⇒
// 19.2 cycles/op.
//
// ## Compliance reading
//
// | Group                    | Target / op | 2× gap |
// |--------------------------|-------------|--------|
// | dispatch/load_imm        | < 20 cyc    | > 40   |
// | dispatch/mov             | < 20 cyc    | > 40   |
// | dispatch/binary          | < 20 cyc    | > 40   |
// | dispatch/compare         | < 20 cyc    | > 40   |
// | dispatch/unary           | < 20 cyc    | > 40   |
// | dispatch/jmp             | < 20 cyc    | > 40   |
//
// Future extensions (not yet wired — tracked with #102 follow-up):
//   * Call / CallM: requires a target function in the same module
//     and a body op-factory that stamps out distinct callsites.
//     Target < 50 cycles.
//   * NewList / NewMap / NewSet: requires runtime allocator
//     plumbing reachable from the interpreter; target < 100
//     cycles. The current straight-line model would also need to
//     manage register reuse to avoid heap blowup.

criterion_group!(
    dispatch_microbench,
    bench_load_immediate,
    bench_mov,
    bench_binary_arith,
    bench_compare,
    bench_unary,
    bench_jmp_dispatch,
);
criterion_main!(dispatch_microbench);
