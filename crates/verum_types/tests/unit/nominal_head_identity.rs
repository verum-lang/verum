//! T1676: metadata and source applications retain the same declared owner.
use super::*;
use crate::core_metadata::{
    CoreMetadata, FunctionDescriptor, GenericParam, ParamDescriptor, TypeDescriptor,
    TypeDescriptorKind, VariantCase, VariantPayload,
};
use crate::infer::parse_descriptor_type_string as parse_type;
use verum_common::ResourceDiscipline;
use verum_fast_parser::Parser;

fn descriptor(name: &str, owner: &str, params: &[&str]) -> TypeDescriptor {
    TypeDescriptor {
        name: name.into(),
        module_path: "bundle".into(),
        origin_module_path: Some(owner.into()),
        generic_params: params
            .iter()
            .enumerate()
            .map(|(pid, name)| GenericParam {
                name: (*name).into(),
                bounds: List::new(),
                default: None,
                type_bounds: List::new(),
                pid: Some(pid as u16),
            })
            .collect(),
        kind: TypeDescriptorKind::Record {
            fields: List::new(),
        },
        size: None,
        alignment: None,
        methods: List::new(),
        implements: List::new(),
        decl_span: None,
        is_public: true,
        is_transparent_wrapper: false,
        resource_discipline: ResourceDiscipline::Unrestricted,
    }
}

fn metadata(owner: &str, reverse: bool) -> CoreMetadata {
    let mut metadata = CoreMetadata::default();
    let chosen = descriptor("Table", owner, &["Key", "Value"]);
    let foreign = descriptor("Table", "foreign", &["Key", "Value"]);
    let entries = [
        (format!("{owner}.Table"), chosen.clone()),
        ("foreign.Table".into(), foreign),
    ];
    for index in if reverse { [1, 0] } else { [0, 1] } {
        let (key, desc) = &entries[index];
        metadata.types.insert(key.as_str().into(), desc.clone());
        metadata.type_declaration_order.push(key.as_str().into());
    }
    metadata.types.insert("Table".into(), chosen.clone());
    metadata.types.insert("exports.Renamed".into(), chosen);
    serde_json::from_slice(&serde_json::to_vec(&metadata).unwrap()).unwrap()
}

fn checker(metadata: CoreMetadata, eager: bool) -> TypeChecker {
    let metadata = std::sync::Arc::new(metadata);
    if eager {
        TypeChecker::new_with_core_eager(metadata)
    } else {
        TypeChecker::new_with_core(metadata)
    }
}

fn generic(name: &str, value: Type) -> Type {
    Type::Generic {
        name: name.into(),
        args: List::from_iter([Type::Text, value]),
    }
}

fn compatible(checker: &mut TypeChecker, left: Type, right: Type) -> bool {
    checker.unifier.unify(&left, &right, Span::dummy()).is_ok()
}

#[test]
fn metadata_generic_and_bare_named_have_one_declared_owner() {
    for eager in [false, true] {
        for reverse in [false, true] {
            for swap in [false, true] {
                let mut checker = checker(metadata("alpha", reverse), eager);
                let left = generic("alpha.Table", Type::Int);
                let right = parse_type("Table<Text, Int>");
                let (left, right) = if swap { (right, left) } else { (left, right) };
                assert!(
                    compatible(&mut checker, left, right),
                    "eager={eager} reverse={reverse} swap={swap}"
                );
            }
        }
    }
}

#[test]
fn fully_qualified_generic_and_named_heads_are_identical() {
    let mut checker = checker(metadata("alpha", false), false);
    assert!(compatible(
        &mut checker,
        generic("alpha.Table", Type::Int),
        parse_type("alpha.Table<Text, Int>")
    ));
}

#[test]
fn registered_export_alias_uses_its_declared_owner() {
    for eager in [false, true] {
        let mut checker = checker(metadata("alpha", false), eager);
        assert!(compatible(
            &mut checker,
            generic("alpha.Table", Type::Int),
            parse_type("exports.Renamed<Text, Int>")
        ));
    }
}

#[test]
fn qualified_foreign_owner_never_borrows_the_bare_binding() {
    for eager in [false, true] {
        for reverse in [false, true] {
            let mut checker = checker(metadata("alpha", reverse), eager);
            assert!(!compatible(
                &mut checker,
                generic("Table", Type::Int),
                parse_type("foreign.Table<Text, Int>")
            ));
            assert!(!compatible(
                &mut checker,
                generic("alpha.Table", Type::Int),
                parse_type("foreign.Table<Text, Int>")
            ));
        }
    }
}

#[test]
fn unknown_qualified_owner_never_borrows_the_bare_binding() {
    let mut checker = checker(metadata("alpha", false), false);
    assert!(!compatible(
        &mut checker,
        generic("Table", Type::Int),
        parse_type("missing.Table<Text, Int>")
    ));
    assert!(!compatible(
        &mut checker,
        generic("missing.Table", Type::Int),
        parse_type("Table<Text, Int>")
    ));
}

#[test]
fn matching_leaf_without_declaration_is_not_identity() {
    let mut checker = TypeChecker::new();
    assert!(!compatible(
        &mut checker,
        generic("Table", Type::Int),
        parse_type("missing.Table<Text, Int>")
    ));
}

#[test]
fn value_arguments_still_constrain_declared_heads() {
    let mut checker = checker(metadata("alpha", false), false);
    assert!(!compatible(
        &mut checker,
        generic("alpha.Table", Type::Int),
        parse_type("Table<Text, Bool>")
    ));
}

#[test]
fn key_arguments_and_arity_still_constrain_declared_heads() {
    let mut checker = checker(metadata("alpha", false), false);
    assert!(!compatible(
        &mut checker,
        generic("alpha.Table", Type::Int),
        parse_type("Table<Bool, Int>")
    ));
    assert!(!compatible(
        &mut checker,
        generic("alpha.Table", Type::Int),
        parse_type("Table<Text>")
    ));
}

#[test]
fn replacing_metadata_replaces_the_head_authority() {
    let mut checker = checker(metadata("alpha", false), false);
    checker.set_core_metadata(std::sync::Arc::new(metadata("beta", false)));
    assert!(compatible(
        &mut checker,
        generic("beta.Table", Type::Int),
        parse_type("Table<Text, Int>")
    ));
    assert!(!compatible(
        &mut checker,
        generic("alpha.Table", Type::Int),
        parse_type("Table<Text, Int>")
    ));
}

fn json_metadata() -> CoreMetadata {
    let mut metadata = CoreMetadata::default();
    let map = descriptor("Map", "core.collections.map", &["Key", "Value"]);
    let mut json = descriptor("JsonValue", "core.encoding.json", &[]);
    json.kind = TypeDescriptorKind::Variant {
        cases: List::from_iter([
            VariantCase {
                name: "JsonNull".into(),
                payload: None,
            },
            VariantCase {
                name: "JsonObject".into(),
                payload: Some(VariantPayload::Tuple(List::from_iter([
                    "Map<Text, JsonValue>".into(),
                ]))),
            },
        ]),
    };
    json.methods.push("as_object".into());
    let mut maybe = descriptor("Maybe", "core.base.maybe", &["T"]);
    maybe.kind = TypeDescriptorKind::Variant {
        cases: List::from_iter([
            VariantCase {
                name: "None".into(),
                payload: None,
            },
            VariantCase {
                name: "Some".into(),
                payload: Some(VariantPayload::Tuple(List::from_iter(["T".into()]))),
            },
        ]),
    };
    for desc in [map, json, maybe] {
        let owner = TypeChecker::metadata_declaring_key(&desc);
        metadata.types.insert(desc.name.clone(), desc.clone());
        metadata.type_declaration_order.push(desc.name.clone());
        metadata.types.insert(owner, desc);
    }
    let accessor = FunctionDescriptor {
        name: "as_object".into(),
        module_path: "core.encoding.json".into(),
        origin_module_path: None,
        generic_params: List::new(),
        params: List::from_iter([ParamDescriptor {
            name: "self".into(),
            ty: "&JsonValue".into(),
            declared_ty: "&JsonValue".into(),
            has_default: false,
            default_literal: None,
        }]),
        return_type: "Maybe<&Map<Text, JsonValue>>".into(),
        contexts: List::new(),
        is_async: false,
        is_unsafe: false,
        intrinsic_id: None,
        parent_type: Some("JsonValue".into()),
        impl_generic_names: List::new(),
        is_const: false,
        decl_span: None,
        is_public: true,
        explicit_type_param_ids: Some(List::new()),
    };
    metadata
        .functions
        .insert("JsonValue.as_object".into(), accessor.clone());
    metadata
        .functions
        .insert("core.encoding.json.JsonValue.as_object".into(), accessor);
    serde_json::from_slice(&serde_json::to_vec(&metadata).unwrap()).unwrap()
}

fn source_errors(value: &str, eager: bool) -> List<Text> {
    let mut checker = checker(json_metadata(), eager);
    checker.set_current_module_path("consumer");
    let source = format!(
        r#"
        fn consume(fields: &Map<Text, {value}>) -> Bool {{ true }}
        fn probe(document: &JsonValue) -> Bool {{
            match document.as_object() {{
                Maybe.Some(fields) => consume(fields),
                Maybe.None => false,
            }}
        }}
    "#
    );
    let ast = Parser::new(&source).parse_module().expect("source grammar");
    checker.register_stdlib_types_for_module(&ast);
    for item in &ast.items {
        if let verum_ast::ItemKind::Function(decl) = &item.kind {
            checker
                .register_function_signature(decl)
                .expect("source signature");
        }
    }
    let mut errors: List<Text> = ast
        .items
        .iter()
        .filter_map(|item| {
            checker
                .check_item(item)
                .err()
                .map(|e| Text::from(format!("{e:?}")))
        })
        .collect();
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors
}

#[test]
fn borrowed_json_object_crosses_a_source_helper_signature() {
    for eager in [false, true] {
        let errors = source_errors("JsonValue", eager);
        assert!(errors.is_empty(), "eager={eager}: {errors:#?}");
    }
}

#[test]
fn borrowed_json_object_cannot_be_forwarded_as_integer_values() {
    for eager in [false, true] {
        let errors = source_errors("Int", eager);
        assert!(
            errors
                .iter()
                .any(|error| error.contains("Mismatch") || error.contains("Type mismatch")),
            "eager={eager}: {errors:#?}"
        );
    }
}
