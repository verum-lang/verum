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
// Tests for cache_manager module
// Migrated from src/cache_manager.rs per CLAUDE.md standards

use verum_cli::cache_manager::*;

use tempfile::TempDir;

#[test]
fn test_cache_manager_creation() {
    let temp_dir = TempDir::new().unwrap();
    let manager = CacheManager::new(temp_dir.path().to_path_buf());
    assert!(manager.is_ok());
}

#[test]
fn test_is_cached() {
    let temp_dir = TempDir::new().unwrap();
    let manager = CacheManager::new(temp_dir.path().to_path_buf()).unwrap();

    assert!(!manager.is_cached("test_pkg", "1.0.0"));
}

#[test]
fn test_stats() {
    let temp_dir = TempDir::new().unwrap();
    let manager = CacheManager::new(temp_dir.path().to_path_buf()).unwrap();

    let stats = manager.stats().unwrap();
    assert_eq!(stats.total_packages, 0);
    assert_eq!(stats.total_versions, 0);
}
