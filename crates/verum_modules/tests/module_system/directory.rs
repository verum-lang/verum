//! Directory module tests.
//!
//! Tests directory-based module structures with mod.vr files
//! and child modules in subdirectories.
//!
//! Tests directory-based modules (foo/mod.vr), mixed hierarchy (file + directory),
//! and module tree organization with public/private child modules.

use std::fs;
use tempfile::TempDir;
use verum_modules::*;

struct TestProject {
    temp_dir: TempDir,
}

impl TestProject {
    fn new() -> Self {
        Self {
            temp_dir: TempDir::new().unwrap(),
        }
    }

    fn create_file(&self, path: &str, content: &str) {
        let full_path = self.temp_dir.path().join(path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full_path, content).unwrap();
    }

    fn root_path(&self) -> &std::path::Path {
        self.temp_dir.path()
    }
}

#[test]
fn test_directory_module_with_mod_file() {
    // Directory-based module: foo/mod.vr defines module foo with child modules in foo/
    let project = TestProject::new();

    // Create directory structure:
    // src/
    //   utils/
    //     mod.vr
    //     string.vr
    //     math.vr

    project.create_file(
        "utils/mod.vr",
        r#"
public module string;
public module math;

public import string.format;
public import math.add;
"#,
    );

    project.create_file(
        "utils/string.vr",
        r#"
public fn format(s: &str) -> Text {
    Text.from(s)
}
"#,
    );

    project.create_file(
        "utils/math.vr",
        r#"
public fn add(a: Int, b: Int) -> Int {
    a + b
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    // Load utils module (should load utils/mod.vr)
    let utils_path = ModulePath::from_str("utils");
    let utils = loader.load_module(&utils_path, ModuleId::new(1));
    assert!(
        utils.is_ok(),
        "Failed to load utils module: {:?}",
        utils.err()
    );
    let utils = utils.unwrap();
    assert!(utils.source.as_str().contains("public module string"));
    assert!(utils.source.as_str().contains("public module math"));

    // Load child modules
    let string_path = ModulePath::from_str("utils.string");
    let string = loader.load_module(&string_path, ModuleId::new(2));
    assert!(string.is_ok(), "Failed to load string module");

    let math_path = ModulePath::from_str("utils.math");
    let math = loader.load_module(&math_path, ModuleId::new(3));
    assert!(math.is_ok(), "Failed to load math module");
}

#[test]
fn test_nested_directory_modules() {
    // Mixed hierarchy: files and directories coexist (foo.vr + foo/ with children)
    let project = TestProject::new();

    // Create nested structure:
    // network/
    //   mod.vr
    //   tcp.vr
    //   udp/
    //     mod.vr
    //     socket.vr
    //     stream.vr

    project.create_file(
        "network/mod.vr",
        r#"
public module tcp;
public module udp;

public import tcp.TcpStream;
public import udp.UdpSocket;
"#,
    );

    project.create_file(
        "network/tcp.vr",
        r#"
public type TcpStream is {
    public fd: Int,
}
"#,
    );

    project.create_file(
        "network/udp/mod.vr",
        r#"
module socket;
module stream;

public import socket.UdpSocket;
public import stream.UdpStream;
"#,
    );

    project.create_file(
        "network/udp/socket.vr",
        r#"
public type UdpSocket is {
    public fd: Int,
}
"#,
    );

    project.create_file(
        "network/udp/stream.vr",
        r#"
public type UdpStream is {
    public socket: UdpSocket,
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    // Load top-level network module
    let network = loader
        .load_module(&ModulePath::from_str("network"), ModuleId::new(1))
        .unwrap();
    assert!(network.source.as_str().contains("public module tcp"));
    assert!(network.source.as_str().contains("public module udp"));

    // Load tcp module
    let tcp = loader
        .load_module(&ModulePath::from_str("network.tcp"), ModuleId::new(2))
        .unwrap();
    assert!(tcp.source.as_str().contains("public type TcpStream"));

    // Load nested udp module
    let udp = loader
        .load_module(&ModulePath::from_str("network.udp"), ModuleId::new(3))
        .unwrap();
    assert!(udp.source.as_str().contains("module socket"));

    // Load deep child modules
    let socket = loader
        .load_module(
            &ModulePath::from_str("network.udp.socket"),
            ModuleId::new(4),
        )
        .unwrap();
    assert!(socket.source.as_str().contains("public type UdpSocket"));
}

#[test]
fn test_module_file_vs_directory() {
    // Test that foo.vr takes precedence over foo/mod.vr
    // File system mapping: lib.vr/main.vr = root, foo.vr = module foo, foo/mod.vr = directory module
    let project = TestProject::new();

    // Create both foo.vr and foo/mod.vr
    project.create_file(
        "foo.vr",
        r#"
// File-based module
public type FileModule is {
    value: Int,
}
"#,
    );

    project.create_file(
        "foo/mod.vr",
        r#"
// Directory-based module (should be ignored if foo.vr exists)
public type DirectoryModule is {
    value: Int,
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    // Load foo module - should get foo.vr
    let foo = loader
        .load_module(&ModulePath::from_str("foo"), ModuleId::new(1))
        .unwrap();

    // Should contain FileModule, not DirectoryModule
    assert!(foo.source.as_str().contains("type FileModule"));
    // Note: Current implementation may not strictly enforce this precedence,
    // but the spec suggests foo.vr should be preferred
}

#[test]
fn test_sibling_modules_in_directory() {
    // Test multiple sibling modules in same directory
    let project = TestProject::new();

    project.create_file(
        "parser/mod.vr",
        r#"
public module lexer;
public module ast;
public module grammar;
"#,
    );

    project.create_file(
        "parser/lexer.vr",
        r#"
public type Token is {
    kind: Int,
}
"#,
    );

    project.create_file(
        "parser/ast.vr",
        r#"
public type Expr is {
    value: Int,
}
"#,
    );

    project.create_file(
        "parser/grammar.vr",
        r#"
public fn parse() -> Bool {
    true
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    // Load all modules
    let parser = loader
        .load_module(&ModulePath::from_str("parser"), ModuleId::new(1))
        .unwrap();
    assert!(parser.source.as_str().contains("public module lexer"));
    assert!(parser.source.as_str().contains("public module ast"));

    let lexer = loader
        .load_module(&ModulePath::from_str("parser.lexer"), ModuleId::new(2))
        .unwrap();
    assert!(lexer.source.as_str().contains("public type Token"));

    let ast = loader
        .load_module(&ModulePath::from_str("parser.ast"), ModuleId::new(3))
        .unwrap();
    assert!(ast.source.as_str().contains("public type Expr"));

    let grammar = loader
        .load_module(&ModulePath::from_str("parser.grammar"), ModuleId::new(4))
        .unwrap();
    assert!(grammar.source.as_str().contains("public fn parse"));
}

#[test]
fn test_deep_directory_hierarchy() {
    // Test deeply nested directory structure
    let project = TestProject::new();

    // Create: a/b/c/d/mod.vr
    project.create_file(
        "a/b/c/d/mod.vr",
        r#"
public type DeepType is {
    value: Int,
}
"#,
    );

    project.create_file(
        "a/b/c/mod.vr",
        r#"
public module d;
"#,
    );

    project.create_file(
        "a/b/mod.vr",
        r#"
public module c;
"#,
    );

    project.create_file(
        "a/mod.vr",
        r#"
public module b;
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    // Load deepest module
    let deep = loader
        .load_module(&ModulePath::from_str("a.b.c.d"), ModuleId::new(4))
        .unwrap();
    assert!(deep.source.as_str().contains("public type DeepType"));

    // Load parent modules
    let c = loader
        .load_module(&ModulePath::from_str("a.b.c"), ModuleId::new(3))
        .unwrap();
    assert!(c.source.as_str().contains("public module d"));

    let b = loader
        .load_module(&ModulePath::from_str("a.b"), ModuleId::new(2))
        .unwrap();
    assert!(b.source.as_str().contains("public module c"));

    let a = loader
        .load_module(&ModulePath::from_str("a"), ModuleId::new(1))
        .unwrap();
    assert!(a.source.as_str().contains("public module b"));
}

#[test]
fn test_relative_imports_in_directory_modules() {
    // Relative paths: self.child, super.sibling, super.super.uncle navigate the module hierarchy
    let project = TestProject::new();

    project.create_file(
        "network/mod.vr",
        r#"
module tcp;
module udp;
"#,
    );

    project.create_file(
        "network/tcp.vr",
        r#"
import super.udp;  // Sibling module

public fn tcp_to_udp() {
    // Use udp module
}
"#,
    );

    project.create_file(
        "network/udp.vr",
        r#"
import super.tcp;  // Sibling module

public fn udp_to_tcp() {
    // Use tcp module
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let tcp = loader
        .load_module(&ModulePath::from_str("network.tcp"), ModuleId::new(1))
        .unwrap();
    assert!(tcp.source.as_str().contains("import super.udp"));

    let udp = loader
        .load_module(&ModulePath::from_str("network.udp"), ModuleId::new(2))
        .unwrap();
    assert!(udp.source.as_str().contains("import super.tcp"));
}

#[test]
fn test_module_path_resolution() {
    // Test ModulePath helper methods
    let path = ModulePath::from_str("network.tcp.TcpStream");

    // Test segments
    assert_eq!(path.segments().len(), 3);
    assert_eq!(path.segments()[0].as_str(), "network");
    assert_eq!(path.segments()[1].as_str(), "tcp");
    assert_eq!(path.segments()[2].as_str(), "TcpStream");

    // Test name
    assert_eq!(path.name().unwrap().as_str(), "TcpStream");

    // Test parent
    let parent = path.parent().unwrap();
    assert_eq!(parent.to_string(), "network.tcp");

    // Test join
    let joined = parent.join("UdpStream");
    assert_eq!(joined.to_string(), "network.tcp.UdpStream");
}

#[test]
fn test_module_hierarchy_is_descendant() {
    let ancestor = ModulePath::from_str("std.collections");
    let descendant = ModulePath::from_str("std.collections.hash.Map");
    let sibling = ModulePath::from_str("std.io");

    assert!(descendant.is_descendant_of(&ancestor));
    assert!(!ancestor.is_descendant_of(&descendant));
    assert!(!sibling.is_descendant_of(&ancestor));
    assert!(!ancestor.is_descendant_of(&sibling));
}
