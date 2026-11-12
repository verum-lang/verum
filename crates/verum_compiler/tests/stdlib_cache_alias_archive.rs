//! Parser-owned alias meaning must reach a real bootstrap archive and metadata.
//! This uses compile_core discovery/parsing/registration/writing on a tiny core
//! tree; no manufactured descriptor or inherited embedded archive supplies it.

use std::fs;
use verum_common::Text;
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
use verum_types::core_metadata::TypeDescriptorKind;
use verum_vbc::{
    archive::read_archive_from_file,
    types::{TypeParamId, TypeRef},
};

#[test]
fn generic_parameter_aliases_survive_the_actual_bootstrap_archive() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    fs::create_dir(&core).unwrap();
    let input = core.join("mod.vr");
    fs::write(
        &input,
        "module core; public type Identity<T> is T; public type Pick<L, R> is R; public type Marker is T;",
    )
    .unwrap();
    let output = root.path().join("runtime.vbca");
    let config = CoreConfig::new(&core).with_output(&output);
    let mut session = Session::new(CompilerOptions {
        input,
        ..Default::default()
    });
    let result = CompilationPipeline::new_core(&mut session, config)
        .compile_core()
        .expect("actual bootstrap producer compiles the parsed alias source");
    assert_eq!(result.modules_compiled, 1);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert!(result.output_size > 0);
    assert_eq!(result.output_path, output);

    let archive = read_archive_from_file(&output).expect("decode the produced archive from disk");
    let module = archive.load_module("core").expect("actual core module");
    let metadata = verum_compiler::archive_metadata::archive_to_core_metadata(&archive).expect("valid archive metadata");
    let metadata: verum_types::core_metadata::CoreMetadata =
        bincode::deserialize(&bincode::serialize(&metadata).unwrap()).unwrap();
    for (name, parameter, slot) in [("Identity", "T", 0), ("Pick", "R", 1)] {
        let descriptor = module
            .types
            .iter()
            .find(|descriptor| module.get_string(descriptor.name) == Some(name))
            .unwrap_or_else(|| panic!("archive must contain {name}"));
        assert_eq!(
            descriptor.alias_target,
            Some(TypeRef::Generic(TypeParamId(slot)))
        );
        assert_eq!(
            descriptor
                .alias_target_name
                .and_then(|id| module.get_string(id)),
            Some(parameter)
        );
        assert!(
            descriptor.variants.is_empty(),
            "parameter alias became a marker variant"
        );
        let TypeDescriptorKind::Alias { target } = &metadata
            .types
            .get(&Text::from(format!("core.{name}")))
            .expect("exact core alias metadata")
            .kind
        else {
            panic!("{name} metadata must retain alias identity");
        };
        assert_eq!(target.as_str(), parameter);
    }
    let marker = module
        .types
        .iter()
        .find(|descriptor| module.get_string(descriptor.name) == Some("Marker"))
        .expect("unrelated marker declaration");
    assert!(marker.alias_target.is_none());
    assert_eq!(marker.variants.len(), 1);
    assert_eq!(module.get_string(marker.variants[0].name), Some("T"));
}

#[test]
fn source_attested_sidecars_require_the_exact_full_bake() {
    use verum_compiler::precompile::{
        PrecompileConfig, precompile_stdlib, regenerate_stdlib_sidecars,
    };
    use verum_vbc::archive::{ArchiveFlags, read_archive_from_file, write_archive_to_file};

    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("core");
    std::fs::create_dir(&root).unwrap();
    let original = "public fn value() -> Int { 41 }";
    std::fs::write(root.join("mod.vr"), original).unwrap();
    let archive = fixture.path().join("runtime.vbca");
    precompile_stdlib(&PrecompileConfig {
        stdlib_path: root.clone(),
        output_path: archive.clone(),
        target_triple: None,
        threads: None,
        verbose: false,
    })
    .unwrap();
    let bytes = std::fs::read(&archive).unwrap();
    std::fs::remove_file(archive.with_extension("core_metadata")).unwrap();
    std::fs::write(archive.with_extension("symbol_graph"), b"stale graph").unwrap();
    regenerate_stdlib_sidecars(&archive, &root, false).expect("source and original VBC");
    assert_eq!(std::fs::read(&archive).unwrap(), bytes);
    let receipt = std::fs::read_to_string(archive.with_added_extension("receipt")).unwrap();
    for (field, ext) in [("metadata", "core_metadata"), ("symbols", "symbol_graph")] {
        let repaired = std::fs::read(archive.with_extension(ext)).unwrap();
        assert!(!repaired.is_empty() && repaired != b"stale graph");
        assert!(receipt.lines().any(|line| line == format!("{field}={}", blake3::hash(&repaired))));
    }

    let metadata = archive.with_extension("core_metadata");
    std::fs::write(&metadata, b"preserve on refusal").unwrap();
    let refused = || {
        let error = regenerate_stdlib_sidecars(&archive, &root, false).unwrap_err();
        assert!(
            format!("{error:#}").contains("matching full bake"),
            "{error:#}"
        );
        assert_eq!(std::fs::read(&metadata).unwrap(), b"preserve on refusal");
    };
    std::fs::write(root.join("mod.vr"), "public fn value() -> Int { 42 }").unwrap();
    refused();
    std::fs::write(root.join("mod.vr"), original).unwrap();

    let receipt = archive.with_added_extension("receipt");
    let contents = std::fs::read(&receipt).unwrap();
    std::fs::remove_file(&receipt).unwrap();
    refused();
    std::fs::write(&receipt, contents).unwrap();
    let mut replacement = read_archive_from_file(&archive).unwrap();
    replacement
        .header
        .flags
        .toggle(ArchiveFlags::STRIP_FIELD_NAMES);
    write_archive_to_file(&replacement, &archive).unwrap();
    read_archive_from_file(&archive).expect("replacement is a valid, different archive");
    refused();
}
