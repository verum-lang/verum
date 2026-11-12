//! Grouped mounts retain complete paths in both imports and re-export tables.
use verum_ast::{FileId, ItemKind};
use verum_common::{Map, Text};
use verum_fast_parser::VerumParser;
use verum_modules::{
    ExportKind, ImportResolver, ModuleId, ModuleInfo, ModulePath, ModuleRegistry,
    extract_exports_from_module, resolve_glob_reexports, resolve_specific_reexport_kinds,
};

fn register(registry: &mut ModuleRegistry, owner: &str, source: &str) -> ModuleId {
    let id = registry.allocate_id();
    let file = FileId::new(id.as_u32());
    let ast = VerumParser::new().parse_module_str(source, file).unwrap();
    let path = ModulePath::from_str(owner);
    let exports = extract_exports_from_module(&ast, id, &path).unwrap();
    let mut info = ModuleInfo::new(id, path, ast, file, source.into());
    info.exports = exports;
    registry.register(info)
}

fn import(
    registry: &ModuleRegistry,
    owner: ModuleId,
) -> verum_modules::ModuleResult<verum_modules::ResolvedImport> {
    let info = registry.get(owner).unwrap();
    let ItemKind::Mount(mount) = &info.ast.items[0].kind else {
        panic!("mount fixture")
    };
    let exports: Map<_, _> = registry
        .all_modules()
        .map(|(id, m)| (*id, m.exports.clone()))
        .collect();
    let paths: Map<_, _> = registry
        .all_modules()
        .map(|(id, m)| (*id, m.path.clone()))
        .collect();
    ImportResolver::new().resolve_import(mount, owner, &exports, &paths)
}

#[test]
fn complete_nested_paths_agree_for_imports_and_exports() {
    for (prefix, root) in [
        ("packages", "packages"),
        (".packages", "workspace.consumer.packages"),
        ("super.packages", "workspace.packages"),
        ("cog.packages", "cog.packages"),
    ] {
        for body in [
            "{one.answer as First, two.other}",
            "{one.{answer as First}, two.other}",
            "{one.answer as First, two.*}",
            "{one.{answer as First}, two.{nested.*}}",
        ] {
            let mut registry = ModuleRegistry::new();
            let first = register(
                &mut registry,
                &format!("{root}.one"),
                "public fn answer() -> Int { 1 }",
            );
            let second_owner = if body.contains("nested") {
                format!("{root}.two.nested")
            } else {
                format!("{root}.two")
            };
            let second = register(
                &mut registry,
                &second_owner,
                "public fn other() -> Int { 2 }",
            );
            // A plausible parent homonym must never replace a complete leaf.
            register(
                &mut registry,
                root,
                "public fn answer() -> Bool { false } public fn other() -> Bool { false }",
            );
            let source = format!("public mount {prefix}.{body};");
            let consumer = register(&mut registry, "workspace.consumer", &source);
            let imported =
                import(&registry, consumer).unwrap_or_else(|error| panic!("{source}: {error}"));
            assert_eq!(imported.items.len(), 2, "{source}");
            assert_eq!(imported.items[0].name.as_str(), "First", "{source}");
            assert_eq!(imported.items[0].source_module, first, "{source}");
            assert_eq!(imported.items[1].name.as_str(), "other", "{source}");
            assert_eq!(imported.items[1].source_module, second, "{source}");
            resolve_glob_reexports(&mut registry).unwrap();
            resolve_specific_reexport_kinds(&mut registry).unwrap();
            let info = registry.get(consumer).unwrap();
            for (name, expected) in [("First", first), ("other", second)] {
                let item = info
                    .exports
                    .get(&Text::from(name))
                    .unwrap_or_else(|| panic!("{source}: {name}"));
                assert_eq!(item.kind, ExportKind::Function, "{source}: {name}");
                assert_eq!(item.source_module, expected, "{source}: {name}");
            }
            assert_eq!(
                info.exports
                    .get(&Text::from("First"))
                    .unwrap()
                    .original_name
                    .as_deref(),
                Some("answer")
            );
            assert!(!info.exports.contains("one"), "{source}");
            assert!(!info.exports.contains("two"), "{source}");
        }
    }
}

#[test]
fn missing_and_private_nested_leaves_cannot_borrow_parent_or_child_homonyms() {
    for provider in [None, Some("fn answer() -> Int { 0 }")] {
        let mut registry = ModuleRegistry::new();
        register(&mut registry, "packages", "public fn answer() -> Int { 1 }");
        register(
            &mut registry,
            "workspace.consumer.packages.owned",
            "public fn answer() -> Int { 2 }",
        );
        if let Some(source) = provider {
            register(&mut registry, "packages.owned", source);
        }
        let consumer = register(
            &mut registry,
            "workspace.consumer",
            "public mount packages.{owned.answer as Alias};",
        );
        assert!(import(&registry, consumer).is_err());
        resolve_specific_reexport_kinds(&mut registry).unwrap();
        let info = registry.get(consumer).unwrap();
        let item = info.exports.get(&Text::from("Alias")).unwrap();
        assert_eq!(item.source_module, consumer);
        assert_ne!(item.kind, ExportKind::Function);
        assert_eq!(item.original_name.as_deref(), Some("answer"));
    }
}
