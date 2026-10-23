//! Imported variant payloads belong to the declaring file's lexical scope.
use verum_ast::{FileId, ItemKind};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::TypeChecker;

fn errors(inner: &str, outer: &str, consumer: &str) -> List<Text> {
    let mut registry = ModuleRegistry::new();
    for (index, (owner, source)) in [
        ("app.inner", inner),
        ("app.outer", outer),
        (
            "app.stranger",
            "public type InnerError is | Stranger(Bool);",
        ),
        ("app.main", consumer),
    ]
    .into_iter()
    .enumerate()
    {
        let id = ModuleId::new(index as u32);
        let file = FileId::new(index as u32);
        let ast = FastParser::new()
            .parse_module_str(source, file)
            .expect("grammar");
        let path = ModulePath::from_str(owner);
        let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), file, source.into());
        info.exports = extract_exports_from_module(&ast, id, &path).expect("exports");
        registry.register(info);
    }
    let ast = FastParser::new()
        .parse_module_str(consumer, FileId::new(3))
        .unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_cog("app");
    checker.set_current_module_path("app.main");
    checker.set_module_registry_direct(registry.clone());
    let mut errors = List::new();
    for item in &ast.items {
        let result = match &item.kind {
            ItemKind::Mount(import) => checker.process_import(import, "app.main", &registry),
            ItemKind::Type(decl) => checker.register_type_declaration(decl),
            ItemKind::Function(decl) => checker.register_function_signature(decl),
            _ => continue,
        };
        if let Err(error) = result {
            errors.push(format!("{error:?}").into());
        }
    }
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}").into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| format!("{error:?}").into()),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|error| format!("{error:?}").into()),
    );
    errors
}

const INNER: &str = "public type InnerError is | Invalid(Int);";
const CONSUMER: &str = r#"
mount app.outer.OuterError;
fn probe(error: OuterError) -> Int {
    match error { OuterError.TooLarge => 413, OuterError.Nested(_) => 422 }
}
fn construct() -> OuterError { OuterError.TooLarge }
"#;

#[test]
fn private_mount_supplies_a_public_variant_payload() {
    let errors = errors(
        INNER,
        r#"
mount app.inner.InnerError;
public type OuterError is | TooLarge | Nested(InnerError);
"#,
        CONSUMER,
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn renamed_private_mount_supplies_a_public_variant_payload() {
    let errors = errors(
        INNER,
        r#"
mount app.inner.{InnerError as Cause};
public type OuterError is | TooLarge | Nested(Cause);
"#,
        CONSUMER,
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn transitive_private_mounts_keep_each_declaration_scope() {
    let errors = errors(
        r#"
mount app.stranger.{InnerError as Detail};
public type InnerError is | Invalid(Detail);
"#,
        r#"
mount app.inner.InnerError;
public type OuterError is | TooLarge | Nested(InnerError);
"#,
        CONSUMER,
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn consumer_names_cannot_replace_the_declaring_payload() {
    let consumer = format!(
        r#"
mount app.stranger.InnerError;
mount app.inner.{{InnerError as Expected}};
{CONSUMER}
fn payload(error: OuterError, fallback: Expected) -> Expected {{
    match error {{ OuterError.Nested(value) => value, _ => fallback }}
}}
"#
    );
    let errors = errors(
        INNER,
        r#"
mount app.inner.InnerError;
public type OuterError is | TooLarge | Nested(InnerError);
"#,
        &consumer,
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn missing_dependency_cannot_borrow_a_consumer_type() {
    let consumer = format!("mount app.stranger.InnerError;\n{CONSUMER}");
    let errors = errors(
        "",
        r#"
mount app.inner.InnerError;
public type OuterError is | TooLarge | Nested(InnerError);
"#,
        &consumer,
    );
    assert!(
        !errors.is_empty(),
        "missing source declaration must be refused"
    );
}

#[test]
fn private_foreign_payload_cannot_be_imported_by_its_name() {
    let errors = errors(
        "type InnerError is | Invalid(Int);",
        r#"
mount app.inner.InnerError;
public type OuterError is | TooLarge | Nested(InnerError);
"#,
        CONSUMER,
    );
    assert!(
        !errors.is_empty(),
        "a private foreign declaration is not a public export"
    );
}
