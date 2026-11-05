use verum_ast::{FileId, ItemKind};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::TypeChecker;
const PROVIDER: &str = r#"
public type Stream is protocol { type Item; };
public type Factory is protocol { type Rows: Stream; };
public type Rows<T> is { value: T };
public type Box<T> is { value: T };
implement<T> Stream for Rows<T> { type Item = T; }
implement<T> Factory for Box<T> { type Rows = Rows<T>; }
"#;
fn check(source: &str, provider: &str) -> Vec<String> {
    check_with_extra(source, provider, None)
}
fn check_with_extra(source: &str, provider: &str, extra: Option<&str>) -> Vec<String> {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("demo.main");
    checker.set_current_cog("demo");
    let mut registry = ModuleRegistry::new();
    let ast = FastParser::new()
        .parse_module_str(provider, FileId::new(1))
        .unwrap();
    let id = ModuleId::new(1);
    let path = ModulePath::from_str("demo.lib");
    let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), ast.file_id, provider.into());
    info.exports = extract_exports_from_module(&ast, id, &path).unwrap();
    registry.register(info);
    if let Some(extra) = extra {
        let ast = FastParser::new()
            .parse_module_str(extra, FileId::new(3))
            .unwrap();
        let id = ModuleId::new(2);
        let path = ModulePath::from_str("demo.foreign");
        let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), ast.file_id, extra.into());
        info.exports = extract_exports_from_module(&ast, id, &path).unwrap();
        registry.register(info);
    }
    let ast = FastParser::new()
        .parse_module_str(source, FileId::new(2))
        .unwrap();
    let id = ModuleId::new(3);
    let path = ModulePath::from_str("demo.main");
    let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), ast.file_id, source.into());
    info.exports = extract_exports_from_module(&ast, id, &path).unwrap();
    registry.register(info);
    checker.set_module_registry_direct(registry.clone());
    let mut errors = Vec::new();
    for item in &ast.items {
        if let ItemKind::Mount(mount) = &item.kind {
            if let Err(e) = checker.process_import(mount, "demo.main", &registry) {
                errors.push(e.to_string());
            }
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .iter()
            .map(ToString::to_string),
    );
    errors.extend(checker.diagnostic_sources().iter().map(ToString::to_string));
    errors
}
fn check_both(provider: &str) -> [Vec<String>; 2] {
    let imported = check("mount demo.lib.Box;", provider);
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_cog("demo");
    checker.set_current_module_path("demo.lib");
    let ast = FastParser::new()
        .parse_module_str(provider, FileId::new(10))
        .unwrap();
    let mut errors = Vec::new();
    for item in &ast.items {
        if let ItemKind::Type(declaration) = &item.kind {
            if let Err(error) = checker.register_type_declaration(declaration) {
                errors.push(error.to_string());
            }
        }
    }
    for item in &ast.items {
        if let ItemKind::Impl(declaration) = &item.kind {
            if let Err(error) = checker.register_impl_block(declaration) {
                errors.push(error.to_string());
            }
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .iter()
            .map(ToString::to_string),
    );
    errors.extend(checker.diagnostic_sources().iter().map(ToString::to_string));
    [imported, errors]
}
const BASE: &str = r#"
public type Stream is protocol { type Item; };
public type Marker is protocol { };
public type Factory is protocol { type Rows: Stream; };
public type Wrap<T> is { value:T };
public type Box is { value:Int };
"#;
fn source(condition: &str, witness: &str) -> String {
    format!(
        "{BASE} {witness} implement<T> Stream for Wrap<T> where {condition} {{ type Item=T; }} implement Factory for Box {{ type Rows=Wrap<Int>; }}"
    )
}
fn accepts(provider: &str) {
    for (route, errors) in check_both(provider).into_iter().enumerate() {
        assert!(errors.is_empty(), "route={route}: {errors:#?}");
    }
}
fn refuses(provider: &str) {
    for (route, errors) in check_both(provider).into_iter().enumerate() {
        assert!(
            !errors.is_empty(),
            "route={route} erased required source condition"
        );
    }
}
#[test]
fn explicit_concrete_condition_requires_actual_evidence() {
    refuses(&source("Int:Marker", ""));
}
#[test]
fn explicit_concrete_condition_accepts_its_actual_witness() {
    accepts(&source("Int:Marker", "implement Marker for Int {}"));
}
#[test]
fn compound_condition_is_not_limited_to_bare_parameter_names() {
    refuses(&source("Wrap<T>:Marker", ""));
}
#[test]
fn compound_condition_accepts_a_generic_witness() {
    accepts(&source(
        "Wrap<T>:Marker",
        "implement<T> Marker for Wrap<T> {}",
    ));
}
#[test]
fn self_condition_resolves_to_the_declared_implementation_receiver() {
    accepts(&source("Self:Marker", "implement<T> Marker for Wrap<T> {}"));
    refuses(&source("Self:Marker", ""));
}
#[test]
fn generic_and_explicit_conditions_are_both_retained() {
    let provider = format!(
        r#"{BASE}
        public type Enabled is protocol {{}};
        implement Enabled for Int {{}}
        implement Marker for Int {{}}
        implement<T:Enabled> Stream for Wrap<T> where T:Marker {{type Item=T;}}
        implement Factory for Box {{type Rows=Wrap<Int>;}}
    "#
    );
    accepts(&provider);
    refuses(&provider.replace("implement Marker for Int {}", ""));
    refuses(&provider.replace("implement Enabled for Int {}", ""));
}
#[test]
fn explicit_callable_condition_retains_the_actual_function_contract() {
    let provider = format!(
        "{BASE} implement<F> Stream for Wrap<F> where F:fn(Int)->Int {{type Item=Int;}} implement Factory for Box {{type Rows=Wrap<fn(Int)->Int>;}}"
    );
    accepts(&provider);
    refuses(&provider.replace(
        "type Rows=Wrap<fn(Int)->Int>",
        "type Rows=Wrap<fn(Int)->Text>",
    ));
}
#[test]
fn blanket_source_condition_preserves_compound_subject() {
    let provider = format!(
        "{BASE} implement<T> Stream for T where Wrap<Int>:Marker {{type Item=Int;}} implement Factory for Box {{type Rows=Int;}}"
    );
    refuses(&provider);
    accepts(&format!("implement Marker for Wrap<Int> {{}} {provider}"));
}
#[test]
fn unsupported_equality_condition_is_not_silently_erased() {
    refuses(&source("Int = Text", ""));
}
