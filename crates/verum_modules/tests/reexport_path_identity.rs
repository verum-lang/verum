//! Imports and re-exports must resolve the same explicit path identity.
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
    registry.register(info);
    id
}

#[test]
fn absolute_and_explicit_relative_paths_agree_for_imports_and_reexports() {
    for (prefix, target) in [
        ("dependency", "dependency"),
        (".dependency", "package.group.facade.dependency"),
        (
            ".nested.dependency",
            "package.group.facade.nested.dependency",
        ),
        ("self.dependency", "package.group.dependency"),
        ("super.dependency", "package.group.dependency"),
        ("super.super.dependency", "package.dependency"),
        ("cog.dependency", "cog.dependency"),
    ] {
        for suffix in [".*", ".{answer}"] {
            let mut registry = ModuleRegistry::new();
            let expected = register(&mut registry, target, "public fn answer() -> Int { 42 }");
            // A competing nested declaration must not redirect an absolute mount.
            if target != "package.group.facade.dependency" {
                register(
                    &mut registry,
                    "package.group.facade.dependency",
                    "public fn answer() -> Int { 0 }",
                );
            }
            let facade = register(
                &mut registry,
                "package.group.facade",
                &format!("public mount {prefix}{suffix};"),
            );
            let info = registry.get(facade).unwrap();
            let ItemKind::Mount(decl) = &info.ast.items[0].kind else {
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
            let imported = ImportResolver::new()
                .resolve_import(decl, facade, &exports, &paths)
                .unwrap();
            assert_eq!(imported.items.len(), 1, "{prefix}{suffix}");
            assert_eq!(
                imported.items[0].source_module, expected,
                "{prefix}{suffix}"
            );

            resolve_glob_reexports(&mut registry).unwrap();
            resolve_specific_reexport_kinds(&mut registry).unwrap();
            let info = registry.get(facade).unwrap();
            let answer = info
                .exports
                .get(&Text::from("answer"))
                .expect("re-export available");
            assert_eq!(answer.kind, ExportKind::Function, "{prefix}{suffix}");
            assert_eq!(answer.source_module, expected, "{prefix}{suffix}");
        }
    }
}

#[test]
fn bare_reexport_never_falls_back_to_an_unrelated_child() {
    for suffix in [".*", ".{answer}"] {
        let mut registry = ModuleRegistry::new();
        register(
            &mut registry,
            "facade.dependency",
            "public fn answer() -> Int { 0 }",
        );
        let facade = register(
            &mut registry,
            "facade",
            &format!("public mount dependency{suffix};"),
        );
        resolve_glob_reexports(&mut registry).unwrap();
        resolve_specific_reexport_kinds(&mut registry).unwrap();
        let info = registry.get(facade).unwrap();
        // Named exports retain their unresolved declaration until the import
        // diagnostic pass. Neither spelling may borrow the child's function.
        if let Some(answer) = info.exports.get(&Text::from("answer")) {
            assert_ne!(answer.kind, ExportKind::Function, "{suffix}");
        }
    }
}
