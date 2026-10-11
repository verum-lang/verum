//! Qualified source lookup retains exact metadata authority in a mixed module.
//! Metadata is synthetic consumer input; this does not exercise archive production.

use std::sync::Arc; // The public TypeChecker metadata constructor takes Arc.
use verum_ast::{FileId, ItemKind, Visibility};
use verum_common::{List, ResourceDiscipline, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::{
    TypeChecker,
    core_metadata::{CoreMetadata, FieldDescriptor, TypeDescriptor, TypeDescriptorKind},
};

const OWNER: &str = "core.mixed.defining";
const KEY: &str = "core.mixed.defining.Packet";

fn metadata(declaring_owner: &str, indexed_key: &str) -> CoreMetadata {
    let descriptor = TypeDescriptor {
        name: "Packet".into(),
        module_path: "archive.bundle".into(),
        origin_module_path: Some(declaring_owner.into()),
        generic_params: List::new(),
        kind: TypeDescriptorKind::Record {
            fields: List::from_iter([FieldDescriptor {
                name: "body".into(),
                ty: "Int".into(),
                is_public: true,
                declared_visibility: Some(Visibility::Public),
            }]),
        },
        size: Some(8),
        alignment: Some(8),
        methods: List::new(),
        implements: List::new(),
        decl_span: None,
        is_public: true,
        is_transparent_wrapper: false,
        resource_discipline: ResourceDiscipline::Unrestricted,
    };
    let mut metadata = CoreMetadata::default();
    metadata
        .types
        .insert(indexed_key.into(), descriptor.clone());
    metadata.types.insert("Packet".into(), descriptor);
    metadata.type_declaration_order.push(indexed_key.into());
    // Exercise the actual metadata reader without inventing a parsed Packet.
    serde_json::from_slice(&serde_json::to_vec(&metadata).unwrap()).unwrap()
}

fn check(
    owner_source: &str,
    caller: &str,
    consumer: &str,
    metadata: CoreMetadata,
    eager: bool,
) -> List<Text> {
    check_with_modules(owner_source, caller, consumer, metadata, eager, &[])
}

fn check_with_modules(
    owner_source: &str,
    caller: &str,
    consumer: &str,
    metadata: CoreMetadata,
    eager: bool,
    additional_modules: &[(&str, &str)],
) -> List<Text> {
    let metadata = Arc::new(metadata);
    let mut checker = if eager {
        TypeChecker::new_with_core_eager(metadata)
    } else {
        TypeChecker::new_with_core(metadata)
    };
    checker.register_primitives();
    checker.set_current_module_path(caller);
    let mut registry = ModuleRegistry::new();
    let mut consumer_ast = None;
    for (index, (owner, source)) in [(OWNER, owner_source)]
        .into_iter()
        .chain(additional_modules.iter().copied())
        .chain([(caller, consumer)])
        .enumerate()
    {
        if index == 0 && caller == OWNER {
            continue;
        }
        let file = FileId::new(index as u32);
        let ast = FastParser::new()
            .parse_module_str(source, file)
            .expect("fixture grammar");
        let path = ModulePath::from_str(owner);
        let id = ModuleId::new(index as u32);
        let mut module = ModuleInfo::new(id, path.clone(), ast.clone(), file, source.into());
        module.exports = extract_exports_from_module(&ast, id, &path).expect("fixture exports");
        registry.register(module);
        if owner == caller {
            consumer_ast = Some(ast);
        }
    }
    checker.set_module_registry_direct(registry);
    let ast = consumer_ast.expect("consumer module");
    checker.register_stdlib_types_for_module(&ast);
    let mut errors = List::new();
    for item in &ast.items {
        if let ItemKind::Function(function) = &item.kind {
            if let Err(error) = checker.register_function_signature(function) {
                errors.push(format!("{error:?}").into());
            }
        }
    }
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}").into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors
}

fn probe() -> Text {
    format!("fn probe(value: &{KEY}) -> Int {{ value.body }}").into()
}

#[test]
fn same_owner_source_function_can_use_its_metadata_only_record() {
    for eager in [false, true] {
        let errors = check("", OWNER, probe().as_str(), metadata(OWNER, KEY), eager);
        assert!(errors.is_empty(), "eager={eager}: {errors:?}");
    }
}

#[test]
fn nonempty_provider_does_not_hide_its_exact_metadata_declaration() {
    for eager in [false, true] {
        let errors = check(
            "public fn unrelated() {}",
            "core.consumer",
            probe().as_str(),
            metadata(OWNER, KEY),
            eager,
        );
        assert!(errors.is_empty(), "eager={eager}: {errors:?}");
    }
}

#[test]
fn empty_provider_keeps_the_metadata_only_route() {
    for eager in [false, true] {
        let errors = check(
            "",
            "core.consumer",
            probe().as_str(),
            metadata(OWNER, KEY),
            eager,
        );
        assert!(errors.is_empty(), "eager={eager}: {errors:?}");
    }
}

#[test]
fn nonempty_source_module_still_refuses_a_missing_type() {
    for eager in [false, true] {
        let errors = check(
            "public fn unrelated() {}",
            "core.consumer",
            probe().as_str(),
            CoreMetadata::default(),
            eager,
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("TypeNotFound") && error.contains(KEY)),
            "eager={eager}: {errors:?}"
        );
    }
}

#[test]
fn foreign_same_leaf_descriptor_cannot_supply_the_missing_owner() {
    for eager in [false, true] {
        for index in ["core.foreign.Packet", KEY] {
            let errors = check(
                "public fn unrelated() {}",
                "core.consumer",
                probe().as_str(),
                metadata("core.foreign", index),
                eager,
            );
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("TypeNotFound") && error.contains(KEY)),
                "eager={eager}, index={index}: {errors:?}"
            );
        }
    }
}

#[test]
fn private_source_declaration_is_not_replaced_by_public_metadata() {
    for eager in [false, true] {
        let errors = check(
            "type Packet is { public body: Int };",
            "core.consumer",
            probe().as_str(),
            metadata(OWNER, KEY),
            eager,
        );
        assert!(
            errors.iter().any(|error| error.contains("VisibilityError")
                && error.contains("name: Text { inner: \"Packet\" }")
                && error.contains(OWNER)),
            "eager={eager}: {errors:?}"
        );
    }
}


fn assert_source_route_refused(owner_source: &str, modules: &[(&str, &str)]) {
    for eager in [false, true] {
        let errors = check_with_modules(
            owner_source, "core.consumer", probe().as_str(),
            metadata(OWNER, KEY), eager, modules,
        );
        assert!(
            errors.iter().any(|error| error.contains("TypeNotFound") && error.contains(KEY)),
            "eager={eager}, source={owner_source}: {errors:?}",
        );
    }
}

#[test]
fn missing_explicit_source_reexports_do_not_borrow_local_metadata() {
    for source in [
        "public mount core.absent.Packet;",
        "public mount core.absent.{Packet};",
        "public mount core.absent.{Target as Packet};",
    ] {
        assert_source_route_refused(source, &[]);
    }
}

#[test]
fn private_target_reexport_does_not_borrow_local_metadata() {
    assert_source_route_refused(
        "public mount core.hidden.{Packet};",
        &[("core.hidden", "type Packet is { public body: Int };")],
    );
}

#[test]
fn private_mount_does_not_export_a_metadata_homonym() {
    assert_source_route_refused(
        "mount core.hidden.{Packet};",
        &[("core.hidden", "public type Packet is { public body: Int };")],
    );
}

#[test]
fn cyclic_source_reexport_does_not_borrow_local_metadata() {
    assert_source_route_refused(
        "public mount core.looping.{Packet};",
        &[("core.looping", "public mount core.mixed.defining.{Packet};")],
    );
}

#[test]
fn public_source_reexports_keep_the_declared_target_before_metadata() {
    for eager in [false, true] {
        for (owner_source, target_source) in [
            ("public mount core.target.{Packet};", "public type Packet is { public body: Bool };"),
            ("public mount core.target.{Target as Packet};", "public type Target is { public body: Bool };"),
        ] {
            let consumer = format!("fn probe(value: &{KEY}) -> Bool {{ value.body }}");
            let errors = check_with_modules(
                owner_source, "core.consumer", &consumer,
                metadata(OWNER, KEY), eager, &[("core.target", target_source)],
            );
            // The conflicting local metadata says Int. A clean Bool result
            // therefore proves the re-export selected its source declaration.
            assert!(errors.is_empty(), "eager={eager}, source={owner_source}: {errors:?}");
        }
    }
}

#[test]
fn renamed_mount_does_not_claim_its_original_spelling() {
    for eager in [false, true] {
        let errors = check(
            "public mount core.absent.{Packet as Other};",
            "core.consumer", probe().as_str(), metadata(OWNER, KEY), eager,
        );
        assert!(errors.is_empty(), "eager={eager}: {errors:?}");
    }
}
