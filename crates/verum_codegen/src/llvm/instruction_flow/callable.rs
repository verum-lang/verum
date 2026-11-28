//! Reaching declarations for capture-free function values. Such a formation
//! needs no environment lifetime receipt. Captured environments continue to
//! use the emitter's local ownership proof for allocation, retain and release.
use super::{Edges, InstructionFlow};
use crate::llvm::Result;
use verum_common::{List, Map, Set};
use verum_vbc::{
    FunctionId, Instruction as I, Reg, VbcModule,
    copy_plan::{self, CopyNode, NodeId},
};

type Declarations = Map<Reg, FunctionId>;

pub(in crate::llvm) type CallableInputs = Map<usize, (Reg, FunctionId)>;

fn copied_register(facts: &mut Declarations, dst: Reg, src: Reg) {
    let producer = facts.get(&src).copied();
    facts.remove(&dst);
    if let Some(producer) = producer {
        facts.insert(dst, producer);
    }
}

/// Only a slot whose address has never been published is independent of
/// writes in another frame. A callee can keep a reference across later
/// assignments to the same slot.
fn borrowed_slot(instruction: &I) -> Option<Reg> {
    match instruction {
        I::Ref { src, .. }
        | I::RefMut { src, .. }
        | I::RefLocal { src, .. }
        | I::RefObj { src, .. }
        | I::RefChecked { src, .. }
        | I::RefUnsafe { src, .. } => Some(*src),
        _ => None,
    }
}

fn transfer(
    module: &VbcModule,
    instruction: &I,
    exposed: &Set<Reg>,
    facts: &mut Declarations,
    fresh_record: &mut Option<(Reg, u32)>,
) {
    // Initializing a field of this block's new allocation cannot modify an
    // existing callable. Keep its extent only while the isolated allocation
    // and its local address are known. A call, an alias or a join ends it.
    match instruction {
        I::New { dst, field_count, .. } => {
            *fresh_record = (!exposed.contains(dst)).then_some((*dst, *field_count));
            facts.remove(dst);
            return;
        }
        I::SetF { obj, field_idx, .. }
            if fresh_record.is_some_and(|(reg, count)| reg == *obj && *field_idx < count) =>
        {
            return;
        }
        I::LoadI { dst, .. } | I::LoadSmallI { dst, .. } | I::LoadK { dst, .. }
        | I::LoadT { dst, .. } | I::LoadF { dst, .. } | I::LoadTrue { dst }
        | I::LoadFalse { dst } | I::LoadUnit { dst }
            if fresh_record.is_some_and(|(reg, _)| reg != *dst) => {}
        _ => *fresh_record = None,
    }
    match instruction {
        I::NewClosure {
            dst,
            func_id,
            captures,
            owns_captures,
            capture_types,
            lexical_types,
        } => {
            facts.remove(dst);
            if captures.is_empty()
                && !owns_captures
                && lexical_types.is_empty()
                && capture_types.as_ref().is_none_or(|types| types.is_empty())
                && *func_id != i32::MAX as u32
            {
                // The same linked ID as NewClosure. Unresolved name-only stubs
                // carry no inter-block proof. The emitter checks both body
                // conventions before actually forming or calling this value.
                let linked = module.resolve_band_id(*func_id).or_else(|| {
                    (!verum_vbc::stub_ranges::is_stub_id(*func_id)
                        && !verum_vbc::stub_ranges::is_xmod_name_reference(*func_id))
                    .then_some(FunctionId(*func_id))
                });
                if let Some(id) = linked
                    && let Some(declaration) = module.get_function(id)
                    && declaration.id == id
                    && declaration.type_params.is_empty()
                    && declaration.validate_closure_environment(0).is_ok()
                {
                    facts.insert(*dst, id);
                }
            } else {
                // Captured values may enter user-defined retain code.
                facts.clear();
            }
        }
        I::Mov { dst, src } => copied_register(facts, *dst, *src),
        I::Clone { dst, src } => {
            if !facts.contains_key(src) {
                facts.retain(|slot, _| !exposed.contains(slot));
            }
            copied_register(facts, *dst, *src);
        }
        I::CloneTyped { dst, src, type_ref } => {
            // The emitter checks the complete callable signature before this
            // copy. An alias is transparent only when its checked plan ends in
            // a callable copy. Other plans may invoke a declared retain.
            let callable = facts.contains_key(src)
                && copy_plan::build(module, type_ref).is_ok_and(|plan| {
                    let mut node = NodeId(0);
                    for _ in 0..plan.nodes.len() {
                        match &plan.nodes[node.0] {
                            CopyNode::Alias(next) => node = *next,
                            CopyNode::Callable(_) => return true,
                            _ => break,
                        }
                    }
                    false
                });
            if callable {
                copied_register(facts, *dst, *src);
            } else {
                facts.remove(dst);
                facts.retain(|slot, _| !exposed.contains(slot));
            }
        }
        I::LoadI { dst, .. }
        | I::LoadSmallI { dst, .. }
        | I::LoadK { dst, .. }
        | I::LoadT { dst, .. }
        | I::LoadF { dst, .. }
        | I::LoadTrue { dst }
        | I::LoadFalse { dst }
        | I::LoadUnit { dst }
        | I::BinaryI { dst, .. }
        | I::BinaryF { dst, .. }
        | I::UnaryI { dst, .. }
        | I::UnaryF { dst, .. }
        | I::Not { dst, .. }
        | I::Bitwise { dst, .. }
        | I::CmpI { dst, .. }
        | I::CmpF { dst, .. }
        | I::CmpU { dst, .. }
        | I::CvtIF { dst, .. }
        | I::CvtFI { dst, .. }
        | I::CvtIC { dst, .. }
        | I::CvtCI { dst, .. }
        | I::CvtBI { dst, .. } => {
            facts.remove(dst);
        }
        I::Call { dst, .. } | I::CallClosure { dst, .. } => {
            facts.remove(dst);
            facts.retain(|slot, _| !exposed.contains(slot));
        }
        I::DropTyped { src, .. } | I::DropRef { src } => {
            facts.remove(src);
            facts.retain(|slot, _| !exposed.contains(slot));
        }
        I::Nop
        | I::Jmp { .. }
        | I::JmpIf { .. }
        | I::JmpNot { .. }
        | I::JmpCmp { .. }
        | I::Switch { .. }
        | I::TryBegin { .. }
        | I::TryEnd => {}
        I::SysExtended { sub_op, operands }
            if verum_vbc::instruction::SysSubOpcode::from_byte(*sub_op)
                == Some(verum_vbc::instruction::SysSubOpcode::LinuxSignalWatch) =>
        {
            if let Some((_, dst, _)) =
                verum_vbc::signal_watch::SignalWatchOp::decode_operands(operands)
            {
                facts.remove(&dst);
            } else {
                facts.clear();
            }
        }
        // Unmodelled writes, stores through references and published aliases
        // end the proof. A declaration type cannot re-establish the value.
        _ => facts.clear(),
    }
}

fn consumer(instruction: &I) -> Option<Reg> {
    match instruction {
        I::CallClosure { closure, .. } => Some(*closure),
        I::Clone { src, .. } | I::CloneTyped { src, .. } => Some(*src),
        _ => None,
    }
}

impl InstructionFlow {
    /// Intersect all reachable predecessors, including loop backedges. Keep
    /// maps at block entries and a single register at each actual consumer.
    pub(in crate::llvm) fn callable_inputs(
        &self,
        module: &VbcModule,
        instructions: &[I],
    ) -> Result<CallableInputs> {
        let mut inputs = Map::new();
        if !instructions.iter().any(|instruction| {
            matches!(instruction,
            I::NewClosure { owns_captures: false, captures, .. } if captures.is_empty())
        }) || !instructions
            .iter()
            .any(|instruction| consumer(instruction).is_some())
        {
            return Ok(inputs);
        }
        let exposed: Set<_> = instructions.iter().filter_map(borrowed_slot).collect();
        let mut incoming = List::from_iter(self.starts.iter().map(|_| None));
        incoming[0] = Some(Declarations::new());
        let mut pending = List::from_iter([0usize]);
        let mut queued = List::from_iter(self.starts.iter().map(|_| false));
        queued[0] = true;
        let empty = Declarations::new();
        while let Some(block) = pending.pop() {
            queued[block] = false;
            let mut facts = incoming[block]
                .clone()
                .expect("a queued block has an incoming edge");
            let start = self.starts[block];
            let end = self
                .starts
                .get(block + 1)
                .copied()
                .unwrap_or(instructions.len());
            let mut last = None;
            let mut fresh_record = None;
            for pc in start..end {
                if !self.reachable[pc] {
                    break;
                }
                let instruction = &instructions[pc];
                if let Some(reg) = consumer(instruction) {
                    if let Some(target) = facts.get(&reg) {
                        inputs.insert(pc, (reg, *target));
                    } else {
                        inputs.remove(&pc);
                    }
                }
                transfer(module, instruction, &exposed, &mut facts, &mut fresh_record);
                last = Some(pc);
            }
            let Some(pc) = last else {
                continue;
            };
            let instruction = &instructions[pc];
            let handler = match instruction {
                I::TryBegin { handler_offset } => pc.checked_add_signed(*handler_offset as isize),
                _ => None,
            };
            Edges::of(instruction).successors(pc, instructions.len(), |next| {
                let Ok(successor) = self.starts.binary_search(&next) else {
                    return;
                };
                // TryBegin describes reachability of recovery, not the state
                // of the protected body at the actual throwing instruction.
                let offered = if handler == Some(next) {
                    &empty
                } else {
                    &facts
                };
                let changed = match &mut incoming[successor] {
                    None => {
                        incoming[successor] = Some(offered.clone());
                        true
                    }
                    Some(previous) => {
                        let count = previous.len();
                        previous.retain(|reg, id| offered.get(reg) == Some(id));
                        previous.len() != count
                    }
                };
                if changed && !queued[successor] {
                    pending.push(successor);
                    queued[successor] = true;
                }
            })?;
        }
        Ok(inputs)
    }
}
