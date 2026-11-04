//! The action files declare their actual sibling dependencies explicitly.
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};
use verum_ast::{FileId, ItemKind, Visitor};
use verum_common::Map;
use verum_fast_parser::VerumParser;
use verum_modules::{
    ImportResolver, ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
    resolve_specific_reexport_kinds,
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

fn imports(registry: &ModuleRegistry, id: ModuleId) -> Result<BTreeMap<String, ModuleId>, String> {
    let exports: Map<_, _> = registry
        .all_modules()
        .map(|(id, m)| (*id, m.exports.clone()))
        .collect();
    let paths: Map<_, _> = registry
        .all_modules()
        .map(|(id, m)| (*id, m.path.clone()))
        .collect();
    let mut bindings = BTreeMap::new();
    for item in &registry.get(id).unwrap().ast.items {
        if let ItemKind::Mount(decl) = &item.kind {
            let resolved = ImportResolver::new()
                .resolve_import(decl, id, &exports, &paths)
                .map_err(|error| error.to_string())?;
            for item in resolved.items {
                if let Some(previous) = bindings.insert(item.name.to_string(), item.source_module) {
                    assert_eq!(previous, item.source_module, "conflicting explicit import");
                }
            }
        }
    }
    Ok(bindings)
}

#[derive(Default)]
struct SiblingUses {
    names: BTreeSet<String>,
    variants: BTreeSet<String>,
}
impl SiblingUses {
    fn path(&mut self, path: &verum_ast::Path) {
        if let Some(name) = path.as_ident() {
            self.names.insert(name.name.to_string());
        }
    }
}
impl Visitor for SiblingUses {
    fn visit_type(&mut self, ty: &verum_ast::Type) {
        if let verum_ast::ty::TypeKind::Path(path) = &ty.kind {
            self.path(path);
        }
        verum_ast::visitor::walk_type(self, ty);
    }
    fn visit_expr(&mut self, expr: &verum_ast::Expr) {
        match &expr.kind {
            verum_ast::ExprKind::Call { func, .. } => {
                if let verum_ast::ExprKind::Path(path) = &func.kind {
                    self.path(path);
                }
            }
            verum_ast::ExprKind::Record { path, .. } => self.path(path),
            _ => {}
        }
        verum_ast::visitor::walk_expr(self, expr);
    }
    fn visit_pattern(&mut self, pattern: &verum_ast::Pattern) {
        match &pattern.kind {
            verum_ast::PatternKind::Ident { name, .. }
                if self.variants.contains(name.name.as_str()) =>
            {
                self.names.insert(name.name.to_string());
            }
            verum_ast::PatternKind::Variant { path, .. } => self.path(path),
            _ => {}
        }
        verum_ast::visitor::walk_pattern(self, pattern);
    }
}

fn check_action_source(leaf: &str) {
    let core = std::env::var_os("VERUM_TEST_ACTION_CORE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../core"));
    let mut registry = ModuleRegistry::new();
    let mut ids = BTreeMap::new();
    let mut variants = BTreeSet::new();
    for name in [
        "primitives",
        "articulation",
        "enactments",
        "gauge",
        "verify",
    ] {
        let source = std::fs::read_to_string(core.join(format!("action/{name}.vr"))).unwrap();
        let id = register(&mut registry, &format!("core.action.{name}"), &source);
        for item in &registry.get(id).unwrap().ast.items {
            if let ItemKind::Type(decl) = &item.kind {
                if let verum_ast::TypeDeclBody::Variant(items) = &decl.body {
                    variants.extend(items.iter().map(|item| item.name.name.to_string()));
                }
            }
        }
        ids.insert(name, id);
    }
    let protocols = std::fs::read_to_string(core.join("base/protocols.vr")).unwrap();
    register(&mut registry, "core.base.protocols", &protocols);
    // Preserve the real facade: module-only reexports do not publish the
    // modules' children as direct symbols in core.action.
    let facade = std::fs::read_to_string(core.join("action/mod.vr")).unwrap();
    register(&mut registry, "core.action", &facade);
    let id = ids[leaf];
    let bindings = imports(&registry, id).unwrap_or_else(|error| panic!("{leaf}: {error}"));
    let mut uses = SiblingUses {
        names: BTreeSet::new(),
        variants,
    };
    for item in &registry.get(id).unwrap().ast.items {
        uses.visit_item(item);
    }
    let mut checked = 0;
    for name in uses.names {
        if registry.get(id).unwrap().exports.contains(&name) {
            continue;
        }
        let providers: Vec<_> = ids
            .values()
            .filter(|provider| {
                **provider != id && registry.get(**provider).unwrap().exports.contains(&name)
            })
            .collect();
        if providers.is_empty() {
            continue;
        } // Prelude and non-action dependencies are separate contracts.
        assert_eq!(providers.len(), 1, "ambiguous action declaration {name}");
        assert_eq!(
            bindings.get(&name),
            Some(providers[0]),
            "{leaf} uses sibling {name} without its declaring import"
        );
        checked += 1;
    }
    assert!(
        checked > 0,
        "the source gate must observe actual sibling references"
    );
}

#[test]
fn articulation_mounts_its_declaring_sources() {
    check_action_source("articulation");
}
#[test]
fn enactments_mounts_its_declaring_sources() {
    check_action_source("enactments");
}
#[test]
fn gauge_mounts_its_declaring_sources() {
    check_action_source("gauge");
}
#[test]
fn verify_mounts_its_declaring_sources() {
    check_action_source("verify");
}

#[test]
fn module_reexport_does_not_flatten_child_symbols() {
    let mut registry = ModuleRegistry::new();
    register(
        &mut registry,
        "fixture.models",
        "public type PacketFixture is { value: Int };",
    );
    let facade = register(&mut registry, "fixture.api", "public mount fixture.models;");
    resolve_specific_reexport_kinds(&mut registry).unwrap();
    let exports = &registry.get(facade).unwrap().exports;
    assert!(!exports.contains("PacketFixture"));
    assert!(exports.contains("models"));
    let client = register(
        &mut registry,
        "fixture.client",
        "mount fixture.api.PacketFixture;",
    );
    assert!(imports(&registry, client).is_err());
}

#[test]
fn explicit_symbol_reexport_is_importable() {
    let mut registry = ModuleRegistry::new();
    register(
        &mut registry,
        "fixture.models",
        "public type PacketFixture is { value: Int };",
    );
    let facade = register(
        &mut registry,
        "fixture.api",
        "public mount fixture.models.PacketFixture;",
    );
    resolve_specific_reexport_kinds(&mut registry).unwrap();
    let client = register(
        &mut registry,
        "fixture.client",
        "mount fixture.api.PacketFixture;",
    );
    // ImportResolver records the module named by the import; exact type
    // identity through the reexport is checked by the VBC wire test.
    assert_eq!(imports(&registry, client).unwrap()["PacketFixture"], facade);
}
