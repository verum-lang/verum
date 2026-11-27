//! Discharge ownership branches whose substituted declaration fixes the result.
use crate::{Instruction as I, VbcModule, instruction::CompareOp};
use verum_common::{List, Map, ResourceDiscipline};

/// Every alternative entry invalidates facts from the preceding instruction.
pub(super) fn entry_points(body: &mut [I]) -> List<bool> {
    let mut entries = List::from(vec![false; body.len()]);
    for (pc, op) in body.iter_mut().enumerate() {
        crate::bytecode::map_control_offsets(op, |offset| {
            if let Some(target) = pc.checked_add_signed(offset as isize)
                && let Some(entry) = entries.get_mut(target)
            {
                *entry = true;
            }
            offset
        });
    }
    entries
}

#[derive(Clone, Copy)]
enum Fact {
    Resource(i64),
    Integer(i64),
    Predicate(bool),
}

pub(super) fn fold(module: &VbcModule, entries: &[bool], body: &mut [I]) {
    if !body.iter().any(|op| matches!(op, I::TypeResource { .. })) {
        return;
    }
    let mut facts = Map::new();
    for (op, &entry) in body.iter_mut().zip(entries) {
        if entry {
            facts.clear();
        }
        match op {
            I::TypeResource { dst, type_ref } => {
                facts.remove(&dst.0);
                if super::discovery::concrete(type_ref) {
                    let mode = module.resource_discipline(type_ref);
                    if mode != ResourceDiscipline::Unknown {
                        let value = i64::from(mode as u8);
                        facts.insert(dst.0, Fact::Resource(value));
                        *op = I::LoadI { dst: *dst, value };
                    }
                }
            }
            I::LoadI { dst, value } => {
                facts.insert(dst.0, Fact::Integer(*value));
            }
            I::Mov { dst, src } => {
                let incoming = facts.get(&src.0).copied();
                facts.remove(&dst.0);
                if let Some(value) = incoming {
                    facts.insert(dst.0, value);
                }
            }
            I::CmpI { dst, op, a, b } => {
                let values = (facts.get(&a.0).copied(), facts.get(&b.0).copied());
                facts.remove(&dst.0);
                let (left, right) = match values {
                    (Some(Fact::Resource(a)), Some(Fact::Integer(b)))
                    | (Some(Fact::Integer(a)), Some(Fact::Resource(b)))
                    | (Some(Fact::Resource(a)), Some(Fact::Resource(b))) => (a, b),
                    _ => continue,
                };
                let predicate = match op {
                    CompareOp::Eq => left == right,
                    CompareOp::Ne => left != right,
                    CompareOp::Lt => left < right,
                    CompareOp::Le => left <= right,
                    CompareOp::Gt => left > right,
                    CompareOp::Ge => left >= right,
                };
                facts.insert(dst.0, Fact::Predicate(predicate));
            }
            I::JmpIf { cond, offset } | I::JmpNot { cond, offset } => {
                let target = *offset;
                if let Some(Fact::Predicate(condition)) = facts.get(&cond.0).copied() {
                    *op = if condition == matches!(op, I::JmpIf { .. }) {
                        I::Jmp { offset: target }
                    } else {
                        I::Nop
                    };
                }
                facts.clear();
            }
            I::Assert { .. } | I::Nop => {}
            _ => facts.clear(),
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/mono_resource_branches.rs"]
mod tests;
