//! Typed nominal declarations survive independently of runtime TypeRef layout.
use verum_ast::{
    Ident, ItemKind, TypeDecl, TypeDeclBody,
    decl::{FunctionParamKind, ProtocolItemKind, VariantData},
    ty::{GenericParamKind, Type, TypeKind, UniverseLevelExpr},
};
use verum_common::{Heap, List};
use verum_fast_parser::Parser;
use verum_vbc::{
    deserialize::deserialize_module,
    module::VbcModule,
    serialize::serialize_module,
    types::{
        FieldDescriptor, TypeDescriptor, TypeId, TypeKind as RuntimeTypeKind, TypeParamDescriptor,
        TypeParamId, TypeRef, VariantDescriptor, VariantKind,
    },
};

fn declaration(source: &str) -> TypeDecl {
    let mut module = Parser::new(source)
        .parse_module()
        .expect("valid source grammar");
    let ItemKind::Type(declaration) = module.items.remove(0).kind else {
        panic!("type declaration")
    };
    declaration
}

fn module(declaration: TypeDecl) -> VbcModule {
    let mut module = VbcModule::new("nominal_fixture".into());
    let name = module.intern_string(declaration.name.name.as_str());
    let mut descriptor = TypeDescriptor {
        id: TypeId(1000),
        name,
        resource_discipline: declaration.resource_discipline(),
        ..Default::default()
    };
    match &declaration.body {
        TypeDeclBody::Record(fields) => {
            descriptor.kind = RuntimeTypeKind::Record;
            descriptor.fields = fields
                .iter()
                .map(|field| runtime_field(&mut module, field.name.name.as_str()))
                .collect();
        }
        TypeDeclBody::Variant(variants) => {
            descriptor.kind = RuntimeTypeKind::Sum;
            for (tag, variant) in variants.iter().enumerate() {
                let mut runtime = VariantDescriptor {
                    name: module.intern_string(variant.name.name.as_str()),
                    tag: tag.try_into().unwrap(),
                    // The current producer carries payload slots through fields.
                    payload: None,
                    ..Default::default()
                };
                match &variant.data {
                    Some(VariantData::Tuple(types)) if !types.is_empty() => {
                        runtime.kind = VariantKind::Tuple;
                        runtime.arity = types.len().try_into().unwrap();
                        runtime.fields = types
                            .iter()
                            .enumerate()
                            .map(|(index, _)| runtime_field(&mut module, &format!("_{index}")))
                            .collect();
                    }
                    Some(VariantData::Record(fields)) => {
                        runtime.kind = VariantKind::Record;
                        runtime.fields = fields
                            .iter()
                            .map(|field| runtime_field(&mut module, field.name.name.as_str()))
                            .collect();
                    }
                    _ => {}
                }
                descriptor.variants.push(runtime);
            }
        }
        TypeDeclBody::Protocol(body) => {
            descriptor.kind = RuntimeTypeKind::Protocol;
            for item in &body.items {
                if let ProtocolItemKind::Function { decl, .. } = &item.kind {
                    descriptor.variants.push(VariantDescriptor {
                        name: module.intern_string(decl.name.name.as_str()),
                        tag: descriptor.variants.len().try_into().unwrap(),
                        payload: Some(TypeRef::Function {
                            params: decl
                                .params
                                .iter()
                                .filter(|parameter| {
                                    matches!(parameter.kind, FunctionParamKind::Regular { .. })
                                })
                                .map(|_| TypeRef::Concrete(TypeId::PTR))
                                .collect(),
                            return_type: Heap::new(TypeRef::Concrete(TypeId::PTR)),
                            contexts: Default::default(),
                        }),
                        ..Default::default()
                    });
                }
            }
        }
        TypeDeclBody::Alias(_) => {
            descriptor.kind = RuntimeTypeKind::Alias;
            descriptor.alias_target = Some(TypeRef::Concrete(TypeId::PTR));
        }
        TypeDeclBody::Newtype(_) => {
            descriptor.kind = RuntimeTypeKind::Record;
            descriptor.is_transparent_wrapper = true;
            descriptor.fields.push(runtime_field(&mut module, "_0"));
        }
        TypeDeclBody::Tuple(types) => {
            descriptor.kind = RuntimeTypeKind::Record;
            descriptor.is_transparent_wrapper = types.len() == 1;
            descriptor.fields = types
                .iter()
                .enumerate()
                .map(|(index, _)| runtime_field(&mut module, &format!("_{index}")))
                .collect();
        }
        TypeDeclBody::Unit => descriptor.kind = RuntimeTypeKind::Unit,
        _ => panic!("fixture requires an AST shape with a supported runtime descriptor"),
    }
    if matches!(
        declaration.body,
        TypeDeclBody::Record(_)
            | TypeDeclBody::Protocol(_)
            | TypeDeclBody::Variant(_)
            | TypeDeclBody::Alias(_)
    ) {
        for parameter in &declaration.generics {
            if let GenericParamKind::Type { name, .. }
            | GenericParamKind::HigherKinded { name, .. }
            | GenericParamKind::Const { name, .. }
            | GenericParamKind::Meta { name, .. } = &parameter.kind {
                descriptor.type_params.push(TypeParamDescriptor {
                    name: module.intern_string(name.name.as_str()),
                    id: TypeParamId(descriptor.type_params.len().try_into().unwrap()),
                    ..Default::default()
                });
            }
        }
    }
    descriptor.source_declaration = Some(declaration);
    module.types.push(descriptor);
    module
}

fn runtime_field(module: &mut VbcModule, name: &str) -> FieldDescriptor {
    FieldDescriptor {
        name: module.intern_string(name),
        // A universe's semantic Type(n) remains in the AST; PTR is its
        // explicitly erased runtime representation in this fixture.
        type_ref: TypeRef::Concrete(TypeId::PTR),
        ..Default::default()
    }
}

#[test]
fn nominal_carrier_roundtrips_without_codegen() {
    for source in [
        "public type Carrier<A> is { value: Type(1), extra: A };",
        "public type Carrier<A> is fn(A) -> Type(2);",
        "public type Carrier is Left(Type(1)) | Right(Type(2));",
        "public type Carrier is protocol { fn value(&self) -> Type(1); };",
        "public type Carrier<N: meta Int, A, B> is { value: Type(1), left: A, right: B };",
        "public type Carrier<N: meta Int, A, B> is fn(A, B) -> Type(2);",
        "public type Carrier<N: meta Int, A, B> is Left(Type(1), A) | Right { value: Type(2), extra: B } | Empty;",
        "public type Carrier<N: meta Int, A, B> is protocol { fn value(&self) -> Type(1); };",
        "public type Carrier<A> is protocol { type Item; fn first(&self, input: A) -> Type(1); fn second() -> Type(2); };",
    ] {
        let carrier = verum_ast::source_signature::source_type_declaration(&declaration(source))
            .unwrap()
            .expect("uses universes");
        let loaded =
            deserialize_module(&serialize_module(&module(carrier.clone())).unwrap()).unwrap();
        assert_eq!(loaded.types[0].source_declaration, Some(carrier));
    }
}

#[test]
fn transparent_newtype_tuple_and_unit_shapes_roundtrip_without_codegen() {
    let original = declaration("public type Carrier is { value: Type(1) };");
    let TypeDeclBody::Record(fields) = &original.body else {
        panic!("record")
    };
    let universe = fields[0].ty.clone();
    for body in [
        TypeDeclBody::Newtype(universe.clone()),
        TypeDeclBody::Tuple(List::from_single(universe.clone())),
        TypeDeclBody::Tuple(List::from_iter([universe.clone(), universe])),
        TypeDeclBody::Unit,
    ] {
        let mut carrier = original.clone();
        carrier.body = body;
        let fixture = module(carrier.clone());
        let loaded = deserialize_module(&serialize_module(&fixture).unwrap()).unwrap();
        assert_eq!(loaded.types[0].source_declaration, Some(carrier));
    }
}

#[test]
fn writer_refuses_invalid_owner_levels_and_executable_method_bodies() {
    let valid = declaration("public type Carrier is { value: Type(1) };");
    let mut wrong_owner = module(valid);
    wrong_owner.types[0].name = wrong_owner.intern_string("Different");
    assert!(serialize_module(&wrong_owner).is_err());
    for source in [
        "public type Carrier<universe u> is { value: Type(u) };",
        "public type Carrier is { value: Type(missing) };",
        "public type Carrier is protocol { fn value(&self) -> Type(1) { 1 } };",
    ] {
        assert!(
            serialize_module(&module(declaration(source))).is_err(),
            "{source}"
        );
    }
}

#[test]
fn writer_refuses_runtime_kind_field_roster_resource_and_wrapper_mismatches() {
    let valid = module(declaration(
        "public type Carrier<A, B> is { value: Type(1), left: A, right: B };",
    ));
    let mut wrong_kind = valid.clone();
    wrong_kind.types[0].kind = RuntimeTypeKind::Protocol;
    let mut missing_field = valid.clone();
    missing_field.types[0].fields.pop();
    let mut reordered_fields = valid.clone();
    reordered_fields.types[0].fields.swap(0, 1);
    let mut renamed_field = valid.clone();
    renamed_field.types[0].fields[0].name = renamed_field.intern_string("invented");
    let mut wrong_resource = valid.clone();
    wrong_resource.types[0].resource_discipline = verum_common::ResourceDiscipline::Affine;
    let mut wrong_wrapper = valid.clone();
    wrong_wrapper.types[0].is_transparent_wrapper = true;
    for (reason, fixture) in [
        ("kind", wrong_kind),
        ("missing field", missing_field),
        ("field order", reordered_fields),
        ("field name", renamed_field),
        ("resource discipline", wrong_resource),
        ("transparent wrapper", wrong_wrapper),
    ] {
        let error = serialize_module(&fixture).unwrap_err().to_string();
        assert!(error.contains("semantic type Carrier"), "{reason}: {error}");
    }
    let mut alias = module(declaration("public type Carrier is fn() -> Type(1);"));
    assert_eq!(alias.types[0].kind, RuntimeTypeKind::Alias);
    alias.types[0].alias_target = None;
    assert!(serialize_module(&alias).is_err());
}

#[test]
fn writer_refuses_variant_shape_and_generic_slot_mismatches() {
    let valid = module(declaration(
        "public type Carrier<N: meta Int, A, B> is Left(Type(1), A) | Right { first: Type(2), second: B } | Empty;",
    ));
    let mut wrong_arity = valid.clone();
    // Keep the runtime tuple internally consistent; it still disagrees with AST.
    wrong_arity.types[0].variants[0].arity -= 1;
    wrong_arity.types[0].variants[0].fields.pop();
    let mut wrong_tag = valid.clone();
    wrong_tag.types[0].variants[0].tag = 7;
    let mut wrong_variant = valid.clone();
    wrong_variant.types[0].variants[0].name = wrong_variant.intern_string("Invented");
    let mut wrong_record_fields = valid.clone();
    wrong_record_fields.types[0].variants[1].fields.swap(0, 1);
    let mut wrong_order = valid.clone();
    // Preserve correct slot IDs while changing only declared binder order.
    let first_name = wrong_order.types[0].type_params[0].name;
    wrong_order.types[0].type_params[0].name = wrong_order.types[0].type_params[1].name;
    wrong_order.types[0].type_params[1].name = first_name;
    let mut wrong_sum_id = valid.clone();
    assert_eq!(wrong_sum_id.types[0].type_params[0].id, TypeParamId(0));
    wrong_sum_id.types[0].type_params[0].id = TypeParamId(1);
    let mut wrong_record_id = module(declaration(
        "public type Carrier<N: meta Int, A, B> is { value: Type(1), left: A, right: B };",
    ));
    assert_eq!(wrong_record_id.types[0].type_params[0].id, TypeParamId(0));
    wrong_record_id.types[0].type_params[0].id = TypeParamId(1);
    for (reason, fixture) in [
        ("tuple arity", wrong_arity),
        ("variant tag", wrong_tag),
        ("variant name", wrong_variant),
        ("variant field order", wrong_record_fields),
        ("generic name order", wrong_order),
        ("source-position generic ID", wrong_sum_id),
        ("dense generic ID", wrong_record_id),
    ] {
        let error = serialize_module(&fixture).unwrap_err().to_string();
        assert!(error.contains("semantic type Carrier"), "{reason}: {error}");
    }
}

#[test]
fn writer_refuses_protocol_method_roster_and_parameter_count_mismatches() {
    let valid = module(declaration(
        "public type Carrier<A> is protocol { type Item; fn first(&self, input: A) -> Type(1); fn second() -> Type(2); };",
    ));
    let mut missing = valid.clone();
    missing.types[0].variants.pop();
    let mut wrong_name = valid.clone();
    wrong_name.types[0].variants[0].name = wrong_name.intern_string("invented");
    let mut wrong_count = valid.clone();
    let Some(TypeRef::Function { params, .. }) = &mut wrong_count.types[0].variants[0].payload
    else {
        panic!("callable method")
    };
    params.clear();
    for (reason, fixture) in [
        ("method count", missing),
        ("method name", wrong_name),
        ("parameter count", wrong_count),
    ] {
        let error = serialize_module(&fixture).unwrap_err().to_string();
        assert!(error.contains("semantic type Carrier"), "{reason}: {error}");
    }
}

fn varint(mut value: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        bytes.push(byte | if value == 0 { 0 } else { 0x80 });
        if value == 0 {
            return bytes;
        }
    }
}

fn corrupt_carrier(original: &TypeDecl, replacement: &TypeDecl) -> Vec<u8> {
    let mut original = original.clone();
    let replacement = serde_json::to_vec(replacement).unwrap();
    let mut payload = serde_json::to_vec(&original).unwrap();
    if replacement.len() > payload.len() {
        // Reserve space through ordinary serialization, so the serializer owns
        // every section offset. This retained annotation changes no shape.
        original.attributes.push(verum_ast::attr::Attribute::new(
            "fixture_padding".into(),
            Some(List::from_single(verum_ast::Expr::literal(
                verum_ast::Literal::string(
                    "x".repeat(replacement.len() - payload.len()).into(),
                    original.span,
                ),
            ))),
            original.span,
        ));
        payload = serde_json::to_vec(&original).unwrap();
    }
    let mut bytes = serialize_module(&module(original)).unwrap();
    let start = bytes
        .windows(payload.len())
        .position(|part| part == payload)
        .unwrap();
    let old_length = varint(payload.len());
    assert_eq!(&bytes[start - old_length.len()..start], old_length);
    assert!(replacement.len() <= payload.len());
    // Trailing JSON whitespace keeps both the frame size and every downstream
    // VBC offset unchanged. Reader failures must therefore reach AST checking.
    let mut padded = replacement;
    padded.resize(payload.len(), b' ');
    bytes[start..start + payload.len()].copy_from_slice(&padded);
    bytes
}

#[test]
fn reader_refuses_unbound_and_excessively_nested_type_payloads() {
    let valid = declaration("public type Carrier is { value: Type(1) };");
    let mut bad = valid.clone();
    let TypeDeclBody::Record(fields) = &mut bad.body else {
        panic!("record")
    };
    let span = fields[0].ty.span;
    fields[0].ty = Type::new(
        TypeKind::Universe {
            level: Some(UniverseLevelExpr::Variable(Ident::new("missing", span))),
        },
        span,
    );
    assert!(deserialize_module(&corrupt_carrier(&valid, &bad)).is_err());
    let mut deep = valid.clone();
    let TypeDeclBody::Record(fields) = &mut deep.body else {
        panic!("record")
    };
    for _ in 0..180 {
        fields[0].ty = Type::new(
            TypeKind::Tuple(List::from_single(fields[0].ty.clone())),
            span,
        );
    }
    let error = deserialize_module(&corrupt_carrier(&valid, &deep)).unwrap_err();
    assert!(error.to_string().contains("recursion limit"), "{error}");
}

#[test]
fn reader_refuses_tampered_ast_kind_fields_and_generic_rosters() {
    let valid = declaration("public type Carrier<A, B> is { value: Type(1), left: A, right: B };");
    let mut wrong_kind = valid.clone();
    wrong_kind.body = TypeDeclBody::Alias(Type::new(
        TypeKind::Universe {
            level: Some(UniverseLevelExpr::Concrete(1)),
        },
        valid.span,
    ));
    let mut wrong_field = valid.clone();
    let TypeDeclBody::Record(fields) = &mut wrong_field.body else {
        panic!("record")
    };
    fields[0].name = Ident::new("invented", fields[0].name.span);
    let mut wrong_field_order = valid.clone();
    let TypeDeclBody::Record(fields) = &mut wrong_field_order.body else {
        panic!("record")
    };
    fields.swap(0, 1);
    let mut missing_field = valid.clone();
    let TypeDeclBody::Record(fields) = &mut missing_field.body else {
        panic!("record")
    };
    fields.pop();
    let mut wrong_generic_order = valid.clone();
    wrong_generic_order.generics.swap(0, 1);
    for (reason, replacement) in [
        ("kind", wrong_kind),
        ("field name", wrong_field),
        ("field order", wrong_field_order),
        ("missing field", missing_field),
        ("generic order", wrong_generic_order),
    ] {
        let error = deserialize_module(&corrupt_carrier(&valid, &replacement))
            .unwrap_err()
            .to_string();
        assert!(error.contains("semantic type Carrier"), "{reason}: {error}");
    }
}

#[test]
fn reader_refuses_tampered_ast_variant_rosters_and_payloads() {
    let valid = declaration(
        "public type Carrier is Left(Type(1), Type(2)) | Right { first: Type(1), second: Type(2) } | Empty;",
    );
    let mut wrong_order = valid.clone();
    let TypeDeclBody::Variant(variants) = &mut wrong_order.body else {
        panic!("variants")
    };
    variants.swap(0, 1);
    let mut wrong_arity = valid.clone();
    let TypeDeclBody::Variant(variants) = &mut wrong_arity.body else {
        panic!("variants")
    };
    let Some(VariantData::Tuple(types)) = &mut variants[0].data else {
        panic!("tuple")
    };
    types.pop();
    let mut wrong_fields = valid.clone();
    let TypeDeclBody::Variant(variants) = &mut wrong_fields.body else {
        panic!("variants")
    };
    let Some(VariantData::Record(fields)) = &mut variants[1].data else {
        panic!("record")
    };
    fields.swap(0, 1);
    for (reason, replacement) in [
        ("variant order", wrong_order),
        ("tuple arity", wrong_arity),
        ("record payload order", wrong_fields),
    ] {
        let error = deserialize_module(&corrupt_carrier(&valid, &replacement))
            .unwrap_err()
            .to_string();
        assert!(error.contains("semantic type Carrier"), "{reason}: {error}");
    }
}

#[test]
fn reader_refuses_tampered_ast_protocol_method_rosters() {
    let valid = declaration(
        "public type Carrier<A> is protocol { type Item; fn first(&self, input: A) -> Type(1); fn second() -> Type(2); };",
    );
    let mut missing = valid.clone();
    let TypeDeclBody::Protocol(body) = &mut missing.body else {
        panic!("protocol")
    };
    body.items.pop();
    let mut wrong_name = valid.clone();
    let TypeDeclBody::Protocol(body) = &mut wrong_name.body else {
        panic!("protocol")
    };
    let ProtocolItemKind::Function { decl, .. } = &mut body.items[1].kind else {
        panic!("method")
    };
    decl.name = Ident::new("invented", decl.name.span);
    let mut wrong_count = valid.clone();
    let TypeDeclBody::Protocol(body) = &mut wrong_count.body else {
        panic!("protocol")
    };
    let ProtocolItemKind::Function { decl, .. } = &mut body.items[1].kind else {
        panic!("method")
    };
    decl.params.pop();
    for (reason, replacement) in [
        ("method count", missing),
        ("method name", wrong_name),
        ("parameter count", wrong_count),
    ] {
        let error = deserialize_module(&corrupt_carrier(&valid, &replacement))
            .unwrap_err()
            .to_string();
        assert!(error.contains("semantic type Carrier"), "{reason}: {error}");
    }
}

#[test]
#[cfg(feature = "codegen")]
fn source_producer_attaches_declaring_type_and_keeps_ordinary_types_empty() {
    let source = "public type Carrier<A> is { value: Type(1), extra: A }; public type Ordinary is { value: Int };";
    let ast = Parser::new(source).parse_module().unwrap();
    let compiled = verum_vbc::codegen::VbcCodegen::new()
        .compile_module(&ast)
        .unwrap();
    let carrier = compiled
        .types
        .iter()
        .find(|ty| compiled.get_string(ty.name) == Some("Carrier"))
        .unwrap();
    assert_eq!(
        carrier
            .source_declaration
            .as_ref()
            .unwrap()
            .name
            .name
            .as_str(),
        "Carrier"
    );
    let ordinary = compiled
        .types
        .iter()
        .find(|ty| compiled.get_string(ty.name) == Some("Ordinary"))
        .unwrap();
    assert!(ordinary.source_declaration.is_none());
    let loaded = deserialize_module(&serialize_module(&compiled).unwrap()).unwrap();
    assert!(
        loaded
            .types
            .iter()
            .any(|ty| ty.source_declaration.is_some())
    );
}

#[test]
fn cubical_alias_preserves_the_kernel_marker_without_inventing_storage() {
    for marker in ["@builtin_glue", "@builtin_path", "@builtin_interval"] {
        let source = format!("public type Carrier<A>(a: A) is {marker};");
        let declaration = declaration(&source);
        let mut alias = module(declaration.clone());
        alias.types[0].alias_target = None;
        alias.types[0].alias_target_name = Some(alias.intern_string(marker));
        let loaded = deserialize_module(&serialize_module(&alias).unwrap()).unwrap();
        assert_eq!(
            loaded.types[0].source_declaration.as_ref(),
            Some(&declaration)
        );
        assert!(loaded.types[0].alias_target.is_none());

        let mut forged = alias.clone();
        forged.types[0].alias_target = Some(TypeRef::Concrete(TypeId::PTR));
        assert!(serialize_module(&forged).is_err());
        let mut mismatch = alias.clone();
        mismatch.types[0].alias_target_name = Some(mismatch.intern_string("@different"));
        assert!(serialize_module(&mismatch).is_err());
        alias.types[0].alias_target_name = None;
        assert!(serialize_module(&alias).is_err());
    }
    let mut unknown = module(declaration(
        "public type Carrier<A>(a: A) is @unregistered;",
    ));
    unknown.types[0].alias_target = None;
    unknown.types[0].alias_target_name = Some(unknown.intern_string("@unregistered"));
    assert!(serialize_module(&unknown).is_err());
}

#[cfg(feature = "codegen")]
#[test]
fn real_cubical_glue_interface_keeps_its_dependent_telescope_on_wire() {
    let source = include_str!("../../../core/math/cubical.vr");
    let mut ast = Parser::new(source).parse_module().unwrap();
    ast.items
        .retain(|item| matches!(&item.kind, ItemKind::Type(decl) if decl.name.name == "Glue"));
    assert_eq!(ast.items.len(), 1);
    let mut codegen = verum_vbc::codegen::VbcCodegen::new();
    let interface = codegen.collect_declaration_interface(&[&ast]).unwrap();
    let loaded = deserialize_module(&serialize_module(&interface).unwrap()).unwrap();
    let glue = loaded
        .types
        .iter()
        .find(|ty| loaded.get_string(ty.name) == Some("Glue"))
        .unwrap();
    assert!(glue.alias_target.is_none());
    assert_eq!(
        glue.alias_target_name.and_then(|id| loaded.get_string(id)),
        Some("@builtin_glue")
    );
    let instantiated = TypeRef::Instantiated {
        base: glue.id,
        args: vec![TypeRef::Concrete(TypeId::INT)],
    };
    let error = verum_vbc::drop_plan::build(&loaded, &instantiated).unwrap_err();
    assert!(error.contains("no resolved declaration target"), "{error}");
    let ItemKind::Type(source) = &ast.items[0].kind else {
        unreachable!()
    };
    assert_eq!(glue.source_declaration.as_ref(), Some(source));
}
