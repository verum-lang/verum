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

use std::path::PathBuf;
use std::time::UNIX_EPOCH;
use verum_ast::{FileId, Module as AstModule};
use verum_common::{Maybe, Text};
use verum_modules::cache::*;
use verum_modules::{ModuleId, ModuleInfo, ModulePath};

fn create_test_module() -> ModuleInfo {
    ModuleInfo::new(
        ModuleId::new(1),
        ModulePath::from_str("test"),
        AstModule::empty(FileId::new(0)),
        FileId::new(0),
        Text::from("test source"),
    )
}

#[test]
fn test_cache_insert_and_get() {
    let cache = ModuleCache::new();
    let module = create_test_module();
    let file_path = PathBuf::from("test.vr");
    let entry = ModuleCacheEntry::new(module, UNIX_EPOCH, 12345);

    cache.insert(file_path.clone(), entry);

    let retrieved = cache.get_by_path(&file_path);
    assert!(matches!(retrieved, Maybe::Some(_)));

    let retrieved_by_id = cache.get_by_id(ModuleId::new(1));
    assert!(matches!(retrieved_by_id, Maybe::Some(_)));
}

#[test]
fn test_cache_validity() {
    let cache = ModuleCache::new();
    let module = create_test_module();
    let file_path = PathBuf::from("test.vr");
    let mtime = UNIX_EPOCH;
    let hash = 12345;

    let entry = ModuleCacheEntry::new(module, mtime, hash);
    cache.insert(file_path.clone(), entry);

    assert!(cache.is_valid(&file_path, mtime, hash));
    assert!(!cache.is_valid(&file_path, mtime, 99999)); // Different hash
}

#[test]
fn test_cache_stats() {
    let cache = ModuleCache::new();
    let file_path = PathBuf::from("test.vr");

    // Miss
    let _ = cache.get_by_path(&file_path);

    // Insert and hit
    let module = create_test_module();
    let entry = ModuleCacheEntry::new(module, UNIX_EPOCH, 12345);
    cache.insert(file_path.clone(), entry);
    let _ = cache.get_by_path(&file_path);

    let stats = cache.stats();
    assert_eq!(stats.hits, 1);
    assert_eq!(stats.misses, 1);
    assert_eq!(stats.hit_rate(), 50.0);
}

#[test]
fn test_cache_remove() {
    let cache = ModuleCache::new();
    let module = create_test_module();
    let file_path = PathBuf::from("test.vr");
    let entry = ModuleCacheEntry::new(module, UNIX_EPOCH, 12345);

    cache.insert(file_path.clone(), entry);
    assert_eq!(cache.len(), 1);

    cache.remove(&file_path);
    assert_eq!(cache.len(), 0);
}
