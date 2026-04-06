#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
// Tests for resolver module
// Migrated from src/resolver.rs per CLAUDE.md standards

use verum_common::Maybe;
use verum_modules::resolver::*;
use verum_modules::{ModuleId, ModulePath};

#[test]
fn test_scope_basic() {
    let mut scope = Scope::new(ModuleId::new(1));
    let resolved = ResolvedName::new(
        ModuleId::new(1),
        ModulePath::from_str("cog.test"),
        NameKind::Function,
        "test_fn",
    );

    scope.add_binding("test_fn", resolved.clone());
    assert!(scope.contains_local("test_fn"));

    let looked_up = scope.lookup("test_fn");
    assert!(matches!(looked_up, Maybe::Some(_)));
}

#[test]
fn test_scope_child() {
    let mut parent = Scope::new(ModuleId::new(1));
    parent.add_binding(
        "parent_fn",
        ResolvedName::new(
            ModuleId::new(1),
            ModulePath::from_str("cog.parent"),
            NameKind::Function,
            "parent_fn",
        ),
    );

    let mut child = parent.child();
    child.add_binding(
        "child_fn",
        ResolvedName::new(
            ModuleId::new(1),
            ModulePath::from_str("cog.child"),
            NameKind::Function,
            "child_fn",
        ),
    );

    // Child can see both
    assert!(matches!(child.lookup("child_fn"), Maybe::Some(_)));
    assert!(matches!(child.lookup("parent_fn"), Maybe::Some(_)));

    // Parent can only see its own
    assert!(matches!(parent.lookup("parent_fn"), Maybe::Some(_)));
    assert!(matches!(parent.lookup("child_fn"), Maybe::None));
}

#[test]
fn test_name_resolver() {
    let mut resolver = NameResolver::new();
    let mod1 = ModuleId::new(1);

    let scope = resolver.create_scope(mod1);
    scope.add_binding(
        "test_fn",
        ResolvedName::new(
            mod1,
            ModulePath::from_str("cog.test"),
            NameKind::Function,
            "test_fn",
        ),
    );

    let resolved = resolver.resolve_name("test_fn", mod1).unwrap();
    assert_eq!(resolved.local_name.as_str(), "test_fn");
    assert_eq!(resolved.module_id, mod1);
}
