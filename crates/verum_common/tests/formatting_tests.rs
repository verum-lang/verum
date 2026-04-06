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
// Tests for formatting module
// Migrated from src/formatting.rs per CLAUDE.md standards

use verum_common::formatting::*;

#[test]
fn test_format_list() {
    assert_eq!(format_list(&["a", "b", "c"], ", "), "a, b, c");
    assert_eq!(format_list(&["x"], ", "), "x");
    assert_eq!(format_list::<&str>(&[], ", "), "");
}

#[test]
fn test_format_list_pretty() {
    assert_eq!(format_list_pretty(&["a"]), "a");
    assert_eq!(format_list_pretty(&["a", "b"]), "a and b");
    assert_eq!(format_list_pretty(&["a", "b", "c"]), "a, b, and c");
}

#[test]
fn test_format_cycle() {
    assert_eq!(format_cycle(&["a", "b", "c"]), "a -> b -> c -> a");
    assert_eq!(format_cycle::<&str>(&[]), "[]");
}

#[test]
fn test_truncate_with_ellipsis() {
    assert_eq!(truncate_with_ellipsis("hello world", 20), "hello world");
    assert_eq!(truncate_with_ellipsis("hello world", 8), "hello...");
    assert_eq!(truncate_with_ellipsis("hello", 8), "hello");
}

#[test]
fn test_list_formatter() {
    let formatter = ListFormatter::new()
        .separator(", ")
        .brackets(BracketStyle::Square);

    assert_eq!(formatter.format(&["a", "b", "c"]), "[a, b, c]");
}
