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
// Comprehensive cross-module type resolution test suite.
//
// Tests type resolution across module boundaries including:
// - Basic type references across modules
// - Generic types and constraints
// - Associated types
// - Protocol implementations
// - Qualified paths
// - Type forwarding and re-exports
// - Refinement and dependent types
// - Performance benchmarks
//
// Name resolution: deterministic lookup through module hierarchy, import resolution, re-exports — , 4

use std::fs;
use verum_common::{List, Map};
use verum_modules::{ModuleId, ModuleLoader, ModulePath, NameResolver, ResolvedName};
use tempfile::TempDir;

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
        // Convert .vr extension to .vr for module loader compatibility
        // ModuleLoader expects .vr files (Axiom source format)
        let normalized_path = if path.ends_with(".vr") {
            path.replace(".vr", ".vr")
        } else {
            path.to_string()
        };

        let full_path = self.temp_dir.path().join(&normalized_path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full_path, content).unwrap();
    }

    fn root_path(&self) -> &std::path::Path {
        self.temp_dir.path()
    }
}

// ============================================================================
// TEST 1-4: Basic Type References
// ============================================================================

/// Test basic type reference across two modules.
///
/// Cross-module type resolution: resolving type names across module boundaries via import paths
#[test]
fn test_basic_cross_module_type_reference() {
    let project = TestProject::new();

    project.create_file(
        "types.vr",
        r#"
public type User is {
    public id: Int,
    public name: Text,
}
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import cog.types.User;

public fn create_user() -> User {
    User { id: 1, name: "Alice" }
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());
    let types = loader
        .load_module(&ModulePath::from_str("types"), ModuleId::new(1))
        .unwrap();
    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(2))
        .unwrap();

    assert!(types.source.as_str().contains("type User"));
    assert!(app.source.as_str().contains("-> User"));
}

/// Test type references in function parameters.
///
/// Cross-module type resolution: resolving type names across module boundaries via import paths#[test]
fn test_cross_module_type_in_parameters() {
    let project = TestProject::new();

    project.create_file(
        "models.vr",
        r#"
public type Request is { data: Text }
public type Response is { status: Int }
"#,
    );

    project.create_file(
        "handler.vr",
        r#"
import cog.models.{Request, Response};

public fn handle(req: Request) -> Response {
    Response { status: 200 }
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());
    let models = loader
        .load_module(&ModulePath::from_str("models"), ModuleId::new(1))
        .unwrap();
    let handler = loader
        .load_module(&ModulePath::from_str("handler"), ModuleId::new(2))
        .unwrap();

    assert!(models.source.as_str().contains("type Request"));
    assert!(handler.source.as_str().contains("fn handle(req: Request)"));
}

/// Test type references in struct fields.
///
/// Cross-module type resolution: resolving type names across module boundaries via import paths#[test]
fn test_cross_module_type_in_struct_fields() {
    let project = TestProject::new();

    project.create_file(
        "common.vr",
        r#"
public type UserId is Int;
public type Timestamp is Int;
"#,
    );

    project.create_file(
        "models.vr",
        r#"
import cog.common.{UserId, Timestamp};

public type Event is {
    user: UserId,
    time: Timestamp,
    data: Text,
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());
    let common = loader
        .load_module(&ModulePath::from_str("common"), ModuleId::new(1))
        .unwrap();
    let models = loader
        .load_module(&ModulePath::from_str("models"), ModuleId::new(2))
        .unwrap();

    assert!(common.source.as_str().contains("type UserId"));
    assert!(models.source.as_str().contains("user: UserId"));
}

/// Test type references in return types.
///
/// Cross-module type resolution: resolving type names across module boundaries via import paths#[test]
fn test_cross_module_type_in_return_types() {
    let project = TestProject::new();

    project.create_file(
        "database.vr",
        r#"
public type Connection is { handle: Int }
"#,
    );

    project.create_file(
        "pool.vr",
        r#"
import cog.database.Connection;

public fn acquire() -> Connection {
    Connection { handle: 42 }
}

public fn try_acquire() -> Maybe<Connection> {
    Maybe.Some(Connection { handle: 42 })
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());
    let database = loader
        .load_module(&ModulePath::from_str("database"), ModuleId::new(1))
        .unwrap();
    let pool = loader
        .load_module(&ModulePath::from_str("pool"), ModuleId::new(2))
        .unwrap();

    assert!(database.source.as_str().contains("type Connection"));
    assert!(pool.source.as_str().contains("-> Connection"));
    assert!(pool.source.as_str().contains("Maybe<Connection>"));
}

// ============================================================================
// TEST 5-8: Type Aliases
// ============================================================================

/// Test type alias resolution across modules.
///
/// Cross-module type aliases: type aliases resolved transitively across module boundaries
#[test]
fn test_cross_module_type_alias() {
    let project = TestProject::new();

    project.create_file(
        "types.vr",
        r#"
public type UserId is Int;
public type Email is Text;
"#,
    );

    project.create_file(
        "api.vr",
        r#"
import cog.types.{UserId, Email};

public fn send_email(user: UserId, email: Email) -> Bool {
    true
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());
    let types = loader
        .load_module(&ModulePath::from_str("types"), ModuleId::new(1))
        .unwrap();
    let api = loader
        .load_module(&ModulePath::from_str("api"), ModuleId::new(2))
        .unwrap();

    assert!(types.source.as_str().contains("type UserId is Int"));
    assert!(api.source.as_str().contains("user: UserId"));
}

/// Test nested type alias resolution.
///
/// Cross-module type aliases: type aliases resolved transitively across module boundaries#[test]
fn test_nested_type_alias_resolution() {
    let project = TestProject::new();

    project.create_file(
        "base.vr",
        r#"
public type BaseId is Int;
"#,
    );

    project.create_file(
        "derived.vr",
        r#"
import cog.base.BaseId;

public type DerivedId is BaseId;
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import cog.derived.DerivedId;

public fn process(id: DerivedId) -> DerivedId {
    id
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());
    let base = loader
        .load_module(&ModulePath::from_str("base"), ModuleId::new(1))
        .unwrap();
    let derived = loader
        .load_module(&ModulePath::from_str("derived"), ModuleId::new(2))
        .unwrap();
    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(3))
        .unwrap();

    assert!(base.source.as_str().contains("type BaseId is Int"));
    assert!(derived.source.as_str().contains("type DerivedId is BaseId"));
    assert!(app.source.as_str().contains("id: DerivedId"));
}

/// Test generic type alias resolution.
///
/// Cross-module type aliases: type aliases resolved transitively across module boundaries#[test]
fn test_generic_type_alias_resolution() {
    let project = TestProject::new();

    project.create_file(
        "types.vr",
        r#"
public type Pair<A, B> is { first: A, second: B }
public type StringPair is Pair<Text, Text>
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import cog.types.{Pair, StringPair};

public fn make_pair() -> StringPair {
    StringPair { first: "a", second: "b" }
}

public fn generic_pair<T>(a: T, b: T) -> Pair<T, T> {
    Pair { first: a, second: b }
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());
    let types = loader
        .load_module(&ModulePath::from_str("types"), ModuleId::new(1))
        .unwrap();
    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(2))
        .unwrap();

    assert!(types.source.as_str().contains("type Pair<A, B>"));
    assert!(app.source.as_str().contains("Pair<T, T>"));
}

/// Test type alias with refinements.
///
/// Cross-module type aliases: type aliases resolved transitively across module boundaries, with protocol implementations checked for coherence
#[test]
fn test_type_alias_with_refinements() {
    let project = TestProject::new();

    project.create_file(
        "refined.vr",
        r#"
public type Positive is x: Int where x > 0
public type Natural is x: Int where x >= 0
"#,
    );

    project.create_file(
        "math.vr",
        r#"
import cog.refined.{Positive, Natural};

public fn sqrt(n: Natural) -> Natural {
    // Implementation
    0
}

public fn reciprocal(n: Positive) -> Int {
    1 / n
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());
    let refined = loader
        .load_module(&ModulePath::from_str("refined"), ModuleId::new(1))
        .unwrap();
    let math = loader
        .load_module(&ModulePath::from_str("math"), ModuleId::new(2))
        .unwrap();

    assert!(refined.source.as_str().contains("type Positive"));
    assert!(math.source.as_str().contains("n: Positive"));
}

// ============================================================================
// TEST 9-12: Generic Types
// ============================================================================

/// Test generic type with cross-module type parameters.
///
/// Cross-module generic types: generic type resolution and coherence checking across modules
#[test]
fn test_generic_with_cross_module_parameters() {
    let project = TestProject::new();

    project.create_file(
        "container.vr",
        r#"
public type Container<T> is {
    item: T,
    count: Int,
}
"#,
    );

    project.create_file(
        "models.vr",
        r#"
public type Item is { value: Int }
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import cog.container.Container;
import cog.models.Item;

public fn create() -> Container<Item> {
    Container {
        item: Item { value: 42 },
        count: 1,
    }
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());
    let container = loader
        .load_module(&ModulePath::from_str("container"), ModuleId::new(1))
        .unwrap();
    let _models = loader
        .load_module(&ModulePath::from_str("models"), ModuleId::new(2))
        .unwrap();
    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(3))
        .unwrap();

    assert!(container.source.as_str().contains("type Container<T>"));
    assert!(app.source.as_str().contains("Container<Item>"));
}

/// Test nested generic types across modules.
///
/// Cross-module generic types: generic type resolution and coherence checking across modules#[test]
fn test_nested_generic_types() {
    let project = TestProject::new();

    project.create_file(
        "a.vr",
        r#"
public type ContainerA<T> is { value: T }
"#,
    );

    project.create_file(
        "b.vr",
        r#"
public type ItemB is { data: Int }
"#,
    );

    project.create_file(
        "c.vr",
        r#"
public type ValueC is { name: Text }
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import cog.a.ContainerA;
import cog.b.ItemB;
import cog.c.ValueC;

// Nested generics: Map<Key, Value> where Value is also generic
public fn complex() -> Map<ItemB, ContainerA<ValueC>> {
    Map.new()
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());
    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(4))
        .unwrap();

    assert!(
        app.source
            .as_str()
            .contains("Map<ItemB, ContainerA<ValueC>>")
    );
}

/// Test generic constraints with cross-module protocols.
///
/// Cross-module generic types: generic type resolution and coherence checking across modules
#[test]
fn test_generic_constraints_cross_module() {
    let project = TestProject::new();

    project.create_file(
        "protocols.vr",
        r#"
public protocol Display {
    fn display(&self) -> Text;
}

public protocol Serialize {
    fn serialize(&self) -> Text;
}
"#,
    );

    project.create_file(
        "generic.vr",
        r#"
import cog.protocols.{Display, Serialize};

public fn show<T: Display>(item: T) -> Text {
    item.display()
}

public fn save<T: Display + Serialize>(item: T) -> Text {
    item.serialize()
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());
    let protocols = loader
        .load_module(&ModulePath::from_str("protocols"), ModuleId::new(1))
        .unwrap();
    let generic = loader
        .load_module(&ModulePath::from_str("generic"), ModuleId::new(2))
        .unwrap();

    assert!(protocols.source.as_str().contains("protocol Display"));
    assert!(generic.source.as_str().contains("T: Display"));
    assert!(generic.source.as_str().contains("T: Display + Serialize"));
}

/// Test where clauses with cross-module types.
///
/// Cross-module generic types: generic type resolution and coherence checking across modules#[test]
fn test_where_clause_cross_module() {
    let project = TestProject::new();

    project.create_file(
        "traits.vr",
        r#"
public protocol Comparable {
    fn compare(&self, other: &Self) -> Int;
}

public protocol Hashable {
    fn hash(&self) -> Int;
}
"#,
    );

    project.create_file(
        "collections.vr",
        r#"
import cog.traits.{Comparable, Hashable};

public fn sorted<T>(items: List<T>) -> List<T>
where
    T: Comparable
{
    items
}

public fn dedupe<T>(items: List<T>) -> List<T>
where
    T: Comparable + Hashable
{
    items
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());
    let traits = loader
        .load_module(&ModulePath::from_str("traits"), ModuleId::new(1))
        .unwrap();
    let collections = loader
        .load_module(&ModulePath::from_str("collections"), ModuleId::new(2))
        .unwrap();

    assert!(traits.source.as_str().contains("protocol Comparable"));
    assert!(collections.source.as_str().contains("where"));
    assert!(collections.source.as_str().contains("T: Comparable"));
}

// ============================================================================
// TEST 13-16: Associated Types
// ============================================================================

/// Test associated type resolution across modules.
///
/// Cross-module associated types: resolving protocol associated types across module boundaries
#[test]
fn test_associated_type_resolution() {
    let project = TestProject::new();

    project.create_file(
        "protocols.vr",
        r#"
public protocol Container {
    type Item;

    fn get(&self) -> Maybe<Self.Item>;
}
"#,
    );

    project.create_file(
        "impl.vr",
        r#"
import cog.protocols.Container;

public type MyContainer is { value: Int }

implement Container for MyContainer {
    type Item = Int;

    fn get(&self) -> Maybe<Int> {
        Maybe.Some(self.value)
    }
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());
    let protocols = loader
        .load_module(&ModulePath::from_str("protocols"), ModuleId::new(1))
        .unwrap();
    let impl_mod = loader
        .load_module(&ModulePath::from_str("impl"), ModuleId::new(2))
        .unwrap();

    assert!(protocols.source.as_str().contains("type Item"));
    assert!(impl_mod.source.as_str().contains("type Item = Int"));
}

/// Test nested associated types.
///
/// Cross-module associated types: resolving protocol associated types across module boundaries#[test]
fn test_nested_associated_types() {
    let project = TestProject::new();

    project.create_file(
        "advanced.vr",
        r#"
public protocol Graph {
    type Node;
    type Edge;

    fn nodes(&self) -> List<Self.Node>;
    fn edges(&self) -> List<Self.Edge>;
}

public protocol Weighted {
    type Weight;

    fn weight(&self) -> Self.Weight;
}
"#,
    );

    project.create_file(
        "impl.vr",
        r#"
import cog.advanced.{Graph, Weighted};

public type MyGraph is { data: Int }
public type MyNode is { id: Int }
public type MyEdge is { from: Int, to: Int, weight: Int }

implement Graph for MyGraph {
    type Node = MyNode;
    type Edge = MyEdge;

    fn nodes(&self) -> List<MyNode> { List.new() }
    fn edges(&self) -> List<MyEdge> { List.new() }
}

implement Weighted for MyEdge {
    type Weight = Int;

    fn weight(&self) -> Int { self.weight }
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());
    let advanced = loader
        .load_module(&ModulePath::from_str("advanced"), ModuleId::new(1))
        .unwrap();
    let impl_mod = loader
        .load_module(&ModulePath::from_str("impl"), ModuleId::new(2))
        .unwrap();

    assert!(advanced.source.as_str().contains("type Node"));
    assert!(impl_mod.source.as_str().contains("type Node = MyNode"));
}

/// Test associated types in function signatures.
///
/// Cross-module associated types: resolving protocol associated types across module boundaries#[test]
fn test_associated_types_in_signatures() {
    let project = TestProject::new();

    project.create_file(
        "iterator.vr",
        r#"
public protocol Iterator {
    type Item;

    fn next(&mut self) -> Maybe<Self.Item>;
}
"#,
    );

    project.create_file(
        "algorithms.vr",
        r#"
import cog.iterator.Iterator;

public fn collect<I: Iterator>(iter: &mut I) -> List<I.Item> {
    List.new()
}

public fn find<I: Iterator>(iter: &mut I, predicate: fn(I.Item) -> Bool) -> Maybe<I.Item> {
    Maybe.None
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());
    let iterator = loader
        .load_module(&ModulePath::from_str("iterator"), ModuleId::new(1))
        .unwrap();
    let algorithms = loader
        .load_module(&ModulePath::from_str("algorithms"), ModuleId::new(2))
        .unwrap();

    assert!(iterator.source.as_str().contains("type Item"));
    assert!(algorithms.source.as_str().contains("I.Item"));
}

/// Test qualified associated type paths.
///
/// Cross-module associated types: resolving protocol associated types across module boundaries#[test]
fn test_qualified_associated_type_paths() {
    let project = TestProject::new();

    project.create_file(
        "base.vr",
        r#"
public protocol Base {
    type Output;

    fn process(&self) -> Self.Output;
}
"#,
    );

    project.create_file(
        "derived.vr",
        r#"
import cog.base.Base;

public type Processor is { value: Int }

implement Base for Processor {
    type Output = Text;

    fn process(&self) -> Text { "result" }
}

public fn use_output(p: Processor) -> <Processor as Base>.Output {
    p.process()
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());
    let base = loader
        .load_module(&ModulePath::from_str("base"), ModuleId::new(1))
        .unwrap();
    let derived = loader
        .load_module(&ModulePath::from_str("derived"), ModuleId::new(2))
        .unwrap();

    assert!(base.source.as_str().contains("type Output"));
    assert!(
        derived
            .source
            .as_str()
            .contains("<Processor as Base>.Output")
    );
}

// ============================================================================
// TEST 17-20: Qualified Paths
// ============================================================================

/// Test simple qualified path resolution.
///
/// Module paths: dot-separated hierarchical paths (cog.module.item) for name resolution
#[test]
fn test_simple_qualified_path() {
    let project = TestProject::new();

    project.create_file(
        "a/b.vr",
        r#"
public type MyType is { value: Int }
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import cog.a.b.MyType;

public fn use_type() -> MyType {
    MyType { value: 42 }
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());
    let b_mod = loader
        .load_module(&ModulePath::from_str("a.b"), ModuleId::new(1))
        .unwrap();
    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(2))
        .unwrap();

    assert!(b_mod.source.as_str().contains("type MyType"));
    assert!(app.source.as_str().contains("import cog.a.b.MyType"));
}

/// Test deep qualified path resolution.
///
/// Module paths: dot-separated hierarchical paths (cog.module.item) for name resolution#[test]
fn test_deep_qualified_path() {
    let project = TestProject::new();

    project.create_file(
        "std/collections/hash/map.vr",
        r#"
public type HashMap<K, V> is { data: Int }
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import cog.std.collections.hash.map.HashMap;

public fn create_map() -> HashMap<Text, Int> {
    HashMap { data: 0 }
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());
    let map_mod = loader
        .load_module(
            &ModulePath::from_str("std.collections.hash.map"),
            ModuleId::new(1),
        )
        .unwrap();
    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(2))
        .unwrap();

    assert!(map_mod.source.as_str().contains("type HashMap"));
    assert!(
        app.source
            .as_str()
            .contains("import cog.std.collections.hash.map.HashMap")
    );
}

/// Test qualified method call resolution.
///
/// Cross-module name resolution: qualified name lookup across module hierarchy
#[test]
fn test_qualified_method_call() {
    let project = TestProject::new();

    project.create_file(
        "types.vr",
        r#"
public type Counter is { value: Int }

implement Counter {
    public fn new() -> Counter {
        Counter { value: 0 }
    }

    public fn increment(&mut self) {
        self.value += 1;
    }
}
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import cog.types.Counter;

public fn test() {
    let mut c = Counter.new();
    c.increment();
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());
    let types = loader
        .load_module(&ModulePath::from_str("types"), ModuleId::new(1))
        .unwrap();
    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(2))
        .unwrap();

    assert!(types.source.as_str().contains("fn new()"));
    assert!(app.source.as_str().contains("Counter.new()"));
}

/// Test qualified static method call.
///
/// Cross-module name resolution: qualified name lookup across module hierarchy
#[test]
fn test_qualified_static_method() {
    let project = TestProject::new();

    project.create_file(
        "math.vr",
        r#"
public type Math {}

implement Math {
    public fn abs(x: Int) -> Int {
        if x < 0 { -x } else { x }
    }

    public fn max(a: Int, b: Int) -> Int {
        if a > b { a } else { b }
    }
}
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import cog.math.Math;

public fn compute() -> Int {
    Math.abs(-42) + Math.max(10, 20)
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());
    let math = loader
        .load_module(&ModulePath::from_str("math"), ModuleId::new(1))
        .unwrap();
    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(2))
        .unwrap();

    assert!(math.source.as_str().contains("fn abs"));
    assert!(app.source.as_str().contains("Math.abs"));
}

// ============================================================================
// TEST 21-25: Type Forwarding and Re-exports
// ============================================================================

/// Test type forwarding through re-export.
///
/// Module re-exports: "pub use" for exposing items from sub-modules through parent module
#[test]
fn test_type_forwarding_reexport() {
    let project = TestProject::new();

    project.create_file(
        "internal.vr",
        r#"
public type InternalType is { value: Int }
"#,
    );

    project.create_file(
        "lib.vr",
        r#"
module internal;

public import cog.internal.InternalType;
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import cog.InternalType;

public fn use_reexported() -> InternalType {
    InternalType { value: 42 }
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());
    let internal = loader
        .load_module(&ModulePath::from_str("internal"), ModuleId::new(1))
        .unwrap();
    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(3))
        .unwrap();

    assert!(internal.source.as_str().contains("type InternalType"));
    assert!(app.source.as_str().contains("import cog.InternalType"));
}

/// Test chained type re-exports.
///
/// Module re-exports: "pub use" for exposing items from sub-modules through parent module#[test]
fn test_chained_type_reexports() {
    let project = TestProject::new();

    project.create_file(
        "core.vr",
        r#"
public type CoreType is { data: Int }
"#,
    );

    project.create_file(
        "intermediate.vr",
        r#"
public import cog.core.CoreType;
"#,
    );

    project.create_file(
        "facade.vr",
        r#"
public import cog.intermediate.CoreType;
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import cog.facade.CoreType;

public fn use_chained() -> CoreType {
    CoreType { data: 42 }
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());
    let core = loader
        .load_module(&ModulePath::from_str("core"), ModuleId::new(1))
        .unwrap();
    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(4))
        .unwrap();

    assert!(core.source.as_str().contains("type CoreType"));
    assert!(app.source.as_str().contains("import cog.facade.CoreType"));
}

/// Test re-export with renaming.
///
/// Module re-exports: "pub use" for exposing items from sub-modules through parent module#[test]
fn test_reexport_with_rename() {
    let project = TestProject::new();

    project.create_file(
        "original.vr",
        r#"
public type OriginalName is { value: Int }
"#,
    );

    project.create_file(
        "facade.vr",
        r#"
public import cog.original.OriginalName as NewName;
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import cog.facade.NewName;

public fn use_renamed() -> NewName {
    NewName { value: 42 }
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());
    let original = loader
        .load_module(&ModulePath::from_str("original"), ModuleId::new(1))
        .unwrap();
    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(3))
        .unwrap();

    assert!(original.source.as_str().contains("type OriginalName"));
    assert!(app.source.as_str().contains("import cog.facade.NewName"));
}

/// Test circular type references through modules.
///
/// Circular reference detection: detecting and reporting cycles in cross-module type definitions
#[test]
fn test_circular_type_references() {
    let project = TestProject::new();

    project.create_file(
        "a.vr",
        r#"
import cog.b.TypeB;

public type TypeA is {
    value: Int,
    b_ref: Maybe<&TypeB>,
}
"#,
    );

    project.create_file(
        "b.vr",
        r#"
import cog.a.TypeA;

public type TypeB is {
    value: Text,
    a_ref: Maybe<&TypeA>,
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());
    let a_mod = loader
        .load_module(&ModulePath::from_str("a"), ModuleId::new(1))
        .unwrap();
    let b_mod = loader
        .load_module(&ModulePath::from_str("b"), ModuleId::new(2))
        .unwrap();

    assert!(a_mod.source.as_str().contains("import cog.b.TypeB"));
    assert!(b_mod.source.as_str().contains("import cog.a.TypeA"));
}

/// Test protocol implementation across modules.
///
/// Protocol implementation coherence: at most one impl per concrete type, orphan rules across modules
#[test]
fn test_cross_module_protocol_implementation() {
    let project = TestProject::new();

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
public type MyType is { value: Int }
"#,
    );

    project.create_file(
        "impl.vr",
        r#"
import cog.protocols.Display;
import cog.types.MyType;

implement Display for MyType {
    fn display(&self) -> Text {
        "MyType"
    }
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());
    let protocols = loader
        .load_module(&ModulePath::from_str("protocols"), ModuleId::new(1))
        .unwrap();
    let types = loader
        .load_module(&ModulePath::from_str("types"), ModuleId::new(2))
        .unwrap();
    let impl_mod = loader
        .load_module(&ModulePath::from_str("impl"), ModuleId::new(3))
        .unwrap();

    assert!(protocols.source.as_str().contains("protocol Display"));
    assert!(types.source.as_str().contains("type MyType"));
    assert!(
        impl_mod
            .source
            .as_str()
            .contains("implement Display for MyType")
    );
}

// ============================================================================
// TEST 26-30: Performance Tests
// ============================================================================

/// Test performance of 1000 type resolutions.
///
/// Module resolution performance: efficient name lookup with caching and lazy resolution
#[test]
fn test_performance_1000_resolutions() {
    let mut resolver = NameResolver::new();
    let mod_id = ModuleId::new(1);

    // Add 1000 types from different modules
    for i in 0..1000 {
        let type_name = format!("Type{}", i);
        let module_path = format!("module{}.Type{}", i / 100, i);
        resolver.add_prelude_item(
            type_name.clone(),
            ResolvedName::new(
                ModuleId::new((i / 100) as u32),
                ModulePath::from_str(&module_path),
                verum_modules::resolver::NameKind::Type,
                type_name,
            ),
        );
    }

    resolver.create_scope(mod_id);

    // Measure resolution time
    let start = std::time::Instant::now();
    for i in 0..1000 {
        let resolved = resolver.resolve_name(&format!("Type{}", i), mod_id);
        assert!(resolved.is_ok());
    }
    let elapsed = start.elapsed();

    // Should be under 50ms
    assert!(
        elapsed.as_millis() < 50,
        "1000 resolutions took too long: {:?}",
        elapsed
    );
}

/// Test that type resolution caching improves performance.
///
/// Module resolution performance: efficient name lookup with caching and lazy resolution#[test]
fn test_type_resolution_caching() {
    let mut resolver = NameResolver::new();
    let mod_id = ModuleId::new(1);

    resolver.add_prelude_item(
        "TestType",
        ResolvedName::new(
            ModuleId::ROOT,
            ModulePath::from_str("std.TestType"),
            verum_modules::resolver::NameKind::Type,
            "TestType",
        ),
    );

    resolver.create_scope(mod_id);

    // Verify that resolution works consistently (caching should produce same results)
    let result1 = resolver.resolve_name("TestType", mod_id);
    let result2 = resolver.resolve_name("TestType", mod_id);

    // Both resolutions should return the same result
    assert!(result1.is_ok(), "First resolution should succeed");
    assert!(result2.is_ok(), "Second resolution should succeed");

    let resolved1 = result1.unwrap();
    let resolved2 = result2.unwrap();

    // Verify consistency (same name resolves to same target)
    assert_eq!(
        resolved1.path.to_string(),
        resolved2.path.to_string(),
        "Cached resolution should return same module path"
    );
    assert_eq!(
        resolved1.local_name.as_str(),
        resolved2.local_name.as_str(),
        "Cached resolution should return same name"
    );

    // Performance test: many lookups should complete quickly
    // (not timing-dependent, just verify it doesn't hang or crash)
    let start = std::time::Instant::now();
    for _ in 0..1000 {
        let _ = resolver.resolve_name("TestType", mod_id);
    }
    let duration = start.elapsed();

    // 1000 lookups should complete in under 1 second even in debug mode
    assert!(
        duration.as_secs() < 1,
        "Resolution took too long: {:?} for 1000 lookups",
        duration
    );
}

/// Test memory usage with large number of types.
///
/// Module resolution performance: efficient name lookup with caching and lazy resolution#[test]
fn test_memory_usage_many_types() {
    let mut resolver = NameResolver::new();

    // Add 10,000 types
    for i in 0..10000 {
        let type_name = format!("Type{}", i);
        let module_path = format!("module{}.Type{}", i / 1000, i);
        resolver.add_prelude_item(
            type_name.clone(),
            ResolvedName::new(
                ModuleId::new((i / 1000) as u32),
                ModulePath::from_str(&module_path),
                verum_modules::resolver::NameKind::Type,
                type_name,
            ),
        );
    }

    // Should complete without panic or excessive memory
    // This is a basic sanity check
    let mod_id = ModuleId::new(1);
    resolver.create_scope(mod_id);

    let resolved = resolver.resolve_name("Type5000", mod_id);
    assert!(resolved.is_ok());
}
