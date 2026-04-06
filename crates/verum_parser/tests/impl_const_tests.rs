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
//! Tests for const declarations inside implement blocks.
//!
//! This module tests parsing of const declarations within impl blocks,
//! supporting the syntax:
//! ```verum
//! implement Duration {
//!     pub const ZERO: Duration = Duration { secs: 0, nanos: 0 };
//!
//!     pub fn new(secs: Int) -> Duration { ... }
//! }
//! ```

use verum_ast::{ItemKind, decl::ImplItemKind, span::FileId};
use verum_lexer::Lexer;
use verum_parser::VerumParser;

/// Helper to parse and extract impl items
fn parse_impl_items(source: &str) -> Vec<ImplItemKind> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();

    match parser.parse_module(lexer, file_id) {
        Ok(module) => {
            if let Some(item) = module.items.first()
                && let ItemKind::Impl(impl_decl) = &item.kind {
                    return impl_decl
                        .items
                        .iter()
                        .map(|item| item.kind.clone())
                        .collect();
                }
            vec![]
        }
        Err(_) => vec![],
    }
}

#[test]
fn test_const_in_inherent_impl() {
    let source = r#"
        implement Duration {
            pub const ZERO: Duration = Duration { secs: 0, nanos: 0 };
        }
    "#;

    let items = parse_impl_items(source);
    assert_eq!(items.len(), 1, "Should parse one impl item");

    match &items[0] {
        ImplItemKind::Const { name, ty, value } => {
            assert_eq!(name.name.as_str(), "ZERO");
        }
        _ => panic!("Expected const item, got {:?}", items[0]),
    }
}

#[test]
fn test_multiple_consts_in_impl() {
    let source = r#"
        implement Duration {
            pub const ZERO: Duration = Duration { secs: 0, nanos: 0 };
            pub const MAX: Duration = Duration { secs: 9999, nanos: 999999999 };
        }
    "#;

    let items = parse_impl_items(source);
    assert_eq!(items.len(), 2, "Should parse two impl items");

    for item in &items {
        assert!(matches!(item, ImplItemKind::Const { .. }));
    }
}

#[test]
fn test_const_and_function_in_impl() {
    let source = r#"
        implement Duration {
            pub const ZERO: Duration = Duration { secs: 0, nanos: 0 };

            pub fn new(secs: Int) -> Duration {
                Duration { secs: secs, nanos: 0 }
            }
        }
    "#;

    let items = parse_impl_items(source);
    assert_eq!(items.len(), 2, "Should parse two impl items");

    match &items[0] {
        ImplItemKind::Const { name, .. } => {
            assert_eq!(name.name.as_str(), "ZERO");
        }
        _ => panic!("Expected const as first item"),
    }

    match &items[1] {
        ImplItemKind::Function(decl) => {
            assert_eq!(decl.name.name.as_str(), "new");
        }
        _ => panic!("Expected function as second item"),
    }
}

#[test]
fn test_private_const_in_impl() {
    let source = r#"
        implement Duration {
            const INTERNAL_MAX: Int = 1000000;
        }
    "#;

    let items = parse_impl_items(source);
    assert_eq!(items.len(), 1);
    assert!(matches!(items[0], ImplItemKind::Const { .. }));
}

#[test]
fn test_const_with_simple_value() {
    let source = r#"
        implement Counter {
            pub const DEFAULT_START: Int = 0;
        }
    "#;

    let items = parse_impl_items(source);
    assert_eq!(items.len(), 1);

    match &items[0] {
        ImplItemKind::Const { name, .. } => {
            assert_eq!(name.name.as_str(), "DEFAULT_START");
        }
        _ => panic!("Expected const item"),
    }
}

#[test]
fn test_const_in_protocol_impl() {
    let source = r#"
        implement Display for Duration {
            const FORMAT: Text = "HH:MM:SS";

            fn fmt(self: &Self) -> Text {
                "formatted"
            }
        }
    "#;

    let items = parse_impl_items(source);
    assert_eq!(items.len(), 2, "Should have const and function");

    assert!(matches!(items[0], ImplItemKind::Const { .. }));
    assert!(matches!(items[1], ImplItemKind::Function(_)));
}

#[test]
fn test_const_with_expression_value() {
    let source = r#"
        implement Math {
            pub const PI_TIMES_TWO: Float = 3.14159 * 2.0;
        }
    "#;

    let items = parse_impl_items(source);
    assert_eq!(items.len(), 1);
    assert!(matches!(items[0], ImplItemKind::Const { .. }));
}

#[test]
fn test_impl_with_type_const_and_function() {
    let source = r#"
        implement Iterator for Range {
            type Item is Int;

            const DEFAULT_STEP: Int = 1;

            fn next(self: &mut Self) -> Maybe<Int> {
                Maybe.None
            }
        }
    "#;

    let items = parse_impl_items(source);
    assert_eq!(items.len(), 3, "Should have type, const, and function");

    assert!(matches!(items[0], ImplItemKind::Type { .. }));
    assert!(matches!(items[1], ImplItemKind::Const { .. }));
    assert!(matches!(items[2], ImplItemKind::Function(_)));
}

#[test]
fn test_const_ordering_in_impl() {
    // Test that consts can appear before or after functions
    let source = r#"
        implement Container {
            fn size(self: &Self) -> Int { 0 }

            const MAX_CAPACITY: Int = 1000;

            fn is_full(self: &Self) -> Bool { false }
        }
    "#;

    let items = parse_impl_items(source);
    assert_eq!(items.len(), 3);

    assert!(matches!(items[0], ImplItemKind::Function(_)));
    assert!(matches!(items[1], ImplItemKind::Const { .. }));
    assert!(matches!(items[2], ImplItemKind::Function(_)));
}

#[test]
fn test_generic_impl_with_const() {
    let source = r#"
        implement<T> Container<T> {
            const DEFAULT_CAPACITY: Int = 16;

            fn new() -> Self {
                Container { items: [] }
            }
        }
    "#;

    let items = parse_impl_items(source);
    assert_eq!(items.len(), 2);

    assert!(matches!(items[0], ImplItemKind::Const { .. }));
    assert!(matches!(items[1], ImplItemKind::Function(_)));
}

#[test]
fn test_const_with_struct_literal_value() {
    let source = r#"
        implement Point {
            pub const ORIGIN: Point = Point { x: 0, y: 0 };
        }
    "#;

    let items = parse_impl_items(source);
    assert_eq!(items.len(), 1);

    match &items[0] {
        ImplItemKind::Const { name, .. } => {
            assert_eq!(name.name.as_str(), "ORIGIN");
        }
        _ => panic!("Expected const item"),
    }
}
