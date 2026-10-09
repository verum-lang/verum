//! Emission-derived array storage facts shared by VBC codegen and native lowering.
//! This module describes actual producers, not semantic array types or ownership.
use crate::encoding::decode_reg;
use crate::instruction::{Instruction, MemSubOpcode, Reg};
use verum_common::{Map, Maybe};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Storage or integer constant established by an emitted producer.
pub enum ArrayResultFact {
    /// Growable List object, not a packed payload.
    List,
    /// Packed allocation with checked geometry and a known element count.
    Packed {
        /// Bytes per element.
        width: usize,
        /// Elements use IEEE floating-point encoding.
        float: bool,
        /// Exact allocated element count.
        count: u64,
    },
    /// Integer constant used to establish allocation length.
    Integer(i64),
}

/// Conservative straight-line result facts. Unknown producers, calls and control
/// flow discard proof; they never authorize a raw read. Facts follow actual Mov
/// instructions, so block result handoff does not depend on a departed binding.
#[derive(Clone, Debug, Default)]
pub struct ArrayResultFacts {
    values: Map<Reg, ArrayResultFact>,
}

impl ArrayResultFacts {
    /// The current value in this register has this proven producer.
    pub fn get(&self, register: Reg) -> Maybe<ArrayResultFact> {
        self.values.get(&register).copied()
    }

    /// Discard proof across an unknown execution boundary.
    pub fn clear(&mut self) {
        self.values.clear();
    }

    /// Discard the fact when this register is overwritten.
    pub fn forget(&mut self, register: Reg) {
        self.values.remove(&register);
    }

    /// Advance proof using the actual emitted instruction.
    pub fn observe(&mut self, instruction: &Instruction) {
        use Instruction as I;
        match instruction {
            I::Mov { dst, src } => {
                let fact = self.get(*src);
                self.forget(*dst);
                if let Some(fact) = fact {
                    self.values.insert(*dst, fact);
                }
            }
            I::LoadI { dst, value } => {
                self.values.insert(*dst, ArrayResultFact::Integer(*value));
            }
            I::LoadSmallI { dst, value } => {
                self.values
                    .insert(*dst, ArrayResultFact::Integer(i64::from(*value)));
            }
            I::NewList { dst, .. } => {
                self.values.insert(*dst, ArrayResultFact::List);
            }
            I::MemExtended { sub_op, operands } => {
                let Some(op) = MemSubOpcode::from_byte(*sub_op) else {
                    self.clear();
                    return;
                };
                if matches!(
                    op,
                    MemSubOpcode::ByteArrayStore | MemSubOpcode::TypedArrayStore
                ) {
                    return; // Element stores do not replace a register or resize a fixed array.
                }
                let mut cursor = 0;
                let Ok(dst) = decode_reg(operands, &mut cursor) else {
                    self.clear();
                    return;
                };
                if matches!(
                    op,
                    MemSubOpcode::ByteArrayLoad | MemSubOpcode::TypedArrayLoad
                ) {
                    self.forget(dst);
                    return;
                }
                if !matches!(op, MemSubOpcode::NewByteArray | MemSubOpcode::NewTypedArray) {
                    self.clear();
                    return;
                }
                let count =
                    decode_reg(operands, &mut cursor)
                        .ok()
                        .and_then(|reg| match self.get(reg) {
                            Some(ArrayResultFact::Integer(n)) => u64::try_from(n).ok(),
                            _ => None,
                        });
                let geometry = if op == MemSubOpcode::NewByteArray {
                    Some((1, false))
                } else {
                    operands.get(cursor).copied().map(|encoded| {
                        cursor += 1;
                        (usize::from(encoded & 0x7f), encoded & 0x80 != 0)
                    })
                };
                let valid_init = decode_reg(operands, &mut cursor).is_ok();
                self.forget(dst);
                if let (Some(count), Some((width, float))) = (count, geometry)
                    && valid_init
                    && cursor == operands.len()
                    && matches!(width, 1 | 2 | 4 | 8)
                    && (!float || matches!(width, 4 | 8))
                    && count
                        .checked_mul(width as u64)
                        .is_some_and(|n| n <= isize::MAX as u64)
                {
                    self.values.insert(
                        dst,
                        ArrayResultFact::Packed {
                            width,
                            float,
                            count,
                        },
                    );
                }
            }
            I::LoadK { dst, .. }
            | I::LoadF { dst, .. }
            | I::LoadTrue { dst }
            | I::LoadFalse { dst }
            | I::LoadUnit { dst }
            | I::LoadT { dst, .. }
            | I::BinaryI { dst, .. }
            | I::BinaryF { dst, .. }
            | I::UnaryI { dst, .. }
            | I::UnaryF { dst, .. }
            | I::Bitwise { dst, .. }
            | I::CmpI { dst, .. }
            | I::CmpF { dst, .. }
            | I::CmpU { dst, .. }
            | I::Not { dst, .. }
            | I::CvtIF { dst, .. }
            | I::CvtFI { dst, .. } => self.forget(*dst),
            // Dropping can invoke user glue, with the same unknown side effects
            // as a call. It is not merely a register overwrite.
            I::DropRef { .. } => self.clear(),
            I::Nop | I::ListPush { .. } => {}
            // Clones, reference accesses and calls need their selected producer
            // contract. Branch/loop joins need a meet, not last-emitted-wins.
            _ => self.clear(),
        }
    }
}

/// Summarize one completed, straight-line executable body. This establishes the
/// returned storage only; the caller must independently prove that this exact
/// body is selected. Declarations, stubs and signatures supply no storage fact.
///
/// Branches, multiple returns, parameter forwarding and unknown returned values
/// remain unproved. No last-emitted-wins rule crosses a control-flow join. Calls
/// clear facts, but a subsequent independent allocation can establish a result.
pub fn straight_line_array_return(instructions: &[Instruction]) -> Maybe<ArrayResultFact> {
    if instructions.iter().any(|instruction| {
        matches!(
            instruction,
            Instruction::Jmp { .. }
                | Instruction::JmpIf { .. }
                | Instruction::JmpNot { .. }
                | Instruction::JmpCmp { .. }
                | Instruction::Switch { .. }
                | Instruction::TryBegin { .. }
        )
    }) {
        return None;
    }
    let mut facts = ArrayResultFacts::default();
    let mut result = None;
    let mut returned = false;
    for instruction in instructions {
        match instruction {
            Instruction::Ret { value } => {
                if returned {
                    return None;
                }
                returned = true;
                result = match facts.get(*value) {
                    Some(fact @ (ArrayResultFact::List | ArrayResultFact::Packed { .. })) => {
                        Some(fact)
                    }
                    _ => None,
                };
            }
            Instruction::RetV => return None,
            Instruction::Nop if returned => {}
            _ if returned => return None,
            _ => facts.observe(instruction),
        }
    }
    result
}

#[cfg(test)]
#[path = "../tests/codegen/array_result_facts.rs"]
mod tests;
