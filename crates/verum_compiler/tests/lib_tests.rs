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
// Migrated from src/lib.rs to comply with CLAUDE.md test organization.

use verum_compiler::{BUILD_INFO, VERSION};

#[test]
fn test_version() {
    assert!(!VERSION.is_empty());
    assert!(!BUILD_INFO.is_empty());
}
