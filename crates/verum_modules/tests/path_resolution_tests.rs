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
// Tests for centralized module path resolution
// Tests for resolve_import function in path.rs

use verum_modules::{ModulePath, resolve_import};

#[test]
fn test_resolve_super_basic() {
    let current = ModulePath::from_str("services.package_service");

    // super -> parent
    let resolved = resolve_import("super", &current).unwrap();
    assert_eq!(resolved.to_string(), "services");
}

#[test]
fn test_resolve_super_sibling() {
    let current = ModulePath::from_str("services.package_service");

    // super.checksum_service -> sibling
    let resolved = resolve_import("super.checksum_service", &current).unwrap();
    assert_eq!(resolved.to_string(), "services.checksum_service");
}

#[test]
fn test_resolve_super_super() {
    let current = ModulePath::from_str("services.package_service");

    // super.super.domain -> uncle
    let resolved = resolve_import("super.super.domain", &current).unwrap();
    assert_eq!(resolved.to_string(), "domain");
}

#[test]
fn test_resolve_cog_absolute() {
    let current = ModulePath::from_str("services.package_service");

    // cog.domain.Package -> absolute path from cog root
    let resolved = resolve_import("cog.domain.Package", &current).unwrap();
    assert_eq!(resolved.to_string(), "domain.Package");
}

#[test]
fn test_resolve_self_goes_to_parent() {
    let current = ModulePath::from_str("services.package_service");

    // self -> goes to parent (sibling access base)
    // From services.package_service, self -> services
    let resolved = resolve_import("self", &current).unwrap();
    assert_eq!(resolved.to_string(), "services");
}

#[test]
fn test_resolve_self_with_sibling() {
    let current = ModulePath::from_str("services.package_service");

    // self.utils -> sibling module
    // From services.package_service, self.utils -> services.utils
    let resolved = resolve_import("self.utils", &current).unwrap();
    assert_eq!(resolved.to_string(), "services.utils");
}

#[test]
fn test_resolve_self_version_from_package() {
    // Key test case: domain/package.vr importing from domain/version.vr
    let current = ModulePath::from_str("domain.package");

    // self.version.SemVer -> domain.version.SemVer
    let resolved = resolve_import("self.version.SemVer", &current).unwrap();
    assert_eq!(resolved.to_string(), "domain.version.SemVer");
}

#[test]
fn test_resolve_absolute_name() {
    let current = ModulePath::from_str("services.package_service");

    // A bare name without self/super/cog prefix is an absolute path from root
    let resolved = resolve_import("utils", &current).unwrap();
    assert_eq!(resolved.to_string(), "utils");
}

#[test]
fn test_resolve_self_child() {
    let current = ModulePath::from_str("services.package_service");

    // self.utils -> sibling: services.utils
    let resolved = resolve_import("self.utils", &current).unwrap();
    assert_eq!(resolved.to_string(), "services.utils");
}

#[test]
fn test_resolve_super_from_root_fails() {
    let current = ModulePath::root();

    // super from root should fail
    let result = resolve_import("super", &current);
    assert!(result.is_err());
}

#[test]
fn test_resolve_super_super_from_single_segment_fails() {
    let current = ModulePath::from_str("services");

    // super.super from single segment should fail (goes above root)
    let result = resolve_import("super.super", &current);
    assert!(result.is_err());
}

#[test]
fn test_resolve_cog_in_middle_fails() {
    let current = ModulePath::from_str("services.package_service");

    // cog in middle of path should fail
    let result = resolve_import("domain.cog.utils", &current);
    assert!(result.is_err());
}

#[test]
fn test_resolve_self_in_middle_fails() {
    let current = ModulePath::from_str("services.package_service");

    // self in middle of path should fail
    let result = resolve_import("domain.self.utils", &current);
    assert!(result.is_err());
}

#[test]
fn test_resolve_deeply_nested() {
    let current = ModulePath::from_str("app.services.package.internal.helpers");

    // super.super.super -> app.services
    let resolved = resolve_import("super.super.super", &current).unwrap();
    assert_eq!(resolved.to_string(), "app.services");
}

#[test]
fn test_resolve_cog_from_deep_module() {
    let current = ModulePath::from_str("app.services.package.internal.helpers");

    // cog.domain.models.User -> domain.models.User (absolute from cog root)
    let resolved = resolve_import("cog.domain.models.User", &current).unwrap();
    assert_eq!(resolved.to_string(), "domain.models.User");
}

#[test]
fn test_resolve_super_then_descend() {
    let current = ModulePath::from_str("services.package_service.internal");

    // super.super.checksum_service.utils -> services.checksum_service.utils
    let resolved = resolve_import("super.super.checksum_service.utils", &current).unwrap();
    assert_eq!(resolved.to_string(), "services.checksum_service.utils");
}
