#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{codegen::VbcCodegen, interpreter::Interpreter, type_identity::TypeIdentities};

#[test]
fn complete_generic_identity_agrees_with_properties_and_survives_archive() {
    let source = r#"
        meta fn type_id<T>() -> UInt64 { @intrinsic("type_id", T) }
        meta fn opcode<T>() -> UInt64 { @vbc(TYPE_ID, T) }
        fn property<T>() -> UInt64 { T.id }
        fn outer<T>() -> UInt64 { type_id<T>() }
        type Word is UInt16;
        type Tag<T> is { value: T };
        type Alias<T> is Tag<T>;
        type Other<T> is { value: T };
        fn probe() -> Int {
            assert(type_id<Int>() == property<Int64>());
            assert(type_id<USize>() == type_id<UInt64>());
            assert(type_id<Tag<Int>>() == opcode<Tag<Int>>());
            assert(type_id<Float>() == type_id<Float64>());
            assert(outer<Word>() == property<UInt16>());
            assert(type_id<Tag<Int>>() == type_id<Alias<Int>>());
            assert(type_id<Tag<Int>>() != type_id<Tag<UInt64>>());
            assert(type_id<Tag<Int>>() != type_id<Other<Int>>());
            assert(type_id<&Int>() != type_id<&mut Int>());
            assert(type_id<&Int>() != type_id<&checked Int>());
            assert(type_id<[Byte; 2]>() != type_id<[Byte; 3]>());
            assert(type_id<(Int, Bool)>() != type_id<(Bool, Int)>());
            73
        }
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    let module = VbcCodegen::new().compile_module(&ast).unwrap();
    let restored = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    for module in [module, restored] {
        let probe = module.find_function_by_name("probe").unwrap();
        let listing = verum_vbc::disassemble::disassemble_module(&module);
        let mut vm = Interpreter::new(Arc::new(module));
        let result = vm.execute_function(probe).unwrap_or_else(|error| panic!("{error:?}\n{listing}"));
        assert_eq!(result.try_as_i64(), Some(73));
    }
}

#[test]
fn unknown_vbc_operation_cannot_return_a_placeholder_identity() {
    let source = "fn probe() -> UInt64 { @vbc(NO_SUCH_TYPE_QUERY, Int) }";
    let ast = Parser::new(source).parse_module().unwrap();
    let error = VbcCodegen::new().compile_module(&ast).unwrap_err();
    assert!(error.to_string().contains("NO_SUCH_TYPE_QUERY"), "{error}");
}

#[test]
fn erased_refinements_and_callable_contracts_cannot_become_identities() {
    for argument in ["Positive", "[Positive; 2]", "fn(Int) -> Int"] {
        let source = format!("type Positive is Int {{ it > 0 }};
            meta fn identity<T>() -> UInt64 {{ @intrinsic(\"type_id\", T) }}
            fn probe() -> UInt64 {{ identity<{argument}>() }}");
        let ast = Parser::new(&source).parse_module().unwrap();
        if let Ok(module) = VbcCodegen::new().compile_module(&ast) {
            let probe = module.find_function_by_name("probe").unwrap();
            let mut vm = Interpreter::new(Arc::new(module));
            assert!(vm.execute_function(probe).is_err(), "erased semantic witness: {argument}");
        }
    }
}

#[test]
fn compact_identity_collision_is_refused_without_replacing_evidence() {
    let mut identities = TypeIdentities::default();
    assert_eq!(identities.observe(42, b"first").unwrap(), 42);
    assert!(
        identities
            .observe(42, b"second")
            .unwrap_err()
            .contains("collision")
    );
    assert_eq!(identities.observe(42, b"first").unwrap(), 42);
    assert_eq!(identities.observe(43, b"second").unwrap(), 43);
}

#[test]
fn well_known_generic_records_require_their_declaration_and_arguments() {
    use verum_vbc::{VbcModule, types::{TypeDescriptor, TypeId, TypeKind, TypeParamDescriptor, TypeRef}};
    let mut module = VbcModule::new("program".into());
    let applied = |element| TypeRef::Instantiated { base: TypeId::LIST, args: vec![TypeRef::Concrete(element)] };
    assert!(module.type_identity(&applied(TypeId::INT)).is_err());
    assert!(module.type_identity(&TypeRef::Concrete(TypeId::LIST)).is_err());
    module.types.push(TypeDescriptor { id: TypeId::LIST, kind: TypeKind::Record,
        type_params: smallvec::smallvec![TypeParamDescriptor::default()], ..Default::default() });
    assert_ne!(module.type_identity(&applied(TypeId::INT)).unwrap(), module.type_identity(&applied(TypeId::BOOL)).unwrap());
    assert!(module.type_identity(&TypeRef::Concrete(TypeId::LIST)).is_err());
}

#[test]
fn incomplete_and_excessively_nested_identity_never_becomes_a_number() {
    use verum_vbc::{
        VbcModule,
        types::{TypeId, TypeParamId, TypeRef},
    };
    let module = VbcModule::new("test".into());
    let mut identities = TypeIdentities::default();
    for ty in [
        TypeRef::Generic(TypeParamId(0)),
        TypeRef::Concrete(TypeId(99)),
        TypeRef::Concrete(TypeId::PTR),
        TypeRef::Concrete(TypeId(10000)),
        TypeRef::ConstValue(42),
    ] {
        assert!(identities.query(&module, &ty).is_err(), "{ty:?}");
    }
    // Ordinary worker stack, including the rejected input and its destruction.
    std::thread::spawn(move || {
        let mut ty = TypeRef::Concrete(TypeId::INT);
        for _ in 0..32 {
            ty = TypeRef::Slice(verum_common::Heap::new(ty));
        }
        assert!(identities.query(&module, &ty).is_ok());
        for _ in 0..96 {
            ty = TypeRef::Slice(verum_common::Heap::new(ty));
        }
        assert!(
            identities
                .query(&module, &ty)
                .unwrap_err()
                .contains("resource limit")
        );
    })
    .join()
    .unwrap();
}

#[test]
fn nominal_spelling_does_not_replace_identity_and_alias_cycles_refuse() {
    use verum_vbc::{
        VbcModule,
        types::{TypeDescriptor, TypeId, TypeKind, TypeRef},
    };
    let mut module = VbcModule::new("program".into());
    let name = module.strings.intern("Byte");
    for id in [TypeId(1000), TypeId(1001)] {
        module.types.push(TypeDescriptor {
            id,
            name,
            kind: TypeKind::Record,
            ..Default::default()
        });
    }
    let first = module
        .type_identity(&TypeRef::Concrete(TypeId(1000)))
        .unwrap();
    assert_ne!(
        first,
        module
            .type_identity(&TypeRef::Concrete(TypeId(1001)))
            .unwrap()
    );
    assert_ne!(
        first,
        module
            .type_identity(&TypeRef::Concrete(TypeId::U8))
            .unwrap()
    );
    for (id, target) in [(TypeId(1002), TypeId(1003)), (TypeId(1003), TypeId(1002))] {
        module.types.push(TypeDescriptor {
            id,
            name,
            kind: TypeKind::Alias,
            alias_target: Some(TypeRef::Concrete(target)),
            ..Default::default()
        });
    }
    assert!(
        module
            .type_identity(&TypeRef::Concrete(TypeId(1002)))
            .unwrap_err()
            .contains("resource limit")
    );
}
