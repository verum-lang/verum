//! Structural method catalogues do not replace the retained implementation's binders.
#![cfg(feature = "codegen")]

use verum_common::{List, Map};
use verum_fast_parser::Parser;
use verum_vbc::{
    FunctionDescriptor, FunctionId, TypeRef, VbcModule,
    codegen::{CodegenConfig, CodegenError, CodegenErrorKind, VbcCodegen},
    types::{CbgrTier, Mutability, TypeId, TypeKind, TypeParamId},
};

const SLICE: &str = r#"
module api;
implement<Element> [Element] {
    public fn marker(&self) -> Int { Element.size + 65 }
}
fn main() -> Int {
    let values: [Int; 2] = [11, 42];
    let part: &[Int] = &values[..];
    part.marker()
}
"#;

fn compile(source: &str) -> VbcModule {
    VbcCodegen::with_config(CodegenConfig::new("api"))
        .compile_module(&Parser::new(source).parse_module().expect("source grammar"))
        .expect("source VBC")
}

fn wire(module: &VbcModule) -> VbcModule {
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(module).expect("source wire"),
    )
    .expect("source roundtrip")
}

fn method(module: &VbcModule) -> &FunctionDescriptor {
    let matches: List<_> = module
        .functions
        .iter()
        .filter(|function| {
            module
                .get_string(function.name)
                .is_some_and(|name| name.ends_with(".marker"))
        })
        .collect();
    assert_eq!(matches.len(), 1, "one selected source method");
    matches[0]
}

fn method_mut(module: &mut VbcModule) -> &mut FunctionDescriptor {
    let id = method(module).id;
    module
        .functions
        .iter_mut()
        .find(|function| function.id == id)
        .unwrap()
}

fn import(module: &VbcModule) -> Result<VbcModule, CodegenError> {
    import_into(
        module,
        VbcCodegen::with_config(CodegenConfig::new("consumer")),
    )
}

fn import_into(module: &VbcModule, mut consumer: VbcCodegen) -> Result<VbcModule, CodegenError> {
    for index in 0..20 {
        consumer
            .ctx_mut()
            .intern_string_raw(&format!("unrelated_{index}"));
    }
    consumer.import_archive_module_types(module);
    let remap: Map<_, _> = module
        .functions
        .iter()
        .enumerate()
        .map(|(index, function)| (function.id.0, FunctionId(7000 + index as u32)))
        .collect();
    consumer.merge_archive_function_bodies(module, &remap.into());
    consumer.finalize_module_from_state()
}

fn execute(module: VbcModule) {
    let entry = module
        .find_function_by_name("api.main")
        .expect("declared entry");
    let value = verum_vbc::interpreter::Interpreter::try_new_validated(module.into())
        .expect("validated module")
        .execute_function(entry)
        .expect("selected source body");
    assert_eq!(value.try_as_i64(), Some(73));
}

fn assert_import_refuses(module: &VbcModule, reason: &str) {
    let error = import(module).expect_err("forged declaration must be refused");
    assert!(
        matches!(error.kind, CodegenErrorKind::InvalidSourceSignature(_)),
        "{error}"
    );
    assert!(
        error.to_string().contains(reason),
        "expected {reason}: {error}"
    );
}

fn verify_roundtrips(source: &str, expected_receiver: TypeRef, expected_kind: TypeKind) {
    let source = compile(source);
    for source in [source.clone(), wire(&source)] {
        let imported = import(&source).expect("declaration-owned archive import");
        for module in [source, imported.clone(), wire(&imported)] {
            let selected = method(&module);
            assert_eq!(selected.source_receiver.as_ref(), Some(&expected_receiver));
            let parent = module
                .types
                .iter()
                .find(|ty| Some(ty.id) == selected.parent_type)
                .expect("method catalogue");
            assert_eq!(parent.kind, expected_kind);
            assert!(!parent.source_inherent_implementations.is_empty());
            assert_eq!(
                selected.semantic_params.as_ref().unwrap()[0],
                Some(TypeRef::Reference {
                    inner: Box::new(expected_receiver.clone()),
                    tier: CbgrTier::Tier0,
                    mutability: Mutability::Immutable,
                })
            );
            execute(module);
        }
    }
}

#[test]
fn slice_receiver_binders_survive_source_wire_and_archive_body_merge() {
    verify_roundtrips(
        SLICE,
        TypeRef::Slice(Box::new(TypeRef::Generic(TypeParamId(0)))),
        TypeKind::Slice,
    );
}

#[test]
fn fixed_array_receiver_retains_its_exact_element_and_extent() {
    verify_roundtrips(
        r#"
        module api;
        implement<Element> [Element; 2] {
            public fn marker(&self) -> Int { Element.size + 65 }
        }
        fn main() -> Int { let values: [Int; 2] = [11, 42]; values.marker() }
    "#,
        TypeRef::Array {
            element: Box::new(TypeRef::Generic(TypeParamId(0))),
            length: 2,
        },
        TypeKind::Array,
    );
}

#[test]
fn structural_receiver_binder_is_not_replaced_by_a_shadowing_method_binder() {
    let source = compile(
        r#"
        module api;
        implement<Element> [Element] {
            public fn marker<Element>(&self, value: Element) -> Int { Element.size + 72 }
        }
        fn main() -> Int {
            let values: [Int; 2] = [11, 42];
            let part: &[Int] = &values[..];
            part.marker(true)
        }
    "#,
    );
    for source in [source.clone(), wire(&source)] {
        let imported = import(&source).expect("separate source binder roles");
        for module in [source, imported.clone(), wire(&imported)] {
            let selected = method(&module);
            assert_eq!(
                selected.source_receiver,
                Some(TypeRef::Slice(Box::new(TypeRef::Generic(TypeParamId(0)))))
            );
            assert_eq!(
                selected.explicit_type_param_ids.as_slice(),
                &[Some(TypeParamId(0x8000))]
            );
            assert_eq!(
                selected
                    .type_params
                    .iter()
                    .map(|parameter| parameter.id)
                    .collect::<List<_>>()
                    .as_slice(),
                &[TypeParamId(0), TypeParamId(0x8000)]
            );
            execute(module);
        }
    }
}

#[test]
fn structural_receiver_requires_its_retained_implementation() {
    let mut source = wire(&compile(SLICE));
    let parent = method(&source).parent_type.unwrap();
    source
        .types
        .iter_mut()
        .find(|ty| ty.id == parent)
        .unwrap()
        .source_inherent_implementations
        .clear();
    assert_import_refuses(
        &source,
        "selected method requires one exact retained implementation",
    );
}

#[test]
fn a_foreign_implementation_cannot_supply_structural_binder_roles() {
    let mut source = wire(&compile(SLICE));
    let parent = method(&source).parent_type.unwrap();
    let parent = source.types.iter_mut().find(|ty| ty.id == parent).unwrap();
    for retained in &mut parent.source_inherent_implementations {
        *retained = verum_ast::source_implementation::AssociatedImplementationSource::new(
            "foreign".into(),
            &retained.declaration().unwrap(),
        )
        .unwrap();
    }
    assert_import_refuses(
        &source,
        "selected method requires one exact retained implementation",
    );
}

#[test]
fn structural_receiver_requires_its_own_source_and_parameter_metadata() {
    let source = wire(&compile(SLICE));
    let mut no_source = source.clone();
    method_mut(&mut no_source).source_receiver = None;
    assert_import_refuses(
        &no_source,
        "structural receiver differs from its selected implementation",
    );
    let mut no_parameter = source;
    method_mut(&mut no_parameter)
        .semantic_params
        .as_mut()
        .unwrap()[0] = None;
    assert_import_refuses(
        &no_parameter,
        "imported method lacks its declaration-owned semantic receiver",
    );
}

#[test]
fn a_forged_structural_payload_does_not_rebind_the_selected_implementation() {
    let mut source = wire(&compile(SLICE));
    let selected = method_mut(&mut source);
    let forged = TypeRef::Slice(Box::new(TypeRef::Concrete(TypeId::BOOL)));
    selected.source_receiver = Some(forged.clone());
    selected.semantic_params.as_mut().unwrap()[0] = Some(TypeRef::Reference {
        inner: Box::new(forged),
        tier: CbgrTier::Tier0,
        mutability: Mutability::Immutable,
    });
    assert_import_refuses(
        &source,
        "structural receiver differs from its selected implementation",
    );
}

#[test]
fn structural_parameter_cannot_gain_mutability_or_change_reference_tier() {
    let source = wire(&compile(SLICE));
    for (tier, mutability) in [
        (CbgrTier::Tier0, Mutability::Mutable),
        (CbgrTier::Tier2, Mutability::Immutable),
    ] {
        let mut source = source.clone();
        let parameter = method_mut(&mut source).semantic_params.as_mut().unwrap()[0]
            .as_mut()
            .unwrap();
        let TypeRef::Reference {
            tier: actual_tier,
            mutability: actual_mutability,
            ..
        } = parameter
        else {
            panic!("reference receiver");
        };
        *actual_tier = tier;
        *actual_mutability = mutability;
        assert_import_refuses(
            &source,
            "structural receiver parameter differs from its selected source signature",
        );
    }
}

#[test]
fn structural_owner_slots_must_match_the_selected_implementation() {
    let mut source = wire(&compile(SLICE));
    method_mut(&mut source).type_params[0].id = TypeParamId(9);
    assert_import_refuses(
        &source,
        "generic slots differ from the selected implementation and method",
    );
}

#[test]
fn structural_semantics_cannot_be_attached_to_a_nominal_method_catalogue() {
    let mut source = wire(&compile(SLICE));
    let parent = method(&source).parent_type.unwrap();
    source
        .types
        .iter_mut()
        .find(|ty| ty.id == parent)
        .unwrap()
        .kind = TypeKind::Record;
    assert_import_refuses(
        &source,
        "structural receiver conflicts with its declaring method catalogue",
    );
}

#[test]
fn structural_element_reelaboration_uses_the_source_owner_not_a_consumer_homonym() {
    let source = compile(
        r#"
        module api;
        type Cell<Element> is { value: Element };
        implement<Element> [Cell<Element>] {
            public fn marker(&self) -> Int { Element.size + 65 }
        }
        fn main() -> Int {
            let values: [Cell<Int>; 1] = [Cell { value: 11 }];
            let part: &[Cell<Int>] = &values[..];
            part.marker()
        }
    "#,
    );
    let consumer_source = Parser::new(
        r#"
        module consumer;
        type Cell<Left, Right> is { left: Left, right: Right };
    "#,
    )
    .parse_module()
    .expect("consumer grammar");
    for source in [source.clone(), wire(&source)] {
        let mut consumer = VbcCodegen::with_config(CodegenConfig::new("consumer"));
        consumer
            .collect_unit_declarations(&[&consumer_source])
            .expect("consumer declarations");
        let imported = import_into(&source, consumer).expect("source-owned structural element");
        for module in [source, imported.clone(), wire(&imported)] {
            let TypeRef::Slice(element) = method(&module).source_receiver.as_ref().unwrap() else {
                panic!("structural receiver");
            };
            let TypeRef::Instantiated { base, args } = element.as_ref() else {
                panic!("declared generic element: {element:?}");
            };
            assert_eq!(args.as_slice(), &[TypeRef::Generic(TypeParamId(0))]);
            let owner = module.types.iter().find(|ty| ty.id == *base).unwrap();
            assert_eq!(
                verum_vbc::module::qualify_module_name(
                    owner
                        .origin_module
                        .and_then(|id| module.get_string(id))
                        .unwrap(),
                    module.get_string(owner.name).unwrap(),
                ),
                "api.Cell"
            );
            assert_eq!(
                owner.origin_module.and_then(|id| module.get_string(id)),
                Some("api")
            );
            assert_eq!(owner.type_params.len(), 1);
            if let Some(homonym) = module.types.iter().find(|ty| {
                ty.origin_module.and_then(|id| module.get_string(id)) == Some("consumer")
                    && module.get_string(ty.name).is_some_and(|name| {
                        verum_vbc::module::qualify_module_name("consumer", name) == "consumer.Cell"
                    })
            }) {
                assert_ne!(homonym.id, owner.id);
                assert_eq!(homonym.type_params.len(), 2);
            } else {
                assert_eq!(
                    module.name, "api",
                    "imported module must retain the actual consumer homonym"
                );
            }
            execute(module);
        }
    }
}

#[test]
fn fixed_array_method_refuses_a_different_extent() {
    let source = Parser::new(
        r#"
        module api;
        implement<Element> [Element; 2] {
            public fn marker(&self) -> Int { Element.size + 65 }
        }
        fn main() -> Int { let values: [Int; 3] = [11, 42, 7]; values.marker() }
    "#,
    )
    .parse_module()
    .unwrap();
    let error = VbcCodegen::with_config(CodegenConfig::new("api"))
        .compile_module(&source)
        .expect_err("extent is part of receiver identity");
    assert!(
        matches!(error.kind, CodegenErrorKind::InvalidSourceSignature(_)),
        "{error}"
    );
    assert!(
        error
            .to_string()
            .contains("no declared structural receiver"),
        "{error}"
    );
}

#[test]
fn structural_method_roles_survive_renamed_display_parameters() {
    for source in [
        SLICE.to_owned(),
        "module api; implement<Element> [Element; 2] { public fn marker(&self) -> Int { 73 } } fn main() -> Int { let values: [Int; 2] = [11, 42]; values.marker() }".to_owned(),
        "module api; implement<Element> [Element; 3] { public fn marker(&self) -> Int { 97 } } implement<Element> [Element; 2] { public fn marker(&self) -> Int { 73 } } fn main() -> Int { let values: [Int; 2] = [11, 42]; values.marker() }".to_owned(),
    ] {
        let source = Parser::new(&source).parse_module().expect("source grammar");
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new("api"));
        codegen.collect_unit_declarations(&[&source]).expect("source declarations");
        let mut renamed = 0;
        for function in codegen.ctx_mut().functions.values_mut() {
            if function.param_names.first().is_some_and(|name| name == "self") {
                function.param_names[0] = "subject".to_owned();
                renamed += 1;
            }
        }
        assert!(renamed > 0, "receiver display metadata was actually changed");
        codegen.compile_function_bodies(&source).expect("declared receiver role");
        let module = codegen.finalize_module_from_state().expect("source module");
        execute(wire(&module));
    }
}

#[test]
fn a_display_name_does_not_turn_a_static_structural_parameter_into_a_receiver() {
    let source = Parser::new(
        r#"
        module api;
        implement<Element> [Element; 2] {
            public fn marker(value: &[Element; 2]) -> Int { 73 }
        }
        fn main() -> Int { let values: [Int; 2] = [11, 42]; values.marker() }
    "#,
    )
    .parse_module()
    .expect("source grammar");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("api"));
    codegen
        .collect_unit_declarations(&[&source])
        .expect("source declarations");
    let mut renamed = 0;
    for function in codegen.ctx_mut().functions.values_mut() {
        if function
            .param_names
            .first()
            .is_some_and(|name| name == "value")
        {
            function.param_names[0] = "self".to_owned();
            renamed += 1;
        }
    }
    assert!(
        renamed > 0,
        "ordinary parameter display metadata was actually changed"
    );
    let error = codegen
        .compile_function_bodies(&source)
        .expect_err("a display name cannot manufacture a receiver role");
    assert!(
        matches!(error.kind, CodegenErrorKind::InvalidSourceSignature(_)),
        "{error}"
    );
    assert!(
        error
            .to_string()
            .contains("no declared structural receiver"),
        "{error}"
    );
}

#[test]
fn archive_entry_owner_is_distinct_from_an_invalid_owner_id() {
    let source = compile(SLICE);
    let mut local = source.clone();
    method_mut(&mut local).origin_module = None;
    for local in [local.clone(), wire(&local)] {
        execute(import(&local).expect("None explicitly denotes this archive entry's owner"));
    }
    let mut invalid = source;
    method_mut(&mut invalid).origin_module = Some(verum_vbc::types::StringId(u32::MAX));
    assert_import_refuses(&invalid, &format!("invalid declaration owner string {}", u32::MAX));
}

#[test]
fn invalid_forward_strings_are_not_internal_compiler_errors() {
    use verum_vbc::types::StringId;
    const INVALID: StringId = StringId(u32::MAX);
    let source = compile(SLICE);
    let corruptions: [(&str, fn(&mut FunctionDescriptor)); 7] = [
        ("callable name", |function| function.name = INVALID),
        ("source declaration name", |function| {
            function.declaration_identity.as_mut().expect("source identity").source_name = INVALID;
        }),
        ("parameter name", |function| function.params[0].name = INVALID),
        ("parameter type name", |function| function.params[0].type_name = INVALID),
        ("parameter name", |function| function.type_params[0].name = INVALID),
        ("return type name", |function| function.return_type_name = Some(INVALID)),
        ("intrinsic name", |function| function.intrinsic_name = Some(INVALID)),
    ];
    for (role, corrupt) in corruptions {
        let mut invalid = source.clone();
        corrupt(method_mut(&mut invalid));
        assert_import_refuses(&invalid, &format!("invalid {role} string"));
    }
}
