//! T1698: call-site and linked endian intrinsics preserve fixed byte storage.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_vbc::{
    bytecode::encode_instructions,
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    instruction::{Instruction, Reg, RegRange},
    interpreter::{Interpreter, ObjectHeader},
    module::{FunctionDescriptor, FunctionId, VbcModule, XMOD_CALL_ID_BAND_BASE},
    serialize::serialize_module,
    types::{TypeId, TypeRef},
};

fn assert_bytes(module: VbcModule, expected: &[u8]) {
    let wire = deserialize_module(&serialize_module(&module).expect("serialize")).expect("deserialize");
    for (route, module) in [("source", module), ("serialized", wire)] {
        let entry = module.functions.iter().find(|function|
            module.get_string(function.name) == Some("endian_expansion.probe")
        ).expect("exact entry").id;
        let mut interpreter = Interpreter::new(Arc::new(module));
        let value = interpreter.execute_function(entry).expect("execute endian producer");
        assert!(value.is_ptr(), "{route}: byte array pointer");
        // The returned object remains live in this interpreter. Validate its
        // header before reading the packed bytes promised by the declaration.
        let header = unsafe { &*value.as_ptr::<ObjectHeader>() };
        assert_eq!(header.type_id, TypeId::U8, "{route}: packed byte carrier");
        assert_eq!(header.size as usize, expected.len(), "{route}: fixed width");
        let bytes = unsafe { std::slice::from_raw_parts(
            value.as_ptr::<u8>().add(std::mem::size_of::<ObjectHeader>()), expected.len()
        ) };
        assert_eq!(bytes, expected, "{route}: every byte");
    }
}

fn wide_source(local_count: usize, little_endian: bool) -> VbcModule {
    let mut source = Text::from("module endian_expansion; fn probe() -> [Byte; 8] {");
    for index in 0..local_count {
        source.push_str(&format!("let retained_{index} = {index}; "));
    }
    source.push_str(if little_endian {
        "to_le_bytes_8(0x0807060504030201) }"
    } else {
        "to_be_bytes_8(0x0102030405060708) }"
    });
    let module = VbcCodegen::with_config(CodegenConfig::new("endian_expansion"))
        .compile_module(&Parser::new(&source).parse_module().expect("grammar"))
        .expect("codegen");
    let entry = module.functions.iter().find(|function|
        module.get_string(function.name) == Some("endian_expansion.probe")
    ).expect("entry");
    assert!(entry.register_count as usize > local_count, "fixture must retain the large register frame");
    module
}

#[test]
fn inline_endian_operands_preserve_registers_above_127() {
    for little in [false, true] {
        assert_bytes(wide_source(140, little), &[1,2,3,4,5,6,7,8]);
    }
}

#[test]
fn inline_endian_operands_preserve_registers_above_255() {
    for little in [false, true] {
        assert_bytes(wide_source(270, little), &[1,2,3,4,5,6,7,8]);
    }
}

#[test]
fn linked_endian_wrappers_have_the_same_fixed_carrier() {
    for width in [2u8, 4, 8] {
        for endian in ["be", "le"] {
            let mut module = VbcModule::new("endian_expansion".into());
            let band = XMOD_CALL_ID_BAND_BASE + 53;
            let external = module.intern_string(&format!("core.intrinsics.conversion.to_{endian}_bytes_{width}"));
            module.external_function_names.push((FunctionId(band), external));
            let mut function = FunctionDescriptor::new(module.intern_string("endian_expansion.probe"));
            function.return_type = TypeRef::Array { element: Box::new(TypeRef::Concrete(TypeId::U8)), length: u64::from(width) };
            function.register_count = 2;
            let body: List<_> = [
                Instruction::LoadI { dst: Reg(1), value: 0x0102030405060708 },
                Instruction::Call { dst: Reg(0), func_id: band, args: RegRange { start: Reg(1), count: 1 } },
                Instruction::Ret { value: Reg(0) },
            ].into_iter().collect();
            function.bytecode_length = encode_instructions(&body, &mut module.bytecode) as u32;
            function.instructions = Some(body.into());
            module.add_function(function);
            assert_eq!(module.resolve_external_bands().len(), 1);
            assert_eq!(module.synthesize_intrinsic_band_wrappers(), 1);
            let mut expected: List<u8> = (9-width..=8).collect();
            if endian == "le" { expected.reverse(); }
            assert_bytes(module, &expected);
        }
    }
}
