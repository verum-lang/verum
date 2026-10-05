//! Source explicit structural arguments survive CallG and wire into layout queries.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::VbcCodegen,
    instruction::Instruction,
    interpreter::Interpreter,
    types::{CbgrTier, Mutability, TypeId, TypeRef},
};

#[test]
fn structural_call_witnesses_execute_after_wire_roundtrip() {
    let byte = TypeRef::Concrete(TypeId::U8);
    for (spelling, witness, expected) in [
        (
            "&Byte",
            TypeRef::Reference {
                inner: Box::new(byte.clone()),
                mutability: Mutability::Immutable,
                tier: CbgrTier::Tier0,
            },
            16,
        ),
        (
            "&mut Byte",
            TypeRef::Reference {
                inner: Box::new(byte.clone()),
                mutability: Mutability::Mutable,
                tier: CbgrTier::Tier0,
            },
            16,
        ),
        (
            "&checked Byte",
            TypeRef::Reference {
                inner: Box::new(byte.clone()),
                mutability: Mutability::Immutable,
                tier: CbgrTier::Tier1,
            },
            16,
        ),
        (
            "&unsafe Byte",
            TypeRef::Reference {
                inner: Box::new(byte.clone()),
                mutability: Mutability::Immutable,
                tier: CbgrTier::Tier2,
            },
            8,
        ),
        ("Byte", byte, 1),
    ] {
        let source =
            format!("fn size<T>() -> Int {{ T.size }} fn probe() -> Int {{ size<{spelling}>() }}");
        let ast = Parser::new(&source).parse_module().expect("source grammar");
        let module = VbcCodegen::new()
            .compile_module(&ast)
            .expect("source lowering");
        let bytes = verum_vbc::serialize::serialize_module(&module).expect("wire writer");
        let module = verum_vbc::deserialize::deserialize_module(&bytes).expect("wire reader");
        let id = module.find_function_by_name("probe").expect("source probe");
        let function = module.functions.iter().find(|f| f.id == id).unwrap();
        let instructions = verum_vbc::bytecode::decode_instructions(
            &module.bytecode[function.bytecode_offset as usize
                ..(function.bytecode_offset + function.bytecode_length) as usize],
        )
        .expect("decode call");
        assert!(instructions.iter().any(|instruction| matches!(instruction, Instruction::CallG { type_args, .. } if type_args == &[witness.clone()])), "{spelling}: {instructions:?}");
        assert_eq!(
            Interpreter::new(Arc::new(module))
                .execute_function(id)
                .expect("layout execution")
                .as_i64(),
            expected,
            "{spelling}"
        );
    }
}
