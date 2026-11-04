//! Yielding is part of callable identity; eager output is never a nominal Future.
use verum_vbc::{
    Instruction, Reg, TypeRef, VbcModule,
    archive::ArchiveBuilder,
    bytecode::{decode_instructions, encode_instruction},
    deserialize::deserialize_module,
    error::VbcError,
    module::FunctionDescriptor,
    mono::TypeSubstitution,
    serialize::serialize_module,
    types::{TypeId, TypeParamDescriptor, TypeParamId},
};

fn callable(is_async: bool, ranked: bool, output: TypeRef) -> TypeRef {
    if ranked {
        TypeRef::Rank2Function {
            is_async,
            type_param_count: 1,
            params: vec![],
            return_type: Box::new(output),
            contexts: Default::default(),
        }
    } else {
        TypeRef::Function {
            is_async,
            params: vec![],
            return_type: Box::new(output),
            contexts: Default::default(),
        }
    }
}

fn module(ty: TypeRef) -> VbcModule {
    let mut module = VbcModule::new("async-callable-wire".into());
    let mut descriptor = FunctionDescriptor::new(module.intern_string("callback"));
    descriptor.return_type = ty;
    module.add_function(descriptor);
    module
}

#[test]
fn yielding_identity_round_trips_through_instruction_module_and_archive() {
    for ranked in [false, true] {
        for is_async in [false, true] {
            let ty = callable(is_async, ranked, TypeRef::Concrete(TypeId::INT));
            let mut code = Vec::new();
            encode_instruction(&Instruction::LoadT { dst: Reg(0), type_ref: ty.clone() }, &mut code);
            assert_eq!(code[2], if ranked { 0x0C } else { 0x03 } | if is_async { 0x10 } else { 0 });
            assert!(matches!(&decode_instructions(&code).unwrap()[0],
                Instruction::LoadT { type_ref, .. } if *type_ref == ty));
            let source = module(ty.clone());
            let loaded = deserialize_module(&serialize_module(&source).unwrap()).unwrap();
            assert_eq!(loaded.functions[0].return_type, ty);
            assert_eq!(loaded.display_type_ref(&ty).starts_with("async "), is_async);
            let mut builder = ArchiveBuilder::new();
            builder.add_module("callbacks", &source, &[]).unwrap();
            assert_eq!(builder.finish().load_module("callbacks").unwrap().functions[0].return_type, ty);
        }
    }
}

#[test]
fn scoped_substitution_preserves_yielding_and_eager_output() {
    let substitution = TypeSubstitution::new(
        &[TypeParamDescriptor { id: TypeParamId(0), ..Default::default() }],
        &[TypeRef::Concrete(TypeId::INT)],
    );
    for ranked in [false, true] {
        let free_id = u16::from(ranked);
        let ty = callable(true, ranked, TypeRef::Generic(TypeParamId(free_id)));
        assert_eq!(substitution.apply(&ty).unwrap(), callable(true, ranked, TypeRef::Concrete(TypeId::INT)));
        assert_ne!(ty, callable(false, ranked, TypeRef::Generic(TypeParamId(free_id))));
        let json = serde_json::to_string(&ty).unwrap();
        assert_eq!(serde_json::from_str::<TypeRef>(&json).unwrap(), ty);
        let mut missing_property: serde_json::Value = serde_json::from_str(&json).unwrap();
        missing_property.as_object_mut().unwrap().values_mut().next().unwrap()
            .as_object_mut().unwrap().remove("is_async");
        assert!(serde_json::from_value::<TypeRef>(missing_property).is_err());
    }
}

#[test]
fn older_headers_cannot_reinterpret_yielding_as_sync_and_reserved_bits_refuse() {
    for ranked in [false, true] {
        let ty = callable(true, ranked, TypeRef::Concrete(TypeId::INT));
        let mut bytes = serialize_module(&module(ty.clone())).unwrap();
        bytes[6..8].copy_from_slice(&38u16.to_le_bytes());
        let expected = if ranked { 0x1C } else { 0x14 };
        assert!(matches!(deserialize_module(&bytes), Err(VbcError::InvalidTypeRefTag(tag)) if tag == expected));
        let mut code = Vec::new();
        encode_instruction(&Instruction::LoadT { dst: Reg(0), type_ref: ty }, &mut code);
        code[2] |= 0x20;
        assert!(decode_instructions(&code).is_err());
    }
}
