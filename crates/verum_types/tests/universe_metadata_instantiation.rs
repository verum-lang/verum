//! Actual call checking after the portable metadata boundary.
use std::sync::Arc;
use verum_ast::{FileId, FunctionDecl, ItemKind, Module};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_types::{TypeChecker, core_metadata::*};

fn parse(source: &str) -> Module {
    FastParser::new()
        .parse_module_str(source, FileId::new(1))
        .expect("source grammar")
}

fn declaration() -> FunctionDecl {
    let module = parse("public fn retain<universe u>(x: Type(u)) -> Type(u) { x }");
    let ItemKind::Function(function) = &module.items[0].kind else {
        panic!("function")
    };
    verum_ast::source_signature::source_signature(function, false)
        .unwrap()
        .unwrap()
}

fn descriptor(source: FunctionDecl) -> FunctionDescriptor {
    FunctionDescriptor {
        name: "retain".into(),
        module_path: "test.provider".into(),
        origin_module_path: None,
        source_signature: Some(source),
        generic_params: List::new(),
        params: List::from_single(ParamDescriptor {
            name: "x".into(),
            ty: "__opaque_src".into(),
            declared_ty: "__opaque_src".into(),
            has_default: false,
            default_literal: None,
        }),
        return_type: "__opaque_src".into(),
        contexts: List::new(),
        is_async: false,
        is_unsafe: false,
        intrinsic_id: None,
        parent_type: None,
        impl_generic_names: List::new(),
        is_const: false,
        decl_span: None,
        is_public: true,
        explicit_type_param_ids: None,
    }
}

fn metadata(source: FunctionDecl) -> CoreMetadata {
    let mut metadata = CoreMetadata {
        version: CORE_METADATA_VERSION,
        ..CoreMetadata::default()
    };
    metadata
        .functions
        .insert("test.provider.retain".into(), descriptor(source));
    metadata
}

fn check(metadata: CoreMetadata, source: &str, enabled: bool, eager: bool) -> List<Text> {
    let metadata = Arc::new(metadata);
    let mut checker = if eager {
        TypeChecker::new_with_core_eager(metadata)
    } else {
        let mut checker = TypeChecker::new();
        checker.set_core_metadata(metadata);
        checker
    };
    checker.register_primitives();
    checker.set_current_cog("test");
    checker.set_current_module_path("test.consumer");
    checker.set_universe_poly_enabled(enabled);
    let module = parse(source);
    let mut errors: List<Text> = checker
        .prepare_module_declarations(&module)
        .iter()
        .map(|error| format!("{error}").into())
        .collect();
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error}").into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .iter()
            .map(|error| Text::from(format!("{error}"))),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|d| Text::from(format!("{d:?}"))),
    );
    errors
}

#[test]
fn round_trip_metadata_calls_instantiate_levels_independently() {
    let bytes = encode_metadata(&metadata(declaration())).unwrap();
    for eager in [false, true] {
        let source = "fn one(x: Type(1)) -> Type(1) { test.provider.retain(x) } fn two(x: Type(2)) -> Type(2) { test.provider.retain(x) }";
        let errors = check(decode_metadata(&bytes).unwrap(), source, true, eager);
        assert!(errors.is_empty(), "eager={eager}: {errors:?}");
        let errors = check(
            decode_metadata(&bytes).unwrap(),
            "fn bad(x: Type(2)) -> Type(1) { test.provider.retain(x) }",
            true,
            eager,
        );
        assert!(
            !errors.is_empty(),
            "metadata must not erase a downward universe cast"
        );
    }
}

#[test]
fn invalid_carriers_cannot_fall_back_to_opaque_signature_strings() {
    let source = "fn use_it(x: Type(1)) -> Type(1) { test.provider.retain(x) }";
    let errors = check(metadata(declaration()), source, false, false);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("universe_polymorphism")),
        "{errors:?}"
    );
    let mut unbound = declaration();
    unbound.generics.clear();
    let errors = check(metadata(unbound), source, true, false);
    assert!(
        errors.iter().any(|error| error.contains("unbound")),
        "{errors:?}"
    );
    let mut wrong_owner = declaration();
    wrong_owner.name.name = "another".into();
    let errors = check(metadata(wrong_owner), source, true, false);
    assert!(
        errors.iter().any(|error| error.contains("descriptor")),
        "{errors:?}"
    );
}

#[test]
fn equal_archived_parameter_spans_do_not_merge_sibling_declarations() {
    let first = declaration();
    let mut second = first.clone();
    second.name.name = "other".into();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_universe_poly_enabled(true);
    checker.register_function_signature(&first).unwrap();
    checker.register_function_signature(&second).unwrap();
    let a = checker
        .ctx_env_lookup("retain")
        .unwrap()
        .universe_parameters();
    let b = checker
        .ctx_env_lookup("other")
        .unwrap()
        .universe_parameters();
    assert_eq!(a.len(), 1);
    assert_eq!(b.len(), 1);
    assert_ne!(a, b);
}

#[test]
fn metadata_replays_callable_universe_bounds_at_actual_calls() {
    let module = parse("public fn retain<universe u, F: fn() -> Type(u)>(f: F) -> Type(u) { f() }");
    let ItemKind::Function(function) = &module.items[0].kind else {
        panic!("function")
    };
    let declaration = verum_ast::source_signature::source_signature(function, false)
        .unwrap()
        .unwrap();
    let bytes = encode_metadata(&metadata(declaration)).unwrap();
    let errors = check(
        decode_metadata(&bytes).unwrap(),
        "fn one(f: fn() -> Type(1)) -> Type(1) { test.provider.retain(f) } fn two(f: fn() -> Type(2)) -> Type(2) { test.provider.retain(f) }",
        true,
        false,
    );
    assert!(errors.is_empty(), "{errors:?}");
    let errors = check(
        decode_metadata(&bytes).unwrap(),
        "fn bad(f: fn() -> Type(2)) -> Type(1) { test.provider.retain(f) }",
        true,
        false,
    );
    assert!(!errors.is_empty());
    for level in [1, 2] {
        let source = format!(
            "fn use_it(f: fn() -> Type({level})) -> Type(1) {{ let alias: fn(fn() -> Type({level})) -> Type(1) = test.provider.retain; alias(f) }}"
        );
        let errors = check(decode_metadata(&bytes).unwrap(), &source, true, false);
        if level == 1 {
            assert!(errors.is_empty(), "{errors:?}");
        } else {
            assert!(!errors.is_empty(), "metadata value lost its bound");
        }
    }
}

fn with_array_signature(source_ty: &str, abi_ty: &str) -> CoreMetadata {
    let module = parse(&format!(
        "public fn retain<universe u, const N: Int>(x: {source_ty}) -> Int {{ 0 }}"
    ));
    let ItemKind::Function(f) = &module.items[0].kind else {
        panic!("function")
    };
    let mut metadata = metadata(
        verum_ast::source_signature::source_signature(f, false)
            .unwrap()
            .unwrap(),
    );
    let descriptor = metadata.functions.values_mut().next().unwrap();
    descriptor.params[0].ty = abi_ty.into();
    descriptor.params[0].declared_ty = abi_ty.into();
    descriptor.return_type = "Int".into();
    metadata
}

#[test]
fn array_evidence_requires_the_same_source_position_and_checked_extent() {
    for (source, abi) in [
        ("[Int; -1]", "[Int; 2]"),
        ("[Int; 3]", "[Int; 2]"),
        ("[Int; 9223372036854775807 + 1]", "__opaque_src"),
        ("Bool", "[Int; 2]"),
        ("([Int; 2], Int)", "([Int; 2], Bool, Int)"),
        ("&[[Int; 3]; 2]", "&[[Int; 4]; 2]"),
        ("Left<[Int; 2]>", "Right<[Int; 2]>"),
        ("[Int; N]", "[Int; 2]"),
    ] {
        let metadata = with_array_signature(source, abi);
        let err = encode_metadata(&metadata).expect_err("unsupported array evidence");
        assert!(err.contains("array"), "{source} versus {abi}: {err}");
        let mut bytes = CORE_METADATA_MAGIC.to_vec();
        bytes.extend_from_slice(&CORE_METADATA_VERSION.to_le_bytes());
        bytes.extend(bincode::serialize(&metadata).unwrap());
        assert!(decode_metadata(&bytes).is_err(), "{source} versus {abi}");
        for eager in [false, true] {
            let errors = check(metadata.clone(), "fn caller() {}", true, eager);
            assert!(
                errors.iter().any(|e| e.contains("array")),
                "{source} versus {abi}: {errors:?}"
            );
        }
    }
}

#[test]
fn matching_and_erased_dimensions_keep_the_checked_source_signature() {
    for (source, abi) in [
        ("[Int; N]", "[Int]"),
        ("[Int; N]", "__opaque_src"),
        ("&[[Int; 2]; 3]", "&[[Int; 2]; 3]"),
        ("Boxed<([Int; 2], Int)>", "Boxed<([Int; 2], Int)>"),
    ] {
        let bytes = encode_metadata(&with_array_signature(source, abi))
            .unwrap_or_else(|e| panic!("{source} with {abi}: {e}"));
        assert!(decode_metadata(&bytes).is_ok(), "{source} with {abi}");
    }
    let bytes = encode_metadata(&with_array_signature("[Int; 1 + 1]", "[Int; 2]")).unwrap();
    for eager in [false, true] {
        let ok = check(
            decode_metadata(&bytes).unwrap(),
            "fn call(x: [Int; 2]) -> Int { test.provider.retain(x) }",
            true,
            eager,
        );
        assert!(ok.is_empty(), "{ok:?}");
        let bad = check(
            decode_metadata(&bytes).unwrap(),
            "fn call(x: [Int; 3]) -> Int { test.provider.retain(x) }",
            true,
            eager,
        );
        assert!(
            bad.iter().any(|e| e.contains("const generic mismatch")),
            "{bad:?}"
        );
    }
}

#[test]
fn bracket_generic_arguments_need_the_owners_declared_type_slot() {
    for (head, admit) in [("T", true), ("const N: Int", false)] {
        let mut m = with_array_signature("Boxed<[Int; 2]>", "Boxed<[Int; 2]>");
        let module = parse(&format!(
            "public type Boxed<{head}> is {{ public level: Type(1) }};"
        ));
        let ItemKind::Type(decl) = &module.items[0].kind else {
            panic!("type")
        };
        let td = TypeDescriptor {
            name: "Boxed".into(),
            module_path: "test.provider".into(),
            origin_module_path: None,
            source_declaration: verum_ast::source_signature::source_type_declaration(decl).unwrap(),
            generic_params: List::new(),
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
            resource_discipline: verum_common::ResourceDiscipline::Unrestricted,
        };
        m.types.insert("test.provider.Boxed".into(), td);
        assert_eq!(encode_metadata(&m).is_ok(), admit, "{head}");
    }
}

#[test]
fn result_and_declared_parameter_spelling_supply_independent_dimension_claims() {
    for returned in [false, true] {
        let mut metadata = with_array_signature("[Int; 2]", "[Int; 2]");
        let descriptor = metadata.functions.values_mut().next().unwrap();
        if returned {
            descriptor.return_type = "[Int; 2]".into();
        } else {
            descriptor.params[0].declared_ty = "[Int; 3]".into();
        }
        assert!(encode_metadata(&metadata).is_err());
    }
}
