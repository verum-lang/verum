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
//! Advanced module system tests for Verum parser
//!
//! Tests cover:
//! - Relative imports (mount .foo.*)
//! - Re-exports (public mount)
//! - Mount with aliases
//! - Mount with groups
//! - Wildcard mounts
//! - Nested module declarations
//! - Visibility modifiers on modules and mounts

use verum_ast::decl::{MountDecl, MountTreeKind, Visibility};
use verum_ast::{FileId, ItemKind, Module};
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

fn parse_module(source: &str) -> Module {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser
        .parse_module(lexer, file_id)
        .unwrap_or_else(|e| panic!("Failed to parse module: {:?}", e))
}

fn assert_parses(source: &str) {
    parse_module(source);
}

fn parse_first_item(source: &str) -> ItemKind {
    let module = parse_module(source);
    module
        .items
        .into_iter()
        .next()
        .expect("Expected at least one item")
        .kind
}

// ============================================================================
// RELATIVE IMPORTS
// ============================================================================

#[test]
fn test_relative_mount_single_dot() {
    // Relative import from current module's parent
    assert_parses("mount .sibling;");
}

#[test]
fn test_relative_mount_with_path() {
    assert_parses("mount .utils.helpers;");
}

#[test]
fn test_relative_mount_wildcard() {
    assert_parses("mount .utils.*;");
}

// ============================================================================
// RE-EXPORTS (PUBLIC MOUNT)
// ============================================================================

#[test]
fn test_public_mount_simple() {
    let source = "public mount collections.List;";
    let item = parse_first_item(source);
    match item {
        ItemKind::Mount(mount) => {
            assert_eq!(mount.visibility, Visibility::Public);
        }
        _ => panic!("Expected Mount item"),
    }
}

#[test]
fn test_public_mount_wildcard() {
    assert_parses("public mount collections.*;");
}

#[test]
fn test_internal_mount() {
    let source = "internal mount implementation.detail;";
    assert_parses(source);
}

// ============================================================================
// MOUNT WITH ALIASES
// ============================================================================

#[test]
fn test_mount_with_alias() {
    let source = "mount collections.HashMap as Map;";
    let item = parse_first_item(source);
    match item {
        ItemKind::Mount(mount) => {
            assert!(mount.alias.is_some(), "Should have alias");
        }
        _ => panic!("Expected Mount item"),
    }
}

#[test]
fn test_mount_long_path_with_alias() {
    assert_parses("mount std.collections.sorted.BTreeMap as SortedMap;");
}

// ============================================================================
// MOUNT WITH GROUPS (SELECTIVE IMPORTS)
// ============================================================================

#[test]
fn test_mount_group_basic() {
    let source = "mount collections.{List, Map, Set};";
    let item = parse_first_item(source);
    match item {
        ItemKind::Mount(mount) => {
            match &mount.tree.kind {
                MountTreeKind::Nested { .. } => {}
                _ => panic!("Expected Nested mount, got {:?}", mount.tree.kind),
            }
        }
        _ => panic!("Expected Mount item"),
    }
}

#[test]
fn test_mount_group_single_item() {
    assert_parses("mount std.io.{File};");
}

#[test]
fn test_mount_group_trailing_comma() {
    assert_parses("mount std.io.{File, BufReader, BufWriter,};");
}

#[test]
fn test_mount_group_nested() {
    assert_parses("mount std.{io.{File, Reader}, collections.{List, Map}};");
}

#[test]
fn test_mount_group_with_alias() {
    assert_parses("mount collections.{HashMap as Map, HashSet as Set};");
}

// ============================================================================
// WILDCARD MOUNTS
// ============================================================================

#[test]
fn test_mount_wildcard_basic() {
    let source = "mount std.prelude.*;";
    let item = parse_first_item(source);
    match item {
        ItemKind::Mount(mount) => {
            match &mount.tree.kind {
                MountTreeKind::Glob(_) => {}
                _ => panic!("Expected Glob mount, got {:?}", mount.tree.kind),
            }
        }
        _ => panic!("Expected Mount item"),
    }
}

#[test]
fn test_mount_wildcard_deep_path() {
    assert_parses("mount std.collections.sorted.*;");
}

// ============================================================================
// NESTED MODULE DECLARATIONS
// ============================================================================

#[test]
fn test_module_declaration_basic() {
    let source = r#"
        module utils {
            fn helper() -> Int { 42 }
        }
    "#;
    let item = parse_first_item(source);
    match item {
        ItemKind::Module(decl) => {
            assert_eq!(decl.name.name.as_str(), "utils");
        }
        _ => panic!("Expected Module item"),
    }
}

#[test]
fn test_module_with_visibility() {
    let source = r#"
        public module api {
            fn endpoint() -> Response { respond() }
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_nested_modules() {
    let source = r#"
        module outer {
            module inner {
                fn deep() -> Int { 1 }
            }
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_module_with_mount_and_functions() {
    let source = r#"
        module handlers {
            mount std.io.File;

            fn read_config(path: Text) -> Config {
                let file = File.open(path);
                parse(file)
            }

            fn write_config(path: Text, config: Config) -> Unit {
                let file = File.create(path);
                serialize(config, file)
            }
        }
    "#;
    assert_parses(source);
}

// ============================================================================
// MODULE WITH TYPE DEFINITIONS
// ============================================================================

#[test]
fn test_module_with_types() {
    let source = r#"
        module models {
            type User is {
                name: Text,
                email: Text,
                age: Int,
            };

            type Role is Admin | Moderator | Member;

            fn create_user(name: Text, email: Text) -> User {
                User { name, email, age: 0 }
            }
        }
    "#;
    assert_parses(source);
}

// ============================================================================
// COMPLEX MOUNT PATTERNS
// ============================================================================

#[test]
fn test_multiple_mounts() {
    let source = r#"
        mount std.io.File;
        mount std.collections.{List, Map};
        mount std.prelude.*;
    "#;
    let module = parse_module(source);
    assert_eq!(module.items.len(), 3);
}

#[test]
fn test_mount_self() {
    // Import the module itself
    assert_parses("mount std.collections as col;");
}

#[test]
fn test_mount_cog_path() {
    // Import from a cog (package)
    assert_parses("mount serde.json.{serialize, deserialize};");
}

#[test]
fn test_mount_with_comments() {
    let source = r#"
        // Core imports
        mount std.prelude.*;
        // Collections
        mount std.collections.{List, Map, Set};
    "#;
    assert_parses(source);
}
