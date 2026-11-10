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

fn instructions(
    module: &verum_vbc::module::VbcModule,
    id: verum_vbc::module::FunctionId,
) -> Vec<Instruction> {
    let function = module.get_function(id).unwrap();
    let start = function.bytecode_offset as usize;
    verum_vbc::bytecode::decode_instructions(
        &module.bytecode[start..start + function.bytecode_length as usize],
    )
    .unwrap()
}

#[test]
fn signed_literal_arguments_carry_their_declared_or_default_witness() {
    let cases = [
        ("positive", "8", TypeId::INT, 8),
        ("negative", "-8", TypeId::INT, 8),
        ("narrow", "-(8_i16)", TypeId::I16, 2),
        ("precise", "-1.25_f32", TypeId::F32, 4),
        ("floating", "-1.25", TypeId::FLOAT, 8),
        ("local", "{ let x = -8_i8; x }", TypeId::I8, 1),
    ];
    let mut source = String::from("fn width<T>(value: T) -> Int { T.size }\n");
    for (name, value, _, _) in &cases {
        source.push_str(&format!("fn {name}() -> Int {{ width({value}) }}\n"));
    }
    let bytes = verum_vbc::serialize::serialize_module(&compile(&source)).unwrap();
    let module = Arc::new(verum_vbc::deserialize::deserialize_module(&bytes).unwrap());
    verum_vbc::validate::validate_module(&module).expect("valid generic calls after serialization");
    let mut vm = Interpreter::new(module.clone());
    for (name, _, ty, size) in cases {
        let id = module.find_function_by_name(name).unwrap();
        let code = instructions(&module, id);
        assert!(
            code.iter()
                .any(|op| matches!(op, Instruction::CallG { type_args, .. }
            if type_args == &[TypeRef::Concrete(ty)])),
            "{name}: {code:?}"
        );
        assert_eq!(vm.execute_function(id).unwrap().as_i64(), size, "{name}");
    }
}

#[test]
fn an_unbound_callee_parameter_cannot_capture_the_same_numbered_caller_slot() {
    let module = compile(
        "
        fn inspect<T, U>(value: T) -> Int { U.size }
        fn relay<A, B>(a: A, b: B) -> Int { inspect(-8) }
        fn direct() -> Int { inspect(-8) }
        fn indirect() -> Int { relay(true, 8_i16) }
    ",
    );
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let module = Arc::new(verum_vbc::deserialize::deserialize_module(&bytes).unwrap());
    verum_vbc::validate::validate_module(&module).unwrap();
    for caller in ["direct", "relay"] {
        let code = instructions(&module, module.find_function_by_name(caller).unwrap());
        assert!(
            code.iter()
                .any(|op| matches!(op, Instruction::CallG { type_args, .. }
            if type_args == &[TypeRef::Concrete(TypeId::INT)])),
            "{caller}: {code:?}"
        );
    }
    for name in ["direct", "indirect"] {
        let id = module.find_function_by_name(name).unwrap();
        let error = Interpreter::new(module.clone())
            .execute_function(id)
            .unwrap_err();
        assert!(error.to_string().contains("layout"), "{name}: {error}");
    }
}

#[test]
fn explicit_arguments_can_complete_an_interior_hole_before_encoding() {
    let declaration = "fn pair<A, B>(right: B) -> Int { A.size * 100 + B.size }";
    let missing = format!("{declaration} fn probe() -> Int {{ pair(-7_i16) }}");
    let error = VbcCodegen::new()
        .compile_module(&Parser::new(&missing).parse_module().unwrap())
        .expect_err("a later witness cannot shift into an earlier slot");
    let diagnostic = error.to_string();
    assert!(
        diagnostic.contains("cannot infer generic argument 1 `A`") && diagnostic.contains("pair"),
        "{diagnostic}"
    );

    let module = compile(&format!(
        "{declaration} fn probe() -> Int {{ pair<Byte>(-7_i16) }}"
    ));
    let id = module.find_function_by_name("probe").unwrap();
    let code = instructions(&module, id);
    assert!(
        code.iter()
            .any(|op| matches!(op, Instruction::CallG { type_args, .. }
        if type_args == &[TypeRef::Concrete(TypeId::U8), TypeRef::Concrete(TypeId::I16)])),
        "{code:?}"
    );
    assert_eq!(
        Interpreter::new(Arc::new(module))
            .execute_function(id)
            .unwrap()
            .as_i64(),
        102
    );
}

#[test]
fn closed_sequences_keep_elements_extents_and_caller_identity_after_serialization() {
    let module = compile(r#"
        const N: Int = 4;
        const COUNT: Int = 3;
        fn digit<const Values: [Int]>() -> Int { Values[0] * 10 + Values[1] }
        fn forward<const Shape: [Int]>() -> Int { digit<Shape>() }
        fn nested<const Rows: [[Int; 2]; 2]>() -> Int { Rows[1][0] }
        fn probe() -> Int {
            forward<[N; COUNT]>() + forward<[2,-3]>() + nested<[[2,3],[7,8]]>()
        }
    "#);
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let module = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    verum_vbc::validate::validate_module(&module).unwrap();
    let entry = module.find_function_by_name("probe").unwrap();
    assert_eq!(Interpreter::new(Arc::new(module)).execute_function(entry).unwrap().as_i64(), 68);
}

#[test]
fn const_sequences_reject_wrong_elements_extents_and_unbounded_repeats() {
    for (domain, argument) in [
        ("[Int; 2]", "[1]"), ("[Int]", "[1, false]"), ("[Int]", "[1_u8,2]"),
        ("[Int]", "[false; 0]"), ("[Int]", "[2; -1]"), ("[Int]", "[0; 1048577]"),
        ("[[Int]]", "[[1; 8192]; 8192]"), ("[Int]", "[Bad]"),
    ] {
        let source = format!("const Bad: Bool = true; fn size<const S: {domain}>() -> Int {{ 1 }} fn probe() -> Int {{ size<{argument}>() }}");
        let ast = Parser::new(&source).parse_module().unwrap();
        let error = VbcCodegen::new().compile_module(&ast).expect_err(&source);
        assert!(error.span.is_some(), "{argument}: {error}");
    }
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
