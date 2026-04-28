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
// Test cases for mount statement parsing.
//
// Tests for mount statement: `mount module.{item1, item2}` and `mount module.*`

use verum_ast::decl::{MountDecl, MountTreeKind};
use verum_ast::{FileId, ItemKind};
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

/// Helper to parse a module from source.
fn parse(source: &str) -> Result<verum_ast::Module, verum_common::List<verum_fast_parser::ParseError>> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser.parse_module(lexer, file_id)
}

/// Helper to extract mount from parsed module.
fn extract_mount(source: &str) -> MountDecl {
    let module = parse(source).expect("parsing failed");
    assert_eq!(module.items.len(), 1, "expected exactly one item");
    match &module.items[0].kind {
        ItemKind::Mount(decl) => decl.clone(),
        _ => panic!("expected mount declaration"),
    }
}

#[test]
fn test_simple_mount() {
    let source = "mount std.io.File;";
    let mount = extract_mount(source);

    match &mount.tree.kind {
        MountTreeKind::Path(path) => {
            assert_eq!(path.segments.len(), 3);
            if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                assert_eq!(ident.name, "std");
            } else {
                panic!("Expected Name segment");
            }
            if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[1] {
                assert_eq!(ident.name, "io");
            } else {
                panic!("Expected Name segment");
            }
            if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[2] {
                assert_eq!(ident.name, "File");
            } else {
                panic!("Expected Name segment");
            }
        }
        _ => panic!("expected path mount"),
    }

    assert!(mount.alias.is_none());
}

#[test]
fn test_glob_mount() {
    let source = "mount std.io.*;";
    let mount = extract_mount(source);

    match &mount.tree.kind {
        MountTreeKind::Glob(path) => {
            assert_eq!(path.segments.len(), 2);
            if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                assert_eq!(ident.name, "std");
            } else {
                panic!("Expected Name segment");
            }
            if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[1] {
                assert_eq!(ident.name, "io");
            } else {
                panic!("Expected Name segment");
            }
        }
        _ => panic!("expected glob mount"),
    }

    assert!(mount.alias.is_none());
}

#[test]
fn test_nested_mount() {
    let source = "mount std.io.{File, Read, Write};";
    let mount = extract_mount(source);

    match &mount.tree.kind {
        MountTreeKind::Nested { prefix, trees } => {
            assert_eq!(prefix.segments.len(), 2);
            if let verum_ast::ty::PathSegment::Name(ident) = &prefix.segments[0] {
                assert_eq!(ident.name, "std");
            } else {
                panic!("Expected Name segment");
            }
            if let verum_ast::ty::PathSegment::Name(ident) = &prefix.segments[1] {
                assert_eq!(ident.name, "io");
            } else {
                panic!("Expected Name segment");
            }

            assert_eq!(trees.len(), 3);

            // Check first mounted item
            match &trees[0].kind {
                MountTreeKind::Path(path) => {
                    assert_eq!(path.segments.len(), 1);
                    if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                        assert_eq!(ident.name, "File");
                    } else {
                        panic!("Expected Name segment");
                    }
                }
                _ => panic!("expected path in nested mount"),
            }

            // Check second mounted item
            match &trees[1].kind {
                MountTreeKind::Path(path) => {
                    assert_eq!(path.segments.len(), 1);
                    if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                        assert_eq!(ident.name, "Read");
                    } else {
                        panic!("Expected Name segment");
                    }
                }
                _ => panic!("expected path in nested mount"),
            }

            // Check third mounted item
            match &trees[2].kind {
                MountTreeKind::Path(path) => {
                    assert_eq!(path.segments.len(), 1);
                    if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                        assert_eq!(ident.name, "Write");
                    } else {
                        panic!("Expected Name segment");
                    }
                }
                _ => panic!("expected path in nested mount"),
            }
        }
        _ => panic!("expected nested mount"),
    }

    assert!(mount.alias.is_none());
}

#[test]
fn test_mount_with_alias() {
    let source = "mount std.io.File as FileIO;";
    let mount = extract_mount(source);

    match &mount.tree.kind {
        MountTreeKind::Path(path) => {
            assert_eq!(path.segments.len(), 3);
            if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                assert_eq!(ident.name, "std");
            } else {
                panic!("Expected Name segment");
            }
            if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[1] {
                assert_eq!(ident.name, "io");
            } else {
                panic!("Expected Name segment");
            }
            if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[2] {
                assert_eq!(ident.name, "File");
            } else {
                panic!("Expected Name segment");
            }
        }
        _ => panic!("expected path mount"),
    }

    assert!(mount.alias.is_some());
    let alias = mount.alias.as_ref().unwrap();
    assert_eq!(alias.name.as_str(), "FileIO");
}

#[test]
fn test_nested_mount_with_trailing_comma() {
    let source = "mount std.collections.{Map, Set,};";
    let mount = extract_mount(source);

    match &mount.tree.kind {
        MountTreeKind::Nested { prefix, trees } => {
            assert_eq!(prefix.segments.len(), 2);
            assert_eq!(trees.len(), 2);
        }
        _ => panic!("expected nested mount"),
    }
}

#[test]
fn test_deeply_nested_mount() {
    let source = "mount std.{io.{File, Read}, net.{TcpStream, UdpSocket}};";
    let mount = extract_mount(source);

    match &mount.tree.kind {
        MountTreeKind::Nested { prefix, trees } => {
            assert_eq!(prefix.segments.len(), 1);
            if let verum_ast::ty::PathSegment::Name(ident) = &prefix.segments[0] {
                assert_eq!(ident.name, "std");
            } else {
                panic!("Expected Name segment");
            }
            assert_eq!(trees.len(), 2);

            // Check first nested group (io)
            match &trees[0].kind {
                MountTreeKind::Nested {
                    prefix: io_prefix,
                    trees: io_trees,
                } => {
                    assert_eq!(io_prefix.segments.len(), 1);
                    if let verum_ast::ty::PathSegment::Name(ident) = &io_prefix.segments[0] {
                        assert_eq!(ident.name, "io");
                    } else {
                        panic!("Expected Name segment");
                    }
                    assert_eq!(io_trees.len(), 2);
                }
                _ => panic!("expected nested mount for io"),
            }

            // Check second nested group (net)
            match &trees[1].kind {
                MountTreeKind::Nested {
                    prefix: net_prefix,
                    trees: net_trees,
                } => {
                    assert_eq!(net_prefix.segments.len(), 1);
                    if let verum_ast::ty::PathSegment::Name(ident) = &net_prefix.segments[0] {
                        assert_eq!(ident.name, "net");
                    } else {
                        panic!("Expected Name segment");
                    }
                    assert_eq!(net_trees.len(), 2);
                }
                _ => panic!("expected nested mount for net"),
            }
        }
        _ => panic!("expected nested mount"),
    }
}

#[test]
fn test_glob_mount_with_long_path() {
    let source = "mount verum.std.collections.hash.*;";
    let mount = extract_mount(source);

    match &mount.tree.kind {
        MountTreeKind::Glob(path) => {
            assert_eq!(path.segments.len(), 4);
            if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                assert_eq!(ident.name, "verum");
            } else {
                panic!("Expected Name segment");
            }
            if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[1] {
                assert_eq!(ident.name, "std");
            } else {
                panic!("Expected Name segment");
            }
            if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[2] {
                assert_eq!(ident.name, "collections");
            } else {
                panic!("Expected Name segment");
            }
            if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[3] {
                assert_eq!(ident.name, "hash");
            } else {
                panic!("Expected Name segment");
            }
        }
        _ => panic!("expected glob mount"),
    }
}

#[test]
fn test_multiple_mounts() {
    let source = r#"
        mount std.io.File;
        mount std.collections.*;
        mount std.net.{TcpStream, UdpSocket};
    "#;

    let module = parse(source).expect("parsing failed");
    assert_eq!(module.items.len(), 3, "expected three mount statements");

    // All should be mounts
    for item in module.items.iter() {
        match item.kind {
            ItemKind::Mount(_) => {}
            _ => panic!("expected all items to be mounts"),
        }
    }
}

// ============================================================================
// #5 / P1.5 — File-relative mount (mount ./foo.vr)
// ============================================================================

#[test]
fn test_file_mount_dot_slash() {
    let source = "mount ./helper.vr;";
    let mount = extract_mount(source);
    match &mount.tree.kind {
        MountTreeKind::File { path, .. } => {
            assert_eq!(path.as_str(), "./helper.vr");
        }
        other => panic!("expected file mount, got {:?}", other),
    }
}

#[test]
fn test_file_mount_dot_dot_slash() {
    let source = "mount ../shared/util.vr;";
    let mount = extract_mount(source);
    match &mount.tree.kind {
        MountTreeKind::File { path, .. } => {
            assert_eq!(path.as_str(), "../shared/util.vr");
        }
        other => panic!("expected file mount, got {:?}", other),
    }
}

#[test]
fn test_file_mount_with_alias() {
    let source = "mount ./helper.vr as Helper;";
    let mount = extract_mount(source);
    match &mount.tree.kind {
        MountTreeKind::File { path, .. } => {
            assert_eq!(path.as_str(), "./helper.vr");
        }
        other => panic!("expected file mount, got {:?}", other),
    }
    assert!(mount.alias.is_some(), "alias must be captured");
    if let verum_common::Maybe::Some(alias) = &mount.alias {
        assert_eq!(alias.name.as_str(), "Helper");
    }
}

#[test]
fn test_file_mount_public() {
    let source = "public mount ./api.vr as Api;";
    let mount = extract_mount(source);
    match &mount.tree.kind {
        MountTreeKind::File { path, .. } => {
            assert_eq!(path.as_str(), "./api.vr");
        }
        other => panic!("expected file mount, got {:?}", other),
    }
}

#[test]
fn test_file_mount_nested_directory() {
    let source = "mount ./subdir/nested/util.vr;";
    let mount = extract_mount(source);
    match &mount.tree.kind {
        MountTreeKind::File { path, .. } => {
            assert_eq!(path.as_str(), "./subdir/nested/util.vr");
        }
        other => panic!("expected file mount, got {:?}", other),
    }
}

#[test]
fn test_file_mount_chained_parent() {
    let source = "mount ../../shared/lib/util.vr;";
    let mount = extract_mount(source);
    match &mount.tree.kind {
        MountTreeKind::File { path, .. } => {
            assert_eq!(path.as_str(), "../../shared/lib/util.vr");
        }
        other => panic!("expected file mount, got {:?}", other),
    }
}

#[test]
fn test_file_mount_rejects_non_vr_extension() {
    // `.txt` (or any non-`.vr`) → parse error.
    let source = "mount ./data.txt;";
    let result = parse(source);
    assert!(result.is_err(), "non-`.vr` file mount must be rejected");
}

#[test]
fn test_file_mount_rejects_traversal_escape() {
    // `./a/../../escape.vr` reduces to `../escape.vr` after
    // `..` cancels `a` then escapes — must be rejected at
    // parse time, not at the loader.
    let source = "mount ./a/../../escape.vr;";
    let result = parse(source);
    assert!(
        result.is_err(),
        "file mount that escapes the source directory must be rejected at parse time"
    );
}

#[test]
fn test_file_mount_rejects_directory_terminus() {
    // Path that ends at `..` instead of a `.vr` file.
    let source = "mount ./..;";
    let result = parse(source);
    assert!(result.is_err());
}

#[test]
fn test_file_mount_distinguished_from_relative_module_path() {
    // `mount .config.X;` (relative module path) must still
    // parse as a Path, NOT as a file mount — the `Slash`
    // disambiguator after the leading dot is what triggers
    // file-mount routing.
    let source = "mount .config.KubeConfig;";
    let mount = extract_mount(source);
    match &mount.tree.kind {
        MountTreeKind::Path(_) => {}
        other => panic!(
            "relative module path must NOT route to File variant, got {:?}",
            other
        ),
    }
}
