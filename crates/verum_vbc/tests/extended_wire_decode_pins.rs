//! Wire-contract pins for the extended carriers' STRUCTURAL decoder (T0430,
//! T0420, T0419).
//!
//! `decode_instruction` is what the linker, disassembler and archive
//! round-trip use to ADVANCE past an instruction. Unlike the interpreter —
//! which reads operands inline as it executes — this decoder must know each
//! carrier's byte layout up front. When it gets one wrong it does not fail
//! loudly: it returns a zero-operand instruction and leaves the offset INSIDE
//! the operand bytes, so every subsequent decode is misaligned.
//!
//! These tests pin the two properties that prevent that: sequential decode
//! stays aligned across a carrier, and an unrecognised sub-op is rejected
//! rather than silently guessed at.

use verum_vbc::bytecode::{decode_instruction, encode_instruction};
use verum_vbc::instruction::{CubicalSubOpcode, Instruction, Opcode, Reg};

fn encode(instrs: &[Instruction]) -> Vec<u8> {
    let mut bc = Vec::new();
    for i in instrs {
        encode_instruction(i, &mut bc);
    }
    bc
}

// ============================================================================
// T0420 — CubicalExtended registers use the canonical variable-width codec.
// ============================================================================

#[test]
fn cubical_wide_registers_round_trip() {
    // Registers >= 128 take two bytes. The encoder previously wrote
    // `reg.0 as u8`, truncating the value — and when the truncated byte landed
    // >= 128 the interpreter's `read_reg` consumed an extra byte too.
    let original = Instruction::CubicalExtended {
        sub_op: CubicalSubOpcode::PathRefl as u8,
        dst: Reg(300),
        args: vec![Reg(130), Reg(7), Reg(511)],
    };
    let bc = encode(&[original.clone()]);
    let mut offset = 0;
    let decoded = decode_instruction(&bc, &mut offset).expect("decode");

    match decoded {
        Instruction::CubicalExtended { sub_op, dst, args } => {
            assert_eq!(sub_op, CubicalSubOpcode::PathRefl as u8);
            assert_eq!(dst, Reg(300), "dst must survive without truncation");
            assert_eq!(args, vec![Reg(130), Reg(7), Reg(511)]);
        }
        other => panic!("expected CubicalExtended, got {other:?}"),
    }
    assert_eq!(offset, bc.len(), "decode must consume exactly the carrier");
}

#[test]
fn cubical_wide_registers_leave_the_next_instruction_aligned() {
    // The property that actually matters for the linker: sequential decode.
    let bc = encode(&[
        Instruction::CubicalExtended {
            sub_op: CubicalSubOpcode::PathRefl as u8,
            dst: Reg(200),
            args: vec![Reg(300)],
        },
        Instruction::LoadSmallI {
            dst: Reg(1),
            value: 33,
        },
    ]);
    let mut offset = 0;
    decode_instruction(&bc, &mut offset).expect("carrier");
    let next = decode_instruction(&bc, &mut offset).expect("following instruction");
    match next {
        Instruction::LoadSmallI { dst, value } => {
            assert_eq!((dst, value), (Reg(1), 33));
        }
        other => panic!("stream desynchronised, decoded {other:?}"),
    }
}

// ============================================================================
// T0430 — the 0x1F structural decoder is exhaustive; unknown sub-ops are loud.
// ============================================================================

#[test]
fn unknown_extended_sub_op_is_rejected_not_silently_zero_operand() {
    // 0xF3 is not a declared `ExtendedSubOpcode`. The decoder used to return
    // `Extended { operands: vec![] }` for anything it did not recognise, which
    // is indistinguishable from a genuine zero-operand carrier and leaves the
    // offset wherever it was. It must now refuse.
    let bc = vec![Opcode::Extended.to_byte(), 0xF3, 0x01, 0x02];
    let mut offset = 0;
    assert!(
        decode_instruction(&bc, &mut offset).is_err(),
        "an unrecognised Extended sub-op must be a loud decode error"
    );
}

#[test]
fn reserved_extended_sub_op_stays_a_zero_operand_carrier() {
    // `Reserved` (0x00) is the forward-compat anchor: encoders must never emit
    // it, and it carries no operands, so it must keep decoding as an empty
    // carrier — the one case the old wildcard got right, now explicit.
    let bc = vec![
        Opcode::Extended.to_byte(),
        0x00,
        Opcode::Nop.to_byte(),
    ];
    let mut offset = 0;
    let decoded = decode_instruction(&bc, &mut offset).expect("Reserved decodes");
    match decoded {
        Instruction::Extended { sub_op, operands } => {
            assert_eq!(sub_op, 0x00);
            assert!(operands.is_empty());
        }
        other => panic!("expected Extended carrier, got {other:?}"),
    }
    assert_eq!(offset, 2, "Reserved must consume only opcode + sub_op");
    // …and the following instruction still decodes.
    decode_instruction(&bc, &mut offset).expect("following instruction");
}

// ============================================================================
// T0419 — MlExtended is length-prefixed and has a real decode arm.
// ============================================================================

#[test]
fn ml_extended_leaves_the_next_instruction_aligned() {
    // The ML carrier had no length prefix and no decode arm, so it fell to the
    // wildcard, which returns an empty `Raw` WITHOUT advancing past the
    // operands — every following instruction then decoded from inside them.
    //
    // Codegen emits it as `Raw`, but the bytes are the standard envelope:
    // [sub_op][varint operand_len][operands], with variable-width registers.
    let operands = vec![0x80u8, 130, 0x80, 131]; // two wide registers
    let mut data = vec![0x6A_u8]; // MlSubOpcode::ZeroGrad
    data.push(operands.len() as u8); // varint, single byte at this size
    data.extend_from_slice(&operands);

    let bc = encode(&[
        Instruction::Raw {
            opcode: Opcode::MlExtended,
            data,
        },
        Instruction::LoadSmallI {
            dst: Reg(2),
            value: 55,
        },
    ]);

    let mut offset = 0;
    decode_instruction(&bc, &mut offset).expect("ml carrier");
    let next = decode_instruction(&bc, &mut offset).expect("following instruction");
    match next {
        Instruction::LoadSmallI { dst, value } => assert_eq!((dst, value), (Reg(2), 55)),
        other => panic!("stream desynchronised, decoded {other:?}"),
    }
    assert_eq!(offset, bc.len());
}
