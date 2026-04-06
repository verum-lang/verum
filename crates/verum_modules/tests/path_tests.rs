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
// Tests for path module
// Migrated from src/path.rs per CLAUDE.md standards

use verum_modules::path::*;

#[test]
fn test_module_path_creation() {
    let path = ModulePath::from_str("std.collections.List");
    assert_eq!(path.segments().len(), 3);
    assert_eq!(path.name().unwrap().as_str(), "List");
}

#[test]
fn test_module_path_parent() {
    let path = ModulePath::from_str("std.collections.List");
    let parent = path.parent().unwrap();
    assert_eq!(parent.to_string(), "std.collections");
}

#[test]
fn test_module_path_join() {
    let path = ModulePath::from_str("std.collections");
    let joined = path.join("List");
    assert_eq!(joined.to_string(), "std.collections.List");
}

#[test]
fn test_module_path_resolve_super() {
    let base = ModulePath::from_str("cog.parser.ast");
    let relative = ModulePath::from_str("super.lexer");
    let resolved = base.resolve(&relative).unwrap();
    assert_eq!(resolved.to_string(), "cog.parser.lexer");
}

#[test]
fn test_module_path_is_descendant() {
    let ancestor = ModulePath::from_str("std.collections");
    let descendant = ModulePath::from_str("std.collections.List");
    assert!(descendant.is_descendant_of(&ancestor));
    assert!(!ancestor.is_descendant_of(&descendant));
}
