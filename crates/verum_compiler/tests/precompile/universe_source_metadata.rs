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

/// Reuse the publication path on an explicitly supplied, freshly baked archive.
/// This gate never substitutes a cached artifact or silently skips absent input.
#[test]
#[ignore = "requires VERUM_TEST_FROZEN_STDLIB and its unchanged declaring source tree"]
fn frozen_stdlib_sidecars_pass_source_validation() {
    let archive = std::path::PathBuf::from(std::env::var_os("VERUM_TEST_FROZEN_STDLIB")
        .expect("explicit freshly baked stdlib archive required"));
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../core");
    super::write_core_metadata_alongside_archive(&archive, Some(&root), true).unwrap();
    super::write_symbol_graph_alongside_archive(&archive, true).unwrap();
}
