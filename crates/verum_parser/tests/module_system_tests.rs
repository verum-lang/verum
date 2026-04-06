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
//! Comprehensive module and mount system tests
//!
//! This test suite covers:
//! 1. Mount statement parsing (simple, wildcard, group, aliased)
//! 2. Module definition parsing (with and without body, visibility)
//! 3. Path parsing (qualified names like std.collections.List)
//! 4. Integration with verum_modules resolution
//!
//! Tests for module system syntax: module declarations, mount statements, visibility
//! Module system: hierarchical modules, path resolution with dots (not ::), cog distribution

use verum_ast::decl::{MountDecl, MountTreeKind, ModuleDecl, Visibility};
use verum_ast::{FileId, ItemKind, PathSegment};
use verum_common::{List, Maybe};
use verum_lexer::Lexer;
use verum_parser::VerumParser;

/// Helper to parse a module from source.
fn parse(source: &str) -> Result<verum_ast::Module, verum_common::List<verum_fast_parser::ParseError>> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser.parse_module(lexer, file_id)
}

/// Helper to extract first item from parsed module.
fn extract_first_item(source: &str) -> ItemKind {
    let module = parse(source).expect("parsing failed");
    assert!(!module.items.is_empty(), "expected at least one item");
    module.items[0].kind.clone()
}

// ============================================================================
// SECTION 1: Link Statement Parsing Tests
// ============================================================================

#[test]
fn test_simple_mount_single_segment() {
    let source = "mount List;";
    match extract_first_item(source) {
        ItemKind::Mount(mount) => {
            match &mount.tree.kind {
                MountTreeKind::Path(path) => {
                    assert_eq!(path.segments.len(), 1);
                    match &path.segments[0] {
                        PathSegment::Name(ident) => assert_eq!(ident.name.as_str(), "List"),
                        _ => panic!("expected Name segment"),
                    }
                }
                _ => panic!("expected Path mount"),
            }
            assert!(mount.alias.is_none());
        }
        _ => panic!("expected mount"),
    }
}

#[test]
fn test_simple_mount_two_segments() {
    let source = "mount collections.List;";
    match extract_first_item(source) {
        ItemKind::Mount(mount) => match &mount.tree.kind {
            MountTreeKind::Path(path) => {
                assert_eq!(path.segments.len(), 2);
                match &path.segments[0] {
                    PathSegment::Name(ident) => assert_eq!(ident.name.as_str(), "collections"),
                    _ => panic!("expected Name segment"),
                }
                match &path.segments[1] {
                    PathSegment::Name(ident) => assert_eq!(ident.name.as_str(), "List"),
                    _ => panic!("expected Name segment"),
                }
            }
            _ => panic!("expected Path mount"),
        },
        _ => panic!("expected mount"),
    }
}

#[test]
fn test_simple_mount_three_segments() {
    let source = "mount std.collections.List;";
    match extract_first_item(source) {
        ItemKind::Mount(mount) => match &mount.tree.kind {
            MountTreeKind::Path(path) => {
                assert_eq!(path.segments.len(), 3);
                match &path.segments[0] {
                    PathSegment::Name(ident) => assert_eq!(ident.name.as_str(), "std"),
                    _ => panic!("expected Name segment"),
                }
                match &path.segments[1] {
                    PathSegment::Name(ident) => assert_eq!(ident.name.as_str(), "collections"),
                    _ => panic!("expected Name segment"),
                }
                match &path.segments[2] {
                    PathSegment::Name(ident) => assert_eq!(ident.name.as_str(), "List"),
                    _ => panic!("expected Name segment"),
                }
            }
            _ => panic!("expected Path mount"),
        },
        _ => panic!("expected mount"),
    }
}

#[test]
fn test_wildcard_mount_single_module() {
    let source = "mount collections.*;";
    match extract_first_item(source) {
        ItemKind::Mount(mount) => match &mount.tree.kind {
            MountTreeKind::Glob(path) => {
                assert_eq!(path.segments.len(), 1);
                match &path.segments[0] {
                    PathSegment::Name(ident) => assert_eq!(ident.name.as_str(), "collections"),
                    _ => panic!("expected Name segment"),
                }
            }
            _ => panic!("expected Glob mount"),
        },
        _ => panic!("expected mount"),
    }
}

#[test]
fn test_wildcard_mount_nested_path() {
    let source = "mount std.collections.*;";
    match extract_first_item(source) {
        ItemKind::Mount(mount) => match &mount.tree.kind {
            MountTreeKind::Glob(path) => {
                assert_eq!(path.segments.len(), 2);
                match &path.segments[0] {
                    PathSegment::Name(ident) => assert_eq!(ident.name.as_str(), "std"),
                    _ => panic!("expected Name segment"),
                }
                match &path.segments[1] {
                    PathSegment::Name(ident) => assert_eq!(ident.name.as_str(), "collections"),
                    _ => panic!("expected Name segment"),
                }
            }
            _ => panic!("expected Glob mount"),
        },
        _ => panic!("expected mount"),
    }
}

#[test]
fn test_group_mount_two_items() {
    let source = "mount std.{List, Map};";
    match extract_first_item(source) {
        ItemKind::Mount(mount) => {
            match &mount.tree.kind {
                MountTreeKind::Nested { prefix, trees } => {
                    assert_eq!(prefix.segments.len(), 1);
                    match &prefix.segments[0] {
                        PathSegment::Name(ident) => assert_eq!(ident.name.as_str(), "std"),
                        _ => panic!("expected Name segment"),
                    }
                    assert_eq!(trees.len(), 2);

                    // Check first item
                    match &trees[0].kind {
                        MountTreeKind::Path(path) => {
                            assert_eq!(path.segments.len(), 1);
                            match &path.segments[0] {
                                PathSegment::Name(ident) => assert_eq!(ident.name.as_str(), "List"),
                                _ => panic!("expected Name segment"),
                            }
                        }
                        _ => panic!("expected Path in nested mount"),
                    }

                    // Check second item
                    match &trees[1].kind {
                        MountTreeKind::Path(path) => {
                            assert_eq!(path.segments.len(), 1);
                            match &path.segments[0] {
                                PathSegment::Name(ident) => assert_eq!(ident.name.as_str(), "Map"),
                                _ => panic!("expected Name segment"),
                            }
                        }
                        _ => panic!("expected Path in nested mount"),
                    }
                }
                _ => panic!("expected Nested mount"),
            }
        }
        _ => panic!("expected mount"),
    }
}

#[test]
fn test_group_mount_three_items() {
    let source = "mount std.collections.{List, Map, Set};";
    match extract_first_item(source) {
        ItemKind::Mount(mount) => match &mount.tree.kind {
            MountTreeKind::Nested { prefix, trees } => {
                assert_eq!(prefix.segments.len(), 2);
                assert_eq!(trees.len(), 3);

                match &trees[0].kind {
                    MountTreeKind::Path(path) => match &path.segments[0] {
                        PathSegment::Name(ident) => assert_eq!(ident.name.as_str(), "List"),
                        _ => panic!("expected Name segment"),
                    },
                    _ => panic!("expected Path"),
                }

                match &trees[1].kind {
                    MountTreeKind::Path(path) => match &path.segments[0] {
                        PathSegment::Name(ident) => assert_eq!(ident.name.as_str(), "Map"),
                        _ => panic!("expected Name segment"),
                    },
                    _ => panic!("expected Path"),
                }

                match &trees[2].kind {
                    MountTreeKind::Path(path) => match &path.segments[0] {
                        PathSegment::Name(ident) => assert_eq!(ident.name.as_str(), "Set"),
                        _ => panic!("expected Name segment"),
                    },
                    _ => panic!("expected Path"),
                }
            }
            _ => panic!("expected Nested mount"),
        },
        _ => panic!("expected mount"),
    }
}

#[test]
fn test_aliased_mount_simple() {
    let source = "mount std.collections.HashMap as Map;";
    match extract_first_item(source) {
        ItemKind::Mount(mount) => {
            match &mount.tree.kind {
                MountTreeKind::Path(path) => {
                    assert_eq!(path.segments.len(), 3);
                    match &path.segments[2] {
                        PathSegment::Name(ident) => assert_eq!(ident.name.as_str(), "HashMap"),
                        _ => panic!("expected Name segment"),
                    }
                }
                _ => panic!("expected Path mount"),
            }

            assert!(mount.alias.is_some());
            let alias = mount.alias.as_ref().unwrap();
            assert_eq!(alias.name.as_str(), "Map");
        }
        _ => panic!("expected mount"),
    }
}

#[test]
fn test_aliased_mount_short_name() {
    let source = "mount VeryLongModuleName as Short;";
    match extract_first_item(source) {
        ItemKind::Mount(mount) => {
            assert!(mount.alias.is_some());
            let alias = mount.alias.as_ref().unwrap();
            assert_eq!(alias.name.as_str(), "Short");
        }
        _ => panic!("expected mount"),
    }
}

#[test]
fn test_mount_with_trailing_comma() {
    let source = "mount std.{List, Map,};";
    match extract_first_item(source) {
        ItemKind::Mount(mount) => match &mount.tree.kind {
            MountTreeKind::Nested { trees, .. } => {
                assert_eq!(trees.len(), 2);
            }
            _ => panic!("expected Nested mount"),
        },
        _ => panic!("expected mount"),
    }
}

// ============================================================================
// SECTION 2: Module Definition Parsing Tests
// ============================================================================

#[test]
fn test_module_with_empty_body() {
    let source = "module empty { }";
    match extract_first_item(source) {
        ItemKind::Module(module) => {
            assert_eq!(module.name.name.as_str(), "empty");
            match &module.items {
                Maybe::Some(items) => assert_eq!(items.len(), 0),
                Maybe::None => panic!("expected Some(empty list), got None"),
            }
            assert_eq!(module.visibility, Visibility::Private);
        }
        _ => panic!("expected module"),
    }
}

#[test]
fn test_module_with_single_function() {
    let source = r#"
        module math {
            fn add(a: Int, b: Int) -> Int {
                a + b
            }
        }
    "#;

    match extract_first_item(source) {
        ItemKind::Module(module) => {
            assert_eq!(module.name.name.as_str(), "math");
            match &module.items {
                Maybe::Some(items) => {
                    assert_eq!(items.len(), 1);
                    match &items[0].kind {
                        ItemKind::Function(_) => {}
                        _ => panic!("expected function"),
                    }
                }
                Maybe::None => panic!("expected Some"),
            }
        }
        _ => panic!("expected module"),
    }
}

#[test]
fn test_module_forward_declaration() {
    let source = "module external;";
    match extract_first_item(source) {
        ItemKind::Module(module) => {
            assert_eq!(module.name.name.as_str(), "external");
            assert!(module.items.is_none());
        }
        _ => panic!("expected module"),
    }
}

#[test]
fn test_module_public_visibility() {
    let source = "pub module api { }";
    match extract_first_item(source) {
        ItemKind::Module(module) => {
            assert_eq!(module.name.name.as_str(), "api");
            assert_eq!(module.visibility, Visibility::Public);
        }
        _ => panic!("expected module"),
    }
}

#[test]
fn test_module_with_multiple_items() {
    let source = r#"
        module utils {
            fn foo() { }
            fn bar() { }
            type Point is { x: Int, y: Int };
        }
    "#;

    match extract_first_item(source) {
        ItemKind::Module(module) => {
            assert_eq!(module.name.name.as_str(), "utils");
            match &module.items {
                Maybe::Some(items) => {
                    assert_eq!(items.len(), 3);
                }
                Maybe::None => panic!("expected Some"),
            }
        }
        _ => panic!("expected module"),
    }
}

#[test]
fn test_nested_module() {
    let source = r#"
        module outer {
            module inner {
                fn foo() { }
            }
        }
    "#;

    match extract_first_item(source) {
        ItemKind::Module(outer) => {
            assert_eq!(outer.name.name.as_str(), "outer");
            match &outer.items {
                Maybe::Some(items) => {
                    assert_eq!(items.len(), 1);
                    match &items[0].kind {
                        ItemKind::Module(inner) => {
                            assert_eq!(inner.name.name.as_str(), "inner");
                        }
                        _ => panic!("expected nested module"),
                    }
                }
                Maybe::None => panic!("expected Some"),
            }
        }
        _ => panic!("expected module"),
    }
}

#[test]
fn test_module_with_mounts() {
    let source = r#"
        module service {
            mount std.collections.List;
            mount std.io.*;

            fn process() { }
        }
    "#;

    match extract_first_item(source) {
        ItemKind::Module(module) => match &module.items {
            Maybe::Some(items) => {
                assert_eq!(items.len(), 3);

                match &items[0].kind {
                    ItemKind::Mount(_) => {}
                    _ => panic!("expected mount"),
                }

                match &items[1].kind {
                    ItemKind::Mount(_) => {}
                    _ => panic!("expected mount"),
                }

                match &items[2].kind {
                    ItemKind::Function(_) => {}
                    _ => panic!("expected function"),
                }
            }
            Maybe::None => panic!("expected Some"),
        },
        _ => panic!("expected module"),
    }
}

// ============================================================================
// SECTION 3: Path Parsing in Different Contexts
// ============================================================================

#[test]
fn test_path_in_type_annotation() {
    // NOTE: All paths in Verum use . (dot) separator, not ::
    // This is consistent with the Verum grammar specification
    let source = "fn foo(x: std.collections.List<Int>) { }";
    let module = parse(source).expect("parsing failed");

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert_eq!(func.name.name.as_str(), "foo");
            assert_eq!(func.params.len(), 1);
            // Type path parsing is verified implicitly
        }
        _ => panic!("expected function"),
    }
}

#[test]
fn test_path_in_expression() {
    let source = "fn foo() { let x = std.math.PI; }";
    let module = parse(source).expect("parsing failed");

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert!(func.body.is_some());
        }
        _ => panic!("expected function"),
    }
}

// ============================================================================
// SECTION 4: Path Segment Special Keywords Tests
// ============================================================================

#[test]
fn test_mount_with_cog_keyword() {
    // Note: This test verifies that 'cog' can be used as a path segment
    // The actual resolution semantics are handled by verum_modules
    let source = "mount cog.utils.helper;";
    match extract_first_item(source) {
        ItemKind::Mount(mount) => match &mount.tree.kind {
            MountTreeKind::Path(path) => {
                assert_eq!(path.segments.len(), 3);
                match &path.segments[0] {
                    PathSegment::Cog => {}
                    PathSegment::Name(ident) if ident.name.as_str() == "cog" => {}
                    _ => panic!("expected Crate segment"),
                }
            }
            _ => panic!("expected Path mount"),
        },
        _ => panic!("expected mount"),
    }
}

#[test]
fn test_mount_with_super_keyword() {
    let source = "mount super.sibling.Type;";
    match extract_first_item(source) {
        ItemKind::Mount(mount) => match &mount.tree.kind {
            MountTreeKind::Path(path) => {
                assert_eq!(path.segments.len(), 3);
                match &path.segments[0] {
                    PathSegment::Super => {}
                    PathSegment::Name(ident) if ident.name.as_str() == "super" => {}
                    _ => panic!("expected Super segment"),
                }
            }
            _ => panic!("expected Path mount"),
        },
        _ => panic!("expected mount"),
    }
}

#[test]
fn test_mount_with_self_keyword() {
    let source = "mount self.child.Type;";
    match extract_first_item(source) {
        ItemKind::Mount(mount) => match &mount.tree.kind {
            MountTreeKind::Path(path) => {
                assert_eq!(path.segments.len(), 3);
                match &path.segments[0] {
                    PathSegment::SelfValue => {}
                    PathSegment::Name(ident) if ident.name.as_str() == "self" => {}
                    _ => panic!("expected Self segment"),
                }
            }
            _ => panic!("expected Path mount"),
        },
        _ => panic!("expected mount"),
    }
}

// ============================================================================
// SECTION 5: Complex Integration Tests
// ============================================================================

#[test]
fn test_module_with_profile_and_mounts() {
    let source = r#"
        @profile(application)
        module app {
            mount std.collections.*;
            mount std.io.{File, Read, Write};

            fn main() { }
        }
    "#;

    match extract_first_item(source) {
        ItemKind::Module(module) => {
            assert_eq!(module.name.name.as_str(), "app");
            assert!(module.profile.is_some());

            match &module.items {
                Maybe::Some(items) => {
                    assert_eq!(items.len(), 3);
                }
                Maybe::None => panic!("expected Some"),
            }
        }
        _ => panic!("expected module"),
    }
}

#[test]
fn test_module_with_using_context() {
    let source = r#"
        @using([Database, Logger])
        module service {
            fn process() { }
        }
    "#;

    match extract_first_item(source) {
        ItemKind::Module(module) => {
            assert_eq!(module.contexts.len(), 2);
        }
        _ => panic!("expected module"),
    }
}

#[test]
fn test_deeply_nested_modules() {
    let source = r#"
        module level1 {
            module level2 {
                module level3 {
                    fn deep_function() { }
                }
            }
        }
    "#;

    match extract_first_item(source) {
        ItemKind::Module(level1) => {
            assert_eq!(level1.name.name.as_str(), "level1");

            match &level1.items {
                Maybe::Some(items) => match &items[0].kind {
                    ItemKind::Module(level2) => {
                        assert_eq!(level2.name.name.as_str(), "level2");

                        match &level2.items {
                            Maybe::Some(items) => match &items[0].kind {
                                ItemKind::Module(level3) => {
                                    assert_eq!(level3.name.name.as_str(), "level3");
                                }
                                _ => panic!("expected level3 module"),
                            },
                            Maybe::None => panic!("expected Some"),
                        }
                    }
                    _ => panic!("expected level2 module"),
                },
                Maybe::None => panic!("expected Some"),
            }
        }
        _ => panic!("expected module"),
    }
}

#[test]
fn test_multiple_top_level_modules() {
    let source = r#"
        module mod1 {
            fn foo() { }
        }

        module mod2 {
            fn bar() { }
        }

        module mod3 {
            fn baz() { }
        }
    "#;

    let module = parse(source).expect("parsing failed");
    assert_eq!(module.items.len(), 3);

    for (i, item) in module.items.iter().enumerate() {
        match &item.kind {
            ItemKind::Module(m) => {
                let expected_name = format!("mod{}", i + 1);
                assert_eq!(m.name.name.as_str(), expected_name);
            }
            _ => panic!("expected module"),
        }
    }
}

#[test]
fn test_module_mixing_mounts_and_declarations() {
    let source = r#"
        module mixed {
            mount std.io.File;

            fn helper() { }

            mount std.collections.List;

            type Data is { value: Int };

            mount std.net.*;

            fn main() { }
        }
    "#;

    match extract_first_item(source) {
        ItemKind::Module(module) => {
            match &module.items {
                Maybe::Some(items) => {
                    assert_eq!(items.len(), 6);

                    // Verify order: mount, fn, mount, type, mount, fn
                    match &items[0].kind {
                        ItemKind::Mount(_) => {}
                        _ => panic!("expected mount at position 0"),
                    }

                    match &items[1].kind {
                        ItemKind::Function(_) => {}
                        _ => panic!("expected function at position 1"),
                    }

                    match &items[2].kind {
                        ItemKind::Mount(_) => {}
                        _ => panic!("expected mount at position 2"),
                    }

                    match &items[3].kind {
                        ItemKind::Type(_) => {}
                        _ => panic!("expected type at position 3"),
                    }

                    match &items[4].kind {
                        ItemKind::Mount(_) => {}
                        _ => panic!("expected mount at position 4"),
                    }

                    match &items[5].kind {
                        ItemKind::Function(_) => {}
                        _ => panic!("expected function at position 5"),
                    }
                }
                Maybe::None => panic!("expected Some"),
            }
        }
        _ => panic!("expected module"),
    }
}

// ============================================================================
// SECTION 6: Error Cases (These should parse successfully but may fail later)
// ============================================================================

#[test]
fn test_empty_mount_group() {
    // Empty mount group should parse (though may be semantically invalid)
    let source = "mount std.{};";
    match extract_first_item(source) {
        ItemKind::Mount(mount) => match &mount.tree.kind {
            MountTreeKind::Nested { trees, .. } => {
                assert_eq!(trees.len(), 0);
            }
            _ => panic!("expected Nested mount"),
        },
        _ => panic!("expected mount"),
    }
}

#[test]
fn test_public_mount() {
    // Public mounts (re-exports)
    let source = "pub mount std.collections.List;";
    match extract_first_item(source) {
        ItemKind::Mount(mount) => {
            assert_eq!(mount.visibility, Visibility::Public);
        }
        _ => panic!("expected mount"),
    }
}

// ============================================================================
// SECTION 7: Keyword Module Names (Contextual Keyword Usage)
// ============================================================================

#[test]
fn test_module_with_keyword_name_result() {
    // Keywords can be used as module names (contextual keyword support)
    let source = "pub module result;";
    match extract_first_item(source) {
        ItemKind::Module(module) => {
            assert_eq!(module.name.name.as_str(), "result");
            assert_eq!(module.visibility, Visibility::Public);
            assert!(module.items.is_none());
        }
        _ => panic!("expected module"),
    }
}

#[test]
fn test_module_with_keyword_name_type() {
    let source = "pub module type;";
    match extract_first_item(source) {
        ItemKind::Module(module) => {
            assert_eq!(module.name.name.as_str(), "type");
            assert_eq!(module.visibility, Visibility::Public);
        }
        _ => panic!("expected module"),
    }
}

#[test]
fn test_module_with_keyword_name_match() {
    let source = "pub module match;";
    match extract_first_item(source) {
        ItemKind::Module(module) => {
            assert_eq!(module.name.name.as_str(), "match");
        }
        _ => panic!("expected module"),
    }
}

#[test]
fn test_module_with_keyword_name_async() {
    let source = "pub module async { }";
    match extract_first_item(source) {
        ItemKind::Module(module) => {
            assert_eq!(module.name.name.as_str(), "async");
            assert!(module.items.is_some());
        }
        _ => panic!("expected module"),
    }
}

#[test]
fn test_module_with_keyword_name_proof() {
    let source = "module proof;";
    match extract_first_item(source) {
        ItemKind::Module(module) => {
            assert_eq!(module.name.name.as_str(), "proof");
        }
        _ => panic!("expected module"),
    }
}

#[test]
fn test_module_with_keyword_name_context() {
    let source = "module context { fn foo() { } }";
    match extract_first_item(source) {
        ItemKind::Module(module) => {
            assert_eq!(module.name.name.as_str(), "context");
            match &module.items {
                Maybe::Some(items) => {
                    assert_eq!(items.len(), 1);
                }
                Maybe::None => panic!("expected Some"),
            }
        }
        _ => panic!("expected module"),
    }
}

#[test]
fn test_multiple_keyword_named_modules() {
    // Test multiple modules with keyword names in one file
    let source = r#"
        pub module result;
        pub module type;
        pub module option;
        module async { }
        module stream { }
    "#;

    let module = parse(source).expect("parsing failed");
    assert_eq!(module.items.len(), 5);

    let expected_names = ["result", "type", "option", "async", "stream"];
    for (i, item) in module.items.iter().enumerate() {
        match &item.kind {
            ItemKind::Module(m) => {
                assert_eq!(m.name.name.as_str(), expected_names[i]);
            }
            _ => panic!("expected module at position {}", i),
        }
    }
}
