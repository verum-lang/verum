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
// Tests for config module
// Migrated from src/config.rs per CLAUDE.md standards

use verum_cli::config::*;

#[test]
fn test_valid_cog_names() {
    assert!(is_valid_cog_name("my_package"));
    assert!(is_valid_cog_name("my-package"));
    assert!(is_valid_cog_name("package123"));
    assert!(!is_valid_cog_name(""));
    assert!(!is_valid_cog_name("-invalid"));
    assert!(!is_valid_cog_name("invalid-"));
    assert!(!is_valid_cog_name("invalid package"));
}
