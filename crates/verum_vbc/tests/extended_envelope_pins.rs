//! Envelope pins for the extended-carrier class-kill (T0409/T0410/T0411/
//! T0412/T0414/T0415/T0416/T0418/T0429).
//!
//! The length prefix bounds handler reads. Surplus bytes are skipped on
//! success. An incomplete operand must fail before it consumes another
//! instruction, even if that instruction begins with a valid register.

use std::sync::Arc;
use verum_vbc::bytecode;
use verum_vbc::instruction::{
    ArithSubOpcode, CharSubOpcode, Instruction, LogSubOpcode, MathSubOpcode, Reg, SimdSubOpcode,
};
use verum_vbc::interpreter::{Interpreter, InterpreterError};
use verum_vbc::module::{FunctionDescriptor, FunctionId, VbcModule};
use verum_vbc::types::StringId;
use verum_vbc::value::Value;

fn create_module(bytecode_data: Vec<u8>, register_count: u16) -> Arc<VbcModule> {
    let mut module = VbcModule::new("envelope_pins".to_string());
    let mut func = FunctionDescriptor::new(StringId::EMPTY);
    func.id = FunctionId(0);
    func.bytecode_offset = 0;
    func.bytecode_length = bytecode_data.len() as u32;
    func.register_count = register_count;
    module.functions.push(func);
    module.bytecode = bytecode_data;
    Arc::new(module)
}

fn run_with_regs(instructions: &[Instruction], register_count: u16) -> Value {
    let mut bc = Vec::new();
    for instr in instructions {
        bytecode::encode_instruction(instr, &mut bc);
    }
    let module = create_module(bc, register_count);
    let mut interp = Interpreter::new(module);
    interp
        .execute_function(FunctionId(0))
        .expect("execution failed")
}

fn run(instructions: &[Instruction]) -> Value {
    run_with_regs(instructions, 256)
}

/// Sentinel returned by the instruction AFTER the malformed carrier. Reading
/// it back proves the stream stayed aligned across the carrier.
const SENTINEL: i64 = 41;

fn survives(carrier: Instruction) -> i64 {
    run(&[
        carrier,
        Instruction::LoadSmallI {
            dst: Reg(0),
            value: SENTINEL as i8,
        },
        Instruction::Ret { value: Reg(0) },
    ])
    .as_i64()
}

// ============================================================================
// Under-declared envelopes cannot supply operands from the next instruction.
// ============================================================================

#[test]
fn incomplete_operands_fail_inside_the_envelope() {
    for instruction in [
        Instruction::MathExtended {
            sub_op: MathSubOpcode::SinF64 as u8,
            operands: vec![2],
        },
        Instruction::ArithExtended {
            sub_op: ArithSubOpcode::CheckedAddI as u8,
            operands: vec![2],
        },
        Instruction::CharExtended {
            sub_op: CharSubOpcode::IsAlphabeticAscii as u8,
            operands: vec![2],
        },
        Instruction::SimdExtended {
            sub_op: SimdSubOpcode::Splat as u8,
            operands: vec![2],
        },
    ] {
        let mut bytes = Vec::new();
        bytecode::encode_instruction(&instruction, &mut bytes);
        let end = bytes.len();
        bytecode::encode_instruction(&Instruction::RetV, &mut bytes);
        let result = Interpreter::new(create_module(bytes, 256)).execute_function(FunctionId(0));
        assert!(matches!(result, Err(InterpreterError::InvalidBytecode { pc, .. }) if pc == end));
    }
}

// ============================================================================
// Over-declared envelopes — surplus bytes must be skipped, not executed.
// ============================================================================

#[test]
fn math_carrier_over_declared_skips_surplus_operand_bytes() {
    // Three operand bytes declared, two consumed. The stray third byte is
    // `Opcode::Ret`'s value — if it were decoded as an instruction the
    // function would return the wrong register (or fault).
    assert_eq!(
        survives(Instruction::MathExtended {
            sub_op: MathSubOpcode::SinF64 as u8,
            operands: vec![2, 1, 0x00],
        }),
        SENTINEL
    );
}

#[test]
fn arith_carrier_over_declared_skips_surplus_operand_bytes() {
    assert_eq!(
        survives(Instruction::ArithExtended {
            sub_op: ArithSubOpcode::CheckedAddI as u8,
            operands: vec![2, 1, 3, 0x00, 0x00],
        }),
        SENTINEL
    );
}

#[test]
fn char_carrier_over_declared_skips_surplus_operand_bytes() {
    assert_eq!(
        survives(Instruction::CharExtended {
            sub_op: CharSubOpcode::IsAlphabeticAscii as u8,
            operands: vec![2, 1, 0x00, 0x00],
        }),
        SENTINEL
    );
}

// ============================================================================
// Wide registers (>= 128) — two-byte operands must survive the envelope.
// ============================================================================

#[test]
fn wide_register_operands_survive_the_envelope() {
    // `encode_operands` emits two bytes for a register >= 128. A carrier
    // built entirely from wide registers must still hand off cleanly to the
    // next instruction.
    let result = run_with_regs(
        &[
            Instruction::LoadI {
                dst: Reg(131),
                value: 4242,
            },
            Instruction::SimdExtended {
                sub_op: SimdSubOpcode::Splat as u8,
                operands: vec![0x80, 130, 0x80, 131],
            },
            Instruction::Mov {
                dst: Reg(0),
                src: Reg(130),
            },
            Instruction::Ret { value: Reg(0) },
        ],
        256,
    );
    assert_eq!(result.as_i64(), 4242);
}

// ============================================================================
// Operand-shape pins: the arm must consume what the emitter packs.
// ============================================================================

#[test]
fn simd_insert_reads_the_lane_as_a_register_not_a_u8_immediate() {
    // T0411. `encode_operands` packs the lane index as a REGISTER, so a lane
    // register >= 128 occupies two bytes. Reading it with `read_u8` consumed
    // only the first, and the following `val` operand was then decoded from
    // the leftover byte — yielding a completely different register.
    //
    // Wire: [dst=r5][vec=r6][lane=r130 (0x80,130)][val=r7].
    // Correct read  -> val = r7   -> dst receives 777.
    // read_u8 lane  -> val = r519 -> dst receives that register's contents.
    let result = run_with_regs(
        &[
            Instruction::LoadI {
                dst: Reg(7),
                value: 777,
            },
            Instruction::SimdExtended {
                sub_op: SimdSubOpcode::Insert as u8,
                operands: vec![5, 6, 0x80, 130, 7],
            },
            Instruction::Mov {
                dst: Reg(0),
                src: Reg(5),
            },
            Instruction::Ret { value: Reg(0) },
        ],
        600,
    );
    assert_eq!(
        result.as_i64(),
        777,
        "Insert must consume the lane operand as a register"
    );
}

/// T0418: the level arms must log the MESSAGE operand, not the destination
/// temp. stderr is the only observable, so the assertion runs against a
/// re-exec of this same test binary — dependency-free, no fd plumbing.
#[test]
fn log_info_logs_the_message_operand_not_the_dst_temp() {
    const CHILD: &str = "VBC_T0418_LOG_PIN_CHILD";
    const TEST: &str = "log_info_logs_the_message_operand_not_the_dst_temp";

    if std::env::var(CHILD).is_ok() {
        // r3 carries the message; r2 is the dst slot the emitter always packs
        // and this void sub-op never writes.
        run_with_regs(
            &[
                Instruction::LoadI {
                    dst: Reg(3),
                    value: 987_654,
                },
                Instruction::LogExtended {
                    sub_op: LogSubOpcode::Info as u8,
                    operands: vec![2, 3],
                },
                Instruction::LoadSmallI {
                    dst: Reg(0),
                    value: 1,
                },
                Instruction::Ret { value: Reg(0) },
            ],
            32,
        );
        return;
    }

    let output = std::process::Command::new(std::env::current_exe().expect("test binary path"))
        .args([TEST, "--exact", "--nocapture"])
        .env(CHILD, "1")
        .output()
        .expect("re-exec test binary");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("[INFO] 987654"),
        "log must emit the message operand; stderr was: {stderr}"
    );
}
