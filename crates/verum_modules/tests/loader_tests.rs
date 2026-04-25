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
// Tests for loader module
// Migrated from src/loader.rs per CLAUDE.md standards

use tempfile::TempDir;
use verum_modules::loader::*;
use verum_modules::{ModuleError, ModuleId, ModulePath};

#[test]
fn test_load_module_not_found() {
    let temp_dir = TempDir::new().unwrap();
    let mut loader = ModuleLoader::new(temp_dir.path());

    let module_path = ModulePath::from_str("nonexistent");
    let result = loader.load_module(&module_path, ModuleId::new(1));

    assert!(result.is_err());
    match result {
        Err(ModuleError::ModuleNotFound { .. }) => {}
        _ => panic!("Expected ModuleNotFound error"),
    }
}

#[test]
fn test_path_collision_file_form_vs_directory_form() {
    // MOD-MED-5 / MOD-CRIT-PathCollision regression.
    // Two filesystem rules — `bar.vr` (Rule 2 file form) AND
    // `bar/mod.vr` (Rule 4 directory form) — both produce a
    // candidate for module `bar`. The loader must surface this
    // as ModuleError::PathCollision instead of silently picking
    // the first-found candidate.
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();

    // Rule 2: file form
    std::fs::write(root.join("bar.vr"), "public fn from_file() -> Int { 1 }").unwrap();
    // Rule 4: directory form
    std::fs::create_dir(root.join("bar")).unwrap();
    std::fs::write(
        root.join("bar").join("mod.vr"),
        "public fn from_dir() -> Int { 2 }",
    ).unwrap();

    let mut loader = ModuleLoader::new(root);
    let module_path = ModulePath::from_str("bar");
    let result = loader.load_module(&module_path, ModuleId::new(1));

    match result {
        Err(ModuleError::PathCollision { path, winning_path, losing_paths, .. }) => {
            assert_eq!(path.to_string(), "bar");
            // Both candidates should be enumerated — the diagnostic
            // must cite both files so the user can navigate to fix.
            let total = 1 + losing_paths.len();
            assert_eq!(total, 2, "both file-form and dir-form should be cited");
            // Winner is whichever the loader's `find_module_file_in_root`
            // emits first; the loser must be the OTHER file. We don't
            // pin which is winner — the diagnostic discipline is "cite
            // both", not "prefer file over dir".
            let losers: Vec<String> = losing_paths.iter()
                .map(|p| p.display().to_string())
                .collect();
            let winning_str = winning_path.display().to_string();
            let names: Vec<&str> = std::iter::once(winning_str.as_str())
                .chain(losers.iter().map(String::as_str))
                .collect();
            assert!(names.iter().any(|n| n.ends_with("bar.vr")));
            assert!(names.iter().any(|n| n.ends_with("mod.vr")));
        }
        Ok(_) => panic!("Expected PathCollision, got Ok — loader silently picked a winner"),
        Err(other) => panic!("Expected PathCollision, got {:?}", other),
    }
}

#[test]
fn test_path_collision_diagnostic_carries_E_MODULE_code() {
    // MOD-MED-4 / MOD-MED-5: every ModuleError variant must carry
    // its stable E_MODULE_* code, regardless of how it was
    // constructed. PathCollision must report E_MODULE_PATH_COLLISION.
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();
    std::fs::write(root.join("foo.vr"), "public fn x() -> Int { 1 }").unwrap();
    std::fs::create_dir(root.join("foo")).unwrap();
    std::fs::write(root.join("foo").join("mod.vr"), "public fn y() -> Int { 2 }").unwrap();

    let mut loader = ModuleLoader::new(root);
    let result = loader.load_module(&ModulePath::from_str("foo"), ModuleId::new(1));
    let err = result.expect_err("path collision should error out");
    assert_eq!(err.code(), "E_MODULE_PATH_COLLISION");
    let docs = err.docs_url();
    assert!(docs.contains("E_MODULE_PATH_COLLISION"));
    assert!(docs.starts_with("https://docs.verum-lang.org/errors/"));
}

#[test]
fn test_module_not_found_carries_E_MODULE_code() {
    // Sibling test to verify E_MODULE_NOT_FOUND code wiring.
    let temp_dir = TempDir::new().unwrap();
    let mut loader = ModuleLoader::new(temp_dir.path());
    let result = loader.load_module(&ModulePath::from_str("missing"), ModuleId::new(1));
    let err = result.expect_err("missing module should error");
    assert_eq!(err.code(), "E_MODULE_NOT_FOUND");
}

#[test]
fn test_no_collision_when_only_one_form_present() {
    // Negative test: with ONLY the file form, no collision is
    // surfaced — the loader returns the loaded module normally.
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();
    std::fs::write(root.join("solo.vr"), "public fn x() -> Int { 1 }").unwrap();

    let mut loader = ModuleLoader::new(root);
    let result = loader.load_module(&ModulePath::from_str("solo"), ModuleId::new(1));
    assert!(result.is_ok(), "single-form module must load cleanly");
}
