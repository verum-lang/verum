use crate::{archive_metadata::archive_to_core_metadata, pipeline::CompilationPipeline};
use verum_ast::{ItemKind, ty::GenericParamKind};
use verum_fast_parser::Parser;
use verum_types::core_metadata::{CORE_METADATA_VERSION, CoreMetadata};
use verum_vbc::{
    archive::ArchiveBuilder,
    codegen::{CodegenConfig, VbcCodegen},
};

const POLYMORPHIC: &str = "public fn retain<universe u>(value: Type(u)) -> Type(u) { value }";

fn scan(source: &str, metadata: &mut CoreMetadata) -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    std::fs::write(directory.path().join("universe_fixture.vr"), source)?;
    super::inject_declared_module_free_fn_keys(metadata, directory.path(), false)
}

fn cached(function: Option<&str>, ty: Option<&str>) -> crate::core_cache::CachedCoreMetadata {
    use crate::core_cache::{CachedCoreMetadata, CachedFunctionEntry, CachedTypeEntry};
    CachedCoreMetadata {
        functions: function
            .into_iter()
            .map(|signature| CachedFunctionEntry {
                path: "core.fixture.function".into(),
                signature: signature.into(),
                is_intrinsic: false,
                intrinsic_name: None,
            })
            .collect(),
        types: ty
            .into_iter()
            .map(|definition| CachedTypeEntry {
                path: "core.fixture.Holder".into(),
                definition: definition.into(),
                kind: "record".into(),
            })
            .collect(),
        modules: Vec::new(),
        context_declarations: Vec::new(),
        meta_functions: Vec::new(),
        macros: Vec::new(),
        derives: Vec::new(),
    }
}

#[test]
fn source_scan_preserves_typed_level_binders_and_concrete_universes() {
    for source in [
        POLYMORPHIC,
        "public fn retain(value: Type(1)) -> Type(1) { value }",
    ] {
        let mut metadata = CoreMetadata::default();
        scan(source, &mut metadata).unwrap();
        let descriptor = metadata
            .functions
            .values()
            .find(|function| function.name == "retain")
            .unwrap();
        let signature = descriptor
            .source_signature
            .as_ref()
            .expect("typed declaration carrier");
        assert!(signature.body.is_none());
        verum_ast::source_signature::validate_source_signature(signature, false).unwrap();
        let bytes = bincode::serialize(&metadata).unwrap();
        let decoded: CoreMetadata = bincode::deserialize(&bytes).unwrap();
        assert_eq!(
            decoded
                .functions
                .values()
                .find(|function| function.name == "retain")
                .unwrap()
                .source_signature,
            descriptor.source_signature
        );
    }
}

#[test]
fn source_scan_refuses_unbound_and_unsupported_nested_level_signatures() {
    for source in [
        "public fn bad(value: Type(missing)) -> Type(missing) { value }",
        "public fn bad<universe u, A: Type(u)>(value: A) -> A { value }",
        "public fn bad<A: Type(1)>(value: A) -> A { value }",
        "public fn bad(value: fn<universe u>(Type(u)) -> Type(u)) { }",
    ] {
        assert!(
            scan(source, &mut CoreMetadata::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn source_scan_refuses_universe_static_annotations_before_rendering() {
    for source in [
        "public static descriptor: Type(1) = 0;",
        "public static descriptor: (Int, Type(0)) = (0, 0);",
    ] {
        let mut metadata = CoreMetadata::default();
        assert!(scan(source, &mut metadata).is_err(), "{source}");
        assert!(metadata.statics.is_empty());
    }
    let mut ordinary = CoreMetadata::default();
    scan("public static universe: Int = 0;", &mut ordinary).unwrap();
    assert_eq!(ordinary.statics.len(), 1);
}

#[test]
fn first_wins_cannot_mask_a_carrier_owned_by_another_declaration() {
    let mut metadata = CoreMetadata::default();
    scan(POLYMORPHIC, &mut metadata).unwrap();
    scan(POLYMORPHIC, &mut metadata).expect("the same declaration is idempotent");
    for function in metadata.functions.values_mut() {
        function.source_signature.as_mut().unwrap().name.name = "different".into();
    }
    assert!(scan(POLYMORPHIC, &mut metadata).is_err());
}

#[test]
fn legacy_cache_refuses_universe_declarations_without_guessing_from_spelling() {
    for fixture in [
        cached(Some(POLYMORPHIC), None),
        cached(
            Some("public fn retain(value: Type(1)) -> Type(1) { value }"),
            None,
        ),
        cached(
            None,
            Some("public type Holder<universe u> is { value: Type(u) };"),
        ),
        cached(None, Some("public type Holder is { value: Type(1) };")),
    ] {
        assert!(CompilationPipeline::convert_cached_metadata_to_stdlib(&fixture).is_err());
    }
    let ordinary = cached(
        Some("public fn universe<Level>(value: Level) -> Level { value }"),
        Some("public type Universe is { value: Int };"),
    );
    assert!(CompilationPipeline::convert_cached_metadata_to_stdlib(&ordinary).is_ok());
    assert!(
        CompilationPipeline::convert_cached_metadata_to_stdlib(&cached(
            Some("public fn incomplete("),
            None
        ))
        .is_err()
    );
}

fn compiled_module() -> verum_vbc::module::VbcModule {
    let source = format!("module core.universe_fixture; {POLYMORPHIC}");
    let ast = Parser::new(&source).parse_module().unwrap();
    VbcCodegen::with_config(CodegenConfig::new("core.universe_fixture"))
        .compile_module(&ast)
        .unwrap()
}

#[test]
fn archive_and_sidecar_roundtrips_preserve_source_universe_identity() {
    let module = compiled_module();
    let mut builder = ArchiveBuilder::new();
    builder.add_module(&module.name, &module, &[]).unwrap();
    let mut bytes = Vec::new();
    verum_vbc::archive::write_archive(&builder.finish(), &mut bytes).unwrap();
    let archive = verum_vbc::archive::read_archive(std::io::Cursor::new(bytes)).unwrap();
    let metadata = archive_to_core_metadata(&archive).unwrap();
    assert_eq!(metadata.version, CORE_METADATA_VERSION);
    let signature = metadata
        .functions
        .values()
        .find_map(|function| function.source_signature.as_ref())
        .unwrap();
    assert!(matches!(
        signature.generics[0].kind,
        GenericParamKind::Level { .. }
    ));
    let ast = Parser::new(POLYMORPHIC).parse_module().unwrap();
    let ItemKind::Function(original) = &ast.items[0].kind else {
        panic!("function fixture")
    };
    assert_eq!(signature.name.name, original.name.name);
    assert!(signature.body.is_none());
    let decoded: CoreMetadata =
        bincode::deserialize(&bincode::serialize(&metadata).unwrap()).unwrap();
    assert_eq!(
        decoded
            .functions
            .values()
            .find_map(|function| function.source_signature.as_ref()),
        Some(signature)
    );
}

#[test]
fn archive_publication_rejects_a_carrier_attached_to_another_declaration() {
    let mut module = compiled_module();
    let function = module
        .functions
        .iter_mut()
        .find(|function| function.source_signature.is_some())
        .unwrap();
    function.source_signature.as_mut().unwrap().name.name = "other".into();
    let mut builder = ArchiveBuilder::new();
    builder.add_module(&module.name, &module, &[]).unwrap();
    assert!(archive_to_core_metadata(&builder.finish()).is_err());
}

#[test]
fn source_publication_accepts_exact_interface_and_body_names_with_producer_file_ids() {
    let source = format!("module core.universe_fixture; {POLYMORPHIC}");
    for interface in [false, true] {
        let ast = verum_fast_parser::FastParser::new()
            .parse_module_str(&source, verum_ast::FileId::new(57)).unwrap();
        let mut producer = VbcCodegen::with_config(CodegenConfig::new("core.universe_fixture"));
        let module = if interface { producer.collect_declaration_interface(&[&ast]).unwrap() }
            else { producer.compile_module(&ast).unwrap() };
        let mut builder = ArchiveBuilder::new();
        builder.add_module(&module.name, &module, &[]).unwrap();
        let mut metadata = archive_to_core_metadata(&builder.finish()).unwrap();
        scan(&source, &mut metadata).unwrap();
        let key = verum_common::Text::from("core.universe_fixture.retain");
        let valid = metadata.clone();
        for bad_name in ["other.retain", "core.universe_fixture.other.retain", "core.universe_fixturex.retain"] {
            let mut forged = valid.clone();
            forged.functions.get_mut(&key).unwrap().name = bad_name.into();
            assert!(scan(&source, &mut forged).is_err(), "{bad_name}");
        }
        let mut forged = valid.clone();
        forged.functions.get_mut(&key).unwrap().origin_module_path = verum_common::Maybe::Some("other".into());
        assert!(scan(&source, &mut forged).is_err());
        // Same owner/name/arity but different level expression must not pass.
        let stale = source.replace("-> Type(u)", "-> Type(1)");
        assert!(scan(&stale, &mut metadata).is_err());
    }
}

#[test]
fn archive_publication_rejects_foreign_prefix_even_when_signature_leaf_matches() {
    let mut module = compiled_module();
    let foreign = module.strings.intern("other.retain");
    module.functions.iter_mut().find(|function| function.source_signature.is_some())
        .unwrap().name = foreign;
    let mut builder = ArchiveBuilder::new();
    builder.add_module(&module.name, &module, &[]).unwrap();
    assert!(archive_to_core_metadata(&builder.finish()).is_err());
}

fn mixed_protocol_with_import_shell(visibility: &str) -> (tempfile::TempDir, CoreMetadata) {
    let source = format!("module core.indexed; public type Parent is protocol {{}}; {visibility} type Contract<N: meta Int> is protocol extends Parent {{ fn dimension(&self) -> Int; }};");
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("indexed.vr"), &source).unwrap();
    let ast = Parser::new(&source).parse_module().unwrap();
    let mut owner = VbcCodegen::with_config(CodegenConfig::new("core.indexed"));
    let declared = owner.compile_module(&ast).unwrap();
    // A legacy archive had an unowned ABI-only slot. Current import_protocols
    // reserves a known owner and is tested through the production completion
    // path below. Construct the historical wire representation directly.
    let mut imported = verum_vbc::VbcModule::new("core.importer".into());
    let name = imported.strings.intern("Contract");
    imported.types.push(verum_vbc::types::TypeDescriptor {
        id: verum_vbc::types::TypeId(verum_vbc::types::TypeId::FIRST_USER),
        name,
        kind: verum_vbc::types::TypeKind::Protocol,
        ..Default::default()
    });
    let mut archive = ArchiveBuilder::new();
    archive.add_module("core.indexed", &declared, &[]).unwrap();
    archive.add_module("core.importer", &imported, &[]).unwrap();
    let metadata = archive_to_core_metadata(&archive.finish()).unwrap();
    (root, metadata)
}

#[test]
fn reserved_semantic_protocols_publish_the_defining_interface() {
    let ast = Parser::new(
        "module core.indexed; public type Parent is protocol {}; public type Contract<N: meta Int> is protocol extends Parent { fn dimension(&self) -> Int; };",
    ).parse_module().unwrap();
    let mut owner = VbcCodegen::with_config(CodegenConfig::new("core.indexed"));
    let declared = owner.compile_module(&ast).unwrap();
    let protocols = owner.export_protocols();
    let publish = |imported: &verum_vbc::VbcModule| {
        let mut archive = ArchiveBuilder::new();
        archive.add_module("core.indexed", &declared, &[]).unwrap();
        archive.add_module("core.importer", imported, &[]).unwrap();
        archive_to_core_metadata(&archive.finish())
    };
    let mut incomplete = VbcCodegen::with_config(CodegenConfig::new("core.importer"));
    incomplete.import_protocols(&protocols);
    let error = publish(&incomplete.finalize_module().unwrap()).unwrap_err();
    assert!(error.contains("semantic and ABI-only declarations disagree for core.indexed.Contract"));

    for source in [
        "module core.importer; fn unrelated() {}",
        "module core.importer; mount core.indexed.{Contract}; fn touch(v: &dyn Contract<1>) {}",
    ] {
        let ast = Parser::new(source).parse_module().unwrap();
        let mut importer = VbcCodegen::with_config(CodegenConfig::new("core.importer"));
        importer.import_protocols(&protocols);
        importer.import_bootstrap_nominal_dependencies(&[&ast], &[&declared]).unwrap();
        let imported = importer.finalize_module().unwrap();
        let metadata = publish(&imported).unwrap();
        let contract = metadata.types.get(&"core.indexed.Contract".into()).unwrap();
        let carried = imported.types.iter().filter_map(|ty| ty.source_declaration.as_ref())
            .find(|decl| decl.name.name == "Contract").expect("imported semantic declaration");
        assert_eq!(contract.source_declaration.as_ref(), Some(carried), "{source}");
        let key = verum_common::Text::from("core.indexed");
        let bindings = declared.declaration_type_bindings.get(&key).expect("declaring scope");
        assert_eq!(imported.declaration_type_bindings.get(&key), Some(bindings), "{source}");
    }
}

#[test]
fn source_binding_uses_the_declaring_protocol_for_an_imported_abi_shell() {
    let (root, mut metadata) = mixed_protocol_with_import_shell("public");
    let imported = verum_common::Text::from("core.importer.Contract");
    let declared = verum_common::Text::from("core.indexed.Contract");
    let original = metadata.types.get(&declared).unwrap().clone();
    assert!(original.source_declaration.is_some());
    assert!(metadata.types.get(&imported).unwrap().source_declaration.is_none());
    super::bind_source_declarations(&mut metadata, root.path(), &[], false);
    let decoded = verum_types::core_metadata::decode_metadata(
        &verum_types::core_metadata::encode_metadata(&metadata).unwrap(),
    ).unwrap();
    let bound = decoded.types.get(&imported).unwrap();
    assert_eq!(bound.source_declaration, original.source_declaration);
    assert_eq!(bound.module_path, original.module_path);
    assert_eq!(bound.origin_module_path, original.origin_module_path);
    assert_eq!(bound.methods, original.methods);
    // Compatibility lookup keys are internal. Only declaring modules and
    // explicit source re-exports contribute to the public module surface.
    let registry = verum_modules::ModuleRegistry::new();
    let modules = verum_types::module_projection::project_modules(&decoded, None, |_| registry.allocate_id()).unwrap();
    let owners: Vec<_> = modules.iter().filter(|m| m.exports.contains("Contract"))
        .map(|m| m.path.to_string()).collect();
    assert_eq!(owners, ["core.indexed"]);
}

#[test]
fn imported_protocol_slots_preserve_source_visibility() {
    let (root, mut metadata) = mixed_protocol_with_import_shell("");
    super::bind_source_declarations(&mut metadata, root.path(), &[], false);
    for key in ["core.indexed.Contract", "core.importer.Contract"] {
        let entry = metadata.types.get(&key.into()).unwrap();
        assert!(entry.source_declaration.is_some());
        assert!(!entry.is_public, "private declaration became public at {key}");
    }
}

#[test]
fn a_reserved_protocol_slot_cannot_compete_with_its_final_glob_export() {
    use verum_common::{List, Text};

    let (root, mut metadata) = mixed_protocol_with_import_shell("public");
    let mut slot = metadata.types.get(&Text::from("core.importer.Contract")).unwrap().clone();
    slot.name = "Parent".into();
    slot.origin_module_path = Some("core".into());
    let slot_key = Text::from("core.importer.Parent");
    metadata.types.insert(slot_key.clone(), slot);
    super::bind_source_declarations(&mut metadata, root.path(), &[], false);
    // Public glob expansion follows binding during the actual sidecar build.
    metadata.module_reexports.insert("core".into(), List::from_iter([
        ("Parent".into(), "Parent".into(), "core.indexed".into()),
    ]));
    let registry = verum_modules::ModuleRegistry::new();
    verum_types::module_projection::project_modules(&metadata, None, |_| registry.allocate_id()).unwrap();
    assert_eq!(metadata.types.get(&slot_key).unwrap().origin_module_path.as_deref(), Some("core.indexed"));
}

#[test]
fn source_binding_cannot_repair_an_abi_claim_that_already_names_the_semantic_owner() {
    let (root, mut metadata) = mixed_protocol_with_import_shell("public");
    metadata.types.get_mut(&"core.importer.Contract".into()).unwrap().origin_module_path = Some("core.indexed".into());
    super::bind_source_declarations(&mut metadata, root.path(), &[], false);
    let error = verum_types::core_metadata::encode_metadata(&metadata).unwrap_err();
    assert!(error.contains("semantic and ABI-only declarations disagree for core.indexed.Contract"), "{error}");
}

#[test]
fn an_ambiguous_import_shell_cannot_acquire_a_semantic_homonym() {
    let (root, mut metadata) = mixed_protocol_with_import_shell("public");
    std::fs::write(root.path().join("other.vr"), "public type Contract is protocol {};").unwrap();
    super::bind_source_declarations(&mut metadata, root.path(), &[], false);
    let bound = metadata.types.get(&"core.importer.Contract".into()).unwrap();
    assert!(bound.origin_module_path.is_none());
    assert!(bound.source_declaration.is_none());
}

#[test]
fn source_binding_does_not_overwrite_a_foreign_protocol_abi() {
    let (root, mut metadata) = mixed_protocol_with_import_shell("public");
    metadata.types.get_mut(&"core.importer.Contract".into()).unwrap().methods.push("other".into());
    super::bind_source_declarations(&mut metadata, root.path(), &[], false);
    assert!(verum_types::core_metadata::encode_metadata(&metadata).unwrap_err()
        .contains("semantic and ABI-only declarations disagree for core.indexed.Contract"));
}

#[test]
fn inline_factory_origin_survives_sidecar_source_binding() {
    use verum_common::Text;
    use verum_vbc::{archive::ArchiveBuilder, codegen::{CodegenConfig, VbcCodegen}};

    let root = tempfile::tempdir().unwrap();
    let source = r#"
        module core.factory;
        public type Factory is { value: Int };
        public module Factory {
            public fn map(value: Int) -> Int { value }
            fn filter(value: Int) -> Int { value }
            module Hidden {
                public fn nested(value: Int) -> Int { value }
            }
        }
    "#;
    std::fs::write(root.path().join("factory.vr"), source).unwrap();
    std::fs::write(root.path().join("unrelated.vr"),
        "fn map(v: Int) -> Int { v } public fn filter(v: Int) -> Int { v } public fn foreign(v: Int) -> Int { v }"
    ).unwrap();
    let ast = verum_fast_parser::Parser::new(source).parse_module().unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new("core")).compile_module(&ast).unwrap();
    let mut archive = ArchiveBuilder::stdlib();
    archive.add_module("core", &module, &[]).unwrap();
    let mut metadata = crate::archive_metadata::archive_to_core_metadata(&archive.finish()).unwrap();
    let mut unresolved = metadata.functions.get(&Text::from("core.factory.Factory.map")).unwrap().clone();
    unresolved.name = "core.absent.foreign".into();
    unresolved.origin_module_path = Some("core.absent".into());
    unresolved.is_public = false;
    metadata.functions.insert(unresolved.name.clone(), unresolved);

    super::bind_source_declarations(&mut metadata, root.path(), &[], false);
    for (key, origin, public) in [
        ("core.factory.Factory.map", "core.factory.Factory", true),
        ("core.factory.Factory.filter", "core.factory.Factory", false),
        ("core.factory.Factory.Hidden.nested", "core.factory.Factory.Hidden", false),
        ("core.absent.foreign", "core.absent", false),
    ] {
        let fd = metadata.functions.get(&Text::from(key)).expect(key);
        assert_eq!(fd.origin_module_path.as_deref(), Some(origin), "{key}");
        assert_eq!(fd.is_public, public, "{key}");
    }
}

/// Reuse the publication path on an explicitly supplied, freshly baked archive.
/// This gate never substitutes a cached artifact or silently skips absent input.
#[test]
#[ignore = "requires VERUM_TEST_FROZEN_STDLIB and its unchanged declaring source tree"]
fn frozen_stdlib_sidecars_pass_source_validation() {
    let archive = std::path::PathBuf::from(std::env::var_os("VERUM_TEST_FROZEN_STDLIB")
        .expect("explicit freshly baked stdlib archive required"));
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../core");
    super::regenerate_stdlib_sidecars(&archive, &root, true).unwrap();
}
