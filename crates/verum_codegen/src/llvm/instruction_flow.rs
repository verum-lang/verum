//! One bounded instruction-index graph for native block boundaries and reachability.
//! TryBegin contributes its handler edge even when the throwing operation is an
//! indirect call. This is conservative reachability, never proof that a call succeeds.
#[path = "instruction_flow/callable.rs"]
mod callable;

use super::{LlvmLoweringError, Result};
use verum_common::List;
use verum_vbc::{Instruction, VbcModule};

pub(super) struct InstructionFlow {
    pub starts: List<usize>,
    pub reachable: List<bool>,
    pub entry_is_target: bool,
}

enum Edges<'a> {
    Next,
    Stop,
    Jump(i32),
    Branch(i32),
    Switch {
        default: i32,
        cases: &'a [(u32, i32)],
    },
}
impl<'a> Edges<'a> {
    fn of(instruction: &'a Instruction) -> Self {
        match instruction {
            Instruction::Jmp { offset } => Self::Jump(*offset),
            Instruction::JmpIf { offset, .. }
            | Instruction::JmpNot { offset, .. }
            | Instruction::JmpCmp { offset, .. } => Self::Branch(*offset),
            Instruction::TryBegin { handler_offset } => Self::Branch(*handler_offset),
            Instruction::Switch {
                default_offset,
                cases,
                ..
            } => Self::Switch {
                default: *default_offset,
                cases,
            },
            Instruction::Ret { .. }
            | Instruction::RetV
            | Instruction::TailCall { .. }
            | Instruction::Panic { .. }
            | Instruction::PanicValue { .. }
            | Instruction::Unreachable
            | Instruction::Throw { .. }
            | Instruction::RethrowException => Self::Stop,
            Instruction::Extended { sub_op, .. }
                if matches!(verum_vbc::instruction::ExtendedSubOpcode::from_byte(*sub_op),
                    Some(verum_vbc::instruction::ExtendedSubOpcode::ProcessExit
                        | verum_vbc::instruction::ExtendedSubOpcode::RethrowException)) =>
            {
                Self::Stop
            }
            _ => Self::Next,
        }
    }
    fn successors(&self, pc: usize, count: usize, mut visit: impl FnMut(usize)) -> Result<()> {
        let target = |offset: i32| {
            pc.checked_add_signed(offset as isize)
                .filter(|target| *target <= count)
                .ok_or_else(|| {
                    LlvmLoweringError::unsupported(format!(
                        "native control-flow target is outside the function at instruction {pc}"
                    ))
                })
        };
        match self {
            Self::Next => visit(pc + 1),
            Self::Stop => (),
            Self::Jump(offset) => visit(target(*offset)?),
            Self::Branch(offset) => {
                visit(target(*offset)?);
                visit(pc + 1);
            }
            Self::Switch { default, cases } => {
                visit(target(*default)?);
                for (_, offset) in *cases {
                    visit(target(*offset)?);
                }
            }
        }
        Ok(())
    }
}

pub(in crate::llvm) fn has_linear_fallthrough(body: &[Instruction]) -> bool {
    let Some((last, prefix)) = body.split_last() else { return false };
    prefix.iter().all(|op| matches!(Edges::of(op), Edges::Next))
        && matches!(Edges::of(last), Edges::Next | Edges::Stop)
}

impl InstructionFlow {
    pub fn build(module: &VbcModule, instructions: &[Instruction]) -> Result<Self> {
        let mut starts = List::new();
        let mut reachable = List::new();
        let count = instructions
            .len()
            .checked_add(1)
            .ok_or_else(|| LlvmLoweringError::unsupported("native instruction count overflow"))?;
        for storage in [&mut starts, &mut reachable] {
            storage.try_reserve_exact(count).map_err(|_| {
                LlvmLoweringError::unsupported("native control-flow allocation limit")
            })?;
        }
        starts.resize(count, false);
        reachable.resize(count, false);
        starts[0] = true;
        let mut entry_is_target = false;
        // Validate every structural target, including dead instructions. Being
        // unreachable does not make malformed bytecode admissible.
        for (pc, instruction) in instructions.iter().enumerate() {
            let edges = Edges::of(instruction);
            if !matches!(edges, Edges::Next) {
                edges.successors(pc, instructions.len(), |target| {
                    starts[target] = true;
                })?;
                if pc + 1 < instructions.len() {
                    starts[pc + 1] = true;
                }
            }
        }
        let mut pending = List::new();
        pending
            .try_reserve_exact(count)
            .map_err(|_| LlvmLoweringError::unsupported("native control-flow allocation limit"))?;
        pending.push(0);
        reachable[0] = true;
        while let Some(pc) = pending.pop() {
            let Some(instruction) = instructions.get(pc) else {
                continue;
            };
            if let Instruction::NewClosure {
                captures,
                capture_types,
                lexical_types,
                ..
            } = instruction
                && crate::llvm::instruction::native_capture_admission(
                    module,
                    capture_types.as_deref(),
                    captures.len(),
                    lexical_types,
                )
                .is_err()
            {
                // The emitter runs this same admission before allocation and
                // terminates the path with its executable refusal. A surrounding
                // TryBegin has already contributed the exceptional successor.
                continue;
            }
            Edges::of(instruction).successors(pc, instructions.len(), |next| {
                entry_is_target |= next == 0;
                if !reachable[next] {
                    reachable[next] = true;
                    pending.push(next);
                }
            })?;
        }
        Ok(Self {
            starts: starts
                .into_iter()
                .enumerate()
                .filter_map(|(pc, start)| (start && reachable[pc]).then_some(pc))
                .collect(),
            reachable,
            entry_is_target,
        })
    }
}

#[cfg(test)]
#[path = "../../tests/llvm/instruction_flow.rs"]
mod tests;
