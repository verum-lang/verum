use verum_common::Map;
use verum_modules::{ModuleId, ModuleLoader, ModulePath};

#[test]
fn closed_source_generation_resolves_unsaved_files_and_never_reads_new_disk_contents() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("model.vr");
    let mut sources = Map::new();
    sources.insert(path.clone(), "public type Item is { value: Int };".into());
    let mut loader = ModuleLoader::new(root.path());
    loader.set_source_snapshot(sources);
    std::fs::write(&path, "broken disk source").unwrap();
    let loaded = loader
        .load_module(&ModulePath::from_str("model"), ModuleId::new(1))
        .unwrap();
    assert_eq!(loaded.source, "public type Item is { value: Int };");
    std::fs::write(root.path().join("unexpected.vr"), "fn main() {}").unwrap();
    assert!(
        loader
            .load_module(&ModulePath::from_str("unexpected"), ModuleId::new(2))
            .is_err()
    );
    loader.set_source_snapshot(Map::new());
    assert!(
        loader
            .load_module(&ModulePath::from_str("model"), ModuleId::new(1))
            .is_err()
    );
}

#[test]
fn snapshot_candidates_preserve_compiler_collision_rules() {
    let root = tempfile::tempdir().unwrap();
    let mut sources = Map::new();
    for path in ["model.vr", "model/mod.vr"] {
        sources.insert(root.path().join(path), "".into());
    }
    let mut loader = ModuleLoader::new(root.path());
    loader.set_source_snapshot(sources);
    let error = loader
        .load_module(&ModulePath::from_str("model"), ModuleId::new(1))
        .unwrap_err();
    assert_eq!(error.code(), "E_MODULE_PATH_COLLISION");
}
