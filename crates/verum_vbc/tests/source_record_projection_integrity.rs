//! Archive-bound record templates retain their exact declaration-owned binder.
use verum_ast::{ItemKind, TypeDeclBody, ty::GenericParamKind};
use verum_common::Heap;
use verum_fast_parser::Parser;
use verum_vbc::{
    deserialize::deserialize_module,
    module::VbcModule,
    serialize::serialize_module,
    source_declaration_integrity::validate_type_descriptor,
    types::{
        CbgrTier, FieldDescriptor, Mutability, ProtocolId, ProtocolImpl, StringId, TypeDescriptor,
        TypeId, TypeKind, TypeParamDescriptor, TypeParamId, TypeRef, VariantDescriptor,
    },
};

const OWNER: TypeId = TypeId(1000);

fn projection() -> TypeRef {
    TypeRef::RecordFieldProjection {
        owner: OWNER,
        ordinal: 0,
    }
}

fn fixture(source: &str) -> VbcModule {
    let mut ast = Parser::new(source).parse_module().expect("valid source");
    let ItemKind::Type(declaration) = ast.items.remove(0).kind else {
        panic!("type")
    };
    let TypeDeclBody::Record(fields) = &declaration.body else {
        panic!("record")
    };
    let mut module = VbcModule::new("record_projection_fixture".into());
    let mut descriptor = TypeDescriptor {
        id: OWNER,
        name: module.intern_string(declaration.name.name.as_str()),
        kind: TypeKind::Record,
        resource_discipline: declaration.resource_discipline(),
        ..Default::default()
    };
    for field in fields {
        descriptor.fields.push(FieldDescriptor {
            name: module.intern_string(field.name.name.as_str()),
            type_ref: TypeRef::Concrete(TypeId::PTR),
            ..Default::default()
        });
    }
    for parameter in &declaration.generics {
        let name = match &parameter.kind {
            GenericParamKind::Type { name, .. }
            | GenericParamKind::HigherKinded { name, .. }
            | GenericParamKind::Const { name, .. }
            | GenericParamKind::Meta { name, .. } => name,
            GenericParamKind::Lifetime { .. } => continue,
            _ => panic!("supported fixture roster"),
        };
        descriptor.type_params.push(TypeParamDescriptor {
            name: module.intern_string(name.name.as_str()),
            id: TypeParamId(descriptor.type_params.len().try_into().unwrap()),
            ..Default::default()
        });
    }
    descriptor.source_declaration = Some(declaration);
    module.types.push(descriptor);
    module
}

fn record() -> VbcModule {
    fixture("type Carrier is { carrier: Type, value: carrier, later: Type(1) };")
}

fn check(module: &VbcModule) -> Result<(), verum_common::Text> {
    validate_type_descriptor(&module.types[0], &module.strings)
}

fn nested_types(inner: TypeRef) -> Vec<TypeRef> {
    vec![
        inner.clone(),
        TypeRef::Instantiated {
            base: TypeId(1001),
            args: vec![inner.clone()],
        },
        TypeRef::Tuple(vec![inner.clone(), inner.clone()]),
        TypeRef::Function {
            params: vec![inner.clone()],
            return_type: Heap::new(inner.clone()),
            contexts: Default::default(),
        },
        TypeRef::Rank2Function {
            type_param_count: 1,
            params: vec![inner.clone()],
            return_type: Heap::new(inner.clone()),
            contexts: Default::default(),
        },
        TypeRef::Reference {
            inner: Heap::new(inner.clone()),
            mutability: Mutability::Immutable,
            tier: CbgrTier::Tier0,
        },
        TypeRef::Array {
            element: Heap::new(inner.clone()),
            length: 3,
        },
        TypeRef::Slice(Heap::new(inner.clone())),
        TypeRef::AssociatedProjection {
            qualifier: None, args: Vec::new(),
            base: Heap::new(inner),
            assoc: "Item".into(),
        },
    ]
}

#[test]
fn record_projection_accepts_only_its_preceding_universe_binder_through_nested_types() {
    let source_types = [
        ("carrier", "carrier"),
        ("carrier", "Holder<carrier>"),
        ("carrier", "(carrier, carrier)"),
        ("carrier", "fn(carrier) -> carrier"),
        ("carrier", "fn<R>(carrier) -> carrier"),
        ("carrier", "&carrier"),
        ("carrier", "[carrier; 3]"),
        ("carrier", "[carrier]"),
        // Use a parsed associated projection, rather than a module-qualified
        // nominal path, for this structural carrier test.
        ("Element", "Element.Item"),
    ];
    for ((binder, source_type), ty) in source_types.into_iter().zip(nested_types(projection())) {
        let mut module = fixture(&format!(
            "type Carrier is {{ {binder}: Type, value: {source_type} }};"
        ));
        module.types[0].fields[1].type_ref = if matches!(ty, TypeRef::Reference { .. }) {
            projection()
        } else {
            ty.clone()
        };
        module.types[0].fields[1].declaration_type = Some(ty);
        assert!(
            check(&module).is_ok(),
            "source {source_type}: {:?}",
            check(&module)
        );
        let loaded = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
        assert_eq!(loaded.types[0].fields, module.types[0].fields);
    }
}

#[test]
fn a_nominal_path_cannot_be_replaced_by_a_record_projection() {
    let mut module = fixture("type Carrier is { carrier: Type, value: carrier.Item };");
    module.types[0].fields[1].declaration_type = Some(TypeRef::AssociatedProjection {
            qualifier: None, args: Vec::new(),
        base: Heap::new(projection()),
        assoc: "Item".into(),
    });
    assert!(
        check(&module)
            .unwrap_err()
            .contains("exact source binder occurrence")
    );
}

fn two_carriers(value_type: &str, value: TypeRef) -> VbcModule {
    let mut module = fixture(&format!(
        "type Carrier is {{ left: Type, right: Type, value: {value_type} }};"
    ));
    module.types[0].fields[2].type_ref = value.clone();
    module.types[0].fields[2].declaration_type = Some(value);
    module
}

fn right_projection() -> TypeRef {
    TypeRef::RecordFieldProjection {
        owner: OWNER,
        ordinal: 1,
    }
}

#[test]
fn valid_owner_and_prefix_cannot_replace_the_source_selected_carrier() {
    let mut module = two_carriers("left", projection());
    check(&module).unwrap();
    for replacement in [
        right_projection(),
        TypeRef::Generic(TypeParamId(0)),
        TypeRef::Concrete(TypeId::PTR),
    ] {
        module.types[0].fields[2].type_ref = replacement;
        assert!(
            check(&module)
                .unwrap_err()
                .contains("exact source binder occurrence")
        );
        assert!(serialize_module(&module).is_err());
    }
    module.types[0].fields[2].type_ref = projection();
    module.types[0].fields[2].declaration_type = Some(right_projection());
    assert!(
        check(&module)
            .unwrap_err()
            .contains("exact source binder occurrence")
    );
    module.types[0].fields[2].declaration_type = None;
    assert!(
        check(&module)
            .unwrap_err()
            .contains("no declaration type template")
    );
}

#[test]
fn carrier_occurrences_are_bound_by_structural_position_not_counts() {
    let value = TypeRef::Tuple(vec![projection(), right_projection()]);
    let mut module = two_carriers("(left, right)", value.clone());
    check(&module).unwrap();
    module.types[0].fields[2].type_ref = TypeRef::Tuple(vec![right_projection(), projection()]);
    assert!(
        check(&module)
            .unwrap_err()
            .contains("exact source binder occurrence")
    );
    let mut module = two_carriers(
        "Pair<left, right>",
        TypeRef::Instantiated {
            base: TypeId(1001),
            args: vec![projection(), right_projection()],
        },
    );
    check(&module).unwrap();
    module.types[0].fields[2].declaration_type = Some(TypeRef::Instantiated {
        base: TypeId(1001),
        args: vec![right_projection(), projection()],
    });
    assert!(
        check(&module)
            .unwrap_err()
            .contains("exact source binder occurrence")
    );
    let mut module = two_carriers(
        "(left, Int)",
        TypeRef::Tuple(vec![projection(), TypeRef::Concrete(TypeId::INT)]),
    );
    check(&module).unwrap();
    module.types[0].fields[2].type_ref = value;
    assert!(
        check(&module)
            .unwrap_err()
            .contains("exact source binder occurrence")
    );
}

#[test]
fn locally_quantified_binders_shadow_only_their_own_source_names() {
    let mut value = TypeRef::Rank2Function {
        type_param_count: 1,
        params: vec![TypeRef::Generic(TypeParamId(0)), right_projection()],
        return_type: Heap::new(TypeRef::Generic(TypeParamId(0))),
        contexts: Default::default(),
    };
    let mut module = two_carriers("fn<left>(left, right) -> left", value.clone());
    check(&module).unwrap();
    let TypeRef::Rank2Function { params, .. } = &mut value else {
        unreachable!()
    };
    params[0] = projection();
    module.types[0].fields[2].declaration_type = Some(value);
    assert!(
        check(&module)
            .unwrap_err()
            .contains("exact source binder occurrence")
    );
    let inner = TypeRef::Rank2Function {
        type_param_count: 1,
        params: vec![TypeRef::Generic(TypeParamId(0)), right_projection()],
        return_type: Heap::new(TypeRef::Generic(TypeParamId(0))),
        contexts: Default::default(),
    };
    let mut module = two_carriers(
        "fn<R>(fn<left>(left, right) -> left, left) -> right",
        TypeRef::Rank2Function {
            type_param_count: 1,
            params: vec![inner, projection()],
            return_type: Heap::new(right_projection()),
            contexts: Default::default(),
        },
    );
    check(&module).unwrap();
    let Some(TypeRef::Rank2Function { return_type, .. }) =
        &mut module.types[0].fields[2].declaration_type
    else {
        unreachable!()
    };
    *return_type = Heap::new(projection());
    assert!(
        check(&module)
            .unwrap_err()
            .contains("exact source binder occurrence")
    );
}

#[test]
fn reference_layout_erasure_does_not_erase_the_declaration_binder() {
    for (source, tier) in [
        ("&left", CbgrTier::Tier0),
        ("&checked left", CbgrTier::Tier1),
        ("&unsafe left", CbgrTier::Tier2),
    ] {
        let mut module = two_carriers(
            source,
            if tier == CbgrTier::Tier2 {
                TypeRef::Concrete(TypeId::PTR)
            } else {
                projection()
            },
        );
        module.types[0].fields[2].declaration_type = Some(TypeRef::Reference {
            inner: Heap::new(projection()),
            mutability: Mutability::Immutable,
            tier,
        });
        check(&module).unwrap();
        module.types[0].fields[2].declaration_type = Some(TypeRef::Reference {
            inner: Heap::new(right_projection()),
            mutability: Mutability::Immutable,
            tier,
        });
        assert!(
            check(&module)
                .unwrap_err()
                .contains("exact source binder occurrence")
        );
    }
}

#[test]
fn unsupported_dependent_source_shape_cannot_be_silently_erased() {
    let module = two_carriers("*const left", TypeRef::Concrete(TypeId::PTR));
    assert!(
        check(&module)
            .unwrap_err()
            .contains("no runtime template representation")
    );
}

#[test]
fn record_projection_rejects_foreign_forward_self_and_out_of_range_binders() {
    for (owner, ordinal, expected) in [
        (TypeId(1001), 0, "different owner"),
        (OWNER, 1, "preceding source field"),
        (OWNER, 2, "preceding source field"),
        (OWNER, u32::MAX, "preceding source field"),
    ] {
        for ty in nested_types(TypeRef::RecordFieldProjection { owner, ordinal }) {
            let mut module = record();
            module.types[0].fields[1].declaration_type = Some(ty.clone());
            assert!(check(&module).unwrap_err().contains(expected), "{ty:?}");
            module.types[0].fields[1].declaration_type = None;
            module.types[0].fields[1].type_ref = ty;
            assert!(check(&module).unwrap_err().contains(expected));
            assert!(serialize_module(&module).is_err());
        }
    }
}

#[test]
fn record_projection_requires_an_actual_source_universe_field() {
    let mut module = fixture("type Carrier is { carrier: Int, value: Int, later: Type };");
    module.types[0].fields[1].type_ref = projection();
    assert!(
        check(&module)
            .unwrap_err()
            .contains("not a source universe field")
    );
    let mut module = record();
    module.types[0].fields[1].type_ref = projection();
    module.types[0].source_declaration = None;
    assert!(
        check(&module)
            .unwrap_err()
            .contains("no source record field binder")
    );
}

#[test]
fn record_projection_cannot_escape_into_descriptor_headers_or_impls() {
    for placement in 0..7 {
        let mut module = fixture("type Carrier<A> is { carrier: Type, value: carrier };");
        let descriptor = &mut module.types[0];
        let ty = TypeRef::Tuple(vec![projection()]);
        match placement {
            0 => descriptor.type_params[0].default = Some(ty),
            1 => descriptor.type_params[0].type_bounds.push(ty),
            2 => descriptor.alias_target = Some(ty),
            3 => descriptor.protocols.push(ProtocolImpl {
                protocol: ProtocolId(1002),
                methods: vec![],
                associated_types: vec![(StringId::EMPTY, ty)],
                protocol_args_text: vec![],
                type_param_fn_bounds: vec![],
            }),
            4 => descriptor.variants.push(VariantDescriptor {
                payload: Some(ty),
                ..Default::default()
            }),
            5 | 6 => descriptor.variants.push(VariantDescriptor {
                fields: [FieldDescriptor {
                    type_ref: if placement == 5 {
                        ty.clone()
                    } else {
                        TypeRef::Concrete(TypeId::PTR)
                    },
                    declaration_type: (placement == 6).then_some(ty),
                    ..Default::default()
                }]
                .into_iter()
                .collect(),
                ..Default::default()
            }),
            _ => unreachable!(),
        }
        assert!(
            check(&module)
                .unwrap_err()
                .contains("no source record field binder"),
            "placement {placement}"
        );
    }
}

#[test]
fn mixed_runtime_generic_roster_includes_value_binders_in_dense_source_order() {
    let module = fixture("type Carrier<A, N: meta Int, B> is { carrier: Type, value: A };");
    check(&module).expect("value witness occupies slot 1");
    let loaded = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    assert_eq!(loaded.types[0].type_params, module.types[0].type_params);
    for mutation in 0..4 {
        let mut changed = module.clone();
        let parameters = &mut changed.types[0].type_params;
        match mutation {
            0 => {
                parameters.remove(1);
            }
            1 => parameters.swap(0, 1),
            2 => parameters[2].id = TypeParamId(1),
            3 => parameters.push(TypeParamDescriptor {
                name: parameters[0].name,
                id: TypeParamId(3),
                ..Default::default()
            }),
            _ => unreachable!(),
        }
        assert!(check(&changed).is_err(), "mutation {mutation}");
    }
}

#[test]
fn value_generic_slots_cannot_claim_type_only_constraints() {
    for mutation in 0..3 {
        let mut module = fixture("type Carrier<N: meta Int> is { carrier: Type };");
        let parameter = &mut module.types[0].type_params[0];
        match mutation {
            0 => parameter.default = Some(TypeRef::Concrete(TypeId::INT)),
            1 => parameter.bounds.push(ProtocolId(1002)),
            2 => parameter.type_bounds.push(TypeRef::Concrete(TypeId::INT)),
            _ => unreachable!(),
        }
        assert!(
            check(&module)
                .unwrap_err()
                .contains("value parameter contains type-only constraints")
        );
    }
}

#[test]
fn generic_kinds_come_from_the_source_header_without_erasing_runtime_binders() {
    let mut module = fixture("type Carrier<N: meta Int, F<_>, A> is { carrier: Type };");
    check(&module).expect("higher-kinded and value slots retain their positions");
    let declaration = module.types[0].source_declaration.as_mut().unwrap();
    let GenericParamKind::Meta { name, ty, .. } = declaration.generics[0].kind.clone() else {
        panic!("meta parameter")
    };
    declaration.generics[0].kind = GenericParamKind::Const {
        name: name.clone(),
        ty,
    };
    check(&module).expect("legacy const remains a value witness slot");
    module.types[0]
        .source_declaration
        .as_mut()
        .unwrap()
        .generics[0]
        .kind = GenericParamKind::Context { name };
    assert!(
        check(&module)
            .unwrap_err()
            .contains("unsupported runtime generic parameter kind")
    );
}

#[test]
#[cfg(feature = "codegen")]
fn source_codegen_cannot_publish_an_ambiguous_record_binder_as_an_unrelated_generic() {
    use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
    for argument in ["[left]", "[left; 3]", "[&left]"] {
        let source = format!(
            "module telescope; type Wrap<A> is {{ value: A }}; public type Envelope<T> is {{ left: Type, value: Wrap<{argument}> }};"
        );
        let ast = Parser::new(&source).parse_module().unwrap();
        let result = VbcCodegen::with_config(CodegenConfig::new("telescope"))
            .collect_declaration_interface(&[&ast]);
        match result {
            Err(error) => {
                assert!(
                    matches!(
                        error.kind,
                        verum_vbc::codegen::CodegenErrorKind::InvalidSourceSignature(_)
                    ),
                    "{argument}: {error}"
                );
                let span = error.span.expect("signature refusal identifies its source");
                let start = source.rfind(argument).unwrap() as u32;
                assert!(
                    span.start >= start && span.end <= start + argument.len() as u32,
                    "{argument}: {error}"
                );
            }
            Ok(module) => {
                let error = serialize_module(&module)
                    .expect_err("dependent binder cannot become T on wire")
                    .to_string();
                assert!(
                    error.contains("ambiguous generic argument containing a record field"),
                    "{argument}: {error}"
                );
            }
        }
    }
}

#[test]
#[cfg(feature = "codegen")]
fn archive_import_refusal_preserves_the_exact_dependent_argument_span() {
    use verum_vbc::codegen::{CodegenConfig, CodegenErrorKind, VbcCodegen};
    let source = "type Carrier<T> is { left: Type, value: Counted<[left]> };";
    let mut module = fixture(source);
    let ty = TypeRef::Instantiated {
        base: TypeId(1001),
        args: vec![TypeRef::Slice(Box::new(projection()))],
    };
    module.types[0].fields[1].type_ref = ty.clone();
    module.types[0].fields[1].declaration_type = Some(ty);
    for whole_module in [false, true] {
        let mut consumer = VbcCodegen::with_config(CodegenConfig::new("consumer"));
        if whole_module {
            consumer.import_archive_module_types(&module);
        } else {
            consumer.import_archive_type_with_protocol_remap_qualified(
                &module.types[0],
                &module.strings,
                &Default::default(),
                Some(&module.name),
            );
        }
        let error = consumer
            .finalize_module_from_state()
            .expect_err("ambiguous dependent source must be refused");
        assert!(
            matches!(error.kind, CodegenErrorKind::InvalidSourceSignature(_)),
            "{error}"
        );
        assert!(
            error
                .to_string()
                .contains("ambiguous generic argument containing a record field"),
            "{error}"
        );
        let span = error
            .span
            .expect("source argument location survives import refusal");
        let start = source.find("[left]").unwrap() as u32;
        assert_eq!((span.start, span.end), (start, start + 6));
    }
}

#[test]
fn ambiguous_argument_gate_preserves_closed_constants_and_locally_shadowed_types() {
    let mut module = fixture("type Carrier<T> is { left: Type, value: Counted<3> };");
    let ty = TypeRef::Instantiated {
        base: TypeId(1001),
        args: vec![TypeRef::ConstValue(3)],
    };
    module.types[0].fields[1].type_ref = ty.clone();
    module.types[0].fields[1].declaration_type = Some(ty);
    check(&module).unwrap();
    deserialize_module(&serialize_module(&module).unwrap()).unwrap();

    let mut module =
        fixture("type Carrier<T> is { left: Type, value: fn<left>(Wrap<[left]>) -> left };");
    let local = TypeRef::Generic(TypeParamId(0));
    let ty = TypeRef::Rank2Function {
        type_param_count: 1,
        params: vec![TypeRef::Instantiated {
            base: TypeId(1001),
            args: vec![TypeRef::Slice(Heap::new(local.clone()))],
        }],
        return_type: Heap::new(local),
        contexts: Default::default(),
    };
    module.types[0].fields[1].type_ref = ty.clone();
    module.types[0].fields[1].declaration_type = Some(ty);
    check(&module).unwrap();
    deserialize_module(&serialize_module(&module).unwrap()).unwrap();
}
