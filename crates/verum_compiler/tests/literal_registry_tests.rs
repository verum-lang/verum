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
// Migrated from src/literal_registry.rs per CLAUDE.md standards

use verum_compiler::literal_registry::*;
use verum_common::{Maybe, Text};

#[test]
fn test_registry_creation() {
    let registry = LiteralRegistry::new();
    assert!(registry.get_handler(&Text::from("rx")).is_none());
}

#[test]
fn test_handler_registration() {
    let registry = LiteralRegistry::new();
    let handler = TaggedLiteralHandler {
        tag: Text::from("test"),
        handler_fn: Text::from("test::parse"),
        compile_time: true,
        runtime: false,
    };

    let result = registry.register_handler(handler.clone());
    assert!(result.is_ok());

    let retrieved = registry.get_handler(&Text::from("test"));
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap(), handler);
}

#[test]
fn test_duplicate_handler_registration() {
    let registry = LiteralRegistry::new();
    let handler = TaggedLiteralHandler {
        tag: Text::from("test"),
        handler_fn: Text::from("test::parse"),
        compile_time: true,
        runtime: false,
    };

    let result1 = registry.register_handler(handler.clone());
    assert!(result1.is_ok());

    let result2 = registry.register_handler(handler);
    assert!(result2.is_err());
}

#[test]
fn test_builtin_handlers() {
    let registry = LiteralRegistry::new();
    registry.register_builtin_handlers();

    // Check all builtin handlers are registered
    let tags = vec![
        "d", "rx", "interval", "mat", "url", "email", "json", "xml", "yaml",
    ];
    for tag in tags {
        let handler = registry.get_handler(&Text::from(tag));
        assert!(
            handler.is_some(),
            "Handler for '{}' not found",
            tag
        );
    }
}
