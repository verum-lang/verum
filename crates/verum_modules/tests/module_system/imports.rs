//! Import pattern tests.
//!
//! Tests various import patterns including glob imports, nested imports,
//! aliasing, and import resolution.
//!
//! Tests glob imports (path.*), nested imports (path.{A, B}), renaming
//! (path.X as Y), relative paths (self/super), name shadowing, and the
//! full path resolution algorithm.

use std::fs;
use tempfile::TempDir;
use verum_common::{Heap, List, Map, Set};
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
fn test_glob_import() {
    // Glob imports: `mount module.*` imports all public items. Can create
    // ambiguity if multiple glob imports define the same name (compile error).
    let project = TestProject::new();

    project.create_file(
        "collections.vr",
        r#"
public type List<T> is { data: List<T> }
public type Map<K, V> is { data: List<(K, V)> }
public type Set<T> is { data: List<T> }
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import crate.collections.*;

public fn example() {
    let list: List<Int> = List { data: List.new() };
    let map: Map<Int, Text> = Map { data: List.new() };
    let set: Set<Int> = Set { data: List.new() };
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let collections = loader
        .load_module(&ModulePath::from_str("collections"), ModuleId::new(1))
        .unwrap();
    assert!(collections.source.as_str().contains("public type List"));
    assert!(collections.source.as_str().contains("public type Map"));
    assert!(collections.source.as_str().contains("public type Set"));

    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(2))
        .unwrap();
    assert!(app.source.as_str().contains("import crate.collections.*"));
}

#[test]
fn test_nested_import_simple() {
    // Multiple item import: `mount module.{A, B, C}` imports specific items.
    let project = TestProject::new();

    project.create_file(
        "io.vr",
        r#"
public type Read is { }
public type Write is { }
public type Error is { }
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import crate.io.{Read, Write, Error};

public fn example() {
    let r: Read = Read { };
    let w: Write = Write { };
    let e: Error = Error { };
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let io = loader
        .load_module(&ModulePath::from_str("io"), ModuleId::new(1))
        .unwrap();
    assert!(io.source.as_str().contains("public type Read"));

    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(2))
        .unwrap();
    assert!(
        app.source
            .as_str()
            .contains("import crate.io.{Read, Write, Error}")
    );
}

#[test]
fn test_nested_import_deep() {
    // Nested imports: `mount std.{collections.{Map, Set}, io.{Read, Write}}`
    // supports arbitrary nesting depth for concise multi-module imports.
    let project = TestProject::new();

    project.create_file(
        "std/collections.vr",
        r#"
public type Map<K, V> is { }
public type Set<T> is { }
"#,
    );

    project.create_file(
        "std/io.vr",
        r#"
public type Read is { }
public type Write is { }
"#,
    );

    project.create_file(
        "std/sync.vr",
        r#"
public type Heap<T> is { }
public type Mutex<T> is { }
"#,
    );

    project.create_file(
        "std/mod.vr",
        r#"
public module collections;
public module io;
public module sync;
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import crate.std.{
    collections.{Map, Set},
    io.{Read, Write},
    sync.{Heap, Mutex},
};

public fn example() {
    let map: Map<Int, Text> = Map { };
    let reader: Read = Read { };
    let heap: Heap<Int> = Heap { };
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let std_mod = loader
        .load_module(&ModulePath::from_str("std"), ModuleId::new(1))
        .unwrap();
    assert!(
        std_mod
            .source
            .as_str()
            .contains("public module collections")
    );

    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(2))
        .unwrap();
    assert!(app.source.as_str().contains("import crate.std"));
}

#[test]
fn test_import_with_alias() {
    // Renaming imports: `mount module.Type as Alias` avoids name conflicts.
    let project = TestProject::new();

    project.create_file(
        "parser.vr",
        r#"
public type Error is { msg: Text }
"#,
    );

    project.create_file(
        "network.vr",
        r#"
public type Error is { code: Int }
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import crate.parser.Error as ParseError;
import crate.network.Error as NetworkError;

public fn handle_parse_error(e: ParseError) { }
public fn handle_network_error(e: NetworkError) { }
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let parser = loader
        .load_module(&ModulePath::from_str("parser"), ModuleId::new(1))
        .unwrap();
    assert!(parser.source.as_str().contains("public type Error"));

    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(2))
        .unwrap();
    assert!(
        app.source
            .as_str()
            .contains("import crate.parser.Error as ParseError")
    );
    assert!(
        app.source
            .as_str()
            .contains("import crate.network.Error as NetworkError")
    );
}

#[test]
fn test_self_import() {
    // Nested with self: `mount io.{self, Read, Write}` imports both the
    // module itself and specific items from it.
    let project = TestProject::new();

    project.create_file(
        "io.vr",
        r#"
public type Read is { }
public type Write is { }
public type Error is { }
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import crate.io.{self, Read, Write};

public fn example() {
    let r: Read = Read { };
    let e: io.Error = io.Error { };
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(1))
        .unwrap();
    assert!(
        app.source
            .as_str()
            .contains("import crate.io.{self, Read, Write}")
    );
}

#[test]
fn test_relative_import_super() {
    // Relative paths: `super` refers to parent module, `super.super` to
    // grandparent. Used for sibling module access within a package.
    let project = TestProject::new();

    project.create_file(
        "network/tcp.vr",
        r#"
public type TcpStream is { }
"#,
    );

    project.create_file(
        "network/udp.vr",
        r#"
import super.tcp;

public fn use_tcp() {
    let stream: tcp.TcpStream = tcp.TcpStream { };
}
"#,
    );

    project.create_file(
        "network/mod.vr",
        r#"
module tcp;
module udp;
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let udp = loader
        .load_module(&ModulePath::from_str("network.udp"), ModuleId::new(1))
        .unwrap();
    assert!(udp.source.as_str().contains("import super.tcp"));
}

#[test]
fn test_relative_import_self() {
    // Relative paths with self: `import self.child_module` imports from a
    // child module of the current module.
    let project = TestProject::new();

    project.create_file(
        "parser/types.vr",
        r#"
public type Expr is { }
"#,
    );

    project.create_file(
        "parser/mod.vr",
        r#"
module types;

import self.types;

public fn example() {
    let expr: types.Expr = types.Expr { };
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let parser = loader
        .load_module(&ModulePath::from_str("parser"), ModuleId::new(1))
        .unwrap();
    assert!(parser.source.as_str().contains("import self.types"));
}

#[test]
fn test_module_path_resolution() {
    // Test path resolution utilities
    let base = ModulePath::from_str("cog.parser.ast");

    // Resolve super
    let relative_super = ModulePath::from_str("super.lexer");
    let resolved = base.resolve(&relative_super).unwrap();
    assert_eq!(resolved.to_string(), "cog.parser.lexer");

    // Resolve self
    let relative_self = ModulePath::from_str("self.types");
    let resolved_self = base.resolve(&relative_self).unwrap();
    assert_eq!(resolved_self.to_string(), "cog.parser.ast.types");

    // Resolve multiple super
    let multi_super = ModulePath::from_str("super.super.types");
    let resolved_multi = base.resolve(&multi_super).unwrap();
    assert_eq!(resolved_multi.to_string(), "cog.types");
}

#[test]
fn test_import_shadowing() {
    // Name shadowing: local bindings shadow all imports. `let Map = "..."` in
    // a scope shadows any imported Map type. Use absolute path to bypass.
    let project = TestProject::new();

    project.create_file(
        "types.vr",
        r#"
public type Map is { }
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import crate.types.Map;

public fn example() {
    // Local binding shadows import
    let Map = "not a type";

    // Can still use absolute path
    let map: crate.types.Map = crate.types.Map { };
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(1))
        .unwrap();
    assert!(app.source.as_str().contains("import crate.types.Map"));
    assert!(app.source.as_str().contains("let Map = \"not a type\""));
}

#[test]
fn test_complex_nested_imports() {
    // Complex nesting: deeply nested import syntax with mixed glob and
    // specific imports, e.g., `import web.{http.{Request, Response}, router.*}`.
    let project = TestProject::new();

    project.create_file(
        "web/http/mod.vr",
        r#"
public type Request is { }
public type Response is { }
public type StatusCode is { }
public type Method is { }
"#,
    );

    project.create_file(
        "web/router.vr",
        r#"
public type Router is { }
public type Route is { }
"#,
    );

    project.create_file(
        "web/middleware.vr",
        r#"
public type Middleware is { }
"#,
    );

    project.create_file(
        "web/mod.vr",
        r#"
public module http;
public module router;
public module middleware;
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import crate.web.{
    http.{Request, Response, StatusCode, Method},
    router.{Router, Route},
    middleware.*,
};

public fn example() {
    let req: Request = Request { };
    let router: Router = Router { };
    let middleware: Middleware = Middleware { };
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(1))
        .unwrap();
    assert!(app.source.as_str().contains("import crate.web"));
}

#[test]
fn test_import_trailing_comma() {
    // Test imports with trailing commas (should be allowed)
    let project = TestProject::new();

    project.create_file(
        "types.vr",
        r#"
public type A is { }
public type B is { }
public type C is { }
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import crate.types.{
    A,
    B,
    C,
};

public fn example() {
    let a: A = A { };
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(1))
        .unwrap();
    assert!(app.source.as_str().contains("import crate.types"));
}

#[test]
fn test_absolute_vs_relative_imports() {
    // Test difference between absolute and relative imports
    let project = TestProject::new();

    project.create_file(
        "utils.vr",
        r#"
public type Helper is { }
"#,
    );

    project.create_file(
        "parser/mod.vr",
        r#"
// Absolute import
import crate.utils.Helper;

// Relative import (if utils was sibling)
// import super.utils.Helper;

public fn example() {
    let h: Helper = Helper { };
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let parser = loader
        .load_module(&ModulePath::from_str("parser"), ModuleId::new(1))
        .unwrap();
    assert!(parser.source.as_str().contains("import crate.utils.Helper"));
}

#[test]
fn test_import_resolution_priority() {
    // Path resolution priority (6 steps): (1) local scope, (2) explicit imports,
    // (3) glob imports, (4) prelude, (5) parent modules, (6) error if ambiguous.
    // Explicit imports take precedence over globs; local bindings shadow all imports.

    let project = TestProject::new();

    project.create_file(
        "types.vr",
        r#"
public type Thing is { }
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import crate.types.Thing;

public fn example() {
    // Import takes precedence
    let x: Thing = Thing { };
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(1))
        .unwrap();
    assert!(app.source.as_str().contains("import crate.types.Thing"));
}
