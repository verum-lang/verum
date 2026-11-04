use verum_vbc::{
    bytecode_remap::{IdRemap, rewrite_type_ref_ids},
    module::FunctionId,
    mono::{InstantiationRequest, TypeSubstitution},
    types::{ProtocolApplication, ProtocolId, TypeId, TypeParamId, TypeRef},
};

fn parameter(id: u16) -> TypeRef { TypeRef::Generic(TypeParamId(id)) }
fn projection(base: TypeRef, qualifier: TypeRef, argument: TypeRef) -> TypeRef {
    TypeRef::AssociatedProjection {
        base: Box::new(base),
        qualifier: Some(ProtocolApplication { protocol: ProtocolId(71), args: vec![qualifier] }),
        assoc: "Rows".into(), args: vec![argument],
    }
}

#[test]
fn nested_projection_substitution_preserves_local_and_outer_binders() {
    let input = TypeRef::Rank2Function {
        is_async: false,
        type_param_count: 1, params: vec![parameter(0)],
        return_type: Box::new(projection(parameter(1), parameter(2), parameter(0))),
        contexts: Default::default(),
    };
    let mut substitution = TypeSubstitution::empty();
    substitution.bind(TypeParamId(0), parameter(7));
    substitution.bind(TypeParamId(1), TypeRef::Concrete(TypeId::TEXT));
    let expected = TypeRef::Rank2Function {
        is_async: false,
        type_param_count: 1, params: vec![parameter(0)],
        return_type: Box::new(projection(parameter(8), TypeRef::Concrete(TypeId::TEXT), parameter(0))),
        contexts: Default::default(),
    };
    assert_eq!(substitution.apply(&input).unwrap(), expected);
}

#[test]
fn projection_cache_identity_includes_family_and_protocol_arguments() {
    let base = TypeRef::Concrete(TypeId(3100));
    let hash = |ty| InstantiationRequest::compute_hash(FunctionId(3), &[ty]);
    let initial = hash(projection(base.clone(), TypeRef::ConstValue(1), TypeRef::Concrete(TypeId::INT)));
    assert_ne!(initial, hash(projection(base.clone(), TypeRef::ConstValue(2), TypeRef::Concrete(TypeId::INT))));
    assert_ne!(initial, hash(projection(base, TypeRef::ConstValue(1), TypeRef::Concrete(TypeId::TEXT))));
}

#[test]
fn remapping_visits_all_projection_owners_without_rebinding_generics() {
    struct Relocate;
    impl IdRemap for Relocate {
        fn map_type_id(&self, id: TypeId) -> TypeId { TypeId(id.0 + 100) }
        fn map_protocol(&self, id: ProtocolId) -> ProtocolId { ProtocolId(id.0 + 200) }
    }
    let mut ty = projection(TypeRef::Concrete(TypeId(3000)),
        TypeRef::Instantiated { base: TypeId(3001), args: vec![parameter(5)] },
        TypeRef::Concrete(TypeId(3002)));
    rewrite_type_ref_ids(&mut ty, &Relocate);
    assert_eq!(ty, TypeRef::AssociatedProjection {
        base: Box::new(TypeRef::Concrete(TypeId(3100))),
        qualifier: Some(ProtocolApplication { protocol: ProtocolId(271), args: vec![
            TypeRef::Instantiated { base: TypeId(3101), args: vec![parameter(5)] },
        ] }),
        assoc: "Rows".into(), args: vec![TypeRef::Concrete(TypeId(3102))],
    });
}


#[test]
fn applied_projection_roundtrips_in_instruction_and_archive() {
    use verum_vbc::{Instruction, bytecode::{decode_instruction, encode_instruction},
        instruction::Reg, module::{FunctionDescriptor, VbcModule},
        serialize::serialize_module, deserialize::deserialize_module};
    let ty = projection(parameter(0), TypeRef::ConstValue(23),
        TypeRef::Tuple(vec![parameter(1), TypeRef::ConstValue(7)]));
    let instruction = Instruction::LoadT { dst: Reg(0), type_ref: ty.clone() };
    let mut bytes = Vec::new();
    encode_instruction(&instruction, &mut bytes);
    let mut offset = 0;
    assert_eq!(decode_instruction(&bytes, &mut offset).unwrap(), instruction);
    assert_eq!(offset, bytes.len());
    let mut module = VbcModule::new("transport".into());
    let name = module.intern_string("signature");
    let mut function = FunctionDescriptor::new(name);
    function.return_type = ty.clone();
    module.functions.push(function);
    let restored = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    assert_eq!(restored.functions[0].return_type, ty);
}

#[cfg(feature = "codegen")]
fn load_projection(binding: Option<TypeRef>, args: Vec<TypeRef>) -> verum_vbc::module::VbcModule {
    use verum_vbc::{Instruction, bytecode::encode_instruction, instruction::Reg,
        module::{FunctionDescriptor, VbcModule}, types::{ProtocolImpl, TypeDescriptor, TypeKind}};
    let mut module = VbcModule::new("projection_runtime".into());
    let name = module.intern_string("Carrier");
    let mut carrier = TypeDescriptor { id: TypeId(3000), name, kind: TypeKind::Record, ..Default::default() };
    if let Some(body) = binding {
        let assoc = module.intern_string("Item");
        carrier.protocols.push(ProtocolImpl { protocol: ProtocolId(3100), methods: vec![],
            associated_types: vec![(assoc, body)], protocol_args_text: vec![], type_param_fn_bounds: vec![] });
    }
    module.types.push(carrier);
    let name = module.intern_string("Source");
    let parsed = verum_fast_parser::Parser::new("type Source is protocol { type Item; };").parse_module().unwrap();
    let verum_ast::ItemKind::Type(source) = &parsed.items[0].kind else { panic!("protocol declaration") };
    module.types.push(TypeDescriptor { id: TypeId(3100), name, kind: TypeKind::Protocol,
        resource_discipline: source.resource_discipline(),
        source_declaration: Some(source.clone()), ..Default::default() });
    let name = module.intern_string("probe");
    let mut function = FunctionDescriptor::new(name);
    function.register_count = 1;
    for instruction in [Instruction::LoadT { dst: Reg(0), type_ref: TypeRef::AssociatedProjection {
        base: Box::new(TypeRef::Concrete(TypeId(3000))), qualifier: None, assoc: "Item".into(), args,
    } }, Instruction::Ret { value: Reg(0) }] { encode_instruction(&instruction, &mut module.bytecode); }
    function.bytecode_length = module.bytecode.len() as u32;
    module.functions.push(function);
    module
}

#[test]
#[cfg(feature = "codegen")]
fn load_type_uses_the_associated_assignment_instead_of_its_receiver() {
    use verum_vbc::{interpreter::Interpreter, serialize::serialize_module, deserialize::deserialize_module};
    let module = load_projection(Some(TypeRef::Concrete(TypeId::TEXT)), vec![]);
    let restored = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for module in [module, restored] {
        let result = Interpreter::new(std::sync::Arc::new(module)).execute_function(FunctionId(0)).unwrap();
        assert_eq!(result.as_type_id(), TypeId::TEXT);
        assert_ne!(result.as_type_id(), TypeId(3000));
    }
}

#[test]
#[cfg(feature = "codegen")]
fn missing_or_unrepresented_family_never_loads_the_receiver_as_its_type() {
    use verum_vbc::interpreter::{Interpreter, InterpreterError};
    let mut erased = load_projection(Some(TypeRef::Concrete(TypeId::TEXT)), vec![]);
    erased.types.iter_mut().find(|ty| ty.id == TypeId(3100)).unwrap().source_declaration = None;
    let mut defaulted = load_projection(Some(TypeRef::Concrete(TypeId::TEXT)), vec![]);
    let parsed = verum_fast_parser::Parser::new("type Source is protocol { type Item<T = Int>; };").parse_module().unwrap();
    let verum_ast::ItemKind::Type(source) = &parsed.items[0].kind else { panic!("family declaration") };
    defaulted.types.iter_mut().find(|ty| ty.id == TypeId(3100)).unwrap().source_declaration = Some(source.clone());
    for module in [load_projection(None, vec![]), erased, defaulted,
        load_projection(Some(TypeRef::Concrete(TypeId::TEXT)), vec![TypeRef::Concrete(TypeId::INT)])] {
        let error = Interpreter::new(std::sync::Arc::new(module)).execute_function(FunctionId(0)).unwrap_err();
        assert!(matches!(error, InterpreterError::InvalidBytecode { message, .. }
            if message.contains("declaration-owned binding")), "must explicitly refuse incomplete evidence");
    }
}

#[test]
fn truncated_applied_projection_bytecode_is_rejected_at_every_boundary() {
    use verum_vbc::{Instruction, bytecode::{decode_instruction, encode_instruction}, instruction::Reg};
    let instruction = Instruction::LoadT { dst: Reg(0), type_ref:
        projection(TypeRef::Concrete(TypeId(3000)), TypeRef::ConstValue(23), parameter(7)) };
    let mut bytes = Vec::new();
    encode_instruction(&instruction, &mut bytes);
    for end in 0..bytes.len() {
        assert!(decode_instruction(&bytes[..end], &mut 0).is_err(), "accepted truncation {end}");
    }
}
