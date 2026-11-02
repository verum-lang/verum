use verum_lsp::library_metadata::LibraryMetadata;
use verum_types::core_metadata::CoreMetadata;

#[test]
fn metadata_decode_rejects_invalid_and_trailing_data() {
    assert!(LibraryMetadata::decode(&[]).is_err());
    assert!(LibraryMetadata::decode(b"invalid").is_err());
    let metadata = CoreMetadata {
        version: verum_types::core_metadata::CORE_METADATA_VERSION,
        ..Default::default()
    };
    let mut bytes = verum_types::core_metadata::encode_metadata(&metadata).unwrap();
    assert!(LibraryMetadata::decode(&bytes).is_ok());
    assert!(LibraryMetadata::decode(&bincode::serialize(&metadata).unwrap()).is_err());
    let mut old_schema = bytes.clone();
    old_schema[8..12].copy_from_slice(&1u32.to_le_bytes());
    assert!(LibraryMetadata::decode(&old_schema).is_err());
    bytes.push(0);
    assert!(LibraryMetadata::decode(&bytes).is_err());
}

#[test]
#[ignore = "requires the matching release VERUM_TEST_LIBRARY_METADATA artifact"]
fn release_metadata_projects_without_conflicting_declarations() {
    let path = std::env::var_os("VERUM_TEST_LIBRARY_METADATA").expect("release artifact path");
    let library = LibraryMetadata::load(std::path::Path::new(&path)).unwrap();
    let metadata = library.metadata();
    eprintln!(
        "metadata: {} types, {} functions, {} protocols, {} implementations; {}",
        metadata.types.len(),
        metadata.functions.len(),
        metadata.protocols.len(),
        metadata.implementations.len(),
        library.identity.sha256
    );
    for (key, descriptor) in metadata
        .functions
        .iter()
        .filter(|(key, d)| key.contains(".") && d.parent_type.is_none())
        .take(12)
    {
        eprintln!(
            "function {key}: name={}, entry={}, origin={:?}, public={}",
            descriptor.name,
            descriptor.module_path,
            descriptor.origin_module_path,
            descriptor.is_public
        );
    }
    let registry = verum_modules::ModuleRegistry::new();
    let modules =
        verum_types::module_projection::project_modules(metadata, None, |_| registry.allocate_id())
            .unwrap();
    assert!(!modules.is_empty());
    eprintln!("projected {} modules", modules.len());
}
