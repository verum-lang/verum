use std::sync::Arc;
use verum_ast::FileId;
use verum_fast_parser::VerumParser;
use verum_modules::{ModuleId, ModuleInfo, ModulePath, ModuleRegistry};

fn module(id: u32, name: &str, source: &str) -> ModuleInfo {
    let file = FileId::new(id);
    ModuleInfo::new(
        ModuleId::new(id),
        ModulePath::from_str(name),
        VerumParser::new().parse_module_str(source, file).unwrap(),
        file,
        source.into(),
    )
}
fn registry() -> Arc<ModuleRegistry> {
    let mut registry = ModuleRegistry::new();
    registry.register(module(7, "dependency", "public fn value() -> Int { 1 }"));
    Arc::new(registry)
}

#[test]
fn observations_follow_exact_reads_and_failed_queries_across_clones() {
    let base = registry();
    let (observed, reads) = base.observe_reads();
    let clone = observed.clone();
    assert!(clone.get_by_path("dependency").is_some());
    assert!(clone.get_by_path("missing").is_none());
    let mut changed = base.deep_clone();
    changed.register(module(8, "unrelated", ""));
    assert!(reads.unchanged(&changed));
    changed.register(module(9, "missing", ""));
    assert!(!reads.unchanged(&changed));
    let mut body = ModuleRegistry::new();
    body.register(module(7, "dependency", "public fn value() -> Int { 2 }"));
    assert!(!reads.unchanged(&body));
    assert!(
        reads.unchanged(&observed),
        "comparing a tracked registry must not deadlock"
    );
}

#[test]
fn aliases_id_reads_and_catalog_queries_are_dependencies() {
    let mut base = registry().deep_clone();
    base.register_path_alias("public", "dependency");
    let base = Arc::new(base);
    let (observed, reads) = base.observe_reads();
    assert!(observed.get_by_path_aliased("public").is_some());
    assert!(observed.contains(ModuleId::new(7)));
    assert!(!observed.is_empty());
    assert!(reads.unchanged(&base));
    let mut changed = base.deep_clone();
    changed.register_path_alias("public", "absent");
    assert!(!reads.unchanged(&changed));
    let (catalog, catalog_reads) = base.observe_reads();
    assert_eq!(catalog.all_modules().count(), 1);
    let mut added = base.deep_clone();
    added.register(module(8, "new", ""));
    assert!(reads.unchanged(&added), "nonempty remains nonempty");
    assert!(!catalog_reads.unchanged(&added));
}

#[test]
fn speculative_registry_writes_cannot_produce_reusable_receipts() {
    let base = registry();
    let (observed, reads) = base.observe_reads();
    let mut clone = observed.deep_clone();
    clone.register(module(8, "local", ""));
    assert!(!reads.unchanged(&base));
    assert!(base.get_by_path("local").is_none());
}

#[test]
fn preserved_module_ids_cannot_collide_with_new_allocations() {
    let mut registry = ModuleRegistry::new();
    registry.reserve_id(ModuleId::new(80));
    assert_eq!(registry.allocate_id(), ModuleId::new(81));
    registry.register(module(100, "loaded", ""));
    assert_eq!(registry.allocate_id(), ModuleId::new(101));
}

#[test]
fn successful_rebase_releases_old_generation_but_failure_preserves_evidence() {
    let base = registry();
    let weak = Arc::downgrade(&base);
    let (observed, mut reads) = base.observe_reads();
    observed.get_by_path("dependency");
    let current = Arc::new(base.deep_clone());
    drop(observed);
    drop(base);
    assert!(weak.upgrade().is_some());
    assert!(!reads.rebase_if_unchanged(Arc::new(ModuleRegistry::new())));
    assert!(weak.upgrade().is_some());
    assert!(reads.rebase_if_unchanged(current));
    assert!(weak.upgrade().is_none());
}
