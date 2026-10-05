//! Private producer checks against the actual register allocator.
use super::*;
use crate::codegen::registers::RegisterAllocator;

#[test]
fn recycled_temporary_cannot_inherit_departed_source_binding() {
    let mut registers = RegisterAllocator::new();
    registers.enter_scope();
    let old = registers.alloc_local("old", false);
    registers.value_uses.set_type(
        old,
        TypeRef::Concrete(crate::types::TypeId::INT),
        ResourceDiscipline::Unrestricted,
    );
    let temporary = registers.alloc_temp();
    registers.value_uses.observe(
        &Instruction::Mov {
            dst: temporary,
            src: old,
        },
        0,
    );
    assert!(registers.value_uses.fact(temporary).is_some());
    registers.free_temp(temporary);
    assert!(
        registers.value_uses.fact(temporary).is_none(),
        "released temporary retains source identity"
    );
    registers.exit_scope();
    let reused = registers.alloc_temp();
    assert_eq!(reused, old);
    assert!(registers.value_uses.fact(reused).is_none());
    let new = registers.alloc_local("new", false);
    registers.value_uses.observe(
        &Instruction::Mov {
            dst: new,
            src: reused,
        },
        1,
    );
    assert!(registers.value_uses.uses.is_empty());
}

#[test]
fn use_budget_exhaustion_discards_the_whole_plan() {
    let mut recorder = ValueUseRecorder::default();
    recorder.allocate(Reg(0));
    for index in 0..=MAX_VALUE_USES {
        recorder.observe(&Instruction::Ret { value: Reg(0) }, index);
    }
    assert!(recorder.uses.is_empty());
    recorder.observe(&Instruction::Ret { value: Reg(0) }, MAX_VALUE_USES + 1);
    assert!(recorder.uses.is_empty());
}
