//! Body-sealed source production receipts, never destructor authority.
use crate::{instruction::Reg, types::TypeRef};
use serde::{Deserialize, Serialize};
use verum_common::value_use::ValueUseEvent;
use verum_common::{List, Maybe};

/// Maximum retained uses per function. Exhaustion discards the entire plan.
pub const MAX_VALUE_USES: usize = 16_384;
/// A source-use fact attached to an actual emitted instruction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValueUseReceipt {
    /// Shared event, with function-local identities.
    pub event: ValueUseEvent,
    /// Index in the sealed decoded body, not a byte offset.
    pub instruction: u32,
    /// Actual operand register at this emission site.
    pub operand: Reg,
    /// Source declaration type, with reference wrappers retained.
    pub declaration_type: Maybe<TypeRef>,
}
/// Producer facts are usable only while this exact body remains current.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValueUsePlan {
    /// Canonical encoded body hash. Rewrites must remap/reseal or invalidate.
    pub body_hash: [u8; 32],
    /// Semantic seal: changed signature or recorded use facts invalidate the plan.
    pub signature_hash: [u8; 32],
    /// Ordered source-use receipts. This is not a lifecycle plan.
    pub uses: List<ValueUseReceipt>,
}
impl ValueUsePlan {
    /// Seal a producer's bounded receipts against the emitted body.
    pub fn new(
        body: &[crate::Instruction],
        uses: List<ValueUseReceipt>,
        descriptor: &crate::module::FunctionDescriptor,
    ) -> Maybe<Self> {
        if uses.is_empty() || uses.len() > MAX_VALUE_USES {
            return None;
        }
        Some(Self {
            body_hash: Self::body_hash(body),
            signature_hash: Self::signature_hash(descriptor, &uses)?,
            uses,
        })
    }
    /// Hash semantic carriers, excluding module-local string IDs and function IDs.
    pub fn signature_hash(
        descriptor: &crate::module::FunctionDescriptor,
        uses: &[ValueUseReceipt],
    ) -> Maybe<[u8; 32]> {
        let params: List<_> = descriptor
            .params
            .iter()
            .map(|p| (&p.type_ref, p.is_mut))
            .collect();
        let type_ids: List<_> = descriptor.type_params.iter().map(|p| p.id).collect();
        // Confine the existing serializer's Vec output to this encoding boundary.
        let encoded = bincode::serialize(&(
            params,
            type_ids,
            &descriptor.return_type,
            &descriptor.yield_type,
            &descriptor.semantic_params,
            descriptor.register_count,
            uses,
        ))
        .ok()?;
        Some(*blake3::hash(&encoded).as_bytes())
    }
    /// Any semantic signature rewrite needs an explicit producer revalidation.
    pub fn matches_signature(&self, descriptor: &crate::module::FunctionDescriptor) -> bool {
        Self::signature_hash(descriptor, &self.uses) == Some(self.signature_hash)
    }
    /// The bytecode encoder owns the body representation.
    pub fn body_hash(body: &[crate::Instruction]) -> [u8; 32] {
        // The existing bytecode encoder accepts Vec; keep that representation
        // confined to this encoder boundary.
        let mut bytes = List::new().into();
        crate::bytecode::encode_instructions_with_fixup(body, &mut bytes);
        *blake3::hash(&bytes).as_bytes()
    }
    /// A changed body cannot inherit stale instruction or register facts.
    pub fn matches_body(&self, body: &[crate::Instruction]) -> bool {
        self.uses.len() <= MAX_VALUE_USES
            && self.body_hash == Self::body_hash(body)
            && self
                .uses
                .iter()
                .enumerate()
                .all(|(i, u)| u.event.id.0 == i as u32 && (u.instruction as usize) < body.len())
    }
}

impl crate::module::VbcModule {
    /// Read validated producer facts from the actual executable bytecode.
    /// Any unaccounted rewrite, missing body, or excessive metadata is Unknown.
    pub fn value_use_receipts(&self, function: crate::FunctionId) -> Maybe<&[ValueUseReceipt]> {
        let descriptor = self.get_function(function)?;
        let plan = descriptor.value_uses.as_ref()?;
        let start = descriptor.bytecode_offset as usize;
        let end = start.checked_add(descriptor.bytecode_length as usize)?;
        let bytes = self.bytecode.get(start..end)?;
        if plan.uses.is_empty()
            || plan.uses.len() > MAX_VALUE_USES
            || !plan.matches_signature(descriptor)
            || *blake3::hash(bytes).as_bytes() != plan.body_hash
        {
            return None;
        }
        let body = decode_bounded(bytes)?;
        for (index, receipt) in plan.uses.iter().enumerate() {
            if receipt.event.id.0 != index as u32
                || (index > 0 && plan.uses[index - 1].instruction > receipt.instruction)
                || receipt.operand.0 >= descriptor.register_count
                || !receipt.matches_instruction(body.get(receipt.instruction as usize)?)
            {
                return None;
            }
            use verum_common::value_use::{DuplicationOperation, ValueUseOperation as Op};
            match receipt.event.operation {
                Op::Unknown => {}
                Op::Borrow
                    if matches!(receipt.declaration_type, Some(TypeRef::Reference { .. })) => {}
                Op::Copy(DuplicationOperation::ValueCopy)
                    if receipt.declaration_type.as_ref().is_some_and(|ty| {
                        !matches!(ty, TypeRef::Reference { .. })
                            && self.resource_discipline(ty)
                                == verum_common::ResourceDiscipline::Unrestricted
                    }) => {}
                _ => return None,
            }
        }
        Some(plan.uses.as_slice())
    }

    /// Materialize the existing shared CFG with separate producer value events.
    /// Initial support is ordinary direct control flow. Exceptional/suspending
    /// control flow is explicitly declined, never treated as linear execution.
    /// This graph deliberately adds no deallocation/reference-use approximations.
    pub fn value_use_cfg(
        &self,
        function: crate::FunctionId,
    ) -> Maybe<verum_cbgr::analysis::ControlFlowGraph> {
        use crate::Instruction;
        use verum_cbgr::analysis::{BasicBlock, BlockId, ControlFlowGraph};
        let uses = self.value_use_receipts(function)?;
        let descriptor = self.get_function(function)?;
        let start = descriptor.bytecode_offset as usize;
        let bytes = self
            .bytecode
            .get(start..start.checked_add(descriptor.bytecode_length as usize)?)?;
        let body = decode_bounded(bytes)?;
        let mut offsets = List::with_capacity(body.len() + 1);
        let mut position = 0;
        for _ in &body {
            offsets.push(position);
            crate::bytecode::decode_instruction(bytes, &mut position).ok()?;
        }
        offsets.push(position);
        let exit = BlockId(body.len() as u64);
        let mut cfg = ControlFlowGraph::new(BlockId(0), exit);
        for i in 0..=body.len() {
            cfg.add_block(BasicBlock::empty(BlockId(i as u64)));
        }
        for (i, instruction) in body.iter().enumerate() {
            let mut successors = List::new();
            let target = |relative: i32| -> Maybe<BlockId> {
                let byte = (offsets[i + 1] as i64).checked_add(i64::from(relative))?;
                usize::try_from(byte)
                    .ok()
                    .and_then(|byte| offsets.binary_search(&byte).ok())
                    .map(|n| BlockId(n as u64))
            };
            match instruction {
                Instruction::Jmp { offset } => successors.push(target(*offset)?),
                Instruction::JmpIf { offset, .. }
                | Instruction::JmpNot { offset, .. }
                | Instruction::JmpCmp { offset, .. } => {
                    successors.push(target(*offset)?);
                    successors.push(BlockId(i as u64 + 1));
                }
                Instruction::Ret { .. }
                | Instruction::RetV
                | Instruction::Panic { .. }
                | Instruction::Unreachable => successors.push(exit),
                Instruction::Switch { .. }
                | Instruction::TryBegin { .. }
                | Instruction::Yield { .. }
                | Instruction::Await { .. }
                | Instruction::AsyncYield
                | Instruction::CtxProvide { .. }
                | Instruction::TailCall { .. }
                | Instruction::Throw { .. } => return None,
                _ => successors.push(BlockId(i as u64 + 1)),
            }
            for successor in successors {
                cfg.blocks
                    .get_mut(&BlockId(i as u64))?
                    .successors
                    .insert(successor);
                cfg.blocks
                    .get_mut(&successor)?
                    .predecessors
                    .insert(BlockId(i as u64));
            }
        }
        for (i, receipt) in uses.iter().enumerate() {
            if receipt.event.id.0 != i as u32 || receipt.instruction as usize >= body.len() {
                return None;
            }
            cfg.value_uses
                .entry(BlockId(u64::from(receipt.instruction)))
                .or_default()
                .push(receipt.event.clone());
        }
        Some(cfg)
    }
}

/// Bound decoding before retaining another instruction, not after allocation.
fn decode_bounded(bytes: &[u8]) -> Maybe<List<crate::Instruction>> {
    const MAX_INSTRUCTIONS: usize = 65_536;
    let mut body = List::new();
    let mut position = 0;
    while position < bytes.len() {
        if body.len() == MAX_INSTRUCTIONS {
            return None;
        }
        body.push(crate::bytecode::decode_instruction(bytes, &mut position).ok()?);
    }
    Some(body)
}

impl ValueUseReceipt {
    fn matches_instruction(&self, instruction: &crate::Instruction) -> bool {
        use crate::Instruction as I;
        use verum_common::value_use::{
            DuplicationOperation, ValueUseOperation as Op, ValueUseSite as Site,
        };
        let actual_site = match (self.event.site, instruction) {
            (Site::Local, I::Mov { src, .. } | I::Clone { src, .. }) => *src == self.operand,
            (Site::Argument(index), I::Call { args, .. } | I::CallG { args, .. }) => {
                self.event.destination.is_none()
                    && index < u16::from(args.count)
                    && args.start.0.checked_add(index) == Some(self.operand.0)
            }
            (Site::Return, I::Ret { value }) => {
                self.event.destination.is_none() && *value == self.operand
            }
            _ => false,
        };
        actual_site
            && match self.event.operation {
                Op::Unknown | Op::Borrow => true,
                Op::Copy(DuplicationOperation::ValueCopy) => matches!(instruction, I::Clone { .. }),
                Op::Copy(DuplicationOperation::Carrier) | Op::Transfer => false,
            }
    }
}
