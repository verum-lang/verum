//! The record telescope survives archives; a template is not an executable type.
use verum_fast_parser::Parser;
use verum_vbc::{
    Instruction, Reg, TypeRef, VbcModule,
    bytecode::{decode_instructions, encode_instruction},
    bytecode_remap::{IdRemap, rewrite_type_ref_ids},
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    module::{FunctionDescriptor, ParamDescriptor},
    serialize::serialize_module,
    types::{TypeId, TypeParamDescriptor, TypeParamId},
    validate::validate_module,
};

fn declaration() -> VbcModule {
    let ast =
        Parser::new("module telescope; public type Envelope is { carrier: Type, value: carrier };")
            .parse_module()
            .unwrap();
    VbcCodegen::with_config(CodegenConfig::new("telescope"))
        .collect_declaration_interface(&[&ast])
        .unwrap()
}

#[test]
fn source_record_fields_keep_their_owner_through_the_archive() {
    let original = declaration();
    let bytes = serialize_module(&original).unwrap();
    let loaded = deserialize_module(&bytes).unwrap();
    validate_module(&loaded).unwrap();
    let owner = loaded
        .types
        .iter()
        .find(|ty| {
            loaded
                .strings
                .get(ty.name)
                .is_some_and(|name| name.ends_with("Envelope"))
        })
        .unwrap();
    let expected = TypeRef::RecordFieldProjection {
        owner: owner.id,
        ordinal: 0,
    };
    assert_eq!(owner.fields[1].type_ref, expected);
    assert_eq!(owner.fields[1].declaration_type.as_ref(), Some(&expected));
    assert!(owner.type_params.is_empty());
    assert!(owner.source_declaration.is_some());
}

#[test]
fn instruction_codec_and_remapping_preserve_the_exact_slot() {
    struct Remap;
    impl IdRemap for Remap {
        fn map_type_id(&self, id: TypeId) -> TypeId {
            TypeId(id.0 + 11)
        }
    }
    let mut ty = TypeRef::Tuple(vec![TypeRef::RecordFieldProjection {
        owner: TypeId(1234),
        ordinal: u32::MAX,
    }]);
    let mut bytes = Vec::new();
    encode_instruction(
        &Instruction::LoadT {
            dst: Reg(0),
            type_ref: ty.clone(),
        },
        &mut bytes,
    );
    assert!(
        matches!(&decode_instructions(&bytes).unwrap()[0], Instruction::LoadT { type_ref, .. } if *type_ref == ty)
    );
    rewrite_type_ref_ids(&mut ty, &Remap);
    assert_eq!(
        ty,
        TypeRef::Tuple(vec![TypeRef::RecordFieldProjection {
            owner: TypeId(1245),
            ordinal: u32::MAX
        }])
    );
}

#[test]
fn a_free_projection_cannot_be_published_as_a_function_result() {
    let mut module = declaration();
    let owner = module
        .types
        .iter()
        .find(|ty| ty.fields.len() == 2)
        .unwrap()
        .id;
    let mut function = FunctionDescriptor::new(module.intern_string("leak"));
    function.return_type =
        TypeRef::Tuple(vec![TypeRef::RecordFieldProjection { owner, ordinal: 0 }]);
    module.add_function(function);
    let loaded = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    let error = validate_module(&loaded).unwrap_err().to_string();
    assert!(
        error.contains("record field projection requires"),
        "{error}"
    );
}

fn generic_function(module: &mut VbcModule) -> FunctionDescriptor {
    let mut function = FunctionDescriptor::new(module.intern_string("generic_identity"));
    function.register_count = 1;
    function.is_generic = true;
    let generic = TypeRef::Generic(TypeParamId(0));
    function.params.push(ParamDescriptor {
        name: module.intern_string("value"), type_ref: generic.clone(),
        is_mut: false, default: None, type_name: Default::default(),
    });
    function.return_type = generic.clone();
    function.semantic_params = Some(vec![Some(generic.clone())].into());
    function.semantic_return = Some(generic.clone());
    function.type_params.push(TypeParamDescriptor {
        id: TypeParamId(0), default: Some(TypeRef::Concrete(TypeId::INT)),
        ..Default::default()
    });
    let mut callable = TypeParamDescriptor { id: TypeParamId(1), ..Default::default() };
    callable.type_bounds.push(TypeRef::Function {
        params: vec![generic.clone()], return_type: Box::new(generic),
        contexts: Default::default(),
    });
    function.type_params.push(callable);
    function
}

#[test]
fn free_record_projections_cannot_escape_through_function_metadata() {
    for position in ["semantic_return", "semantic_parameter", "default", "bound", "yield"] {
        for nested in [false, true] {
            let mut module = declaration();
            let owner = module.types.iter().find(|ty| ty.fields.len() == 2).unwrap().id;
            let projection = TypeRef::RecordFieldProjection { owner, ordinal: 0 };
            let projection = if nested { TypeRef::Slice(Box::new(projection)) } else { projection };
            let mut function = generic_function(&mut module);
            match position {
                "semantic_return" => function.semantic_return = Some(projection),
                "semantic_parameter" => function.semantic_params = Some(vec![Some(projection)].into()),
                "default" => function.type_params[0].default = Some(projection),
                "bound" => function.type_params[1].type_bounds[0] = projection,
                "yield" => {
                    function.is_generator = true;
                    function.yield_type = Some(projection);
                }
                _ => unreachable!(),
            }
            module.add_function(function);
            module.header.function_table_count = module.functions.len() as u32;
            let raw_error = validate_module(&module).expect_err(&format!("raw {position}, nested={nested}: free projection accepted")).to_string();
            assert!(raw_error.contains("record field projection requires"), "raw {position}, nested={nested}: {raw_error}");
            let loaded = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
            let error = validate_module(&loaded).expect_err(&format!("{position}, nested={nested}: free projection accepted")).to_string();
            assert!(error.contains("record field projection requires"), "{position}, nested={nested}: {error}");
        }
    }
}

#[test]
fn ordinary_generic_defaults_and_callable_bounds_survive_validation() {
    let mut module = declaration();
    let mut function = generic_function(&mut module);
    function.is_generator = true;
    function.yield_type = Some(TypeRef::Generic(TypeParamId(0)));
    module.add_function(function);
    module.header.function_table_count = module.functions.len() as u32;
    validate_module(&module).unwrap();
    let loaded = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    validate_module(&loaded).unwrap();
    let function = loaded.functions.iter().find(|function| loaded.get_string(function.name) == Some("generic_identity")).unwrap();
    assert_eq!(function.type_params[0].default, Some(TypeRef::Concrete(TypeId::INT)));
    assert!(function.is_generator);
    assert_eq!(function.yield_type, Some(TypeRef::Generic(TypeParamId(0))));
    assert!(matches!(&function.type_params[1].type_bounds[0], TypeRef::Function { params, return_type, .. }
        if params == &[TypeRef::Generic(TypeParamId(0))] && **return_type == TypeRef::Generic(TypeParamId(0))));
}

#[test]
fn record_fields_do_not_escape_to_the_following_declaration() {
    let ast = Parser::new("module telescope; type Envelope is { carrier: Type, value: carrier }; type Leak is { value: carrier };")
        .parse_module().unwrap();
    let error = VbcCodegen::with_config(CodegenConfig::new("telescope"))
        .collect_declaration_interface(&[&ast])
        .unwrap_err()
        .to_string();
    assert!(error.contains("carrier"), "{error}");
}

#[test]
fn a_forward_field_is_not_a_binder() {
    let ast = Parser::new("type Envelope is { value: carrier, carrier: Type };")
        .parse_module()
        .unwrap();
    let error = VbcCodegen::new()
        .collect_declaration_interface(&[&ast])
        .unwrap_err()
        .to_string();
    assert!(error.contains("carrier"), "{error}");
}
