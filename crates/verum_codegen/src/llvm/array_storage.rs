//! Physical array results selected from the final native callable, not its name
//! or declared array shape. This first consumer slice is straight-line only.
use verum_common::{Map, Maybe, Set};
use verum_llvm::values::FunctionValue;
use verum_vbc::{
    array_storage::{ArrayResultFact, ArrayResultFacts, straight_line_array_return},
    instruction::{Instruction, MemSubOpcode, Reg},
    module::VbcModule,
    types::TypeRef,
};

use super::error::{LlvmLoweringError, Result};

pub(super) type ArrayReturns<'ctx> = Map<FunctionValue<'ctx>, ArrayResultFact>;

/// The backend's existing function-ID map supplies the actual LLVM target. If
/// two descriptors share that target, this bounded analysis supplies no proof;
/// it does not invent another duplicate-body or name-resolution rule.
pub(super) fn selected_source_returns<'ctx>(
    module: &VbcModule,
    selected: impl Fn(u32) -> Maybe<FunctionValue<'ctx>>,
) -> ArrayReturns<'ctx> {
    let mut owners = Map::new();
    for (index, descriptor) in module.functions.iter().enumerate() {
        let Some(target) = selected(descriptor.id.0) else {
            continue;
        };
        owners
            .entry(target)
            .and_modify(|owner| *owner = None)
            .or_insert(Some(index));
    }
    owners
        .into_iter()
        .filter_map(|(target, owner)| {
            let descriptor = &module.functions[owner?];
            if !descriptor.has_source_body
                || !matches!(descriptor.return_type, TypeRef::Array { .. })
                || target.count_basic_blocks() != 0
                || target.count_params() as usize != descriptor.params.len()
            {
                return None;
            }
            // A pre-existing native definition or arity-mismatched declaration
            // cannot be attributed to this source body. Native lowering consumes
            // these very decoded instructions. A wire
            // descriptor without a loaded body cannot establish executable proof.
            let body = descriptor.instructions.as_deref()?;
            straight_line_array_return(body).map(|fact| (target, fact))
        })
        .collect()
}

#[derive(Default)]
pub(super) struct ArrayStorage {
    // Facts describe VBC instruction inputs. Native helper set_register calls
    // do not advance them; finish_instruction performs one atomic transfer.
    facts: ArrayResultFacts,
    array_values: Set<Reg>,
    pending_result: Maybe<(Reg, ArrayResultFact)>,
    moved_array: Maybe<Reg>,
    has_generic_access: bool,
    straight_line: bool,
}

impl ArrayStorage {
    pub(super) fn for_body(body: &[Instruction]) -> Self {
        let mut probe = ArrayResultFacts::default();
        let straight_line = body.iter().all(|instruction| match instruction {
            Instruction::Call { .. }
            | Instruction::CallG { .. }
            | Instruction::CallM { .. }
            // Destruction can run user glue, just like a call, but it does not
            // branch within this VBC body. Instruction transfer still discards
            // every fact at DropRef; a later read needs a fresh producer proof.
            | Instruction::DropRef { .. }
            | Instruction::Ret { .. }
            | Instruction::RetV => true,
            _ => probe.observe(instruction),
        });
        Self {
            has_generic_access: body.iter().any(|i| {
                matches!(
                    i,
                    Instruction::GetE { .. } | Instruction::SetE { .. } | Instruction::Len { .. }
                )
            }),
            straight_line,
            ..Self::default()
        }
    }

    pub(super) fn check_parameters(
        &self,
        parameters: &[verum_vbc::module::ParamDescriptor],
    ) -> Result<()> {
        let has_unproved_array = parameters.iter().any(|parameter| {
            let mut ty = &parameter.type_ref;
            while let TypeRef::Reference { inner, .. } = ty {
                ty = inner;
            }
            matches!(ty, TypeRef::Array { .. })
        });
        if self.has_generic_access && has_unproved_array {
            return Err(LlvmLoweringError::UnprovenArrayStorage(
                "fixed-array parameter needs a selected argument storage contract".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn get(&self, register: Reg) -> Maybe<ArrayResultFact> {
        self.facts.get(register)
    }

    pub(super) fn is_array(&self, register: Reg) -> bool {
        self.array_values.contains(&register)
    }

    pub(super) fn forget_value(&mut self, register: Reg) {
        self.array_values.remove(&register);
    }

    pub(super) fn mark_array(&mut self, register: Reg) {
        self.array_values.insert(register);
    }

    pub(super) fn begin_instruction(&mut self, instruction: &Instruction) {
        self.pending_result = None;
        self.moved_array = match instruction {
            Instruction::Mov { dst, src } if self.is_array(*src) => Some(*dst),
            _ => None,
        };
    }

    pub(super) fn call_result(
        &mut self,
        destination: Reg,
        fact: Maybe<ArrayResultFact>,
    ) -> Result<()> {
        // A CFG meet is not implemented by the legacy native register maps.
        // Refuse the entire callable instead of letting a later branch's List
        // mark authorize an earlier packed/unknown value's container probe.
        if fact != Some(ArrayResultFact::List) && self.has_generic_access && !self.straight_line {
            return Err(LlvmLoweringError::UnprovenArrayStorage(
                "array call result crosses unsupported native control flow".into(),
            ));
        }
        self.mark_array(destination);
        self.pending_result = fact.map(|fact| (destination, fact));
        Ok(())
    }

    pub(super) fn finish_instruction(&mut self, instruction: &Instruction) -> Result<()> {
        self.facts.observe(instruction);
        if let Some((destination, fact)) = self.pending_result.take() {
            self.facts.record_selected_result(destination, fact);
        }
        if let Some(destination) = self.moved_array.take() {
            self.mark_array(destination);
        }
        if let Instruction::MemExtended { sub_op, operands } = instruction
            && matches!(
                MemSubOpcode::from_byte(*sub_op),
                Some(MemSubOpcode::NewByteArray | MemSubOpcode::NewTypedArray)
            )
        {
            let mut cursor = 0;
            if let Ok(destination) = verum_vbc::encoding::decode_reg(operands, &mut cursor) {
                if self.has_generic_access && !self.straight_line {
                    return Err(LlvmLoweringError::UnprovenArrayStorage(
                        "packed array read crosses unsupported native control flow".into(),
                    ));
                }
                self.mark_array(destination);
            }
        }
        Ok(())
    }

    pub(super) fn clear_at_join(&mut self) {
        self.facts.clear();
    }
}

#[cfg(test)]
#[path = "../../tests/unit/array_storage.rs"]
mod tests;
