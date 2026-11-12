//! Lazy writes must never alter another analysis's module generation or receipt.
use std::sync::Arc;
use verum_ast::FileId;
use verum_common::Text;
use verum_fast_parser::VerumParser;
use verum_modules::{
    ExportKind, ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
    resolve_specific_reexport_kinds,
};

fn module(id: u32, owner: &str, source: &str) -> ModuleInfo {
    let file = FileId::new(id);
    let id = ModuleId::new(id);
    let ast = VerumParser::new().parse_module_str(source, file).unwrap();
    let path = ModulePath::from_str(owner);
    let exports = extract_exports_from_module(&ast, id, &path).unwrap();
    let mut info = ModuleInfo::new(id, path, ast, file, source.into());
    info.exports = exports;
    info
}

#[test]
fn aliases_namespaces_exports_and_clear_stay_in_the_mutating_branch() {
    let mut base = ModuleRegistry::new();
    base.set_cog_name("app");
    let id = base.register(module(7, "app.unit", "public fn first() -> Int { 1 }"));
    let old = base.get(id).unwrap();
    let sibling = base.clone();
    let mut branch = sibling.clone();
    let additions = module(8, "other.extra", "public fn second() -> Int { 2 }");
    branch.register(additions.clone());
    assert_eq!(branch.add_exports_to_module(id, &additions.exports), 1);
    branch.register_path_alias("short", "app.unit");
    branch.set_cog_name("other");
    branch.register_external_cog("app");
    assert_eq!(
        branch.get_by_path_aliased("short").unwrap().exports.len(),
        2
    );
    for unchanged in [&base, &sibling] {
        assert_eq!(unchanged.cog_name(), Some("app"));
        assert_eq!(unchanged.get_by_path("unit").unwrap().exports.len(), 1);
        assert!(unchanged.get_by_path_aliased("short").is_none());
        assert!(unchanged.get_by_path("other.extra").is_none());
    }
    assert_eq!(old.exports.len(), 1);
    branch.clear();
    assert!(branch.is_empty());
    assert_eq!(
        branch.path_aliases().get(&Text::from("short")).unwrap(),
        "app.unit"
    );
    assert_eq!(base.len(), 1);
    base.register(module(9, "app.later", ""));
    assert!(sibling.get_by_path("later").is_none());
}

#[test]
fn completing_a_placeholder_preserves_the_other_snapshot_and_shared_allocator() {
    let mut base = ModuleRegistry::new();
    let id = base.register(module(3, "pending", ""));
    let mut branch = base.clone();
    assert_eq!(
        branch.register(module(40, "pending", "public fn answer() -> Int { 42 }")),
        id
    );
    assert!(base.get(id).unwrap().exports.is_empty());
    assert_eq!(branch.get(id).unwrap().exports.len(), 1);
    assert_eq!(base.allocate_id(), ModuleId::new(41));
    let detached = branch.deep_clone();
    assert_eq!(detached.allocate_id(), ModuleId::new(4));
    assert_eq!(branch.allocate_id(), ModuleId::new(42));
}

#[test]
fn reexport_resolution_invalidates_speculative_receipts_without_changing_the_base() {
    let mut base = ModuleRegistry::new();
    let provider = base.register(module(1, "provider", "public fn answer() -> Int { 42 }"));
    let facade = base.register(module(2, "facade", "public mount provider.{answer};"));
    let base = Arc::new(base);
    let name = Text::from("answer");
    let before = base.get(facade).unwrap();
    assert_eq!(before.exports.get(&name).unwrap().kind, ExportKind::Type);
    let (mut observed, receipt) = base.observe_reads();
    assert_eq!(resolve_specific_reexport_kinds(&mut observed).unwrap(), 1);
    let after = observed.get(facade).unwrap();
    assert_eq!(after.exports.get(&name).unwrap().kind, ExportKind::Function);
    assert_eq!(after.exports.get(&name).unwrap().source_module, provider);
    assert_eq!(base.get(facade).unwrap().exports, before.exports);
    assert!(
        !receipt.unchanged(&base),
        "a speculative export write cannot certify an unmodified analysis"
    );
}

#[test]
fn conflicting_speculative_exports_preserve_snapshots_and_invalidate_the_receipt() {
    let mut base = ModuleRegistry::new();
    let id = base.register(module(1, "provider", "public fn answer() -> Int { 42 }"));
    let base = Arc::new(base);
    let (mut observed, receipt) = base.observe_reads();
    let conflicting = module(2, "other", "public fn answer() -> Int { 0 }");
    let before = base.get(id).unwrap();
    assert_eq!(observed.add_exports_to_module(id, &conflicting.exports), 0);
    assert_eq!(observed.get(id).unwrap().exports, before.exports);
    assert_eq!(base.get(id).unwrap().exports, before.exports);
    assert!(!receipt.unchanged(&base));
}

#[test]
fn simultaneous_forks_keep_independent_catalogues() {
    let mut base = ModuleRegistry::new();
    base.register(module(1, "shared", "public fn answer() -> Int { 42 }"));
    std::thread::scope(|scope| {
        for index in 0..4 {
            let mut branch = base.clone();
            scope.spawn(move || {
                let owner = format!("worker{index}");
                branch.register(module(10 + index, &owner, ""));
                branch.register_path_alias("local", owner);
                assert_eq!(branch.len(), 2);
                assert_eq!(
                    branch.get_by_path_aliased("local").unwrap().id,
                    ModuleId::new(10 + index)
                );
            });
        }
    });
    assert_eq!(base.len(), 1);
    assert!(base.get_by_path_aliased("local").is_none());
    for index in 0..4 {
        assert!(base.get_by_path(&format!("worker{index}")).is_none());
    }
}
