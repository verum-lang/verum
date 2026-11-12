#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{codegen::VbcCodegen, interpreter::Interpreter};

#[test]
fn inspection_follows_complete_generic_declarations_after_archive_roundtrip() {
    let source = r#"
        module reflection;
        type Word is UInt32;
        type Tag<T> is { value: T };
        type Alias<T> is Tag<T>;
        fn name<T>() -> Text { T.name }
        fn inspect<T>() -> Text { @intrinsic("type_name", T) }
        fn needs_drop<T>() -> Bool { @vbc(NEEDS_DROP, T) }
        fn probe() -> Int {
            assert(name<Word>() == "UInt32");
            assert(inspect<Tag<Word>>() == "reflection.Tag<UInt32>");
            assert(name<Alias<Word>>() == inspect<Tag<UInt32>>());
            assert(name<Tag<Word>>() != name<Tag<UInt64>>());
            assert(name<&mut UInt32>() == "&mut UInt32");
            assert(name<[Byte; 3]>() != name<[Byte; 4]>());
            assert(!needs_drop<Int>());
            assert(!needs_drop<&Tag<Int>>());
            assert(needs_drop<Tag<Int>>());
            73
        }
    "#;
    let original = VbcCodegen::new()
        .compile_module(&Parser::new(source).parse_module().unwrap())
        .unwrap();
    let restored = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&original).unwrap(),
    )
    .unwrap();
    for module in [original, restored] {
        let probe = module.find_function_by_name("reflection.probe").unwrap();
        let mut vm = Interpreter::new(Arc::new(module));
        assert_eq!(vm.execute_function(probe).unwrap().try_as_i64(), Some(73));
    }
}

#[test]
fn incomplete_queries_and_legacy_reflection_cannot_fabricate_answers() {
    use verum_vbc::{
        Instruction, Reg, VbcModule,
        types::{TypeId, TypeRef},
    };
    let module = VbcModule::new("missing".into());
    for ty in [
        TypeRef::Concrete(TypeId(10000)),
        TypeRef::Generic(Default::default()),
    ] {
        assert!(verum_vbc::type_information::name(&module, &ty).is_err());
        assert!(verum_vbc::type_information::needs_drop(&module, &ty).is_err());
    }
    let mut module = VbcCodegen::new()
        .compile_module(
            &Parser::new("fn probe() -> Int { 73 }")
                .parse_module()
                .unwrap(),
        )
        .unwrap();
    let probe = module.find_function_by_name("probe").unwrap();
    module.bytecode.clear();
    verum_vbc::bytecode::encode_instructions(
        &[
            Instruction::MetaReflect {
                dst: Reg(0),
                type_id: TypeId::INT.0,
            },
            Instruction::Ret { value: Reg(0) },
        ],
        &mut module.bytecode,
    );
    let function = module.functions.iter_mut().find(|f| f.id == probe).unwrap();
    function.bytecode_offset = 0;
    function.bytecode_length = module.bytecode.len() as u32;
    let error = Interpreter::new(Arc::new(module))
        .execute_function(probe)
        .unwrap_err();
    assert!(
        error.to_string().contains("complete type witness"),
        "{error}"
    );
}

#[test]
fn destruction_inspection_refuses_missing_field_evidence() {
    use verum_vbc::{
        VbcModule,
        types::{FieldDescriptor, TypeDescriptor, TypeId, TypeKind, TypeRef},
    };
    let mut module = VbcModule::new("broken".into());
    module.types.push(TypeDescriptor {
        id: TypeId(1000),
        kind: TypeKind::Record,
        fields: smallvec::smallvec![FieldDescriptor::default()],
        ..Default::default()
    });
    assert!(
        verum_vbc::type_information::needs_drop(&module, &TypeRef::Concrete(TypeId(1000)))
            .unwrap_err()
            .contains("field lacks declaration evidence")
    );
}
