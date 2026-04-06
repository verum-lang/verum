//! Re-export tests.
//!
//! Tests re-exporting types and functions through module boundaries,
//! including transitive access and API flattening patterns.
//!
//! Tests re-exports (`public import internal.Item`) for API flattening,
//! transitive access, and the re-export pattern for version migration.

use std::fs;
use tempfile::TempDir;
use verum_common::{List, Map, Maybe, Set};
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
fn test_basic_reexport() {
    // Re-exports: `public import internal.Item` makes items available through a different path
    let project = TestProject::new();

    // Internal module with implementation
    project.create_file(
        "internal.vr",
        r#"
public type Implementation is {
    value: Int,
}

public fn create() -> Implementation {
    Implementation { value: 42 }
}
"#,
    );

    // Public API that re-exports
    project.create_file(
        "api.vr",
        r#"
module internal;

public import internal.Implementation;
public import internal.create;
"#,
    );

    // User imports from api, not internal
    project.create_file(
        "app.vr",
        r#"
import crate.api.{Implementation, create};

public fn example() -> Implementation {
    create()
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let internal = loader
        .load_module(&ModulePath::from_str("internal"), ModuleId::new(1))
        .unwrap();
    assert!(
        internal
            .source
            .as_str()
            .contains("public type Implementation")
    );

    let api = loader
        .load_module(&ModulePath::from_str("api"), ModuleId::new(2))
        .unwrap();
    assert!(
        api.source
            .as_str()
            .contains("public import internal.Implementation")
    );

    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(3))
        .unwrap();
    assert!(
        app.source
            .as_str()
            .contains("import crate.api.{Implementation, create}")
    );
}

#[test]
fn test_reexport_with_rename() {
    // Re-export with renaming: `public import internal.Item as PublicName`
    let project = TestProject::new();

    project.create_file(
        "internal.vr",
        r#"
public type InternalType is {
    value: Int,
}
"#,
    );

    project.create_file(
        "lib.vr",
        r#"
module internal;

// Re-export with different name
public import internal.InternalType as PublicInterface;
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import crate.lib.PublicInterface;

public fn example() -> PublicInterface {
    PublicInterface { value: 1 }
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let lib = loader
        .load_module(&ModulePath::from_str("lib"), ModuleId::new(1))
        .unwrap();
    assert!(
        lib.source
            .as_str()
            .contains("public import internal.InternalType as PublicInterface")
    );

    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(2))
        .unwrap();
    assert!(
        app.source
            .as_str()
            .contains("import crate.lib.PublicInterface")
    );
}

#[test]
fn test_transitive_reexport() {
    // Test re-exports through multiple levels
    let project = TestProject::new();

    // Level 1: Original definition
    project.create_file(
        "core.vr",
        r#"
public type CoreType is {
    value: Int,
}
"#,
    );

    // Level 2: First re-export
    project.create_file(
        "middle.vr",
        r#"
import crate.core.CoreType;

public import core.CoreType;
"#,
    );

    // Level 3: Second re-export
    project.create_file(
        "api.vr",
        r#"
import crate.middle.CoreType;

public import middle.CoreType;
"#,
    );

    // User accesses through final re-export
    project.create_file(
        "app.vr",
        r#"
import crate.api.CoreType;

public fn example() -> CoreType {
    CoreType { value: 1 }
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let core = loader
        .load_module(&ModulePath::from_str("core"), ModuleId::new(1))
        .unwrap();
    assert!(core.source.as_str().contains("public type CoreType"));

    let middle = loader
        .load_module(&ModulePath::from_str("middle"), ModuleId::new(2))
        .unwrap();
    assert!(
        middle
            .source
            .as_str()
            .contains("public import core.CoreType")
    );

    let api = loader
        .load_module(&ModulePath::from_str("api"), ModuleId::new(3))
        .unwrap();
    assert!(
        api.source
            .as_str()
            .contains("public import middle.CoreType")
    );

    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(4))
        .unwrap();
    assert!(app.source.as_str().contains("import crate.api.CoreType"));
}

#[test]
fn test_flatten_module_hierarchy() {
    // Flatten module hierarchy: re-export child module items at parent level
    let project = TestProject::new();

    // Deep hierarchy
    project.create_file(
        "collections/hash_map.vr",
        r#"
public type Map<K, V> is {
    data: List<(K, V)>,
}
"#,
    );

    project.create_file(
        "collections/hash_set.vr",
        r#"
public type Set<T> is {
    data: List<T>,
}
"#,
    );

    // Re-export at collections level
    project.create_file(
        "collections/mod.vr",
        r#"
module hash_map;
module hash_set;

// Flatten: users can import from collections directly
public import hash_map.Map;
public import hash_set.Set;
"#,
    );

    project.create_file(
        "app.vr",
        r#"
// Can import directly from collections, not collections.hash_map
import crate.collections.{Map, Set};

public fn example() {
    let map: Map<Int, Text> = Map { data: List.new() };
    let set: Set<Int> = Set { data: List.new() };
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let hash_map = loader
        .load_module(
            &ModulePath::from_str("collections.hash_map"),
            ModuleId::new(1),
        )
        .unwrap();
    assert!(hash_map.source.as_str().contains("public type Map"));

    let collections = loader
        .load_module(&ModulePath::from_str("collections"), ModuleId::new(2))
        .unwrap();
    assert!(
        collections
            .source
            .as_str()
            .contains("public import hash_map.Map")
    );
    assert!(
        collections
            .source
            .as_str()
            .contains("public import hash_set.Set")
    );

    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(3))
        .unwrap();
    assert!(
        app.source
            .as_str()
            .contains("import crate.collections.{Map, Set}")
    );
}

#[test]
fn test_selective_reexport() {
    // Test re-exporting only some items from a module
    let project = TestProject::new();

    project.create_file(
        "internal.vr",
        r#"
public type PublicType is {
    value: Int,
}

public type InternalType is {
    secret: Int,
}

public fn public_fn() -> Int { 1 }
public fn internal_fn() -> Int { 2 }
"#,
    );

    project.create_file(
        "lib.vr",
        r#"
module internal;

// Only re-export public API, hide internal details
public import internal.PublicType;
public import internal.public_fn;

// InternalType and internal_fn are not re-exported
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import crate.lib.{PublicType, public_fn};

// Cannot import InternalType or internal_fn through lib
// import crate.lib.InternalType;  // Would fail

public fn example() -> PublicType {
    PublicType { value: public_fn() }
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let internal = loader
        .load_module(&ModulePath::from_str("internal"), ModuleId::new(1))
        .unwrap();
    assert!(internal.source.as_str().contains("public type PublicType"));
    assert!(
        internal
            .source
            .as_str()
            .contains("public type InternalType")
    );

    let lib = loader
        .load_module(&ModulePath::from_str("lib"), ModuleId::new(2))
        .unwrap();
    assert!(
        lib.source
            .as_str()
            .contains("public import internal.PublicType")
    );
    assert!(
        !lib.source
            .as_str()
            .contains("public import internal.InternalType")
    );

    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(3))
        .unwrap();
    assert!(
        app.source
            .as_str()
            .contains("import crate.lib.{PublicType, public_fn}")
    );
}

#[test]
fn test_reexport_from_subdirectory() {
    // Test re-exporting from nested directory modules
    let project = TestProject::new();

    project.create_file(
        "network/tcp/stream.vr",
        r#"
public type TcpStream is {
    fd: Int,
}
"#,
    );

    project.create_file(
        "network/tcp/mod.vr",
        r#"
module stream;

public import stream.TcpStream;
"#,
    );

    project.create_file(
        "network/mod.vr",
        r#"
public module tcp;

// Re-export TcpStream at network level
public import tcp.TcpStream;
"#,
    );

    project.create_file(
        "app.vr",
        r#"
// Can import from network directly
import crate.network.TcpStream;

public fn example() -> TcpStream {
    TcpStream { fd: 1 }
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let stream = loader
        .load_module(
            &ModulePath::from_str("network.tcp.stream"),
            ModuleId::new(1),
        )
        .unwrap();
    assert!(stream.source.as_str().contains("public type TcpStream"));

    let network = loader
        .load_module(&ModulePath::from_str("network"), ModuleId::new(2))
        .unwrap();
    assert!(
        network
            .source
            .as_str()
            .contains("public import tcp.TcpStream")
    );

    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(3))
        .unwrap();
    assert!(
        app.source
            .as_str()
            .contains("import crate.network.TcpStream")
    );
}

#[test]
fn test_reexport_multiple_items() {
    // Test re-exporting multiple items in one statement
    let project = TestProject::new();

    project.create_file(
        "types.vr",
        r#"
public type User is { id: Int }
public type Post is { title: Text }
public type Comment is { text: Text }
"#,
    );

    project.create_file(
        "models.vr",
        r#"
module types;

// Re-export multiple items
public import types.{User, Post, Comment};
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import crate.models.{User, Post, Comment};

public fn example() {
    let user = User { id: 1 };
    let post = Post { title: "Hello" };
    let comment = Comment { text: "Nice" };
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let models = loader
        .load_module(&ModulePath::from_str("models"), ModuleId::new(1))
        .unwrap();
    assert!(
        models
            .source
            .as_str()
            .contains("public import types.{User, Post, Comment}")
    );

    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(2))
        .unwrap();
    assert!(
        app.source
            .as_str()
            .contains("import crate.models.{User, Post, Comment}")
    );
}

#[test]
fn test_glob_reexport() {
    // Test re-exporting with glob (*)
    let project = TestProject::new();

    project.create_file(
        "prelude.vr",
        r#"
public type List<T> is { data: List<T> }
public type Maybe<T> is | Some(T) | None
public type Result<T, E> is | Ok(T) | Err(E)
"#,
    );

    project.create_file(
        "std.vr",
        r#"
module prelude;

// Re-export everything from prelude
public import prelude.*;
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import crate.std.*;

public fn example() -> Maybe<Int> {
    Maybe.Some(42)
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let std_mod = loader
        .load_module(&ModulePath::from_str("std"), ModuleId::new(1))
        .unwrap();
    assert!(std_mod.source.as_str().contains("public import prelude.*"));

    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(2))
        .unwrap();
    assert!(app.source.as_str().contains("import crate.std.*"));
}
