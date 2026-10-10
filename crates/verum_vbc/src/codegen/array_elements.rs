//! Declared element meaning, independent of the producer's physical storage.
use super::VbcCodegen;
use crate::encoding::encode_reg;
use crate::instruction::{ArithSubOpcode, Instruction, Reg};
use crate::types::{TypeId, TypeKind, TypeRef};
use verum_common::{List, Maybe, Set};

impl VbcCodegen {
    /// Only canonical signed IDs, including exact declaration-owned aliases,
    /// authorize normalization. A same-spelled record or generic parameter does
    /// not become a signed scalar merely because its storage has that width.
    pub(super) fn array_element_signed_bits(&self, element: &TypeRef) -> Maybe<u8> {
        let mut current = element;
        let mut seen = Set::new();
        loop {
            let TypeRef::Concrete(id) = current else {
                return None;
            };
            match *id {
                TypeId::I8 => return Some(8),
                TypeId::I16 => return Some(16),
                TypeId::I32 => return Some(32),
                _ => {}
            }
            if !seen.insert(*id) {
                return None;
            }
            let descriptor = self.type_by_id(*id)?;
            if descriptor.kind != TypeKind::Alias {
                return None;
            }
            current = descriptor.alias_target.as_ref()?;
        }
    }

    /// Packed reads zero-extend. Normalize using semantic element authority;
    /// this is also idempotent for already signed boxed List elements.
    pub(super) fn emit_signed_array_element(&mut self, value: Reg, bits: Maybe<u8>) {
        let Some(bits) = bits else { return };
        let mut operands = List::with_capacity(6).into();
        encode_reg(value, &mut operands);
        encode_reg(value, &mut operands);
        operands.extend([bits, 64]);
        self.ctx.emit(Instruction::ArithExtended {
            sub_op: ArithSubOpcode::SextI.to_byte(),
            operands,
        });
    }
}

#[cfg(test)]
#[path = "../../tests/codegen/array_element_semantics.rs"]
mod tests;
