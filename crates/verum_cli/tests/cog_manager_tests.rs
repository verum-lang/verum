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
// Tests for cog_manager module
// Migrated from src/cog_manager.rs per CLAUDE.md standards

use verum_cli::cog_manager::*;

use tempfile::TempDir;

#[test]
fn test_cog_manager_creation() {
    let temp_dir = TempDir::new().unwrap();
    let manager = CogManager::new(temp_dir.path().to_path_buf());
    // Verify the package manager was created successfully
    assert!(manager.is_ok(), "CogManager creation should succeed");
}
