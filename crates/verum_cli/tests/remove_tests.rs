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
// Tests for remove module
// Migrated from src/remove.rs per CLAUDE.md standards

use verum_cli::Text;
use verum_cli::remove::*;

#[test]
fn test_remove_options() {
    let options = RemoveOptions {
        name: Text::from("test-dep"),
        dev: false,
        build: false,
    };
    assert_eq!(options.name, "test-dep");
}
