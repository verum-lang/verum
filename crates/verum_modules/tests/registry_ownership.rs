use std::sync::Arc;
use verum_ast::FileId;
use verum_fast_parser::VerumParser;
use verum_modules::{ModuleId, ModuleInfo, ModulePath, ModuleRegistry};

fn module(id: u32, owner: &str) -> ModuleInfo {
    let file = FileId::new(id);
    ModuleInfo::new(
        ModuleId::new(id),
        ModulePath::from_str(owner),
        VerumParser::new().parse_module_str("", file).unwrap(),
        file,
        "".into(),
    )
}

#[test]
fn changing_lookup_context_never_rewrites_declared_owners() {
    let mut registry = ModuleRegistry::new();
    registry.register(module(1, "portable.units"));
    registry.set_cog_name("portable");
    for spelling in ["portable.units", "units"] {
        assert_eq!(registry.get_by_path(spelling).unwrap().id, ModuleId::new(1));
    }
    registry.set_cog_name("application");
    assert_eq!(registry.get_by_path("portable.units").unwrap().id, ModuleId::new(1));
    assert!(registry.get_by_path("units").is_none());
    assert_eq!(registry.all_modules().next().unwrap().1.path.to_string(), "portable.units");
}

#[test]
fn exact_local_and_external_owners_remain_distinct_in_both_registration_orders() {
    for reverse in [false, true] {
        let mut registry = ModuleRegistry::new();
        registry.set_cog_name("application");
        registry.register_external_cog("math");
        let mut owners = [(1, "math.item"), (2, "application.math.item")];
        if reverse { owners.reverse(); }
        for (id, owner) in owners { registry.register(module(id, owner)); }
        assert_eq!(registry.len(), 2);
        assert_eq!(registry.get_by_path("math.item").unwrap().id, ModuleId::new(1));
        assert_eq!(registry.get_by_path("application.math.item").unwrap().id, ModuleId::new(2));
    }
}

#[test]
fn ambiguous_relative_requests_are_not_resolved_by_order_or_alias_fallback() {
    for reverse in [false, true] {
        let mut registry = ModuleRegistry::new();
        registry.set_cog_name("application");
        let mut owners = [(1, "group.item"), (2, "application.group.item")];
        if reverse { owners.reverse(); }
        for (id, owner) in owners { registry.register(module(id, owner)); }
        registry.register(module(3, "unrelated"));
        registry.register_path_alias("group.item", "unrelated");
        assert!(registry.get_by_path("group.item").is_none());
        assert!(registry.get_by_path_aliased("group.item").is_none());
        assert_eq!(registry.get_by_path("application.group.item").unwrap().id, ModuleId::new(2));
        registry.register_external_cog("group");
        assert_eq!(registry.get_by_path("group.item").unwrap().id, ModuleId::new(1));
    }
}

#[test]
fn exact_owner_dedup_and_explicit_aliases_preserve_identity() {
    let mut registry = ModuleRegistry::new();
    registry.register(module(1, "portable.units"));
    registry.set_cog_name("portable");
    assert_eq!(registry.register(module(8, "portable.units")), ModuleId::new(1));
    registry.register_path_alias("measure", "portable.units");
    assert_eq!(registry.get_by_path_aliased("measure").unwrap().id, ModuleId::new(1));
    assert_eq!(registry.len(), 1);
    let clone = registry.deep_clone();
    assert_eq!(clone.get_by_path("units").unwrap().id, ModuleId::new(1));
}

#[test]
fn namespace_policy_changes_invalidate_observations() {
    let mut registry = ModuleRegistry::new();
    registry.set_cog_name("application");
    registry.register(module(1, "group.item"));
    registry.register(module(2, "application.group.item"));
    let registry = Arc::new(registry);
    let (observed, reads) = registry.observe_reads();
    assert!(observed.get_by_path("group.item").is_none());
    let mut current = registry.deep_clone();
    current.register_external_cog("group");
    assert!(!reads.unchanged(&current));
    assert_eq!(current.get_by_path("group.item").unwrap().id, ModuleId::new(1));
}

#[test]
fn retaining_exact_owners_does_not_keep_a_colliding_relative_declaration() {
    let mut registry = ModuleRegistry::new();
    registry.set_cog_name("application");
    registry.register(module(1, "group.item"));
    registry.register(module(2, "application.group.item"));
    let keep = ["application.group.item".to_string()].into_iter().collect();
    assert_eq!(registry.retain_paths(&keep), (1, 1));
    assert_eq!(registry.get_by_path("group.item").unwrap().id, ModuleId::new(2));
    assert!(registry.get(ModuleId::new(1)).is_none());
}
