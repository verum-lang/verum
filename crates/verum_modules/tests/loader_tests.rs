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
