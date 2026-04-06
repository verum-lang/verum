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
// Tests for ast_format module
// Migrated from src/ast_format.rs per CLAUDE.md standards

use verum_lsp::ast_format::*;

#[test]
fn test_get_builtin_info() {
    assert!(get_builtin_info("Int").is_some());
    assert!(get_builtin_info("List").is_some());
    assert!(get_builtin_info("fn").is_some());
    assert!(get_builtin_info("unknown").is_none());
}
