//! Envelope pins for the extended-carrier class-kill (T0409/T0410/T0411/
//! T0412/T0414/T0415/T0416/T0418/T0429).
//!
//! Every extended carrier is encoded `[opcode][sub_op][varint len][operands]`,
//! and `dispatch_enveloped` repositions pc to `operands_start + len` on every
//! path. These tests pin the consequence that matters: an arm whose read count
//! disagrees with the declared envelope may compute a wrong value, but the
//! FOLLOWING instruction still decodes and executes.
//!
//! Both directions are covered, because they fail differently:
//!   * over-declared (arm reads fewer bytes than declared) — the surplus bytes
//!     must be skipped, not decoded as opcodes;
//!   * under-declared (arm reads MORE than declared) — pc must be REWOUND.
//!     This is the historically corrupting direction: the arm consumes the
//!     next instruction's opcode byte as a register operand, and every
//!     subsequent decode is misaligned (the T0177 gpu_memset SIGSEGV and the
//!     task-#8 `GenerationalArena.new` NullPointer both had this shape).

use std::sync::Arc;
use verum_vbc::bytecode;
use verum_vbc::instruction::{
    ArithSubOpcode, CharSubOpcode, Instruction, MathSubOpcode, Reg, SimdSubOpcode,
};
use verum_vbc::interpreter::Interpreter;
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

/// Wide frame on purpose: an under-declared envelope makes the arm read the
/// FOLLOWING instruction's bytes as register indices, so the frame must be
/// large enough that those bogus indices stay in range and the test observes
/// the pc behaviour rather than an out-of-range panic.
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
// Under-declared envelopes — the arm over-reads; pc must be rewound.
// ============================================================================

#[test]
fn math_carrier_under_declared_leaves_next_instruction_executable() {
    // SinF64's arm reads dst + src = 2 registers; declare only 1 operand
    // byte. The arm consumes the following LoadSmallI's opcode byte as its
    // `src` register — the envelope must rewind pc to operands_start + 1.
    assert_eq!(
        survives(Instruction::MathExtended {
            sub_op: MathSubOpcode::SinF64 as u8,
            operands: vec![2],
        }),
        SENTINEL
    );
}

#[test]
fn arith_carrier_under_declared_leaves_next_instruction_executable() {
    // CheckedAddI reads dst + a + b = 3 registers; declare 1.
    assert_eq!(
        survives(Instruction::ArithExtended {
            sub_op: ArithSubOpcode::CheckedAddI as u8,
            operands: vec![2],
        }),
        SENTINEL
    );
}

#[test]
fn char_carrier_under_declared_leaves_next_instruction_executable() {
    // IsAlphabeticAscii reads dst + src = 2 registers; declare 1.
    assert_eq!(
        survives(Instruction::CharExtended {
            sub_op: CharSubOpcode::IsAlphabeticAscii as u8,
            operands: vec![2],
        }),
        SENTINEL
    );
}

#[test]
fn simd_carrier_under_declared_leaves_next_instruction_executable() {
    // Splat reads dst + src = 2 registers; declare 1.
    assert_eq!(
        survives(Instruction::SimdExtended {
            sub_op: SimdSubOpcode::Splat as u8,
            operands: vec![2],
        }),
        SENTINEL
    );
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
