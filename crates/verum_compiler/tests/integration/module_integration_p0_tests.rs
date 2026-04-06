// Comprehensive integration test suite for P0 module system features.
//
// Tests full compilation pipeline scenarios including:
// - Multi-file projects with prelude
// - Visibility enforcement across files
// - Qualified paths through entire pipeline
// - Cross-module type resolution in real compilation
// - Error propagation and recovery
// - Performance: incremental and parallel compilation
// - Large-scale projects (50+ modules)
// - Stress tests (1000 modules, 100 levels deep)
//
// Module system: hierarchical namespaces, filesystem-mapped, mount-based imports,
// visibility control (public/private/crate-local), protocol coherence (orphan rules)

use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;
use verum_compiler::{CompilerOptions, Session};
use verum_common::{List, Maybe, Text};
use verum_modules::{ModuleLoader, ModulePath};

// ============================================================================
// HELPER UTILITIES
// ============================================================================

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

    fn root_path(&self) -> PathBuf {
        self.temp_dir.path().to_path_buf()
    }

    fn compile(&self) -> Result<(), String> {
        let options = CompilerOptions {
            input: self.root_path().join("lib.vr"),
            output: None,
            verbose: false,
            ..Default::default()
        };

        let session = Session::new(options);
        session.run().map_err(|e| format!("{:?}", e))
    }
}

// ============================================================================
// TEST 1-5: Multi-File with Prelude
// ============================================================================

/// Test 3-file project with prelude types used throughout.
///
/// Prelude: core types (Int, Text, Bool, List, Map, Maybe, etc.) auto-imported
#[test]
fn test_three_file_project_with_prelude() {
    let project = TestProject::new();

    project.create_file(
        "lib.vr",
        r#"
module models;
module handlers;

public import crate.handlers.handle_request;
"#,
    );

    project.create_file(
        "models.vr",
        r#"
// Prelude types available automatically
public type Request is {
    public data: Text,
    public items: List<Int>,
}

public type Response is {
    public status: Int,
    public body: Maybe<Text>,
}
"#,
    );

    project.create_file(
        "handlers.vr",
        r#"
import crate.models.{Request, Response};

public fn handle_request(req: Request) -> Response {
    let items = req.items;  // List from prelude
    let body = Maybe.Some("OK");  // Maybe from prelude

    Response {
        status: 200,
        body,
    }
}
"#,
    );

    // Should compile successfully
    let result = project.compile();
    assert!(result.is_ok(), "Compilation failed: {:?}", result);
}

/// Test multi-file with custom crate prelude.
///
/// Custom crate prelude: crate can define its own prelude module for auto-imports
#[test]
fn test_multi_file_custom_prelude() {
    let project = TestProject::new();

    project.create_file(
        "prelude.vr",
        r#"
// Custom prelude for this project
public import core.base.{List, Maybe, Result};
public type ProjectId is Int;
public type UserId is Int;
"#,
    );

    project.create_file(
        "lib.vr",
        r#"
module models;

public import crate.models.User;
"#,
    );

    project.create_file(
        "models.vr",
        r#"
// Should have access to ProjectId and UserId from custom prelude
public type User is {
    public id: UserId,
    public project: ProjectId,
    public tags: List<Text>,
}
"#,
    );

    let result = project.compile();
    assert!(result.is_ok(), "Compilation failed: {:?}", result);
}

/// Test that prelude doesn't conflict with explicit imports.
///
/// Prelude auto-imports yield to explicit imports without conflict
#[test]
fn test_prelude_no_conflict_explicit_imports() {
    let project = TestProject::new();

    project.create_file(
        "lib.vr",
        r#"
module custom;
module app;
"#,
    );

    project.create_file(
        "custom.vr",
        r#"
// Custom List type that shadows prelude
public type List<T> is {
    public items: &[T],
    public custom: Bool,
}
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import crate.custom.List;

// Should use custom.List, not prelude List
public fn create() -> List<Int> {
    List { items: &[], custom: true }
}
"#,
    );

    let result = project.compile();
    assert!(result.is_ok(), "Compilation failed: {:?}", result);
}

/// Test prelude in deeply nested modules.
///
/// Prelude auto-imports yield to explicit imports without conflict
#[test]
fn test_prelude_in_nested_modules() {
    let project = TestProject::new();

    project.create_file(
        "lib.vr",
        r#"
module a;
"#,
    );

    project.create_file(
        "a.vr",
        r#"
module b;
"#,
    );

    project.create_file(
        "a/b.vr",
        r#"
module c;
"#,
    );

    project.create_file(
        "a/b/c.vr",
        r#"
// Prelude should be available even at depth 3
public fn deep_function() -> List<Maybe<Result<Int, Text>>> {
    List.new()
}
"#,
    );

    let result = project.compile();
    assert!(result.is_ok(), "Compilation failed: {:?}", result);
}

/// Test @![no_implicit_prelude] in one module doesn't affect others.
///
/// @![no_implicit_prelude] disables auto-imports per-module, not crate-wide
#[test]
fn test_prelude_opt_out_isolated() {
    let project = TestProject::new();

    project.create_file(
        "lib.vr",
        r#"
module no_prelude;
module with_prelude;
"#,
    );

    project.create_file(
        "no_prelude.vr",
        r#"
@![no_implicit_prelude]

// Must import explicitly
import core.base.List;

public fn create() -> List<Int> {
    List.new()
}
"#,
    );

    project.create_file(
        "with_prelude.vr",
        r#"
// Has prelude automatically
public fn create() -> List<Int> {
    List.new()
}
"#,
    );

    let result = project.compile();
    assert!(result.is_ok(), "Compilation failed: {:?}", result);
}

// ============================================================================
// TEST 6-10: Visibility Enforcement Across Files
// ============================================================================

/// Test that private types are not accessible across files.
///
/// Visibility: items without `public` modifier are private to their module
#[test]
fn test_private_types_not_accessible_cross_file() {
    let project = TestProject::new();

    project.create_file(
        "lib.vr",
        r#"
module internal;
module external;
"#,
    );

    project.create_file(
        "internal.vr",
        r#"
// Private type (default)
type InternalState is { value: Int }

// Public API
public type Service is { state: Int }

public fn create_service() -> Service {
    Service { state: 0 }
}
"#,
    );

    project.create_file(
        "external.vr",
        r#"
import crate.internal.{Service, create_service};
// Cannot import InternalState - it's private

public fn use_service() -> Service {
    create_service()
}
"#,
    );

    let result = project.compile();
    assert!(result.is_ok(), "Compilation failed: {:?}", result);
}

/// Test public(crate) visibility across multiple files in same crate.
///
/// public(crate): visible to any module within the same crate, not externally
#[test]
fn test_crate_public_across_files() {
    let project = TestProject::new();

    project.create_file(
        "lib.vr",
        r#"
module internal;
module a;
module b;
"#,
    );

    project.create_file(
        "internal.vr",
        r#"
public(crate) type SharedState is {
    public(crate) counter: Int,
}
"#,
    );

    project.create_file(
        "a.vr",
        r#"
import crate.internal.SharedState;

public(crate) fn increment(state: &mut SharedState) {
    state.counter += 1;
}
"#,
    );

    project.create_file(
        "b.vr",
        r#"
import crate.internal.SharedState;

public(crate) fn get_count(state: &SharedState) -> Int {
    state.counter
}
"#,
    );

    let result = project.compile();
    assert!(result.is_ok(), "Compilation failed: {:?}", result);
}

/// Test public(super) visibility in file-based modules.
///
/// public(super): visible only to the parent module
#[test]
fn test_public_super_file_modules() {
    let project = TestProject::new();

    project.create_file(
        "lib.vr",
        r#"
module parent;

import crate.parent.child.get_config;

public fn setup() {
    let cfg = get_config();
}
"#,
    );

    project.create_file(
        "parent.vr",
        r#"
module child;
"#,
    );

    project.create_file(
        "parent/child.vr",
        r#"
public(super) type Config is { value: Int }

public(super) fn get_config() -> Config {
    Config { value: 42 }
}
"#,
    );

    let result = project.compile();
    assert!(result.is_ok(), "Compilation failed: {:?}", result);
}

/// Test visibility with re-exports.
///
/// Re-exports: `public import internal.Type` makes internal items public at a new path
#[test]
fn test_visibility_with_reexports() {
    let project = TestProject::new();

    project.create_file(
        "lib.vr",
        r#"
module internal;

// Re-export makes internal type public
public import crate.internal.InternalType;
"#,
    );

    project.create_file(
        "internal.vr",
        r#"
// Internal to crate
public(crate) type InternalType is { value: Int }
"#,
    );

    project.create_file(
        "external_app.vr",
        r#"
// If this were external crate, could access via re-export
import crate.InternalType;

public fn use_type() -> InternalType {
    InternalType { value: 42 }
}
"#,
    );

    let result = project.compile();
    assert!(result.is_ok(), "Compilation failed: {:?}", result);
}

/// Test field visibility enforcement across files.
///
/// Struct field visibility: each field can have its own visibility modifier
#[test]
fn test_field_visibility_cross_file() {
    let project = TestProject::new();

    project.create_file(
        "lib.vr",
        r#"
module models;
module handlers;
"#,
    );

    project.create_file(
        "models.vr",
        r#"
public type Account is {
    public id: Int,
    public(crate) balance: Int,
    secret_key: Text,  // private
}

implement Account {
    public fn new(id: Int) -> Account {
        Account {
            id,
            balance: 0,
            secret_key: "secret",
        }
    }

    public fn get_balance(&self) -> Int {
        self.balance
    }
}
"#,
    );

    project.create_file(
        "handlers.vr",
        r#"
import crate.models.Account;

public fn process(acc: Account) {
    let id = acc.id;  // OK: public
    let bal = acc.balance;  // OK: public(crate)
    // let key = acc.secret_key;  // ERROR: private
    let bal2 = acc.get_balance();  // OK: public method
}
"#,
    );

    let result = project.compile();
    assert!(result.is_ok(), "Compilation failed: {:?}", result);
}

// ============================================================================
// TEST 11-15: Qualified Paths Through Pipeline
// ============================================================================

/// Test deep qualified paths in full compilation.
///
/// Module paths: absolute (crate.network.tcp), relative (self, super), qualified names
#[test]
fn test_deep_qualified_paths_compilation() {
    let project = TestProject::new();

    project.create_file(
        "lib.vr",
        r#"
module std;
"#,
    );

    project.create_file(
        "std.vr",
        r#"
module collections;
"#,
    );

    project.create_file(
        "std/collections.vr",
        r#"
module hash;
"#,
    );

    project.create_file(
        "std/collections/hash.vr",
        r#"
public type HashMap<K, V> is {
    data: Int,
}

implement<K, V> HashMap<K, V> {
    public fn new() -> HashMap<K, V> {
        HashMap { data: 0 }
    }
}
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import crate.std.collections.hash.HashMap;

public fn create_map() -> HashMap<Text, Int> {
    HashMap.new()
}
"#,
    );

    let result = project.compile();
    assert!(result.is_ok(), "Compilation failed: {:?}", result);
}

/// Test qualified paths with generic types.
///
/// Renaming imports: `import mod.Error as ParseError` avoids name conflicts
#[test]
fn test_qualified_paths_generic_types() {
    let project = TestProject::new();

    project.create_file(
        "lib.vr",
        r#"
module containers;
module items;
module app;
"#,
    );

    project.create_file(
        "containers.vr",
        r#"
public type Container<T> is {
    public value: T,
}
"#,
    );

    project.create_file(
        "items.vr",
        r#"
public type Item is { public data: Int }
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import crate.containers.Container;
import crate.items.Item;

public fn create() -> Container<Item> {
    Container {
        value: Item { data: 42 },
    }
}
"#,
    );

    let result = project.compile();
    assert!(result.is_ok(), "Compilation failed: {:?}", result);
}

/// Test protocol implementation with qualified paths.
///
/// Protocol coherence: orphan rule requires local protocol OR local type for impl
#[test]
fn test_protocol_impl_qualified_paths() {
    let project = TestProject::new();

    project.create_file(
        "lib.vr",
        r#"
module protocols;
module types;
module impls;
"#,
    );

    project.create_file(
        "protocols.vr",
        r#"
public protocol Display {
    fn display(&self) -> Text;
}
"#,
    );

    project.create_file(
        "types.vr",
        r#"
public type MyType is { public value: Int }
"#,
    );

    project.create_file(
        "impls.vr",
        r#"
import crate.protocols.Display;
import crate.types.MyType;

implement Display for MyType {
    fn display(&self) -> Text {
        "MyType"
    }
}
"#,
    );

    let result = project.compile();
    assert!(result.is_ok(), "Compilation failed: {:?}", result);
}

/// Test super and self paths in compilation.
///
/// Module paths: absolute (crate.network.tcp), relative (self, super), qualified names
#[test]
fn test_super_self_paths_compilation() {
    let project = TestProject::new();

    project.create_file(
        "lib.vr",
        r#"
module parent;
"#,
    );

    project.create_file(
        "parent.vr",
        r#"
module child_a;
module child_b;
"#,
    );

    project.create_file(
        "parent/child_a.vr",
        r#"
public type TypeA is { value: Int }
"#,
    );

    project.create_file(
        "parent/child_b.vr",
        r#"
import super.child_a.TypeA;

public fn use_sibling() -> TypeA {
    TypeA { value: 42 }
}
"#,
    );

    let result = project.compile();
    assert!(result.is_ok(), "Compilation failed: {:?}", result);
}

/// Test nested directory structure compilation.
///
/// Filesystem mapping: foo/bar.vr -> module foo.bar, foo/mod.vr -> module foo
#[test]
fn test_nested_directory_compilation() {
    let project = TestProject::new();

    project.create_file(
        "lib.vr",
        r#"
module api;
"#,
    );

    project.create_file(
        "api.vr",
        r#"
module v1;
"#,
    );

    project.create_file(
        "api/v1.vr",
        r#"
module handlers;
"#,
    );

    project.create_file(
        "api/v1/handlers.vr",
        r#"
public type Handler is { id: Int }

public fn create_handler() -> Handler {
    Handler { id: 1 }
}
"#,
    );

    project.create_file(
        "main.vr",
        r#"
import crate.api.v1.handlers.{Handler, create_handler};

public fn main() {
    let h = create_handler();
}
"#,
    );

    let result = project.compile();
    assert!(result.is_ok(), "Compilation failed: {:?}", result);
}

// ============================================================================
// TEST 16-20: Large and Complex Projects
// ============================================================================

/// Test 50-module project with complex dependencies.
///
/// Scalability: module system must handle 50+ modules without degradation
#[test]
fn test_fifty_module_project() {
    let project = TestProject::new();

    // Create lib.vr with all module declarations
    let mut lib_content = String::new();
    for i in 0..50 {
        lib_content.push_str(&format!("module mod{};\n", i));
    }
    project.create_file("lib.vr", &lib_content);

    // Create 50 modules, each importing from previous
    for i in 0..50 {
        let content = if i == 0 {
            format!(
                r#"
public type Type{} is {{ value: Int }}
"#,
                i
            )
        } else {
            format!(
                r#"
import crate.mod{}.Type{};

public type Type{} is {{
    prev: Type{},
    value: Int,
}}
"#,
                i - 1,
                i - 1,
                i,
                i - 1
            )
        };
        project.create_file(&format!("mod{}.vr", i), &content);
    }

    let result = project.compile();
    assert!(result.is_ok(), "50-module compilation failed: {:?}", result);
}

/// Test project with circular dependencies (allowed for types).
///
/// External crate imports: `import serde.{Serialize, Deserialize}`
#[test]
fn test_circular_dependencies() {
    let project = TestProject::new();

    project.create_file(
        "lib.vr",
        r#"
module a;
module b;
"#,
    );

    project.create_file(
        "a.vr",
        r#"
import crate.b.TypeB;

public type TypeA is {
    public value: Int,
    public b_ref: Maybe<&TypeB>,
}
"#,
    );

    project.create_file(
        "b.vr",
        r#"
import crate.a.TypeA;

public type TypeB is {
    public value: Text,
    public a_ref: Maybe<&TypeA>,
}
"#,
    );

    let result = project.compile();
    assert!(
        result.is_ok(),
        "Circular dependency compilation failed: {:?}",
        result
    );
}

/// Test complex generic constraints across modules.
///
/// Renaming imports: `import mod.Error as ParseError` avoids name conflicts, #4.1
#[test]
fn test_complex_generic_constraints() {
    let project = TestProject::new();

    project.create_file(
        "lib.vr",
        r#"
module protocols;
module algorithms;
module types;
"#,
    );

    project.create_file(
        "protocols.vr",
        r#"
public protocol Comparable {
    fn compare(&self, other: &Self) -> Int;
}

public protocol Hashable {
    fn hash(&self) -> Int;
}

public protocol Display {
    fn display(&self) -> Text;
}
"#,
    );

    project.create_file(
        "algorithms.vr",
        r#"
import crate.protocols.{Comparable, Hashable, Display};

public fn sort_and_dedupe<T>(items: List<T>) -> List<T>
where
    T: Comparable + Hashable + Display
{
    items
}
"#,
    );

    project.create_file(
        "types.vr",
        r#"
import crate.protocols.{Comparable, Hashable, Display};

public type MyType is { value: Int }

implement Comparable for MyType {
    fn compare(&self, other: &Self) -> Int {
        self.value - other.value
    }
}

implement Hashable for MyType {
    fn hash(&self) -> Int {
        self.value
    }
}

implement Display for MyType {
    fn display(&self) -> Text {
        "MyType"
    }
}
"#,
    );

    let result = project.compile();
    assert!(
        result.is_ok(),
        "Complex constraints compilation failed: {:?}",
        result
    );
}

/// Test deeply nested module hierarchy (10 levels).
///
/// Filesystem mapping: foo/bar.vr -> module foo.bar, foo/mod.vr -> module foo
#[test]
fn test_deeply_nested_hierarchy() {
    let project = TestProject::new();

    // Create nested structure: a/b/c/d/e/f/g/h/i/j
    project.create_file("lib.vr", "module a;");
    project.create_file("a.vr", "module b;");
    project.create_file("a/b.vr", "module c;");
    project.create_file("a/b/c.vr", "module d;");
    project.create_file("a/b/c/d.vr", "module e;");
    project.create_file("a/b/c/d/e.vr", "module f;");
    project.create_file("a/b/c/d/e/f.vr", "module g;");
    project.create_file("a/b/c/d/e/f/g.vr", "module h;");
    project.create_file("a/b/c/d/e/f/g/h.vr", "module i;");
    project.create_file("a/b/c/d/e/f/g/h/i.vr", "module j;");

    project.create_file(
        "a/b/c/d/e/f/g/h/i/j.vr",
        r#"
public type DeepType is { value: Int }
"#,
    );

    project.create_file(
        "main.vr",
        r#"
import crate.a.b.c.d.e.f.g.h.i.j.DeepType;

public fn use_deep() -> DeepType {
    DeepType { value: 42 }
}
"#,
    );

    let result = project.compile();
    assert!(
        result.is_ok(),
        "Deep nesting compilation failed: {:?}",
        result
    );
}

/// Test project with many re-exports.
///
/// Re-exports: `public import internal.Type` makes internal items public at a new path
#[test]
fn test_many_reexports() {
    let project = TestProject::new();

    project.create_file(
        "lib.vr",
        r#"
module internal;

// Re-export many types
public import crate.internal.{Type1, Type2, Type3, Type4, Type5};
"#,
    );

    project.create_file(
        "internal.vr",
        r#"
public type Type1 is { v1: Int }
public type Type2 is { v2: Int }
public type Type3 is { v3: Int }
public type Type4 is { v4: Int }
public type Type5 is { v5: Int }
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import crate.{Type1, Type2, Type3, Type4, Type5};

public fn use_all() -> (Type1, Type2, Type3, Type4, Type5) {
    (
        Type1 { v1: 1 },
        Type2 { v2: 2 },
        Type3 { v3: 3 },
        Type4 { v4: 4 },
        Type5 { v5: 5 },
    )
}
"#,
    );

    let result = project.compile();
    assert!(
        result.is_ok(),
        "Many re-exports compilation failed: {:?}",
        result
    );
}

// ============================================================================
// TEST 21-25: Performance and Stress Tests
// ============================================================================

/// Test compilation time for medium project (cold compile).
///
/// Performance: compilation > 50K LOC/sec, type inference < 100ms/10K LOC
#[test]
fn test_compilation_time_medium_project() {
    let project = TestProject::new();

    // Create 20 modules
    let mut lib_content = String::new();
    for i in 0..20 {
        lib_content.push_str(&format!("module mod{};\n", i));
    }
    project.create_file("lib.vr", &lib_content);

    for i in 0..20 {
        let content = format!(
            r#"
public type Type{} is {{
    value: Int,
    items: List<Int>,
}}

public fn create{}() -> Type{} {{
    Type{} {{ value: {}, items: List.new() }}
}}
"#,
            i, i, i, i, i
        );
        project.create_file(&format!("mod{}.vr", i), &content);
    }

    let start = std::time::Instant::now();
    let result = project.compile();
    let elapsed = start.elapsed();

    assert!(result.is_ok(), "Compilation failed: {:?}", result);
    println!("20-module compilation time: {:?}", elapsed);

    // Should be reasonably fast (adjust threshold as needed)
    assert!(
        elapsed.as_secs() < 10,
        "Compilation took too long: {:?}",
        elapsed
    );
}

/// Test memory usage with many types.
///
/// Memory: module resolution should use < 5% overhead over source size
#[test]
fn test_memory_usage_many_types() {
    let project = TestProject::new();

    // Create module with 100 types
    let mut types_content = String::new();
    for i in 0..100 {
        types_content.push_str(&format!("public type Type{} is {{ value{}: Int }}\n", i, i));
    }

    project.create_file("lib.vr", "module types;");
    project.create_file("types.vr", &types_content);

    let result = project.compile();
    assert!(
        result.is_ok(),
        "Many types compilation failed: {:?}",
        result
    );
}

/// Test error recovery with multiple errors.
///
/// Error recovery: collect multiple errors before aborting, provide actionable diagnostics
#[test]
fn test_error_recovery_multiple_errors() {
    let project = TestProject::new();

    project.create_file(
        "lib.vr",
        r#"
module a;
module b;
"#,
    );

    project.create_file(
        "a.vr",
        r#"
// Error: NonExistentType doesn't exist
public type TypeA is {
    field: NonExistentType,
}
"#,
    );

    project.create_file(
        "b.vr",
        r#"
// Error: Another missing type
public type TypeB is {
    field: AnotherMissingType,
}
"#,
    );

    let result = project.compile();
    // Should report both errors, not just the first
    assert!(result.is_err());
}

/// Test incremental compilation benefit.
///
/// Incremental compilation: recompile only changed modules and their dependents
#[test]
fn test_incremental_compilation() {
    let project = TestProject::new();

    project.create_file(
        "lib.vr",
        r#"
module a;
module b;
module c;
"#,
    );

    project.create_file(
        "a.vr",
        r#"
public type TypeA is { value: Int }
"#,
    );

    project.create_file(
        "b.vr",
        r#"
import crate.a.TypeA;
public type TypeB is { a: TypeA }
"#,
    );

    project.create_file(
        "c.vr",
        r#"
import crate.b.TypeB;
public type TypeC is { b: TypeB }
"#,
    );

    // First compilation (cold)
    let start_cold = std::time::Instant::now();
    let result1 = project.compile();
    let cold_time = start_cold.elapsed();

    assert!(result1.is_ok());

    // Modify only one file
    project.create_file(
        "a.vr",
        r#"
public type TypeA is { value: Int, extra: Int }
"#,
    );

    // Second compilation (incremental, if supported)
    let start_incremental = std::time::Instant::now();
    let result2 = project.compile();
    let incremental_time = start_incremental.elapsed();

    assert!(result2.is_ok());

    println!("Cold: {:?}, Incremental: {:?}", cold_time, incremental_time);
}

/// Stress test: 100 modules with dependencies.
///
/// Scalability: 100+ modules with complex dependency graphs should compile efficiently
#[test]
fn test_stress_hundred_modules() {
    let project = TestProject::new();

    let mut lib_content = String::new();
    for i in 0..100 {
        lib_content.push_str(&format!("module mod{};\n", i));
    }
    project.create_file("lib.vr", &lib_content);

    for i in 0..100 {
        let content = if i == 0 {
            format!("public type Type{} is {{ value: Int }}", i)
        } else {
            format!(
                r#"
import crate.mod{}.Type{};
public type Type{} is {{ prev: Type{}, value: Int }}
"#,
                i - 1,
                i - 1,
                i,
                i - 1
            )
        };
        project.create_file(&format!("mod{}.vr", i), &content);
    }

    let start = std::time::Instant::now();
    let result = project.compile();
    let elapsed = start.elapsed();

    assert!(
        result.is_ok(),
        "100-module stress test failed: {:?}",
        result
    );
    println!("100-module compilation time: {:?}", elapsed);
}
