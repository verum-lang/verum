//! A typed retained-copy implementation is proved from the whole selected body.
#![cfg(feature = "codegen")]
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::VbcCodegen, deserialize::deserialize_module, serialize::serialize_module,
};

const SOURCE: &str = r#"
    type Inner<T> is { strong: UInt32, value: T };
    type Handle<T> is { ptr: &unsafe Inner<T>, generation: UInt32, epoch: UInt16 };
    fn retain<T>(handle: &Handle<T>) -> Handle<T> {
        unsafe { @intrinsic("retain_copy", handle, &mut (*handle.ptr).strong) }
    }
"#;
fn wire() -> verum_vbc::VbcModule {
    let ast = Parser::new(SOURCE).parse_module().unwrap();
    let module = VbcCodegen::new().compile_module(&ast).unwrap();
    deserialize_module(&serialize_module(&module).unwrap()).unwrap()
}
fn owner(module: &verum_vbc::VbcModule) -> verum_vbc::TypeRef {
    let owner = module
        .types
        .iter()
        .find(|d| module.get_string(d.name) == Some("Handle"))
        .unwrap()
        .id;
    verum_vbc::TypeRef::Instantiated {
        base: owner,
        args: vec![verum_vbc::TypeRef::Concrete(verum_vbc::types::TypeId::I64)],
    }
}
fn proof(
    module: &verum_vbc::VbcModule,
) -> Result<verum_vbc::retain_forward::RetainForward, verum_common::Text> {
    let function = module.find_function_by_name("retain").unwrap();
    verum_vbc::retain_forward::RetainForward::verify(
        module,
        function,
        &[verum_vbc::TypeRef::Concrete(verum_vbc::types::TypeId::I64)],
        &owner(module),
    )
}
fn rewrite(
    module: &mut verum_vbc::VbcModule,
    edit: impl FnOnce(&mut verum_common::List<verum_vbc::Instruction>),
) {
    let id = module.find_function_by_name("retain").unwrap();
    let d = module.get_function(id).unwrap();
    let bytes = &module.bytecode
        [d.bytecode_offset as usize..(d.bytecode_offset + d.bytecode_length) as usize];
    let mut pos = 0;
    let mut body = verum_common::List::new();
    while pos < bytes.len() {
        body.push(verum_vbc::bytecode::decode_instruction(bytes, &mut pos).unwrap());
    }
    edit(&mut body);
    let mut bytes = verum_common::List::new().into();
    verum_vbc::bytecode::encode_instructions_with_fixup(&body, &mut bytes);
    module.bytecode = bytes;
    let d = module.functions.iter_mut().find(|d| d.id == id).unwrap();
    d.bytecode_offset = 0;
    d.bytecode_length = module.bytecode.len() as u32;
    d.instructions = None;
}
#[test]
fn source_and_wire_prove_the_selected_generic_counter_path() {
    let module = wire();
    let receipt = proof(&module).unwrap();
    assert_eq!(receipt.counter_path().len(), 2);
    assert_eq!(
        receipt.counter_path()[1].field,
        verum_vbc::TypeRef::Concrete(verum_vbc::types::TypeId::U32)
    );
    receipt.validate(&module).unwrap();
}
#[test]
fn changed_source_counter_or_result_cannot_supply_retained_ownership() {
    use verum_vbc::{Instruction as I, Reg};
    for case in 0..4 {
        let mut module = wire();
        rewrite(&mut module, |body| {
            for instruction in body {
                match instruction {
                    I::RetainCopy {
                        src,
                        counter,
                        type_ref,
                        ..
                    } => match case {
                        0 => *src = Reg(2),
                        1 => *counter = Reg(0),
                        2 => {
                            *type_ref = verum_vbc::TypeRef::Concrete(verum_vbc::types::TypeId::I64)
                        }
                        _ => (),
                    },
                    I::Ret { value } if case == 3 => *value = Reg(0),
                    _ => (),
                }
            }
        });
        assert!(proof(&module).is_err(), "case {case}");
    }
}
#[test]
fn branches_writes_calls_cycles_and_budget_exhaustion_refuse_the_whole_body() {
    use verum_vbc::{Instruction as I, Reg};
    for case in 0..5 {
        let mut module = wire();
        rewrite(&mut module, |body| match case {
            0 => {
                body.insert(
                    0,
                    I::JmpIf {
                        cond: Reg(0),
                        offset: 2,
                    },
                );
            }
            1 => {
                body.insert(
                    0,
                    I::SetF {
                        obj: Reg(0),
                        field_idx: 0,
                        value: Reg(0),
                    },
                );
            }
            2 => {
                body.insert(
                    0,
                    I::Call {
                        dst: Reg(1),
                        func_id: 0,
                        args: verum_vbc::instruction::RegRange {
                            start: Reg(0),
                            count: 1,
                        },
                    },
                );
            }
            3 => {
                body.insert(0, I::Jmp { offset: 0 });
            }
            _ => {
                drop(body.splice(0..0, std::iter::repeat_n(I::Nop, 4097)));
            }
        });
        assert!(proof(&module).is_err(), "case {case}");
    }
}
#[test]
fn changed_body_signature_or_forward_placeholder_invalidates_the_receipt() {
    for case in 0..4 {
        let mut module = wire();
        let receipt = proof(&module).unwrap();
        let id = module.find_function_by_name("retain").unwrap();
        if case == 0 {
            rewrite(&mut module, |body| {
                body.insert(0, verum_vbc::Instruction::Nop)
            });
        } else {
            let d = module.functions.iter_mut().find(|d| d.id == id).unwrap();
            match case {
                1 => d.register_count += 1,
                2 => d.has_source_body = false,
                _ => d.declaration_identity = None,
            }
        }
        assert!(receipt.validate(&module).is_err(), "case {case}");
    }
}

#[test]
fn changed_selected_field_owner_or_name_invalidates_the_receipt() {
    for field_name in [false, true] {
        let mut module = wire();
        let receipt = proof(&module).unwrap();
        let changed = module.intern_string("Unrelated".into());
        let owner = module
            .types
            .iter_mut()
            .find(|d| {
                d.id == match receipt.counter_path()[1].record {
                    verum_vbc::TypeRef::Instantiated { base, .. }
                    | verum_vbc::TypeRef::Concrete(base) => base,
                    _ => panic!("declared record"),
                }
            })
            .unwrap();
        if field_name {
            owner.fields[0].name = changed;
        } else {
            owner.origin_module = Some(changed);
        }
        assert!(receipt.validate(&module).is_err());
    }
}
