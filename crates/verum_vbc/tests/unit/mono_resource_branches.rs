use super::*;
use crate::instruction::Reg;
use crate::types::{TypeId, TypeParamId, TypeRef};

fn ownership_branch(ty: TypeRef) -> List<I> {
    vec![
        I::Nop,
        I::TypeResource {
            dst: Reg(0),
            type_ref: ty,
        },
        I::LoadI {
            dst: Reg(1),
            value: ResourceDiscipline::Unrestricted as i64,
        },
        I::CmpI {
            dst: Reg(2),
            op: CompareOp::Eq,
            a: Reg(0),
            b: Reg(1),
        },
        I::JmpIf {
            cond: Reg(2),
            offset: 2,
        },
        I::Nop,
        I::Nop,
    ]
    .into()
}

#[test]
fn only_closed_resource_facts_decide_ownership_branches() {
    let module = VbcModule::new("test".into());
    for (ty, decided) in [
        (TypeRef::Concrete(TypeId::INT), true),
        (TypeRef::Concrete(TypeId(99999)), false),
        (TypeRef::Generic(TypeParamId(4)), false),
    ] {
        let mut body = ownership_branch(ty);
        let original = body.clone();
        let entries = entry_points(&mut body);
        fold(&module, &entries, &mut body);
        if decided {
            assert_eq!(body[4], I::Jmp { offset: 2 });
        } else {
            assert_eq!(body, original);
        }
    }
}

#[test]
fn joins_including_exception_and_switch_edges_preserve_dynamic_tests() {
    let module = VbcModule::new("test".into());
    for edge in [
        I::Guard {
            reg: Reg(5),
            expected_type: 300,
            deopt_offset: 4,
        },
        I::JmpIf {
            cond: Reg(5),
            offset: 3,
        },
        I::TryBegin { handler_offset: 4 },
        I::CtxProvide {
            ctx_type: 0,
            value: Reg(5),
            body_offset: 3,
        },
        I::Switch {
            value: Reg(5),
            default_offset: 5,
            cases: vec![(3, 4), (9, 3)],
        },
    ] {
        let mut body = ownership_branch(TypeRef::Concrete(TypeId::INT));
        body[0] = edge;
        let entries = entry_points(&mut body);
        fold(&module, &entries, &mut body);
        assert_eq!(
            body[4],
            I::JmpIf {
                cond: Reg(2),
                offset: 2
            }
        );
    }
}

#[cfg(feature = "codegen")]
#[test]
fn affine_declarations_transfer_and_a_reachable_copy_still_has_no_plan() {
    let ast = verum_fast_parser::Parser::new("type affine Ticket is { value: Int };")
        .parse_module()
        .unwrap();
    let module = crate::codegen::VbcCodegen::new()
        .compile_module(&ast)
        .unwrap();
    let declared = module
        .types
        .iter()
        .find(|ty| module.get_string(ty.name) == Some("Ticket"))
        .unwrap();
    let ty = TypeRef::Concrete(declared.id);
    let mut body = ownership_branch(ty.clone());
    body[5] = I::CloneTyped {
        dst: Reg(6),
        src: Reg(5),
        type_ref: ty.clone(),
    };
    let copy = body[5].clone();
    let entries = entry_points(&mut body);
    fold(&module, &entries, &mut body);
    assert_eq!(body[4], I::Nop);
    assert_eq!(body[5], copy);
    assert!(crate::copy_plan::build(&module, &ty).is_err());
}
