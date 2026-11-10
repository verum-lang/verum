//! Baked archive entries own their dense tables and their symbolic call operands.
use std::sync::Arc;
use verum_vbc::{
    archive::ArchiveBuilder,
    bytecode,
    instruction::{BinaryIntOp, Instruction, Reg, RegRange},
    interpreter::Interpreter,
    linker::VbcLinker,
    module::{Constant, FunctionDescriptor, FunctionId, VbcModule, XMOD_CALL_ID_BAND_BASE},
    types::{TypeId, TypeRef},
    validate::validate_module,
};

fn entry(
    owner: &str,
    name: &str,
    constants: &[i64],
    refs: &[&str],
    body: &[Instruction],
) -> VbcModule {
    let mut module = VbcModule::new(owner.into());
    let mut f = FunctionDescriptor::new(module.intern_string(name));
    f.origin_module = Some(module.intern_string(owner));
    f.return_type = TypeRef::Concrete(TypeId::INT);
    f.has_source_body = true;
    f.register_count = 4;
    for c in constants {
        module.constants.push(Constant::Int(*c));
    }
    for (index, reference) in refs.iter().enumerate() {
        let sid = module.intern_string(reference);
        module
            .external_function_names
            .push((FunctionId(XMOD_CALL_ID_BAND_BASE + index as u32), sid));
    }
    for instruction in body {
        bytecode::encode_instruction(instruction, &mut module.bytecode);
    }
    f.bytecode_length = module.bytecode.len() as u32;
    module.add_function(f);
    module
}
fn call(dst: u16, ordinal: u32) -> Instruction {
    Instruction::Call {
        dst: Reg(dst),
        func_id: XMOD_CALL_ID_BAND_BASE + ordinal,
        args: RegRange {
            start: Reg(0),
            count: 0,
        },
    }
}
fn linked(second: &str) -> VbcModule {
    let value = [
        Instruction::LoadK {
            dst: Reg(0),
            const_id: 0,
        },
        Instruction::Ret { value: Reg(0) },
    ];
    let wrapper = [call(0, 0), Instruction::Ret { value: Reg(0) }];
    let mut archive = ArchiveBuilder::new();
    for module in [
        entry("fixture.a", "value", &[17], &[], &value),
        entry("fixture.b", "value", &[31], &[], &value),
        entry(
            "fixture.read_a",
            "read",
            &[],
            &["fixture.a.value"],
            &wrapper,
        ),
        entry("fixture.read_b", "read", &[], &[second], &wrapper),
        entry(
            "fixture.consumer",
            "probe",
            &[],
            &["fixture.read_a.read", "fixture.read_b.read"],
            &[
                call(0, 0),
                call(1, 1),
                Instruction::BinaryI {
                    op: BinaryIntOp::Sub,
                    dst: Reg(2),
                    a: Reg(1),
                    b: Reg(0),
                },
                Instruction::Ret { value: Reg(2) },
            ],
        ),
    ] {
        archive.add_module(&module.name, &module, &[]).unwrap();
    }
    let mut linker = VbcLinker::new("aarch64-apple-darwin");
    linker.add_archive(&archive.finish()).unwrap();
    let module = linker.finalize();
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let mut reloaded = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    reloaded.resolve_external_bands();
    reloaded
}
#[test]
fn same_local_call_and_constant_slots_keep_their_declarations() {
    let module = linked("fixture.b.value");
    validate_module(&module).unwrap();
    let f = module.find_function_by_name("probe").unwrap();
    assert_eq!(
        Interpreter::new(Arc::new(module))
            .execute_function(f)
            .unwrap()
            .try_as_i64(),
        Some(14)
    );
}
#[test]
fn a_foreign_homonym_cannot_satisfy_a_missing_external() {
    let module = linked("fixture.missing.value");
    assert!(validate_module(&module).is_err());
}

#[test]
fn a_name_from_another_entry_cannot_authorize_an_unlabelled_band() {
    let wrapper = [call(0, 0), Instruction::Ret { value: Reg(0) }];
    let mut archive = ArchiveBuilder::new();
    for module in [
        entry("fixture.good", "good", &[], &["fixture.dep.run"], &wrapper),
        entry("fixture.malformed", "bad", &[], &[], &wrapper),
    ] {
        archive.add_module(&module.name, &module, &[]).unwrap();
    }
    assert!(
        VbcLinker::new("aarch64-apple-darwin")
            .add_archive(&archive.finish())
            .is_err()
    );
}

#[test]
fn sparse_legacy_archives_preserve_a_unique_cross_entry_ordinal() {
    let mut caller = entry(
        "fixture.caller",
        "caller",
        &[],
        &[],
        &[
            Instruction::Call {
                dst: Reg(0),
                func_id: 20,
                args: RegRange {
                    start: Reg(0),
                    count: 0,
                },
            },
            Instruction::Ret { value: Reg(0) },
        ],
    );
    caller.functions[0].id = FunctionId(10);
    let mut callee = entry(
        "fixture.callee",
        "callee",
        &[],
        &[],
        &[
            Instruction::LoadI {
                dst: Reg(0),
                value: 39,
            },
            Instruction::Ret { value: Reg(0) },
        ],
    );
    callee.functions[0].id = FunctionId(20);
    let mut archive = ArchiveBuilder::new();
    for module in [caller, callee] {
        archive.add_module(&module.name, &module, &[]).unwrap();
    }
    let mut linker = VbcLinker::new("aarch64-apple-darwin");
    linker.add_archive(&archive.finish()).unwrap();
    let module = linker.finalize();
    validate_module(&module).unwrap();
    let id = module.find_function_by_name("caller").unwrap();
    assert_eq!(
        Interpreter::new(Arc::new(module))
            .execute_function(id)
            .unwrap()
            .try_as_i64(),
        Some(39)
    );
}
