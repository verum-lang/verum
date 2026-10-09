//! Physical array results come from emitted producers, never a declared array type.
use super::{CodegenResult, VbcCodegen};
use crate::encoding::decode_reg;
use crate::instruction::{Instruction, MemSubOpcode, Reg};
use crate::types::{TypeId, TypeRef};
use verum_common::{Map, Maybe};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ArrayResultFact {
    List,
    Packed {
        width: usize,
        float: bool,
        count: u64,
    },
    Integer(i64),
}

/// Conservative straight-line result facts. Unknown producers, calls and control
/// flow discard proof; they never authorize a raw read. Facts follow actual Mov
/// instructions, so block result handoff does not depend on a departed binding.
#[derive(Clone, Debug, Default)]
pub(super) struct ArrayResultFacts {
    values: Map<Reg, ArrayResultFact>,
}

impl ArrayResultFacts {
    pub(super) fn get(&self, register: Reg) -> Maybe<ArrayResultFact> {
        self.values.get(&register).copied()
    }

    pub(super) fn clear(&mut self) {
        self.values.clear();
    }

    fn forget(&mut self, register: Reg) {
        self.values.remove(&register);
    }

    pub(super) fn observe(&mut self, instruction: &Instruction) {
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

impl VbcCodegen {
    /// Only the callable's structural contract selects conversion. Expression
    /// hints belong to initializers/arguments/fields and are not return targets.
    pub(super) fn materialize_list_return(&mut self, source: Reg) -> CodegenResult<Reg> {
        let returns_list = match self.ctx.return_type.as_ref() {
            Some(TypeRef::Instantiated { base, .. }) | Some(TypeRef::Concrete(base)) => {
                *base == TypeId::LIST
            }
            _ => false,
        };
        if returns_list
            && let Some(ArrayResultFact::Packed {
                width,
                float,
                count,
            }) = self.ctx.array_result_facts.get(source)
        {
            // Zero is known empty storage, which still needs a growable List.
            self.emit_unpack_packed_into_list(source, width, float, count)
        } else {
            Ok(source)
        }
    }
}

#[cfg(test)]
#[path = "../../tests/codegen/array_result_facts.rs"]
mod tests;
