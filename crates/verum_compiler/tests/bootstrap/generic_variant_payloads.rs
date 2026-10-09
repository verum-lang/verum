//! T1653: parsed source → bootstrap publication → archive wire → metadata → checker.
//! No embedded archive or hand-built type descriptor supplies the expected identity.
use super::*;
use crate::Session;
use verum_common::{List, Maybe, Set, Shared, Text};
use verum_fast_parser::Parser;
use verum_types::{
    TypeChecker,
    core_metadata::{CoreMetadata, TypeDescriptorKind, VariantPayload},
};
use verum_vbc::{
    VbcModule,
    archive::{ArchiveBuilder, VbcArchive},
    types::{TypeDescriptor, TypeId, TypeRef},
};

const COLLECTIONS: &[(&str, &str)] = &[
    (
        "core.collections.list",
        "public type List<T> is { value: T };",
    ),
    (
        "core.collections.map",
        "public type Map<K, V> is { key: K, value: V };",
    ),
    (
        "core.collections",
        "public mount .list.List; public mount .map.Map;",
    ),
];

fn parse(owner: &str, source: &str) -> verum_ast::Module {
    Parser::new(&format!("module {owner}; {source}"))
        .parse_module()
        .expect("fixture follows the source grammar")
}

fn bootstrap(prior: &[(&str, &str)], source: &str) -> (VbcModule, VbcArchive) {
    let mut session = Session::new(Default::default());
    let config = CoreConfig::new(".");
    let sources: List<_> = prior
        .iter()
        .copied()
        .chain(std::iter::once(("core.encoding.json", source)))
        .map(|(owner, source)| (owner, parse(owner, source)))
        .collect();
    // compile_core registers source exports before compiling any unit. Use the
    // same export extraction and registry, including actual public re-exports.
    {
        let registry = session.module_registry();
        let mut registry = registry.write();
        for (owner, ast) in &sources {
            let id = registry.allocate_id();
            let path = ModulePath::from_str(owner);
            let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), ast.file_id, Text::new());
            info.exports = extract_exports_from_module(ast, id, &path).unwrap();
            registry.register(info);
        }
        resolve_specific_reexport_kinds(&mut registry).unwrap();
        resolve_glob_reexports(&mut registry).unwrap();
    }
    let mut pipeline = CompilationPipeline::new_core(&mut session, config.clone());
    let mut builder = ArchiveBuilder::new();
    let mut produced = None;
    for (owner, ast) in &sources {
        let unit = StdlibModule {
            name: (*owner).into(),
            source_files: List::new().into(),
            dependencies: List::new().into(),
        };
        let (module, _) = pipeline
            .compile_core_module_from_ast(
                &unit,
                &[ast],
                &config,
                &verum_ast::cfg::TargetConfig::host(),
                &Default::default(),
            )
            .expect("the real bootstrap producer accepts the source unit");
        builder.add_module(owner, &module, &[]).unwrap();
        // This is the publication performed by compile_core between units.
        pipeline
            .compiled_stdlib_modules
            .insert((*owner).into(), module.clone());
        produced = Some(module);
    }
    let mut bytes = List::new();
    verum_vbc::archive::write_archive(&builder.finish(), &mut bytes).unwrap();
    let archive = verum_vbc::archive::read_archive(std::io::Cursor::new(bytes.as_slice())).unwrap();
    (produced.unwrap(), archive)
}

fn json(mounts: &str, list: &str, map: &str) -> Text {
    format!("{mounts} public type JsonValue is JsonNull | JsonArray({list}<JsonValue>) | JsonObject({map}<Text, JsonValue>) | JsonCount(USize);").into()
}

fn ty<'a>(module: &'a VbcModule, owner: &str, leaf: &str) -> &'a TypeDescriptor {
    module
        .types
        .iter()
        .find(|ty| {
            ty.origin_module.and_then(|id| module.get_string(id)) == Some(owner)
                && module
                    .get_string(ty.name)
                    .is_some_and(|name| name.rsplit('.').next() == Some(leaf))
        })
        .unwrap_or_else(|| panic!("missing declared {owner}.{leaf}"))
}

fn field<'a>(module: &'a VbcModule, variant: &str) -> &'a verum_vbc::types::FieldDescriptor {
    let json = ty(module, "core.encoding.json", "JsonValue");
    let variant = json
        .variants
        .iter()
        .find(|v| module.get_string(v.name) == Some(variant))
        .unwrap();
    assert_eq!(variant.fields.len(), 1);
    &variant.fields[0]
}

fn assert_payload(module: &VbcModule, name: &str, expected: TypeRef) {
    let field = field(module, name);
    assert_eq!(field.type_ref, expected, "{name}: layout identity");
    assert_eq!(
        field.declaration_type.as_ref(),
        Some(&expected),
        "{name}: declaration identity"
    );
}

fn assert_collections(module: &VbcModule) {
    let json = ty(module, "core.encoding.json", "JsonValue").id;
    assert_payload(
        module,
        "JsonArray",
        TypeRef::Instantiated {
            base: TypeId::LIST,
            args: List::from_iter([TypeRef::Concrete(json)]).into(),
        },
    );
    assert_payload(
        module,
        "JsonObject",
        TypeRef::Instantiated {
            base: TypeId::MAP,
            args: List::from_iter([TypeRef::Concrete(TypeId::TEXT), TypeRef::Concrete(json)])
                .into(),
        },
    );
}

fn metadata(archive: &VbcArchive) -> CoreMetadata {
    let metadata = crate::archive_metadata::archive_to_core_metadata(archive);
    bincode::deserialize(&bincode::serialize(&metadata).unwrap()).unwrap()
}

fn payload(metadata: &CoreMetadata, name: &str) -> List<Text> {
    let desc = metadata
        .types
        .get(&Text::from("core.encoding.json.JsonValue"))
        .unwrap();
    let TypeDescriptorKind::Variant { cases } = &desc.kind else {
        panic!("variant descriptor")
    };
    let case = cases.iter().find(|case| case.name == name).unwrap();
    let Maybe::Some(VariantPayload::Tuple(types)) = &case.payload else {
        panic!("tuple payload")
    };
    types.clone()
}

fn errors(archive: &VbcArchive, source: &str, eager: bool) -> List<Text> {
    let metadata = metadata(archive);
    let mut registry = verum_modules::ModuleRegistry::new();
    let owners: Set<Text> = metadata
        .types
        .values()
        .map(|ty| ty.module_path.clone())
        .collect();
    for owner in &owners {
        let id = registry.allocate_id();
        registry.register(ModuleInfo::new(
            id,
            ModulePath::from_str(owner),
            Parser::new("").parse_module().unwrap(),
            verum_ast::FileId::new(id.as_u32()),
            Text::new(),
        ));
    }
    let metadata = Shared::new(metadata).into_arc();
    let mut checker = if eager {
        TypeChecker::new_with_core_eager(metadata)
    } else {
        TypeChecker::new_with_core(metadata)
    };
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    checker.set_module_registry_direct(registry);
    let ast = Parser::new(source)
        .parse_module()
        .expect("consumer grammar");
    checker.register_stdlib_types_for_module(&ast);
    let mut errors: List<Text> = ast
        .items
        .iter()
        .filter_map(|item| {
            checker
                .check_item(item)
                .err()
                .map(|error| format!("{error:?}").into())
        })
        .collect();
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| format!("{error:?}").into()),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|error| format!("{error:?}").into()),
    );
    errors
}

#[test]
fn declared_generic_variant_exact_owners_survive_bootstrap_and_wire() {
    let source = json(
        "mount core.collections.list.List; mount core.collections.map.Map;",
        "List",
        "Map",
    );
    let (module, archive) = bootstrap(COLLECTIONS, &source);
    assert_collections(&module);
    assert_collections(&archive.load_module("core.encoding.json").unwrap());
}

#[test]
fn declared_generic_variant_umbrella_and_short_mounts_survive_bootstrap_and_wire() {
    for mount in ["core.collections", "collections"] {
        let source = json(&format!("mount {mount}.{{List, Map}};"), "List", "Map");
        let (module, archive) = bootstrap(COLLECTIONS, &source);
        assert_collections(&module);
        assert_collections(&archive.load_module("core.encoding.json").unwrap());
        let metadata = metadata(&archive);
        assert_eq!(
            payload(&metadata, "JsonArray").as_slice(),
            &["List<JsonValue>"]
        );
        assert_eq!(
            payload(&metadata, "JsonObject").as_slice(),
            &["Map<Text, JsonValue>"]
        );
    }
}

#[test]
fn declared_generic_variant_mount_aliases_keep_declared_identity() {
    let source = json(
        "mount collections.{List as Sequence, Map as Dictionary};",
        "Sequence",
        "Dictionary",
    );
    let (module, archive) = bootstrap(COLLECTIONS, &source);
    assert_collections(&module);
    assert_collections(&archive.load_module("core.encoding.json").unwrap());
}

#[test]
fn declared_generic_variant_foreign_same_leaf_is_not_a_builtin() {
    let mut prior: List<_> = COLLECTIONS.iter().copied().collect();
    prior.extend([(
        "foreign.collections",
        "public type List<T> is { item: T }; public type Map<K,V> is { left: K, right: V };",
    )]);
    let source = json("mount foreign.collections.{List, Map};", "List", "Map");
    let (module, archive) = bootstrap(&prior, &source);
    let declared = archive.load_module("foreign.collections").unwrap();
    assert_ne!(
        ty(&declared, "foreign.collections", "List").id,
        TypeId::LIST
    );
    assert_ne!(ty(&declared, "foreign.collections", "Map").id, TypeId::MAP);
    for (owner, leaf) in [
        ("core.collections.list", "List"),
        ("core.collections.map", "Map"),
        ("foreign.collections", "List"),
        ("foreign.collections", "Map"),
    ] {
        let declared = archive.load_module(owner).unwrap();
        eprintln!(
            "source descriptor {owner}.{leaf}: {:?}",
            ty(&declared, owner, leaf).id
        );
    }
    for module in [&module, &archive.load_module("core.encoding.json").unwrap()] {
        let list = ty(module, "foreign.collections", "List").id;
        let map = ty(module, "foreign.collections", "Map").id;
        assert_ne!(
            list,
            TypeId::LIST,
            "foreign declaration cannot claim the canonical carrier"
        );
        assert_ne!(
            map,
            TypeId::MAP,
            "foreign declaration cannot claim the canonical carrier"
        );
        let json = ty(module, "core.encoding.json", "JsonValue").id;
        assert_payload(
            module,
            "JsonArray",
            TypeRef::Instantiated {
                base: list,
                args: List::from_iter([TypeRef::Concrete(json)]).into(),
            },
        );
        assert_payload(
            module,
            "JsonObject",
            TypeRef::Instantiated {
                base: map,
                args: List::from_iter([TypeRef::Concrete(TypeId::TEXT), TypeRef::Concrete(json)])
                    .into(),
            },
        );
    }
}

#[test]
fn declared_generic_variant_local_shadow_and_missing_owner_never_select_a_stranger() {
    let source = json(
        "mount collections.List; type List<T> is { local: T };",
        "List",
        "Map",
    );
    let (module, _) = bootstrap(COLLECTIONS, &source);
    let local = ty(&module, "core.encoding.json", "List").id;
    assert_ne!(local, TypeId::LIST);
    let json_id = ty(&module, "core.encoding.json", "JsonValue").id;
    assert_payload(
        &module,
        "JsonArray",
        TypeRef::Instantiated {
            base: local,
            args: List::from_iter([TypeRef::Concrete(json_id)]).into(),
        },
    );
    let source = json("mount absent.collections.{List, Map};", "List", "Map");
    let (module, _) = bootstrap(COLLECTIONS, &source);
    for (name, canonical) in [("JsonArray", TypeId::LIST), ("JsonObject", TypeId::MAP)] {
        assert!(
            !matches!(field(&module, name).declaration_type,
            Some(TypeRef::Instantiated { base, .. }) if base == canonical),
            "a missing owner must not use an unrelated simple-name binding"
        );
    }
}

fn checker_contract(mounts: &str) {
    let source = json(mounts, "List", "Map");
    let (_, archive) = bootstrap(COLLECTIONS, &source);
    for eager in [false, true] {
        for (variant, value_type, wrong_types) in [
            ("JsonArray", "List<JsonValue>", &["Bool"][..]),
            (
                "JsonObject",
                "Map<Text, JsonValue>",
                &["Bool", "Map<Bool, JsonValue>", "Map<Text, Bool>"][..],
            ),
        ] {
            let source = format!(
                "mount core.encoding.json.{{JsonValue, {variant}}}; fn wrap(value: {value_type})->JsonValue {{ {variant}(value) }}"
            );
            // Keep the consumer spelling identical for direct and umbrella
            // producer mounts. T1212's checker authority resolves the nominal
            // owner; the fixture does not rewrite Map to a qualified alias.
            let good = errors(&archive, &source, eager);
            assert!(good.is_empty(), "{variant} eager={eager}: {good:?}");
            for wrong_type in wrong_types {
                let wrong = source.replace(value_type, wrong_type);
                assert!(
                    !errors(&archive, &wrong, eager).is_empty(),
                    "wrong payload accepted eager={eager}: {wrong}"
                );
            }
        }
    }
}

#[test]
fn declared_generic_variant_genuine_usize_payload_remains_usize() {
    let mut prior: List<_> = COLLECTIONS.iter().copied().collect();
    prior.push((
        "core.base.primitives",
        "public type ScalarMarker is protocol {}; implement ScalarMarker for USize {}",
    ));
    let (module, archive) = bootstrap(&prior, &json("", "List", "Map"));
    assert_payload(&module, "JsonCount", TypeRef::Concrete(TypeId::PTR));
    let decoded = archive.load_module("core.encoding.json").unwrap();
    assert_payload(&decoded, "JsonCount", TypeRef::Concrete(TypeId::PTR));
    assert_eq!(
        decoded.get_string(field(&decoded, "JsonCount").type_name),
        Some("USize")
    );
    // A real source protocol impl emits the scalar descriptor; no hand-built
    // descriptor or carried-name override supplies this metadata expectation.
    assert_eq!(
        payload(&metadata(&archive), "JsonCount").as_slice(),
        &["USize"]
    );
}

#[test]
fn declared_generic_variant_direct_metadata_drives_checker_acceptance_and_refusal() {
    checker_contract("mount core.collections.list.List; mount core.collections.map.Map;");
}

#[test]
fn declared_generic_variant_umbrella_metadata_drives_checker_acceptance_and_refusal() {
    checker_contract("mount collections.{List, Map};");
}

#[test]
fn declared_generic_variant_broken_or_private_reexport_never_borrows_parent_identity() {
    for declarations in [
        [
            ("broken", "public type List<T> is { parent: T };"),
            ("broken.exports", "public mount broken.missing.List;"),
        ],
        [
            ("broken", "type List<T> is { hidden: T };"),
            ("broken.exports", "public mount broken.List;"),
        ],
    ] {
        let mut prior: List<_> = COLLECTIONS.iter().copied().collect();
        prior.extend(declarations);
        let source = json("mount broken.exports.List;", "List", "Map");
        let (module, _) = bootstrap(&prior, &source);
        assert!(
            matches!(
                field(&module, "JsonArray").declaration_type,
                Some(TypeRef::Instantiated {
                    base: TypeId::PTR,
                    ..
                })
            ),
            "an invalid re-export must stay unresolved, not select a parent or private declaration"
        );
    }
}

#[test]
fn declared_generic_variant_renamed_reexport_chains_preserve_identity() {
    let mut prior: List<_> = COLLECTIONS.iter().copied().collect();
    prior.extend([
        (
            "facade",
            "public mount core.collections.{List as Sequence, Map as Dictionary};",
        ),
        (
            "facade.outer",
            "public mount facade.{Sequence as Items, Dictionary as Table};",
        ),
    ]);
    let source = json("mount facade.outer.{Items, Table};", "Items", "Table");
    let (module, archive) = bootstrap(&prior, &source);
    assert_collections(&module);
    assert_collections(&archive.load_module("core.encoding.json").unwrap());
}

#[test]
fn declared_generic_variant_nonbuiltin_export_uses_the_same_source_authority() {
    let (module, archive) = bootstrap(
        &[
            ("alpha", "public type Widget<T> is { item: T };"),
            ("facade", "public mount alpha.{Widget as Wrapper};"),
        ],
        "mount facade.Wrapper; public type JsonValue is JsonArray(Wrapper<Int>);",
    );
    for module in [&module, &archive.load_module("core.encoding.json").unwrap()] {
        let widget = ty(module, "alpha", "Widget").id;
        assert_payload(
            module,
            "JsonArray",
            TypeRef::Instantiated {
                base: widget,
                args: List::from_iter([TypeRef::Concrete(TypeId::INT)]).into(),
            },
        );
    }
}

#[test]
fn declared_generic_variant_exact_cog_dependency_retains_its_descriptor() {
    let (module, archive) = bootstrap(
        &[("core.alpha", "public(cog) type Internal is { value: Int };")],
        "public type JsonValue is JsonArray(core.alpha.Internal);",
    );
    // This exact declaration is already in the bootstrap catalog, while the
    // public-export-only resolver deliberately declines restricted exports.
    // Keep the producer dependency; this does not bypass checker visibility.
    for module in [&module, &archive.load_module("core.encoding.json").unwrap()] {
        let internal = ty(module, "core.alpha", "Internal");
        assert_payload(module, "JsonArray", TypeRef::Concrete(internal.id));
    }
}
