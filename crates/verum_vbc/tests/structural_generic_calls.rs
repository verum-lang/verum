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
        (
            "[Byte; 3]",
            TypeRef::Array {
                element: Box::new(byte.clone()),
                length: 3,
            },
            3,
        ),
        (
            "[[Byte; 2]; 3]",
            TypeRef::Array {
                element: Box::new(TypeRef::Array {
                    element: Box::new(byte.clone()),
                    length: 2,
                }),
                length: 3,
            },
            6,
        ),
        (
            "[Byte; 0]",
            TypeRef::Array {
                element: Box::new(byte.clone()),
                length: 0,
            },
            0,
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

fn compile(source: &str) -> verum_vbc::module::VbcModule {
    VbcCodegen::new()
        .compile_module(&Parser::new(source).parse_module().expect("source grammar"))
        .expect("source lowering")
}

#[test]
fn named_lengths_and_non_type_slots_preserve_the_selected_witness() {
    for source in [
        "type Element is Byte; fn size<T>() -> Int {T.size} fn probe()->Int {size<[Element; 5]>()}",
        "const N: Int = 2; const M: Int = 3; fn size<T>() -> Int {T.size} fn probe()->Int {size<[Byte; N + M]>()}",
        "const N: Int = 2; const M: Int = 3; fn size<const Shape: [Int], T>() -> Int {T.size} fn probe()->Int {size<[N; M], [Byte; 5]>()}",
        "type Factory<T> is { value: T }; implement<T> Factory<T> { fn size<T>(self)->Int {T.size} } fn probe()->Int {let f: Factory<Int> = Factory {value: 7}; f.size<[Byte; 5]>()}",
    ] {
        let module = compile(source);
        let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
        let module = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
        let probe = module.find_function_by_name("probe").unwrap();
        assert_eq!(
            Interpreter::new(Arc::new(module))
                .execute_function(probe)
                .expect("array witness execution")
                .as_i64(),
            5,
            "{source}"
        );
    }
}

#[test]
fn invalid_projected_lengths_fail_before_execution() {
    for argument in [
        "&[Byte; -1]",
        "[Byte; -1]",
        "fn([Byte; -1]) -> Int",
        "fn() -> [Byte; 18446744073709551616]",
        "[Byte; 18446744073709551616]",
        "[Byte; 1 / 0]",
        "[3; 4]",
        "[3, 4]",
    ] {
        let source =
            format!("fn size<T>() -> Int {{T.size}} fn probe()->Int {{size<{argument}>()}}");
        let ast = Parser::new(&source).parse_module().expect("source grammar");
        assert!(
            VbcCodegen::new().compile_module(&ast).is_err(),
            "invalid array accepted: {source}"
        );
    }
}

#[test]
fn unsized_slice_witness_is_preserved_without_inventing_a_size() {
    let module = compile("fn size<T>() -> Int {T.size} fn probe()->Int {size<[Byte]>()}");
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let module = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    let probe = module.find_function_by_name("probe").unwrap();
    let descriptor = module.functions.iter().find(|f| f.id == probe).unwrap();
    let code = verum_vbc::bytecode::decode_instructions(
        &module.bytecode[descriptor.bytecode_offset as usize
            ..(descriptor.bytecode_offset + descriptor.bytecode_length) as usize],
    )
    .unwrap();
    assert!(code.iter().any(|i| matches!(i, Instruction::CallG {type_args, ..} if type_args == &[TypeRef::Slice(Box::new(TypeRef::Concrete(TypeId::U8)))])));
    assert!(
        Interpreter::new(Arc::new(module))
            .execute_function(probe)
            .is_err(),
        "unsized slices have no standalone layout"
    );
}
