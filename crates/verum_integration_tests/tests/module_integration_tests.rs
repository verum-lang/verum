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
#![cfg(test)]

// Cross-Crate Integration Tests
//
// Verifies that all crates work together correctly and data flows
// properly between modules.

use verum_diagnostics::DiagnosticBuilder;
use verum_common::{List, Map, Set, Text};

// ============================================================================
// Standard Library Integration
// ============================================================================

#[test]
fn test_stdlib_list_operations() {
    let mut list = List::new();
    list.push(1);
    list.push(2);
    list.push(3);

    assert_eq!(list.len(), 3);
    assert_eq!(list[0], 1);
}

#[test]
fn test_stdlib_text_operations() {
    let text = Text::from("Hello, Verum!");

    assert_eq!(text.len(), 13);
    assert!(text.starts_with("Hello"));
    assert!(text.ends_with("Verum!"));
    assert!(text.contains("Verum"));
}

#[test]
fn test_stdlib_map_operations() {
    let mut map = Map::new();

    map.insert(Text::from("x"), 10);
    map.insert(Text::from("y"), 20);

    let x_key = Text::from("x");
    let y_key = Text::from("y");
    let z_key = Text::from("z");

    assert_eq!(map.get(&x_key), Some(&10));
    assert_eq!(map.get(&y_key), Some(&20));
    assert_eq!(map.get(&z_key), None);
}

#[test]
fn test_stdlib_set_operations() {
    let mut set = Set::new();

    set.insert(1);
    set.insert(2);
    set.insert(3);

    assert!(set.contains(&1));
    assert!(set.contains(&2));
    assert!(!set.contains(&4));
}

// ============================================================================
// Diagnostics Integration
// ============================================================================

#[test]
fn test_diagnostics_basic() {
    let diag = DiagnosticBuilder::error()
        .message("Test error")
        .label("error occurred here")
        .build();

    assert!(diag.is_error());
}

// ============================================================================
// Re-export Verification Tests
// ============================================================================

#[test]
fn test_reexport_verum_common_types() {
    let _list: List<i32> = List::new();
    let _text: Text = Text::from("test");
    let _map: Map<Text, i32> = Map::new();
    let _set: Set<i32> = Set::new();
}
