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
// Tests for update module
// Migrated from src/update.rs per CLAUDE.md standards

use verum_cli::update::*;

#[test]
fn test_compatibility_check() {
    assert!(check_compatibility("1.0.0", "1.1.0").is_ok());
    assert!(check_compatibility("1.0.0", "1.0.1").is_ok());
    assert!(check_compatibility("1.0.0", "2.0.0").is_err());
}
