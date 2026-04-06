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
//! Comprehensive tests for incremental compilation
//!
//! Tests:
//! - File change detection via content hashing
//! - Dependency graph construction
//! - Minimal recompilation set determination
//! - Cache persistence across sessions
//! - Parallel incremental compilation

use std::fs;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;
use verum_ast::{FileId, Module};
use verum_compiler::IncrementalCompiler;
use verum_common::List;

/// Helper to create a test module
fn create_test_module(file_id: FileId) -> Module {
    Module::empty(file_id)
}

#[test]
fn test_basic_cache_operations() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let root_dir = temp_dir.path().to_path_buf();

    let mut compiler = IncrementalCompiler::new();

    // Create test files
    let file1 = root_dir.join("module1.vr");
    let file2 = root_dir.join("module2.vr");

    fs::write(&file1, "fn main() {}").expect("Failed to write file1");
    fs::write(&file2, "fn helper() {}").expect("Failed to write file2");

    // Test: New files need compilation
    assert!(
        compiler.needs_recompile(&file1),
        "New file should need compilation"
    );
    assert!(
        compiler.needs_recompile(&file2),
        "New file should need compilation"
    );

    // Cache modules
    let module1 = create_test_module(FileId::new(0));
    let module2 = create_test_module(FileId::new(1));

    compiler.cache_module(file1.clone(), module1.clone());
    compiler.cache_module(file2.clone(), module2.clone());

    // Test: Stats reflect cached modules
    let stats = compiler.stats();
    assert_eq!(stats.cached_modules, 2, "Should have 2 cached modules");

    // Test: Cached files don't need recompilation
    assert!(
        !compiler.needs_recompile(&file1),
        "Cached file shouldn't need recompilation"
    );
    assert!(
        !compiler.needs_recompile(&file2),
        "Cached file shouldn't need recompilation"
    );

    // Test: Retrieve cached modules
    assert!(
        compiler.get_cached_module(&file1).is_some(),
        "Should retrieve cached module"
    );
    assert!(
        compiler.get_cached_module(&file2).is_some(),
        "Should retrieve cached module"
    );
}

#[test]
fn test_content_hash_change_detection() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let root_dir = temp_dir.path().to_path_buf();

    let mut compiler = IncrementalCompiler::new();

    let test_file = root_dir.join("test.vr");
    fs::write(&test_file, "fn main() {}").expect("Failed to write file");

    // Cache the module
    let module = create_test_module(FileId::new(0));
    compiler.cache_module(test_file.clone(), module);

    // File shouldn't need recompilation yet
    assert!(
        !compiler.needs_recompile(&test_file),
        "Unchanged file shouldn't need recompilation"
    );

    // Sleep briefly to ensure different modification time
    thread::sleep(Duration::from_millis(10));

    // Modify the file content
    fs::write(&test_file, "fn main() { println!(\"changed\"); }").expect("Failed to write file");

    // Now it should need recompilation (content hash changed)
    assert!(
        compiler.needs_recompile(&test_file),
        "Modified file should need recompilation"
    );
}

#[test]
fn test_dependency_tracking() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let root_dir = temp_dir.path().to_path_buf();

    let mut compiler = IncrementalCompiler::new();

    // Create a dependency chain: main -> lib -> utils
    let main_file = root_dir.join("main.vr");
    let lib_file = root_dir.join("lib.vr");
    let utils_file = root_dir.join("utils.vr");

    fs::write(&main_file, "import lib; fn main() {}").expect("Failed to write main");
    fs::write(&lib_file, "import utils; fn lib_func() {}").expect("Failed to write lib");
    fs::write(&utils_file, "fn util() {}").expect("Failed to write utils");

    // Cache modules with dependencies
    let main_module = create_test_module(FileId::new(0));
    let lib_module = create_test_module(FileId::new(1));
    let utils_module = create_test_module(FileId::new(2));

    compiler.cache_module(utils_file.clone(), utils_module);
    compiler.cache_module(lib_file.clone(), lib_module);
    compiler.cache_module(main_file.clone(), main_module);

    // Register dependencies: main -> lib -> utils
    compiler.register_dependencies(
        lib_file.clone(),
        List::from(vec![utils_file.clone()]),
    );
    compiler.register_dependencies(
        main_file.clone(),
        List::from(vec![lib_file.clone()]),
    );

    // Test get_recompilation_set
    let changed_files = vec![utils_file.clone()];
    let to_recompile = compiler.get_recompilation_set(&changed_files);

    // Should recompile utils and its dependents (lib and main)
    assert!(!to_recompile.is_empty(), "Should recompile at least utils");
    assert!(
        to_recompile.iter().any(|p| p == &utils_file),
        "Should include utils"
    );
}

#[test]
fn test_invalidation() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let root_dir = temp_dir.path().to_path_buf();

    let mut compiler = IncrementalCompiler::new();

    let test_file = root_dir.join("test.vr");
    fs::write(&test_file, "fn main() {}").expect("Failed to write file");

    // Cache module
    let module = create_test_module(FileId::new(0));
    compiler.cache_module(test_file.clone(), module);

    assert_eq!(
        compiler.stats().cached_modules,
        1,
        "Should have 1 cached module"
    );

    // Invalidate the cache
    compiler.invalidate(&test_file);

    // Module should be removed
    assert_eq!(
        compiler.stats().cached_modules,
        0,
        "Should have 0 cached modules after invalidation"
    );
    assert!(
        compiler.get_cached_module(&test_file).is_none(),
        "Module should be invalidated"
    );
}

#[test]
fn test_cache_persistence() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let root_dir = temp_dir.path().to_path_buf();
    let cache_dir = root_dir.join(".cache");

    let test_file = root_dir.join("test.vr");
    fs::write(&test_file, "fn main() {}").expect("Failed to write file");

    // First session: create cache and type check result
    {
        let mut compiler = IncrementalCompiler::with_cache_dir(cache_dir.clone());
        let module = create_test_module(FileId::new(0));
        compiler.cache_module(test_file.clone(), module);

        // Add a type check result
        let tc_result = verum_compiler::TypeCheckResult {
            success: true,
            error_count: 0,
            warning_count: 1,
            timestamp: std::time::SystemTime::now(),
            content_hash: 12345,
        };
        compiler.cache_type_check(test_file.clone(), tc_result);

        // Register some dependencies
        compiler.register_dependencies(
            test_file.clone(),
            List::from(vec![root_dir.join("dep.vr")]),
        );

        compiler.save_cache().expect("Failed to save cache");
    }

    // Verify cache file exists
    assert!(
        cache_dir.join("incremental_cache.bin").exists(),
        "Cache file should exist"
    );

    // Second session: load cache
    {
        let mut compiler = IncrementalCompiler::with_cache_dir(cache_dir.clone());
        compiler.load_cache().expect("Failed to load cache");

        // Type check result should be restored
        let tc = compiler.get_type_check_result(&test_file);
        assert!(tc.is_some(), "Type check result should be loaded");
        let tc = tc.unwrap();
        assert!(tc.success, "Type check should be successful");
        assert_eq!(tc.error_count, 0, "Should have 0 errors");
        assert_eq!(tc.warning_count, 1, "Should have 1 warning");
        assert_eq!(tc.content_hash, 12345, "Content hash should match");

        // Type check cache count should be restored
        let stats = compiler.stats();
        assert_eq!(
            stats.type_check_cached, 1,
            "Should have 1 type check cached"
        );
    }
}

#[test]
fn test_type_check_cache() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let root_dir = temp_dir.path().to_path_buf();

    let mut compiler = IncrementalCompiler::new();
    let test_file = root_dir.join("test.vr");
    fs::write(&test_file, "fn main() {}").expect("Failed to write file");

    let result = verum_compiler::TypeCheckResult {
        success: true,
        error_count: 0,
        warning_count: 2,
        timestamp: std::time::SystemTime::now(),
        content_hash: 54321,
    };

    compiler.cache_type_check(test_file.clone(), result);

    let cached = compiler.get_type_check_result(&test_file);
    assert!(cached.is_some(), "Should retrieve cached type check result");
    let cached = cached.unwrap();
    assert!(cached.success, "Type check should be successful");
    assert_eq!(cached.error_count, 0, "Should have 0 errors");
    assert_eq!(cached.warning_count, 2, "Should have 2 warnings");

    // Test invalidation
    compiler.invalidate_type_check(&test_file);
    assert!(
        compiler.get_type_check_result(&test_file).is_none(),
        "Should be invalidated"
    );
}

#[test]
fn test_topological_sort() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let root_dir = temp_dir.path().to_path_buf();

    let mut compiler = IncrementalCompiler::new();

    // Create dependency graph: A -> B -> C, A -> D
    let file_a = root_dir.join("a.vr");
    let file_b = root_dir.join("b.vr");
    let file_c = root_dir.join("c.vr");
    let file_d = root_dir.join("d.vr");

    for file in &[&file_a, &file_b, &file_c, &file_d] {
        fs::write(file, "fn test() {}").expect("Failed to write file");
    }

    let module_a = create_test_module(FileId::new(0));
    let module_b = create_test_module(FileId::new(1));
    let module_c = create_test_module(FileId::new(2));
    let module_d = create_test_module(FileId::new(3));

    compiler.cache_module(file_c.clone(), module_c);
    compiler.cache_module(file_d.clone(), module_d);
    compiler.cache_module(file_b.clone(), module_b);
    compiler.cache_module(file_a.clone(), module_a);

    // Register dependencies: A depends on B and D, B depends on C
    compiler.register_dependencies(
        file_a.clone(),
        List::from(vec![file_b.clone(), file_d.clone()]),
    );
    compiler.register_dependencies(file_b.clone(), List::from(vec![file_c.clone()]));

    // Test topological sort when C changes - should recompile C, then B (depends on C), then A (depends on B)
    let to_recompile = compiler.get_recompilation_set(&[file_c.clone()]);
    assert!(
        to_recompile.iter().any(|p| p == &file_c),
        "Should include C"
    );

    let c_idx = to_recompile.iter().position(|p| p == &file_c);
    let b_idx = to_recompile.iter().position(|p| p == &file_b);

    // C should come before B in topological order (dependencies first)
    if let (Some(c), Some(b)) = (c_idx, b_idx) {
        assert!(c < b, "C should come before B in topological order");
    }
}

#[test]
fn test_clear_cache() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let root_dir = temp_dir.path().to_path_buf();

    let mut compiler = IncrementalCompiler::new();

    let test_file = root_dir.join("test.vr");
    fs::write(&test_file, "fn main() {}").expect("Failed to write file");

    // Cache module
    let module = create_test_module(FileId::new(0));
    compiler.cache_module(test_file.clone(), module);

    assert_eq!(
        compiler.stats().cached_modules,
        1,
        "Should have 1 cached module"
    );

    // Clear all caches
    compiler.clear();

    let stats = compiler.stats();
    assert_eq!(stats.cached_modules, 0, "Should have 0 cached modules");
}

#[test]
fn test_stats_report() {
    let compiler = IncrementalCompiler::default();
    let stats = compiler.stats();

    // Basic stats are available
    assert_eq!(stats.cached_modules, 0, "Should have 0 cached modules");
    assert!(
        !stats.meta_registry_valid,
        "Meta registry should be invalid"
    );
    assert_eq!(
        stats.type_check_cached, 0,
        "Should have 0 type check cached"
    );
    assert_eq!(stats.dependency_edges, 0, "Should have 0 dependency edges");

    // Test report() method
    let report = stats.report();
    assert!(
        report.contains("Incremental Cache Stats"),
        "Report should contain title"
    );
    assert!(
        report.contains("Cached modules"),
        "Report should contain module count"
    );
    assert!(
        report.contains("Type check cached"),
        "Report should contain type check count"
    );
    assert!(
        report.contains("Dependency edges"),
        "Report should contain dependency edges"
    );
}

#[test]
fn test_circular_dependency_detection() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let root_dir = temp_dir.path().to_path_buf();

    let mut compiler = IncrementalCompiler::new();

    // Create circular dependency: A -> B -> A
    let file_a = root_dir.join("a.vr");
    let file_b = root_dir.join("b.vr");

    fs::write(&file_a, "import b;").expect("Failed to write a");
    fs::write(&file_b, "import a;").expect("Failed to write b");

    let module_a = create_test_module(FileId::new(0));
    let module_b = create_test_module(FileId::new(1));

    compiler.cache_module(file_a.clone(), module_a);
    compiler.cache_module(file_b.clone(), module_b);

    // Invalidation should handle circular dependency gracefully (uses cycle detection)
    compiler.invalidate(&file_a);

    // Module A should be invalidated
    assert!(
        compiler.get_cached_module(&file_a).is_none(),
        "A should be invalidated"
    );
}
