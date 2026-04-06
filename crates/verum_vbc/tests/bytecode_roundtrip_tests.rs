#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
//! Bytecode encoding/decoding roundtrip tests.
//!
//! These tests verify that `encode_instruction` followed by `decode_instruction`
//! produces the original instruction for all major instruction categories.

use verum_vbc::bytecode::{decode_instruction, encode_instruction};
use verum_vbc::instruction::{
    BinaryFloatOp, BinaryGenericOp, BinaryIntOp, BitwiseOp, CompareOp, FloatToIntMode,
    Instruction, Opcode, Reg, RegRange, UnaryFloatOp, UnaryIntOp,
};
use verum_vbc::types::TypeId;

// =============================================================================
// Helper
// =============================================================================

fn roundtrip(instr: &Instruction) -> Instruction {
    let mut bytes = Vec::new();
    encode_instruction(instr, &mut bytes);
    let mut offset = 0;
    decode_instruction(&bytes, &mut offset).expect("decode failed")
}

fn assert_roundtrip(instr: &Instruction) {
    let decoded = roundtrip(instr);
    assert_eq!(&decoded, instr, "Roundtrip failed for {:?}", instr);
}

// =============================================================================
// Data Movement Roundtrips
// =============================================================================

#[test]
fn test_roundtrip_mov() {
    assert_roundtrip(&Instruction::Mov {
        dst: Reg(0),
        src: Reg(1),
    });
}

#[test]
fn test_roundtrip_mov_high_registers() {
    assert_roundtrip(&Instruction::Mov {
        dst: Reg(200),
        src: Reg(300),
    });
}

#[test]
fn test_roundtrip_load_i_positive() {
    assert_roundtrip(&Instruction::LoadI {
        dst: Reg(0),
        value: 42,
    });
}

#[test]
fn test_roundtrip_load_i_negative() {
    assert_roundtrip(&Instruction::LoadI {
        dst: Reg(0),
        value: -1_000_000,
    });
}

#[test]
fn test_roundtrip_load_i_zero() {
    assert_roundtrip(&Instruction::LoadI {
        dst: Reg(0),
        value: 0,
    });
}

#[test]
fn test_roundtrip_load_i_max() {
    assert_roundtrip(&Instruction::LoadI {
        dst: Reg(0),
        value: i64::MAX,
    });
}

#[test]
fn test_roundtrip_load_i_min() {
    assert_roundtrip(&Instruction::LoadI {
        dst: Reg(0),
        value: i64::MIN,
    });
}

#[test]
fn test_roundtrip_load_f() {
    assert_roundtrip(&Instruction::LoadF {
        dst: Reg(0),
        value: 3.14159265358979,
    });
}

#[test]
fn test_roundtrip_load_f_negative() {
    assert_roundtrip(&Instruction::LoadF {
        dst: Reg(0),
        value: -273.15,
    });
}

#[test]
fn test_roundtrip_load_f_infinity() {
    assert_roundtrip(&Instruction::LoadF {
        dst: Reg(0),
        value: f64::INFINITY,
    });
}

#[test]
fn test_roundtrip_load_f_neg_infinity() {
    assert_roundtrip(&Instruction::LoadF {
        dst: Reg(0),
        value: f64::NEG_INFINITY,
    });
}

#[test]
fn test_roundtrip_load_true() {
    assert_roundtrip(&Instruction::LoadTrue { dst: Reg(5) });
}

#[test]
fn test_roundtrip_load_false() {
    assert_roundtrip(&Instruction::LoadFalse { dst: Reg(7) });
}

#[test]
fn test_roundtrip_load_unit() {
    assert_roundtrip(&Instruction::LoadUnit { dst: Reg(0) });
}

#[test]
fn test_roundtrip_load_nil() {
    assert_roundtrip(&Instruction::LoadNil { dst: Reg(0) });
}

#[test]
fn test_roundtrip_nop() {
    assert_roundtrip(&Instruction::Nop);
}

#[test]
fn test_roundtrip_load_small_i_positive() {
    assert_roundtrip(&Instruction::LoadSmallI {
        dst: Reg(0),
        value: 63,
    });
}

#[test]
fn test_roundtrip_load_small_i_negative() {
    assert_roundtrip(&Instruction::LoadSmallI {
        dst: Reg(0),
        value: -64,
    });
}

// =============================================================================
// Integer Arithmetic Roundtrips
// =============================================================================

#[test]
fn test_roundtrip_binary_i_add() {
    assert_roundtrip(&Instruction::BinaryI {
        op: BinaryIntOp::Add,
        dst: Reg(0),
        a: Reg(1),
        b: Reg(2),
    });
}

#[test]
fn test_roundtrip_binary_i_sub() {
    assert_roundtrip(&Instruction::BinaryI {
        op: BinaryIntOp::Sub,
        dst: Reg(3),
        a: Reg(4),
        b: Reg(5),
    });
}

#[test]
fn test_roundtrip_binary_i_mul() {
    assert_roundtrip(&Instruction::BinaryI {
        op: BinaryIntOp::Mul,
        dst: Reg(0),
        a: Reg(1),
        b: Reg(2),
    });
}

#[test]
fn test_roundtrip_binary_i_div() {
    assert_roundtrip(&Instruction::BinaryI {
        op: BinaryIntOp::Div,
        dst: Reg(0),
        a: Reg(1),
        b: Reg(2),
    });
}

#[test]
fn test_roundtrip_binary_i_mod() {
    assert_roundtrip(&Instruction::BinaryI {
        op: BinaryIntOp::Mod,
        dst: Reg(0),
        a: Reg(1),
        b: Reg(2),
    });
}

#[test]
fn test_roundtrip_unary_i_neg() {
    assert_roundtrip(&Instruction::UnaryI {
        op: UnaryIntOp::Neg,
        dst: Reg(0),
        src: Reg(1),
    });
}

#[test]
fn test_roundtrip_unary_i_abs() {
    assert_roundtrip(&Instruction::UnaryI {
        op: UnaryIntOp::Abs,
        dst: Reg(0),
        src: Reg(1),
    });
}

// =============================================================================
// Float Arithmetic Roundtrips
// =============================================================================

#[test]
fn test_roundtrip_binary_f_add() {
    assert_roundtrip(&Instruction::BinaryF {
        op: BinaryFloatOp::Add,
        dst: Reg(0),
        a: Reg(1),
        b: Reg(2),
    });
}

#[test]
fn test_roundtrip_binary_f_sub() {
    assert_roundtrip(&Instruction::BinaryF {
        op: BinaryFloatOp::Sub,
        dst: Reg(0),
        a: Reg(1),
        b: Reg(2),
    });
}

#[test]
fn test_roundtrip_binary_f_mul() {
    assert_roundtrip(&Instruction::BinaryF {
        op: BinaryFloatOp::Mul,
        dst: Reg(0),
        a: Reg(1),
        b: Reg(2),
    });
}

#[test]
fn test_roundtrip_binary_f_div() {
    assert_roundtrip(&Instruction::BinaryF {
        op: BinaryFloatOp::Div,
        dst: Reg(0),
        a: Reg(1),
        b: Reg(2),
    });
}

#[test]
fn test_roundtrip_unary_f_neg() {
    assert_roundtrip(&Instruction::UnaryF {
        op: UnaryFloatOp::Neg,
        dst: Reg(0),
        src: Reg(1),
    });
}

#[test]
fn test_roundtrip_unary_f_abs() {
    assert_roundtrip(&Instruction::UnaryF {
        op: UnaryFloatOp::Abs,
        dst: Reg(0),
        src: Reg(1),
    });
}

// =============================================================================
// Comparison Roundtrips
// =============================================================================

#[test]
fn test_roundtrip_cmp_i_eq() {
    assert_roundtrip(&Instruction::CmpI {
        op: CompareOp::Eq,
        dst: Reg(0),
        a: Reg(1),
        b: Reg(2),
    });
}

#[test]
fn test_roundtrip_cmp_i_ne() {
    assert_roundtrip(&Instruction::CmpI {
        op: CompareOp::Ne,
        dst: Reg(0),
        a: Reg(1),
        b: Reg(2),
    });
}

#[test]
fn test_roundtrip_cmp_i_lt() {
    assert_roundtrip(&Instruction::CmpI {
        op: CompareOp::Lt,
        dst: Reg(0),
        a: Reg(1),
        b: Reg(2),
    });
}

#[test]
fn test_roundtrip_cmp_i_le() {
    assert_roundtrip(&Instruction::CmpI {
        op: CompareOp::Le,
        dst: Reg(0),
        a: Reg(1),
        b: Reg(2),
    });
}

#[test]
fn test_roundtrip_cmp_i_gt() {
    assert_roundtrip(&Instruction::CmpI {
        op: CompareOp::Gt,
        dst: Reg(0),
        a: Reg(1),
        b: Reg(2),
    });
}

#[test]
fn test_roundtrip_cmp_i_ge() {
    assert_roundtrip(&Instruction::CmpI {
        op: CompareOp::Ge,
        dst: Reg(0),
        a: Reg(1),
        b: Reg(2),
    });
}

#[test]
fn test_roundtrip_cmp_f_eq() {
    assert_roundtrip(&Instruction::CmpF {
        op: CompareOp::Eq,
        dst: Reg(0),
        a: Reg(1),
        b: Reg(2),
    });
}

#[test]
fn test_roundtrip_cmp_f_lt() {
    assert_roundtrip(&Instruction::CmpF {
        op: CompareOp::Lt,
        dst: Reg(0),
        a: Reg(1),
        b: Reg(2),
    });
}

// =============================================================================
// Control Flow Roundtrips
// =============================================================================

#[test]
fn test_roundtrip_jmp_forward() {
    assert_roundtrip(&Instruction::Jmp { offset: 10 });
}

#[test]
fn test_roundtrip_jmp_backward() {
    assert_roundtrip(&Instruction::Jmp { offset: -5 });
}

#[test]
fn test_roundtrip_jmp_if() {
    assert_roundtrip(&Instruction::JmpIf {
        cond: Reg(0),
        offset: 3,
    });
}

#[test]
fn test_roundtrip_jmp_not() {
    assert_roundtrip(&Instruction::JmpNot {
        cond: Reg(0),
        offset: -7,
    });
}

#[test]
fn test_roundtrip_ret() {
    assert_roundtrip(&Instruction::Ret { value: Reg(0) });
}

#[test]
fn test_roundtrip_ret_v() {
    assert_roundtrip(&Instruction::RetV);
}

#[test]
fn test_roundtrip_call() {
    assert_roundtrip(&Instruction::Call {
        dst: Reg(0),
        func_id: 42,
        args: RegRange::new(Reg(1), 3),
    });
}

#[test]
fn test_roundtrip_call_no_args() {
    assert_roundtrip(&Instruction::Call {
        dst: Reg(0),
        func_id: 0,
        args: RegRange::new(Reg(0), 0),
    });
}

// =============================================================================
// Bitwise Roundtrips
// =============================================================================

#[test]
fn test_roundtrip_bitwise_and() {
    assert_roundtrip(&Instruction::Bitwise {
        op: BitwiseOp::And,
        dst: Reg(0),
        a: Reg(1),
        b: Reg(2),
    });
}

#[test]
fn test_roundtrip_bitwise_or() {
    assert_roundtrip(&Instruction::Bitwise {
        op: BitwiseOp::Or,
        dst: Reg(0),
        a: Reg(1),
        b: Reg(2),
    });
}

#[test]
fn test_roundtrip_bitwise_xor() {
    assert_roundtrip(&Instruction::Bitwise {
        op: BitwiseOp::Xor,
        dst: Reg(0),
        a: Reg(1),
        b: Reg(2),
    });
}

#[test]
fn test_roundtrip_bitwise_shl() {
    assert_roundtrip(&Instruction::Bitwise {
        op: BitwiseOp::Shl,
        dst: Reg(0),
        a: Reg(1),
        b: Reg(2),
    });
}

#[test]
fn test_roundtrip_bitwise_shr() {
    assert_roundtrip(&Instruction::Bitwise {
        op: BitwiseOp::Shr,
        dst: Reg(0),
        a: Reg(1),
        b: Reg(2),
    });
}

// =============================================================================
// Boolean Logic Roundtrips
// =============================================================================

#[test]
fn test_roundtrip_not() {
    assert_roundtrip(&Instruction::Not {
        dst: Reg(0),
        src: Reg(1),
    });
}

// =============================================================================
// Type Conversion Roundtrips
// =============================================================================

// Note: CvtIF, CvtFI, CvtBI roundtrip tests omitted - known issue where
// the bytecode decoder returns Raw for conversion opcodes 0x0B-0x0F.

// =============================================================================
// Generic Arithmetic Roundtrips
// =============================================================================

#[test]
fn test_roundtrip_binary_g_add() {
    assert_roundtrip(&Instruction::BinaryG {
        op: BinaryGenericOp::Add,
        dst: Reg(0),
        a: Reg(1),
        b: Reg(2),
        protocol_id: 0,
    });
}

#[test]
fn test_roundtrip_binary_g_sub() {
    assert_roundtrip(&Instruction::BinaryG {
        op: BinaryGenericOp::Sub,
        dst: Reg(0),
        a: Reg(1),
        b: Reg(2),
        protocol_id: 0,
    });
}

// =============================================================================
// Memory / Object Roundtrips
// =============================================================================

#[test]
fn test_roundtrip_new() {
    assert_roundtrip(&Instruction::New {
        dst: Reg(0),
        type_id: 10,
        field_count: 3,
    });
}

#[test]
fn test_roundtrip_get_field() {
    assert_roundtrip(&Instruction::GetF {
        dst: Reg(0),
        obj: Reg(1),
        field_idx: 3,
    });
}

#[test]
fn test_roundtrip_set_field() {
    assert_roundtrip(&Instruction::SetF {
        obj: Reg(0),
        field_idx: 2,
        value: Reg(1),
    });
}

// =============================================================================
// Encoding size tests
// =============================================================================

#[test]
fn test_encoding_size_short_register() {
    let mut bytes = Vec::new();
    encode_instruction(
        &Instruction::Mov {
            dst: Reg(0),
            src: Reg(1),
        },
        &mut bytes,
    );
    // Opcode(1) + short_reg(1) + short_reg(1) = 3 bytes
    assert_eq!(bytes.len(), 3, "Mov with short registers should be 3 bytes");
}

#[test]
fn test_encoding_size_long_register() {
    let mut bytes = Vec::new();
    encode_instruction(
        &Instruction::Mov {
            dst: Reg(200),
            src: Reg(300),
        },
        &mut bytes,
    );
    // Opcode(1) + long_reg(2) + long_reg(2) = 5 bytes
    assert_eq!(bytes.len(), 5, "Mov with long registers should be 5 bytes");
}

#[test]
fn test_encoding_deterministic() {
    let instr = Instruction::LoadI {
        dst: Reg(5),
        value: 12345,
    };
    let mut bytes1 = Vec::new();
    let mut bytes2 = Vec::new();
    encode_instruction(&instr, &mut bytes1);
    encode_instruction(&instr, &mut bytes2);
    assert_eq!(bytes1, bytes2, "Same instruction must produce identical bytes");
}

// =============================================================================
// Batch roundtrip: multiple instructions in sequence
// =============================================================================

#[test]
fn test_roundtrip_instruction_sequence() {
    let instructions = vec![
        Instruction::LoadSmallI {
            dst: Reg(0),
            value: 10,
        },
        Instruction::LoadSmallI {
            dst: Reg(1),
            value: 20,
        },
        Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(2),
            a: Reg(0),
            b: Reg(1),
        },
        Instruction::Ret { value: Reg(2) },
    ];

    let mut bytes = Vec::new();
    for instr in &instructions {
        encode_instruction(instr, &mut bytes);
    }

    let mut offset = 0;
    for expected in &instructions {
        let decoded = decode_instruction(&bytes, &mut offset).expect("decode failed");
        assert_eq!(&decoded, expected);
    }
    assert_eq!(offset, bytes.len(), "All bytes should be consumed");
}

// =============================================================================
// Type Conversion Roundtrips (CvtIF, CvtFI, CvtBI, CvtIC, CvtCI, CvtToI, CvtToF)
// =============================================================================

#[test]
fn test_roundtrip_cvt_if() {
    assert_roundtrip(&Instruction::CvtIF {
        dst: Reg(0),
        src: Reg(1),
    });
}

#[test]
fn test_roundtrip_cvt_if_high_registers() {
    assert_roundtrip(&Instruction::CvtIF {
        dst: Reg(200),
        src: Reg(255),
    });
}

#[test]
fn test_roundtrip_cvt_fi_trunc() {
    assert_roundtrip(&Instruction::CvtFI {
        mode: FloatToIntMode::Trunc,
        dst: Reg(0),
        src: Reg(1),
    });
}

#[test]
fn test_roundtrip_cvt_fi_floor() {
    assert_roundtrip(&Instruction::CvtFI {
        mode: FloatToIntMode::Floor,
        dst: Reg(2),
        src: Reg(3),
    });
}

#[test]
fn test_roundtrip_cvt_fi_ceil() {
    assert_roundtrip(&Instruction::CvtFI {
        mode: FloatToIntMode::Ceil,
        dst: Reg(4),
        src: Reg(5),
    });
}

#[test]
fn test_roundtrip_cvt_fi_round() {
    assert_roundtrip(&Instruction::CvtFI {
        mode: FloatToIntMode::Round,
        dst: Reg(6),
        src: Reg(7),
    });
}

#[test]
fn test_roundtrip_cvt_ic() {
    assert_roundtrip(&Instruction::CvtIC {
        dst: Reg(0),
        src: Reg(1),
    });
}

#[test]
fn test_roundtrip_cvt_ci() {
    assert_roundtrip(&Instruction::CvtCI {
        dst: Reg(0),
        src: Reg(1),
    });
}

#[test]
fn test_roundtrip_cvt_bi() {
    assert_roundtrip(&Instruction::CvtBI {
        dst: Reg(0),
        src: Reg(1),
    });
}

#[test]
fn test_roundtrip_cvt_bi_high_registers() {
    assert_roundtrip(&Instruction::CvtBI {
        dst: Reg(128),
        src: Reg(200),
    });
}

#[test]
fn test_roundtrip_cvt_toi() {
    assert_roundtrip(&Instruction::CvtToI {
        dst: Reg(0),
        src: Reg(1),
    });
}

#[test]
fn test_roundtrip_cvt_tof() {
    assert_roundtrip(&Instruction::CvtToF {
        dst: Reg(0),
        src: Reg(1),
    });
}

#[test]
fn test_roundtrip_all_conversion_modes() {
    // Verify all FloatToIntMode variants survive roundtrip
    for mode in [
        FloatToIntMode::Trunc,
        FloatToIntMode::Floor,
        FloatToIntMode::Ceil,
        FloatToIntMode::Round,
    ] {
        let instr = Instruction::CvtFI {
            mode,
            dst: Reg(10),
            src: Reg(20),
        };
        let decoded = roundtrip(&instr);
        assert_eq!(decoded, instr, "Roundtrip failed for CvtFI mode {:?}", mode);
    }
}

#[test]
fn test_roundtrip_conversion_sequence() {
    // Encode multiple conversions in sequence and verify they all decode correctly
    let instructions = vec![
        Instruction::CvtIF { dst: Reg(0), src: Reg(1) },
        Instruction::CvtFI { mode: FloatToIntMode::Trunc, dst: Reg(2), src: Reg(3) },
        Instruction::CvtIC { dst: Reg(4), src: Reg(5) },
        Instruction::CvtCI { dst: Reg(6), src: Reg(7) },
        Instruction::CvtBI { dst: Reg(8), src: Reg(9) },
        Instruction::CvtToI { dst: Reg(10), src: Reg(11) },
        Instruction::CvtToF { dst: Reg(12), src: Reg(13) },
    ];

    let mut bytes = Vec::new();
    for instr in &instructions {
        encode_instruction(instr, &mut bytes);
    }

    let mut offset = 0;
    for expected in &instructions {
        let decoded = decode_instruction(&bytes, &mut offset).expect("decode failed");
        assert_eq!(&decoded, expected, "Sequence roundtrip failed");
    }
    assert_eq!(offset, bytes.len(), "All bytes should be consumed");
}
