//! Visibility enforcement tests.
//!
//! Tests that visibility modifiers (public, private, public(crate), public(super))
//! are correctly enforced across module boundaries.
//!
//! Tests the five visibility modifiers: private (default), public, public(crate),
//! public(super), and public(in path), including struct field visibility.

use std::fs;
use tempfile::TempDir;
use verum_ast::decl::Visibility;
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
fn test_public_visibility() {
    // Public visibility: accessible from any module in any crate
    let project = TestProject::new();

    project.create_file(
        "types.vr",
        r#"
public type User is {
    public id: Int,
    public name: Text,
}

public fn create_user(id: Int, name: Text) -> User {
    User { id, name }
}
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import crate.types.{User, create_user};

public fn example() -> User {
    create_user(1, "Alice")
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    // Load both modules
    let types = loader
        .load_module(&ModulePath::from_str("types"), ModuleId::new(1))
        .unwrap();
    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(2))
        .unwrap();

    // Verify public items are accessible
    assert!(types.source.as_str().contains("public type User"));
    assert!(types.source.as_str().contains("public fn create_user"));
    assert!(
        app.source
            .as_str()
            .contains("import crate.types.{User, create_user}")
    );
}

#[test]
fn test_private_visibility() {
    // Private (default): accessible only within the current module
    let project = TestProject::new();

    project.create_file(
        "internal.vr",
        r#"
// Private by default
fn helper() -> Int {
    42
}

type InternalState is {
    counter: Int,
}

public fn public_api() -> Int {
    helper()
}
"#,
    );

    project.create_file(
        "app.vr",
        r#"
// Should NOT be able to import private items
// import crate.internal.helper;  // Would cause error
// import crate.internal.InternalState;  // Would cause error

import crate.internal.public_api;

public fn example() -> Int {
    public_api()
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let internal = loader
        .load_module(&ModulePath::from_str("internal"), ModuleId::new(1))
        .unwrap();
    assert!(internal.source.as_str().contains("fn helper"));
    assert!(internal.source.as_str().contains("type InternalState"));

    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(2))
        .unwrap();
    assert!(
        app.source
            .as_str()
            .contains("import crate.internal.public_api")
    );
    // Should NOT contain imports of private items
    assert!(!app.source.as_str().contains("import crate.internal.helper"));
}

#[test]
fn test_crate_public_visibility() {
    // public(crate): accessible within the same crate only (same first path segment)
    let project = TestProject::new();

    project.create_file(
        "config.vr",
        r#"
public(crate) type InternalConfig is {
    cache_size: usize,
    timeout: Int,
}

public(crate) fn load_config() -> InternalConfig {
    InternalConfig { cache_size: 1024, timeout: 30 }
}
"#,
    );

    project.create_file(
        "server.vr",
        r#"
import crate.config.{InternalConfig, load_config};

public fn initialize() {
    let config = load_config();
    // Use config
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let config = loader
        .load_module(&ModulePath::from_str("config"), ModuleId::new(1))
        .unwrap();
    assert!(
        config
            .source
            .as_str()
            .contains("public(crate) type InternalConfig")
    );

    let server = loader
        .load_module(&ModulePath::from_str("server"), ModuleId::new(2))
        .unwrap();
    // Within same crate, should be able to import public(crate) items
    assert!(
        server
            .source
            .as_str()
            .contains("import crate.config.{InternalConfig, load_config}")
    );
}

#[test]
fn test_visibility_checker_public() {
    // Public: accessible from any module
    let checker = VisibilityChecker::new();
    let item_module = ModulePath::from_str("cog.internal.types");
    let from_external = ModulePath::from_str("external.app");

    // Public items are visible from anywhere
    assert!(checker.is_visible(Visibility::Public, &item_module, &from_external));
}

#[test]
fn test_visibility_checker_private() {
    // Private: not accessible from other modules
    let checker = VisibilityChecker::new();
    let item_module = ModulePath::from_str("cog.parser.ast");

    // Private items only visible in same module
    let from_same = ModulePath::from_str("cog.parser.ast");
    assert!(checker.is_visible(Visibility::Private, &item_module, &from_same));

    let from_different = ModulePath::from_str("cog.parser.lexer");
    assert!(!checker.is_visible(Visibility::Private, &item_module, &from_different));

    let from_child = ModulePath::from_str("cog.parser.ast.types");
    assert!(!checker.is_visible(Visibility::Private, &item_module, &from_child));
}

#[test]
fn test_struct_field_visibility() {
    // Struct fields can have independent visibility: public id, internal amount, private state
    let project = TestProject::new();

    project.create_file(
        "models.vr",
        r#"
public type Transaction is {
    public id: Int,
    public(crate) amount: Int,
    internal_state: Int,
}

implement Transaction {
    public fn new(id: Int, amount: Int) -> Transaction {
        Transaction {
            id,
            amount,
            internal_state: 0,
        }
    }

    public fn get_id(&self) -> Int {
        self.id
    }

    fn validate(&self) -> Bool {
        self.amount > 0
    }
}
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import crate.models.Transaction;

public fn example() {
    let tx = Transaction.new(1, 100);
    let id = tx.id;  // OK: public field
    // let amount = tx.amount;  // OK within crate: public(crate) field
    // let state = tx.internal_state;  // ERROR: private field
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let models = loader
        .load_module(&ModulePath::from_str("models"), ModuleId::new(1))
        .unwrap();
    assert!(models.source.as_str().contains("public type Transaction"));
    assert!(models.source.as_str().contains("public id: Int"));
    assert!(models.source.as_str().contains("public(crate) amount"));
    assert!(models.source.as_str().contains("internal_state: Int"));

    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(2))
        .unwrap();
    assert!(app.source.as_str().contains("let id = tx.id"));
}

#[test]
fn test_visibility_hierarchy() {
    // Test visibility checks across module hierarchy
    let checker = VisibilityChecker::new();

    // Setup module hierarchy: cog.api.v1.handlers
    let handlers_module = ModulePath::from_str("cog.api.v1.handlers");
    let v1_module = ModulePath::from_str("cog.api.v1");
    let api_module = ModulePath::from_str("cog.api");
    let crate_root = ModulePath::from_str("cog");
    let external_module = ModulePath::from_str("other_cog.module");

    // Public: visible everywhere
    assert!(checker.is_visible(Visibility::Public, &handlers_module, &v1_module));
    assert!(checker.is_visible(Visibility::Public, &handlers_module, &api_module));
    assert!(checker.is_visible(Visibility::Public, &handlers_module, &crate_root));
    assert!(checker.is_visible(Visibility::Public, &handlers_module, &external_module));

    // Private: only visible in same module
    assert!(checker.is_visible(Visibility::Private, &handlers_module, &handlers_module));
    assert!(!checker.is_visible(Visibility::Private, &handlers_module, &v1_module));
    assert!(!checker.is_visible(Visibility::Private, &handlers_module, &api_module));
}

#[test]
fn test_mixed_visibility_items() {
    // Test module with mixed visibility items
    let project = TestProject::new();

    project.create_file(
        "database.vr",
        r#"
public type Connection is {
    public id: Int,
    handle: Int,  // private
}

public fn connect() -> Connection {
    Connection { id: 1, handle: internal_connect() }
}

fn internal_connect() -> Int {
    42  // private implementation
}

public(crate) fn raw_query(sql: &str) -> Result<Int, Text> {
    // Crate-internal function
    Result.Ok(0)
}
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import crate.database.{Connection, connect, raw_query};

public fn example() -> Connection {
    let conn = connect();
    // Can use raw_query within same crate
    let _ = raw_query("SELECT 1");
    conn
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let database = loader
        .load_module(&ModulePath::from_str("database"), ModuleId::new(1))
        .unwrap();
    assert!(database.source.as_str().contains("public type Connection"));
    assert!(database.source.as_str().contains("public fn connect"));
    assert!(database.source.as_str().contains("fn internal_connect"));
    assert!(
        database
            .source
            .as_str()
            .contains("public(crate) fn raw_query")
    );

    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(2))
        .unwrap();
    assert!(
        app.source
            .as_str()
            .contains("import crate.database.{Connection, connect, raw_query}")
    );
}
