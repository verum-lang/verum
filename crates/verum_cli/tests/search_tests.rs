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
// Tests for search module
// Migrated from src/search.rs per CLAUDE.md standards

use verum_cli::search::*;

#[test]
fn test_search_options_default() {
    let options = SearchOptions::default();
    assert_eq!(options.limit, 20);
}
