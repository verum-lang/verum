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

/// A fresh packed allocation can cross an opaque call while no argument,
/// address-taken register or external store can reach it. Track all register
/// aliases of each allocation, so publishing any one retires the entire fact.
#[derive(Default)]
struct PrivatePackedArrays {
    aliases: Map<Reg, (usize, ArrayResultFact)>,
    address_taken: Set<Reg>,
    instruction: usize,
}

impl PrivatePackedArrays {
    fn expose_value(&mut self, register: Reg) {
        if let Some(&(owner, _)) = self.aliases.get(&register) {
            self.aliases.retain(|_, (other, _)| *other != owner);
        }
    }

    fn expose_place(&mut self, register: Reg) {
        self.expose_value(register);
        self.address_taken.insert(register);
    }

    fn assign(&mut self, register: Reg, value: Maybe<(usize, ArrayResultFact)>) {
        self.aliases.remove(&register);
        if let Some(value) = value {
            self.aliases.insert(register, value);
            if self.address_taken.contains(&register) {
                self.expose_value(register);
            }
        }
    }

    fn observe(&mut self, op: &Instruction, facts: &ArrayResultFacts) -> bool {
        use Instruction as I;
        self.instruction += 1;
        match op {
            I::LoadI { dst, .. } | I::LoadSmallI { dst, .. } | I::LoadK { dst, .. }
            | I::LoadUnit { dst } | I::LoadTrue { dst } | I::LoadFalse { dst } => self.assign(*dst, None),
            I::Mov { dst, src } => self.assign(*dst, self.aliases.get(src).copied()),
            I::Ref { dst, src } | I::RefMut { dst, src } | I::RefLocal { dst, src }
            | I::RefObj { dst, src } => {
                self.expose_place(*src);
                self.assign(*dst, None);
            }
            I::Call { dst, args, .. } | I::CallG { dst, args, .. } | I::CallM { dst, args, .. } => {
                if usize::from(args.count) > 64
                    || args.start.0.checked_add(u16::from(args.count.saturating_sub(1))).is_none()
                {
                    return false;
                }
                if let I::CallM { receiver, .. } = op {
                    self.expose_place(*receiver);
                }
                for argument in args.iter() {
                    self.expose_place(argument);
                }
                self.assign(*dst, None);
            }
            I::DropTyped { src, type_ref } if verum_vbc::drop_plan::has_trivial_cleanup(type_ref) => {
                self.assign(*src, None);
            }
            I::BinaryI { dst, a, b, .. } | I::CmpI { dst, a, b, .. } | I::CmpU { dst, a, b, .. } => {
                self.expose_value(*a);
                self.expose_value(*b);
                self.assign(*dst, None);
            }
            I::Assert { .. } | I::Nop => {}
            I::MemExtended { sub_op, operands } => {
                let Some(layout) = verum_vbc::encoding::PackedArrayLayout::for_subopcode(*sub_op) else {
                    return false;
                };
                let Ok(array) = layout.decode(operands, &mut 0) else { return false };
                let [first, second, last] = array.regs;
                match MemSubOpcode::from_byte(*sub_op) {
                    Some(MemSubOpcode::NewByteArray | MemSubOpcode::NewTypedArray) => {
                        self.expose_value(second);
                        self.expose_value(last);
                        let private = facts.get(first).and_then(|fact| {
                            matches!(fact, ArrayResultFact::Packed { .. }).then_some((self.instruction, fact))
                        });
                        self.assign(first, private);
                    }
                    Some(MemSubOpcode::ByteArrayStore | MemSubOpcode::TypedArrayStore) => {
                        self.expose_value(second);
                        self.expose_value(last);
                    }
                    Some(MemSubOpcode::ByteArrayLoad | MemSubOpcode::TypedArrayLoad) => {
                        self.expose_value(last);
                        self.assign(first, None);
                    }
                    _ => return false,
                }
            }
            // An unsupported operation may publish a value or the address of a
            // frame register. A later allocation cannot erase that exposure.
            _ => return false,
        }
        self.aliases.len() <= 64 && self.address_taken.len() <= 64
    }
}

#[derive(Default)]
pub(super) struct ArrayStorage {
    // Facts describe VBC instruction inputs. Native helper set_register calls
    // do not advance them; finish_instruction performs one atomic transfer.
    facts: ArrayResultFacts,
    private_packed: Maybe<PrivatePackedArrays>,
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
        let has_packed_producer = body.iter().any(|op| matches!(op,
            Instruction::MemExtended { sub_op, .. } if matches!(MemSubOpcode::from_byte(*sub_op),
                Some(MemSubOpcode::NewByteArray | MemSubOpcode::NewTypedArray))));
        Self {
            private_packed: (body.len() <= 1024 && has_packed_producer
                && super::vbc_lowering::has_linear_fallthrough(body))
                .then(PrivatePackedArrays::default),
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
        self.facts.get(register).or_else(|| {
            self.private_packed.as_ref()?.aliases.get(&register).map(|&(_, fact)| fact)
        })
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
            Instruction::Mov { dst, src }
            | Instruction::CloneTyped { dst, src, type_ref: TypeRef::Array { .. } }
                if self.is_array(*src) => Some(*dst),
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
        if self.private_packed.as_mut().is_some_and(|private| !private.observe(instruction, &self.facts)) {
            self.private_packed = None;
        }
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
        self.private_packed = None;
    }
}

#[cfg(test)]
#[path = "../../tests/unit/array_storage.rs"]
mod tests;
