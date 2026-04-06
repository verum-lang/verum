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
// Tests for cache module
// Migrated from src/cache.rs per CLAUDE.md standards

use verum_cli::cache::*;

use std::fs;
use tempfile::tempdir;

#[test]
fn test_cache_creation() {
    let cache = BuildCache::new();
    assert_eq!(cache.version, CACHE_VERSION);
    assert!(cache.files.is_empty());
    assert!(cache.artifacts.is_empty());
}

#[test]
fn test_file_hash_detection() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("test.vr");
    fs::write(&file_path, "fn main() {}").unwrap();

    let mut cache = BuildCache::new();
    assert!(cache.is_file_changed(&file_path).unwrap());

    cache.update_file(&file_path).unwrap();
    assert!(!cache.is_file_changed(&file_path).unwrap());

    // Modify file
    fs::write(&file_path, "fn main() { println!(\"changed\"); }").unwrap();
    assert!(cache.is_file_changed(&file_path).unwrap());
}

#[test]
fn test_cache_persistence() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("test.vr");
    fs::write(&file_path, "fn main() {}").unwrap();

    let mut cache = BuildCache::new();
    cache.update_file(&file_path).unwrap();
    cache.save(dir.path()).unwrap();

    let loaded = BuildCache::load(dir.path()).unwrap();
    assert!(!loaded.is_file_changed(&file_path).unwrap());
}
