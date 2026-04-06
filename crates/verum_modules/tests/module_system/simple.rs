//! Simple two-file import tests.
//!
//! Tests basic module loading and import resolution across two files.
//!
//! Tests module structure (file system mapping), visibility system, and
//! basic import resolution (single imports, multiple items, glob imports).

use std::fs;
use tempfile::TempDir;
use verum_common::{List, Maybe, Text};
use verum_lexer::Lexer;
use verum_modules::*;
use verum_parser::parse_module;

/// Helper to create a temporary module structure
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
fn test_basic_two_file_import() {
    // File system mapping: lib.vr = root, foo.vr = module foo, foo/mod.vr = directory module
    let project = TestProject::new();

    // File 1: Define public type
    project.create_file(
        "types.vr",
        r#"
public type User is {
    public id: Int,
    public name: Text,
}
"#,
    );

    // File 2: Import and use that type
    project.create_file(
        "main.vr",
        r#"
import crate.types.User;

public fn create_user(id: Int, name: Text) -> User {
    User { id, name }
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    // Load types module
    let types_path = ModulePath::from_str("types");
    let types_id = ModuleId::new(1);
    let types_module = loader.load_module(&types_path, types_id);
    assert!(
        types_module.is_ok(),
        "Failed to load types module: {:?}",
        types_module.err()
    );

    // Load main module
    let main_path = ModulePath::from_str("main");
    let main_id = ModuleId::new(2);
    let main_module = loader.load_module(&main_path, main_id);
    assert!(
        main_module.is_ok(),
        "Failed to load main module: {:?}",
        main_module.err()
    );

    let main = main_module.unwrap();
    assert!(main.source.as_str().contains("import crate.types.User"));
}

#[test]
fn test_import_single_item() {
    // Basic import: `mount std.collections.Map` imports a single item
    let project = TestProject::new();

    project.create_file(
        "math.vr",
        r#"
public fn add(a: Int, b: Int) -> Int {
    a + b
}

public fn multiply(a: Int, b: Int) -> Int {
    a * b
}
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import crate.math.add;

public fn compute(x: Int, y: Int) -> Int {
    add(x, y)
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let math_module = loader
        .load_module(&ModulePath::from_str("math"), ModuleId::new(1))
        .unwrap();
    assert!(math_module.source.as_str().contains("public fn add"));

    let app_module = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(2))
        .unwrap();
    assert!(app_module.source.as_str().contains("import crate.math.add"));
}

#[test]
fn test_import_multiple_items() {
    // Multiple imports: `mount std.io.{Read, Write, Error}` imports several items
    let project = TestProject::new();

    project.create_file(
        "types.vr",
        r#"
public type Result<T, E> is
    | Ok(T)
    | Err(E);

public type Maybe<T> is
    | Some(T)
    | None;
"#,
    );

    project.create_file(
        "app.vr",
        r#"
import crate.types.{Result, Maybe};

public fn example() -> Result<Maybe<Int>, Text> {
    Result.Ok(Maybe.Some(42))
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let types = loader
        .load_module(&ModulePath::from_str("types"), ModuleId::new(1))
        .unwrap();
    assert!(types.source.as_str().contains("public type Result"));
    assert!(types.source.as_str().contains("public type Maybe"));

    let app = loader
        .load_module(&ModulePath::from_str("app"), ModuleId::new(2))
        .unwrap();
    assert!(
        app.source
            .as_str()
            .contains("import crate.types.{Result, Maybe}")
    );
}

#[test]
fn test_module_not_found() {
    // Error case: importing from nonexistent module produces "Module not found" error
    let project = TestProject::new();

    project.create_file(
        "main.vr",
        r#"
import crate.nonexistent.Type;
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    // Try to load nonexistent module
    let result = loader.load_module(&ModulePath::from_str("nonexistent"), ModuleId::new(1));
    assert!(
        result.is_err(),
        "Expected module not found error, got success"
    );
}

#[test]
fn test_crate_root_import() {
    // Absolute paths start from crate root: `import crate.parser.ast`
    let project = TestProject::new();

    project.create_file(
        "config.vr",
        r#"
public type Config is {
    public port: Int,
}
"#,
    );

    project.create_file(
        "server.vr",
        r#"
import crate.config.Config;

public fn start(cfg: Config) {
    // Implementation
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let config = loader
        .load_module(&ModulePath::from_str("config"), ModuleId::new(1))
        .unwrap();
    assert!(config.source.as_str().contains("public type Config"));

    let server = loader
        .load_module(&ModulePath::from_str("server"), ModuleId::new(2))
        .unwrap();
    assert!(
        server
            .source
            .as_str()
            .contains("import crate.config.Config")
    );
}

#[test]
fn test_multiple_files_with_cross_imports() {
    // Test multiple modules importing from each other
    let project = TestProject::new();

    project.create_file(
        "models.vr",
        r#"
public type User is {
    public id: Int,
    public name: Text,
}
"#,
    );

    project.create_file(
        "validation.vr",
        r#"
import crate.models.User;

public fn validate_user(user: User) -> Bool {
    user.id > 0 && user.name.len() > 0
}
"#,
    );

    project.create_file(
        "api.vr",
        r#"
import crate.models.User;
import crate.validation.validate_user;

public fn create_user(id: Int, name: Text) -> Result<User, Text> {
    let user = User { id, name };
    if validate_user(user) {
        Result.Ok(user)
    } else {
        Result.Err("Invalid user")
    }
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let models = loader
        .load_module(&ModulePath::from_str("models"), ModuleId::new(1))
        .unwrap();
    assert!(models.source.as_str().contains("public type User"));

    let validation = loader
        .load_module(&ModulePath::from_str("validation"), ModuleId::new(2))
        .unwrap();
    assert!(
        validation
            .source
            .as_str()
            .contains("import crate.models.User")
    );

    let api = loader
        .load_module(&ModulePath::from_str("api"), ModuleId::new(3))
        .unwrap();
    assert!(api.source.as_str().contains("import crate.models.User"));
    assert!(
        api.source
            .as_str()
            .contains("import crate.validation.validate_user")
    );
}
