//! Producer-proof safety controls, including malformed geometry and wide registers.
use super::{ArrayResultFact, ArrayResultFacts};
use crate::encoding::encode_reg;
use crate::instruction::{Instruction, MemSubOpcode, Reg};

fn allocation(dst: Reg, count: Reg, init: Reg, geometry: Option<u8>) -> Instruction {
    // The canonical wire encoder and MemExtended operand API take this buffer.
    let mut operands = vec![];
    encode_reg(dst, &mut operands);
    encode_reg(count, &mut operands);
    if let Some(encoded) = geometry {
        operands.push(encoded);
    }
    encode_reg(init, &mut operands);
    Instruction::MemExtended {
        sub_op: if geometry.is_some() {
            MemSubOpcode::NewTypedArray
        } else {
            MemSubOpcode::NewByteArray
        }
        .to_byte(),
        operands,
    }
}

#[test]
fn canonical_wide_registers_and_empty_allocations_carry_exact_geometry() {
    for (count, geometry, width, float) in [
        (0, None, 1, false),
        (3, Some(4), 4, false),
        (2, Some(0x88), 8, true),
    ] {
        let mut facts = ArrayResultFacts::default();
        facts.observe(&Instruction::LoadI {
            dst: Reg(128),
            value: count,
        });
        facts.observe(&allocation(Reg(270), Reg(128), Reg(257), geometry));
        facts.observe(&Instruction::Mov {
            dst: Reg(300),
            src: Reg(270),
        });
        assert_eq!(
            facts.get(Reg(300)),
            Some(ArrayResultFact::Packed {
                width,
                float,
                count: count as u64
            })
        );
    }
}

#[test]
fn unsupported_float_geometry_cannot_authorize_a_packed_load() {
    for encoded in [0x81, 0x82, 0, 3, 16] {
        let mut facts = ArrayResultFacts::default();
        facts.observe(&Instruction::LoadI {
            dst: Reg(128),
            value: 2,
        });
        facts.observe(&allocation(Reg(270), Reg(128), Reg(257), Some(encoded)));
        assert_eq!(facts.get(Reg(270)), None, "geometry {encoded:#x}");
    }
}

#[test]
fn truncated_and_trailing_operands_cannot_authorize_storage() {
    for truncated in [true, false] {
        let mut facts = ArrayResultFacts::default();
        facts.observe(&Instruction::LoadI {
            dst: Reg(128),
            value: 2,
        });
        let mut instruction = allocation(Reg(270), Reg(128), Reg(257), None);
        if let Instruction::MemExtended { operands, .. } = &mut instruction {
            if truncated {
                operands.pop();
            } else {
                operands.push(0);
            }
        }
        facts.observe(&instruction);
        assert_eq!(facts.get(Reg(270)), None);
    }
}

#[test]
fn unknown_counts_overwrites_and_control_flow_discard_proof() {
    for count in [-1, i64::MAX] {
        let mut facts = ArrayResultFacts::default();
        facts.observe(&Instruction::LoadI {
            dst: Reg(128),
            value: count,
        });
        facts.observe(&allocation(Reg(270), Reg(128), Reg(257), Some(8)));
        assert_eq!(facts.get(Reg(270)), None);
    }
    let mut facts = ArrayResultFacts::default();
    facts.observe(&Instruction::LoadI {
        dst: Reg(128),
        value: 2,
    });
    facts.observe(&allocation(Reg(270), Reg(128), Reg(257), None));
    facts.observe(&Instruction::LoadTrue { dst: Reg(270) });
    assert_eq!(facts.get(Reg(270)), None);
    facts.observe(&Instruction::LoadTrue { dst: Reg(128) });
    facts.observe(&allocation(Reg(270), Reg(128), Reg(257), None));
    assert_eq!(facts.get(Reg(270)), None);
    facts.observe(&Instruction::LoadI {
        dst: Reg(128),
        value: 2,
    });
    facts.observe(&allocation(Reg(270), Reg(128), Reg(257), None));
    facts.observe(&Instruction::Jmp { offset: 0 });
    assert_eq!(facts.get(Reg(270)), None);
}

#[test]
fn drop_glue_cannot_preserve_other_storage_proofs() {
    let mut facts = ArrayResultFacts::default();
    facts.observe(&Instruction::LoadI {
        dst: Reg(128),
        value: 2,
    });
    facts.observe(&allocation(Reg(270), Reg(128), Reg(257), None));
    facts.observe(&Instruction::DropRef { src: Reg(1) });
    assert_eq!(facts.get(Reg(270)), None);
}
