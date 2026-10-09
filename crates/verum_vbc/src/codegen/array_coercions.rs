//! Materialize language coercions using shared emitted storage facts.
use super::{CodegenResult, VbcCodegen};
use crate::array_storage::ArrayResultFact;
use crate::instruction::Reg;
use crate::types::{TypeId, TypeRef};

pub(super) use crate::array_storage::ArrayResultFacts;

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
