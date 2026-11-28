//! Per-frame read-only grants for managed borrows converted to raw addresses.
//! The address ABI alone cannot distinguish two borrows of the same allocation.
use super::{
    context::FunctionContext,
    error::{BuildExt, Result},
    instruction::emit_runtime_abort,
};
use verum_common::{List, Map, Set};
use verum_llvm::{
    builder::Builder,
    values::{IntValue, PointerValue},
};
use verum_vbc::{
    Instruction, Reg, encoding::decode_reg,
    instruction::{CbgrSubOpcode as Cbgr, MemSubOpcode},
    value::Capabilities,
};

#[derive(Default)]
pub(super) struct LocalBorrowGrants<'ctx> {
    denied: Map<Reg, PointerValue<'ctx>>,
}

#[derive(Clone, Copy)]
enum Grant {
    ReadOnly,
    From(Reg),
}

fn transfer(instruction: &Instruction) -> Option<(Reg, Grant)> {
    match instruction {
        Instruction::Ref { dst, .. }
        | Instruction::RefChecked { dst, .. }
        | Instruction::RefUnsafe { dst, .. } => Some((*dst, Grant::ReadOnly)),
        Instruction::CbgrExtended { sub_op, operands } => {
            let op = Cbgr::from_byte(*sub_op)?;
            if !matches!(
                op,
                Cbgr::ToRawPtr
                    | Cbgr::RefField
                    | Cbgr::RefFieldNamed
                    | Cbgr::CapAttenuate
                    | Cbgr::Reborrow
            ) {
                return None;
            }
            let mut pos = 0;
            let dst = decode_reg(operands, &mut pos).ok()?;
            let source = decode_reg(operands, &mut pos).ok()?;
            let grant = if op == Cbgr::CapAttenuate
                && u16::from_le_bytes(operands.get(pos..)?.try_into().ok()?) & Capabilities::WRITE
                    == 0
            {
                Grant::ReadOnly
            } else {
                Grant::From(source)
            };
            Some((dst, grant))
        }
        _ => None,
    }
}

fn write_operand(instruction: &Instruction) -> Option<Reg> {
    match instruction {
        Instruction::SetF { obj, .. } | Instruction::SetFieldNamed { obj, .. } => Some(*obj),
        Instruction::DerefMut { ref_reg, .. } => Some(*ref_reg),
        Instruction::RetainCopy { counter, .. } => Some(*counter),
        Instruction::MemExtended { sub_op, operands }
            if *sub_op == MemSubOpcode::DerefMutRaw as u8 =>
        {
            let mut pos = 0;
            decode_reg(operands, &mut pos).ok()
        }
        _ => None,
    }
}

impl<'ctx> LocalBorrowGrants<'ctx> {
    pub(super) fn tracks(&self, reg: Reg) -> bool {
        self.denied.contains_key(&reg)
    }

    pub(super) fn prepare(ctx: &FunctionContext<'_, 'ctx>, body: &[Instruction]) -> Result<Self> {
        let mut grants = Self::default();
        if !body.iter().any(|ins| write_operand(ins).is_some()) {
            return Ok(grants);
        }

        // Only issue local evidence for registers whose storage never escapes.
        // Writing a new word through an alias bypasses an ordinary register definition.
        let mut exposed = Set::new();
        let mut sources: Map<Reg, List<Reg>> = Map::new();
        let mut aliases: Map<Reg, List<Reg>> = Map::new();
        let mut shared = List::new();
        let mut pending = List::new();
        for ins in body {
            if let Instruction::Ref { src, .. }
            | Instruction::RefMut { src, .. }
            | Instruction::RefLocal { src, .. }
            | Instruction::RefObj { src, .. }
            | Instruction::RefChecked { src, .. }
            | Instruction::RefUnsafe { src, .. } = ins
            {
                exposed.insert(*src);
            }
            if let Some(reg) = write_operand(ins) {
                pending.push(reg);
            }
            let edge = match ins {
                Instruction::Mov { dst, src } | Instruction::CloneTyped { dst, src, .. } => {
                    Some((*dst, Grant::From(*src)))
                }
                _ => transfer(ins),
            };
            match edge {
                Some((dst, Grant::From(src))) => {
                    sources.entry(dst).or_default().push(src);
                    aliases.entry(src).or_default().push(dst);
                }
                Some((dst, Grant::ReadOnly)) => shared.push(dst),
                None => {}
            }
        }
        let mut observed = Set::new();
        while let Some(reg) = pending.pop() {
            if exposed.contains(&reg) || !observed.insert(reg) {
                continue;
            }
            if let Some(inputs) = sources.get(&reg) {
                pending.extend(inputs.iter().copied());
            }
        }
        let flag = ctx.llvm_context().bool_type();
        while let Some(reg) = shared.pop() {
            if !observed.remove(&reg) {
                continue;
            }
            let slot = ctx
                .builder()
                .build_alloca(flag, &format!("r{}_readonly", reg.0))
                .or_llvm_err()?;
            ctx.builder()
                .build_store(slot, flag.const_zero())
                .or_llvm_err()?;
            grants.denied.insert(reg, slot);
            if let Some(outputs) = aliases.get(&reg) {
                shared.extend(outputs.iter().copied());
            }
        }
        Ok(grants)
    }

    pub(super) fn flag(
        &self,
        ctx: &FunctionContext<'_, 'ctx>,
        reg: Reg,
    ) -> Result<Option<IntValue<'ctx>>> {
        self.denied
            .get(&reg)
            .map(|slot| {
                ctx.builder()
                    .build_load(ctx.llvm_context().bool_type(), *slot, "borrow_readonly")
                    .or_llvm_err()
                    .map(|value| value.into_int_value())
            })
            .transpose()
    }

    pub(super) fn set(
        &self,
        builder: &Builder<'ctx>,
        reg: Reg,
        value: IntValue<'ctx>,
    ) -> Result<()> {
        if let Some(slot) = self.denied.get(&reg) {
            builder.build_store(*slot, value).or_llvm_err()?;
        }
        Ok(())
    }
}

pub(super) fn before<'ctx>(
    ctx: &FunctionContext<'_, 'ctx>,
    instruction: &Instruction,
) -> Result<Option<(Reg, IntValue<'ctx>)>> {
    if ctx.local_borrow_grants.denied.is_empty() {
        return Ok(None);
    }
    if let Some(place) = write_operand(instruction)
        && let Some(denied) = ctx.local_borrow_grants.flag(ctx, place)?
    {
        let refused = ctx
            .llvm_context()
            .append_basic_block(ctx.function(), "readonly_write");
        let permitted = ctx
            .llvm_context()
            .append_basic_block(ctx.function(), "write_granted");
        ctx.builder()
            .build_conditional_branch(denied, refused, permitted)
            .or_llvm_err()?;
        ctx.builder().position_at_end(refused);
        emit_runtime_abort(
            ctx,
            "CBGR: write through a read-only reference",
            "cbgr_readonly_write",
        )?;
        ctx.builder().build_unreachable().or_llvm_err()?;
        ctx.builder().position_at_end(permitted);
    }
    let Some((dst, grant)) = transfer(instruction) else {
        return Ok(None);
    };
    if !ctx.local_borrow_grants.tracks(dst) {
        return Ok(None);
    }
    let flag = match grant {
        Grant::ReadOnly => Some(ctx.llvm_context().bool_type().const_int(1, false)),
        Grant::From(reg) => ctx.local_borrow_grants.flag(ctx, reg)?,
    };
    Ok(flag.map(|value| (dst, value)))
}
