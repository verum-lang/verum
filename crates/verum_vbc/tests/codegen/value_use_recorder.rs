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

#[test]
fn mandatory_return_guard_survives_optional_receipt_exhaustion() {
    let mut recorder = ValueUseRecorder::default();
    recorder.allocate(Reg(0));
    recorder.set_type(
        Reg(0),
        TypeRef::Concrete(crate::types::TypeId(600)),
        ResourceDiscipline::Affine,
    );
    for index in 0..=MAX_VALUE_USES {
        recorder.observe(&Instruction::Ret { value: Reg(0) }, index);
    }
    assert!(recorder.exhausted);
    assert!(recorder.begin_return_use_capture());
    recorder.observe(
        &Instruction::Mov {
            dst: Reg(1),
            src: Reg(0),
        },
        MAX_VALUE_USES + 1,
    );
    assert!(recorder.finish_return_use_capture());
    assert!(recorder.uses.is_empty());
}

#[test]
fn return_capture_bound_and_recycled_binding_cannot_reuse_affine_identity() {
    let mut recorder = ValueUseRecorder::default();
    recorder.allocate(Reg(0));
    recorder.set_type(
        Reg(0),
        TypeRef::Concrete(crate::types::TypeId(600)),
        ResourceDiscipline::Affine,
    );
    for _ in 0..128 {
        assert!(recorder.begin_return_use_capture());
    }
    assert!(!recorder.begin_return_use_capture());
    recorder.forget(Reg(0));
    recorder.allocate(Reg(0));
    recorder.set_type(
        Reg(0),
        TypeRef::Concrete(crate::types::TypeId::INT),
        ResourceDiscipline::Unrestricted,
    );
    recorder.observe(
        &Instruction::Mov {
            dst: Reg(1),
            src: Reg(0),
        },
        0,
    );
    for _ in 0..128 {
        assert!(!recorder.finish_return_use_capture());
    }
    assert!(recorder.return_use_capture.is_empty());
    assert!(recorder.begin_return_use_capture());
    assert!(!recorder.finish_return_use_capture());
}

#[test]
fn nested_function_capture_is_separate_and_outer_capture_is_restored() {
    let mut registers = RegisterAllocator::new();
    let source = registers.alloc_local("outer", false);
    registers.value_uses.set_type(
        source,
        TypeRef::Concrete(crate::types::TypeId(600)),
        ResourceDiscipline::Affine,
    );
    assert!(registers.value_uses.begin_return_use_capture());
    let saved = registers.snapshot();
    registers.reset();
    assert!(registers.value_uses.return_use_capture.is_empty());
    registers.restore_reg(&saved);
    let temporary = registers.alloc_temp();
    registers.value_uses.observe(
        &Instruction::Mov {
            dst: temporary,
            src: source,
        },
        0,
    );
    assert!(registers.value_uses.finish_return_use_capture());
}

#[test]
fn direct_return_declines_name_only_escaping_even_for_a_shadowed_local() {
    use crate::codegen::{CodegenConfig, VbcCodegen};
    use verum_fast_parser::Parser;
    let ast = Parser::new("type affine Token is { value: Int };")
        .parse_module()
        .unwrap();
    for escaping in [None, Some("unrelated"), Some("held")] {
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new("return_guard"));
        codegen.collect_unit_declarations(&[&ast]).unwrap();
        let owned = TypeRef::Concrete(*codegen.type_name_to_id.get("Token").unwrap());
        codegen
            .ctx
            .begin_function("probe", &[], Some(owned.clone()));
        let outer = codegen.ctx.define_var("held", false);
        codegen
            .ctx
            .registers
            .value_uses
            .set_type(outer, owned.clone(), ResourceDiscipline::Affine);
        codegen.ctx.enter_scope();
        let inner = codegen.ctx.define_var("held", false);
        codegen.ctx.registers.value_uses.set_type(
            inner,
            TypeRef::Concrete(crate::types::TypeId::INT),
            ResourceDiscipline::Unrestricted,
        );
        let returned = codegen.ctx.define_var("returned", false);
        codegen
            .ctx
            .registers
            .value_uses
            .set_type(returned, owned, ResourceDiscipline::Affine);
        if let Some(name) = escaping {
            codegen.ctx.current_fn_escaping_vars.insert(name.into());
        }
        let expr = Parser::new("returned").parse_expr().unwrap();
        let result = codegen.emit_direct_affine_return(&expr, returned, false);
        if escaping == Some("held") {
            assert!(
                result.is_err(),
                "name-only exclusion cannot suppress exact local cleanup"
            );
            assert!(
                codegen.ctx.instructions.is_empty(),
                "refusal must precede source consumption"
            );
        } else {
            assert_eq!(result.unwrap(), true);
            for expected in [outer, inner, returned] {
                assert!(codegen.ctx.instructions.iter().any(|instruction| matches!(instruction, Instruction::DropRef { src } if *src == expected)));
            }
        }
    }
}

#[test]
fn return_expression_error_restores_the_capture_stack() {
    use crate::codegen::{CodegenConfig, VbcCodegen};
    use verum_fast_parser::Parser;
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("return_guard"));
    codegen.ctx.begin_function("probe", &[], None);
    let expr = Parser::new("return break").parse_expr().unwrap();
    assert!(codegen.compile_expr(&expr).is_err());
    assert!(
        codegen
            .ctx
            .registers
            .value_uses
            .return_use_capture
            .is_empty()
    );
    assert!(codegen.ctx.registers.value_uses.begin_return_use_capture());
    assert!(!codegen.ctx.registers.value_uses.finish_return_use_capture());
}
