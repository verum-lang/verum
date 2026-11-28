//! Emission-derived array storage facts shared by VBC codegen and native lowering.
//! This module describes actual producers, not semantic array types or ownership.
use crate::encoding::decode_reg;
use crate::instruction::{ArithSubOpcode, Instruction, MemSubOpcode, Reg};
use crate::types::TypeRef;
use verum_common::{Map, Maybe, Set};

/// Physical scalar encoding allowed by a fixed array's declared element type.
/// A compatible allocation or producer must still establish the actual layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackedElement {
    /// Bytes in an element in this encoding.
    pub width: usize,
    /// Floating-point bits require a different decoder from equally wide integers.
    pub float: bool,
}

impl PackedElement {
    /// An inline scalar has no nested ownership and uses a supported VBC width.
    pub fn for_element(element: &TypeRef) -> Maybe<Self> {
        let TypeRef::Concrete(id) = element else { return None; };
        if !id.is_numeric() {
            return None;
        }
        let width = verum_common::layout::primitive_size_by_name(id.well_known_name()?)?;
        matches!(width, 1 | 2 | 4 | 8).then_some(Self { width: width as usize, float: id.is_float() })
    }
    /// Resolve the declared element for both source allocation and typed copies.
    /// The fast path for builtins requires no descriptor or substitution.
    pub(crate) fn for_declaration<'a>(
        element: &TypeRef,
        lookup: impl FnMut(crate::TypeId) -> Option<&'a crate::types::TypeDescriptor>,
    ) -> Maybe<Self> {
        if let TypeRef::Concrete(id) = element && id.is_builtin() {
            return Self::for_element(element);
        }
        let expanded = element.clone().expand_aliases_while(lookup, |_| true)?;
        Self::for_element(&expanded)
    }

}

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
    // The allocation is still empty across register moves. Writes and escapes
    // invalidate all aliases without changing their physical List layout.
    fresh_lists: Set<Reg>,
}

impl ArrayResultFacts {
    /// The current value in this register has this proven producer.
    pub fn get(&self, register: Reg) -> Maybe<ArrayResultFact> {
        self.values.get(&register).copied()
    }

    /// The value still comes from an empty List allocation and has not escaped.
    #[cfg(any(feature = "codegen", test))]
    pub(crate) fn is_fresh_list(&self, register: Reg) -> bool {
        self.fresh_lists.contains(&register)
    }

    /// Discard proof across an unknown execution boundary.
    pub fn clear(&mut self) {
        self.values.clear();
        self.fresh_lists.clear();
    }

    /// Discard the fact when this register is overwritten.
    pub fn forget(&mut self, register: Reg) {
        self.values.remove(&register);
        self.fresh_lists.remove(&register);
    }

    /// Advance proof using the actual emitted instruction. Returns whether the
    /// instruction has an explicitly supported straight-line transfer. A body
    /// summary must reject every other instruction, even if a later allocation
    /// could establish a new local fact: unknown control flow may skip it.
    pub fn observe(&mut self, instruction: &Instruction) -> bool {
        use Instruction as I;
        match instruction {
            I::Mov { dst, src } => {
                let fact = self.get(*src);
                let fresh = self.fresh_lists.contains(src);
                self.forget(*dst);
                if let Some(fact) = fact {
                    self.values.insert(*dst, fact);
                    if fresh {
                        self.fresh_lists.insert(*dst);
                    }
                }
            }
            I::CloneTyped { dst, src, type_ref: TypeRef::Array { element, length } } => {
                let copied = match self.get(*src) {
                    Some(ArrayResultFact::List) => {
                        // Copying an element can run its retain implementation.
                        // Only the new List is established after arbitrary user code.
                        let immediate = matches!(element.as_ref(), TypeRef::Concrete(owner)
                            if matches!(crate::copy_plan::builtin_copy_node(*owner),
                                Some(crate::copy_plan::CopyNode::Identity)));
                        if !immediate {
                            self.clear();
                        }
                        ArrayResultFact::List
                    }
                    Some(fact @ ArrayResultFact::Packed { width, float, count })
                        if count == *length && PackedElement::for_element(element)
                            == Some(PackedElement { width, float }) => fact,
                    _ => {
                        self.clear();
                        return false;
                    }
                };
                self.record_selected_result(*dst, copied);
            }
            I::CloneTyped { dst, src, type_ref: TypeRef::Concrete(owner) }
                if matches!(crate::copy_plan::builtin_copy_node(*owner), Some(crate::copy_plan::CopyNode::Identity)) =>
            {
                // A completed primitive copy can carry a scalar constant. It
                // cannot certify packed or List storage for its destination.
                let source = self.get(*src);
                self.forget(*dst);
                if let Some(value @ ArrayResultFact::Integer(_)) = source {
                    self.values.insert(*dst, value);
                }
            }
            I::DropTyped { src, type_ref: TypeRef::Concrete(owner) }
                if matches!(crate::drop_plan::builtin_drop_plan(*owner), Some(crate::drop_plan::DropPlan::Ignore)) =>
            {
                self.forget(*src);
            }
            I::LoadI { dst, value } => {
                self.record_selected_result(*dst, ArrayResultFact::Integer(*value));
            }
            I::LoadSmallI { dst, value } => {
                self.record_selected_result(*dst, ArrayResultFact::Integer(i64::from(*value)));
            }
            I::NewList { dst, .. } => {
                self.values.insert(*dst, ArrayResultFact::List);
                self.fresh_lists.insert(*dst);
            }
            I::MemExtended { sub_op, operands } => {
                let Some(layout) = crate::encoding::PackedArrayLayout::for_subopcode(*sub_op) else {
                    self.clear();
                    return false;
                };
                let Ok(array) = layout.decode(operands, &mut 0) else {
                    self.clear();
                    return false;
                };
                let [dst, count_reg, _] = array.regs;
                match MemSubOpcode::from_byte(*sub_op) {
                    Some(MemSubOpcode::ByteArrayStore | MemSubOpcode::TypedArrayStore) => {
                        self.fresh_lists.clear();
                    }
                    Some(MemSubOpcode::ByteArrayLoad | MemSubOpcode::TypedArrayLoad) => {
                        self.forget(dst);
                    }
                    Some(MemSubOpcode::NewByteArray | MemSubOpcode::NewTypedArray) => {
                        let count = match self.get(count_reg) {
                            Some(ArrayResultFact::Integer(n)) => u64::try_from(n).ok(),
                            _ => None,
                        };
                        self.forget(dst);
                        let width = usize::from(array.format & 0x7f);
                        if let Some(count) = count
                            && count.checked_mul(width as u64).is_some_and(|n| n <= isize::MAX as u64)
                        {
                            self.values.insert(dst, ArrayResultFact::Packed {
                                width,
                                float: array.format & 0x80 != 0,
                                count,
                            });
                        }
                    }
                    _ => {
                        self.clear();
                        return false;
                    }
                }
            }
            I::CbgrExtended { sub_op, operands }
                if crate::instruction::CbgrSubOpcode::from_byte(*sub_op)
                    == Some(crate::instruction::CbgrSubOpcode::SliceLen) =>
            {
                let Some((destination, [])) = scalar_register_pair(operands) else {
                    self.clear();
                    return false;
                };
                self.forget(destination);
            }
            I::ArithExtended { sub_op, operands }
                if ArithSubOpcode::from_byte(*sub_op) == Some(ArithSubOpcode::SextI) =>
            {
                let Some((destination, [8 | 16 | 32 | 64, 64])) = scalar_register_pair(operands) else {
                    self.clear();
                    return false;
                };
                self.forget(destination);
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
            | I::CvtFI { dst, .. }
            | I::CvtToI { dst, .. }
            | I::GetF { dst, .. }
            | I::GetE { dst, .. }
            | I::Len { dst, .. } => self.forget(*dst),
            // Dropping can invoke user glue, with the same unknown side effects
            // as a call. It is not merely a register overwrite.
            I::DropRef { .. } => {
                self.clear();
                return false;
            }
            I::ListPush { .. } | I::SetE { .. } => self.fresh_lists.clear(),
            I::Nop | I::Assert { .. } => {}
            // Clones, reference accesses and calls need their selected producer
            // contract. Branch/loop joins need a meet, not last-emitted-wins.
            _ => {
                self.clear();
                return false;
            }
        }
        true
    }

    /// Record a result only after its actual selected producer has been proved.
    /// Type declarations and method spelling are not sufficient evidence.
    pub fn record_selected_result(&mut self, register: Reg, fact: ArrayResultFact) {
        self.fresh_lists.remove(&register);
        self.values.insert(register, fact);
    }
}

/// Read the destination and source before an extended scalar operation's immediates.
fn scalar_register_pair(operands: &[u8]) -> Option<(Reg, &[u8])> {
    let mut cursor = 0;
    let mut read = || {
        let start = cursor;
        let register = decode_reg(operands, &mut cursor).ok()?;
        let width = if register.0 < 128 { 1 } else { 2 };
        (cursor - start == width).then_some(register)
    };
    let destination = read()?;
    read()?;
    Some((destination, &operands[cursor..]))
}

/// Summarize one completed, straight-line executable body. This establishes the
/// returned storage only; the caller must independently prove that this exact
/// body is selected. Declarations, stubs and signatures supply no storage fact.
///
/// Branches, multiple returns, parameter forwarding and unknown returned values
/// remain unproved. No last-emitted-wins rule crosses a control-flow join. The
/// transfer whitelist rejects calls, suspension and every unknown instruction;
/// no later allocation can turn unknown reachability into a body-wide proof.
pub fn straight_line_array_return(instructions: &[Instruction]) -> Maybe<ArrayResultFact> {
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
            _ if !facts.observe(instruction) => return None,
            _ => {}
        }
    }
    result
}

#[cfg(test)]
#[path = "../tests/codegen/array_result_facts.rs"]
mod tests;
