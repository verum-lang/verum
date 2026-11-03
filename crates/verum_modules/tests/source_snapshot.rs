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

#[test]
fn owned_entries_resolve_outside_layout_with_one_identity_and_closed_bytes() {
    use verum_modules::LazyModuleResolver;
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("src");
    std::fs::create_dir(&src).unwrap();
    let draft = root.path().join("research/draft.vr");
    let owner = ModulePath::from_str("demo.research.draft");
    let mut sources = Map::new();
    sources.insert(
        draft.clone(),
        (owner.clone(), "public fn answer() -> Int { 42 }".into()),
    );
    let mut loader = ModuleLoader::new(src);
    loader.set_owned_source_snapshot(sources);
    loader.set_cog_name("demo");
    for name in ["demo.research.draft", "research.draft"] {
        assert!(loader.can_resolve(name));
        let loaded = loader
            .load_module(&ModulePath::from_str(name), ModuleId::new(1))
            .unwrap();
        assert_eq!(loaded.file_path, draft);
        assert!(loaded.source.contains("42"));
    }
    let qualified = loader.resolve_module("demo.research.draft").unwrap();
    let relative = loader.resolve_module("research.draft").unwrap();
    assert_eq!(qualified.id, relative.id);
    std::fs::create_dir(draft.parent().unwrap()).unwrap();
    std::fs::write(&draft, "invalid disk bytes").unwrap();
    assert!(
        loader
            .load_module(&owner, ModuleId::new(1))
            .unwrap()
            .source
            .contains("42")
    );
    loader.set_source_snapshot(Map::new());
    assert!(!loader.can_resolve("demo.research.draft"));
    assert!(loader.load_module(&owner, ModuleId::new(1)).is_err());
}

#[test]
fn explicit_ownership_never_overrides_layout_collisions_or_duplicates_one_file() {
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("src");
    std::fs::create_dir(&src).unwrap();
    let owner = ModulePath::from_str("demo.model");
    let path = src.join("model.vr");
    let mut sources = Map::new();
    sources.insert(path.clone(), (owner.clone(), "fn main() {}".into()));
    let mut loader = ModuleLoader::new(&src).with_cog_name("demo");
    loader.set_owned_source_snapshot(sources.clone());
    assert_eq!(
        loader
            .load_module(&owner, ModuleId::new(1))
            .unwrap()
            .file_path,
        path
    );
    for conflict in [root.path().join("model.vr"), src.join("model/mod.vr")] {
        let mut ambiguous = sources.clone();
        ambiguous.insert(conflict, (owner.clone(), "fn main() {}".into()));
        loader.set_owned_source_snapshot(ambiguous);
        for name in ["model", "demo.model"] {
            let error = loader
                .load_module(&ModulePath::from_str(name), ModuleId::new(1))
                .unwrap_err();
            assert_eq!(error.code(), "E_MODULE_PATH_COLLISION");
        }
    }
}

#[test]
fn external_snapshot_ownership_does_not_alias_a_local_module() {
    use verum_modules::{LazyModuleResolver, cog_resolver::CogResolver};
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("src");
    let dependency = root.path().join("dependency");
    std::fs::create_dir(&src).unwrap();
    std::fs::create_dir(&dependency).unwrap();
    let local = root.path().join("local-draft.vr");
    let external = dependency.join("draft.vr");
    let mut sources = Map::new();
    sources.insert(
        local.clone(),
        (
            ModulePath::from_str("demo.math.item"),
            "fn local() {}".into(),
        ),
    );
    sources.insert(
        external.clone(),
        (ModulePath::from_str("math.item"), "fn external() {}".into()),
    );
    let mut resolver = CogResolver::new();
    resolver.register_cog("math", "", dependency);
    let mut loader = ModuleLoader::new(src)
        .with_cog_name("demo")
        .with_cog_resolver(resolver);
    loader.set_owned_source_snapshot(sources);
    assert!(loader.can_resolve("math.item"));
    assert_eq!(
        loader
            .load_module(&ModulePath::from_str("math.item"), ModuleId::new(1))
            .unwrap()
            .file_path,
        external
    );
    assert_eq!(
        loader
            .load_module(&ModulePath::from_str("demo.math.item"), ModuleId::new(2))
            .unwrap()
            .file_path,
        local
    );
    let external = loader.resolve_module("math.item").unwrap();
    let local = loader.resolve_module("demo.math.item").unwrap();
    assert_ne!(external.id, local.id, "external and local owners cannot share a cached declaration identity");
    assert_eq!(external.path.to_string(), "math.item");
    assert_eq!(local.path.to_string(), "demo.math.item");
}

#[test]
fn captured_project_root_main_and_lib_preserve_their_distinct_owners() {
    let root = tempfile::tempdir().unwrap();
    let mut sources = Map::new();
    for (name, value) in [("mod.vr", 1), ("main.vr", 2), ("lib.vr", 3)] {
        let path = root.path().join(name);
        let owner = ModulePath::for_source_file(root.path(), "", &path);
        sources.insert(
            path,
            (
                owner,
                format!("public fn value() -> Int {{ {value} }}").into(),
            ),
        );
    }
    let mut loader = ModuleLoader::new(root.path());
    loader.set_owned_source_snapshot(sources);
    for (owner, file, value) in [
        (ModulePath::root(), "mod.vr", 1),
        (ModulePath::from_str("main"), "main.vr", 2),
        (ModulePath::from_str("lib"), "lib.vr", 3),
    ] {
        let loaded = loader.load_module(&owner, ModuleId::new(1)).unwrap();
        assert_eq!(loaded.file_path, root.path().join(file));
        assert!(loaded.source.contains(&format!("{{ {value} }}")));
    }
}

#[test]
fn explicit_ownership_cannot_be_reinterpreted_as_a_layout_alias() {
    use verum_modules::LazyModuleResolver;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("layout_name.vr");
    let mut sources = Map::new();
    sources.insert(
        path.clone(),
        (
            ModulePath::from_str("demo.logical_name"),
            "fn main() {}".into(),
        ),
    );
    let mut loader = ModuleLoader::new(root.path()).with_cog_name("demo");
    loader.set_owned_source_snapshot(sources);
    for name in ["logical_name", "demo.logical_name"] {
        assert!(loader.can_resolve(name));
        assert_eq!(
            loader
                .load_module(&ModulePath::from_str(name), ModuleId::new(1))
                .unwrap()
                .file_path,
            path
        );
    }
    for name in ["layout_name", "demo.layout_name"] {
        assert!(!loader.can_resolve(name));
        assert!(
            loader
                .load_module(&ModulePath::from_str(name), ModuleId::new(1))
                .is_err()
        );
    }
}

#[test]
fn lazy_aliases_publish_one_declared_owner_in_either_request_order() {
    use verum_modules::{LazyModuleResolver, ModuleRegistry};
    let root = tempfile::tempdir().unwrap();
    for names in [["model", "demo.model"], ["demo.model", "model"]] {
        let mut snapshot = Map::new();
        snapshot.insert(root.path().join("model.vr"), "public fn value() -> Int { 42 }".into());
        let mut loader = ModuleLoader::new(root.path()).with_cog_name("demo");
        loader.set_source_snapshot(snapshot);
        let mut registry = ModuleRegistry::new();
        registry.set_cog_name("demo");
        let first = loader.resolve_module(names[0]).unwrap();
        let second = loader.resolve_module(names[1]).unwrap();
        assert_eq!(first.path.to_string(), "demo.model");
        assert_eq!(second.path, first.path);
        assert_eq!(first.id, second.id);
        registry.register(first);
        registry.register(second);
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.get_by_path("model").unwrap().id, registry.get_by_path("demo.model").unwrap().id);
    }
}

#[test]
fn parsed_root_owner_keeps_semantic_root_or_declared_cog_identity() {
    let root = tempfile::tempdir().unwrap();
    for cog in [None, Some("demo")] {
        let mut loader = ModuleLoader::new(root.path());
        if let Some(cog) = cog { loader.set_cog_name(cog); }
        let mut snapshot = Map::new();
        snapshot.insert(root.path().join("lib.vr"), "public fn value() -> Int { 1 }".into());
        loader.set_source_snapshot(snapshot);
        let info = loader.load_and_parse(&ModulePath::root(), ModuleId::new(1)).unwrap();
        let expected = cog.map(ModulePath::from_str).unwrap_or_else(ModulePath::root);
        assert_eq!(info.path, expected);
    }
}
