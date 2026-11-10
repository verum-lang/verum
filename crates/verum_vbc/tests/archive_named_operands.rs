use std::sync::Arc;
use verum_vbc::{
    archive::ArchiveBuilder,
    bytecode::{decode_instruction, encode_instruction},
    encoding::{decode_reg, decode_varint, encode_varint},
    instruction::{CbgrSubOpcode, Instruction, Reg},
    interpreter::{Interpreter, InterpreterError},
    linker::VbcLinker,
    module::{FunctionDescriptor, VbcModule},
    types::StringId,
};

fn program(owner: &str, name: &str, build: impl FnOnce(u32) -> Vec<Instruction>) -> VbcModule {
    let mut module = VbcModule::new(owner.to_owned());
    let name = module.intern_string(name).0;
    let mut desc = FunctionDescriptor::new(module.intern_string(&format!("{owner}.probe")));
    desc.register_count = 3;
    desc.has_source_body = true;
    for instr in build(name) {
        encode_instruction(&instr, &mut module.bytecode);
    }
    desc.bytecode_length = module.bytecode.len() as u32;
    module.add_function(desc);
    module
}

#[test]
fn named_operands_follow_their_string_when_archive_offsets_change() {
    let mut archive = ArchiveBuilder::new();
    let padding = program("prefix", "padding".repeat(55).as_str(), |_| {
        vec![Instruction::Ret { value: Reg(0) }]
    });
    let source = program("subject", "declared-field-or-method", |id| {
        let mut ref_field = vec![0, 1];
        encode_varint(id.into(), &mut ref_field);
        vec![
            Instruction::Panic { message_id: id },
            Instruction::CmpG {
                eq: true,
                dst: Reg(2),
                a: Reg(0),
                b: Reg(1),
                protocol_id: id + 1,
            },
            Instruction::CtxGet {
                dst: Reg(2),
                ctx_type: id,
            },
            Instruction::CbgrExtended {
                sub_op: CbgrSubOpcode::RefFieldNamed as u8,
                operands: ref_field,
            },
            Instruction::Ret { value: Reg(0) },
        ]
    });
    for module in [padding, source] {
        archive.add_module(&module.name, &module, &[]).unwrap();
    }
    let mut linker = VbcLinker::new("aarch64-apple-darwin");
    linker.add_archive(&archive.finish()).unwrap();
    let bytes = verum_vbc::serialize::serialize_module(&linker.finalize()).unwrap();
    let module = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    let target = module.find_function_by_name("subject.probe").unwrap();
    let desc = module.get_function(target).unwrap();
    let mut offset = desc.bytecode_offset as usize;
    let mut next = || decode_instruction(&module.bytecode, &mut offset).unwrap();
    let Instruction::Panic { message_id } = next() else {
        panic!()
    };
    assert_eq!(
        module.get_string(StringId(message_id)),
        Some("declared-field-or-method")
    );
    let Instruction::CmpG { protocol_id, .. } = next() else {
        panic!()
    };
    assert_eq!(protocol_id, message_id + 1);
    let Instruction::CtxGet { dst, ctx_type } = next() else {
        panic!()
    };
    assert_eq!((dst, ctx_type), (Reg(2), message_id));
    let Instruction::CbgrExtended { operands, .. } = next() else {
        panic!()
    };
    let mut offset = 0;
    assert_eq!(decode_reg(&operands, &mut offset).unwrap(), Reg(0));
    assert_eq!(decode_reg(&operands, &mut offset).unwrap(), Reg(1));
    assert_eq!(
        decode_varint(&operands, &mut offset).unwrap(),
        u64::from(message_id)
    );
    assert_eq!(offset, operands.len());
    let error = Interpreter::new(Arc::new(module))
        .execute_function(target)
        .unwrap_err();
    assert!(
        matches!(error,InterpreterError::Panic{message} if message.contains("declared-field-or-method"))
    );
}

#[test]
fn malformed_or_unbound_named_operands_fail_during_link() {
    for instr in [
        Instruction::Panic {
            message_id: u32::MAX,
        },
        Instruction::CbgrExtended {
            sub_op: CbgrSubOpcode::RefFieldNamed as u8,
            operands: vec![0, 0, 0x80],
        },
    ] {
        let module = program("damaged", "exists", |_| vec![instr]);
        assert!(
            VbcLinker::new("aarch64-apple-darwin")
                .add_user_module(module)
                .is_err()
        );
    }
}
