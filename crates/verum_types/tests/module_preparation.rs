//! Shared compiler/editor declaration preparation, including failing evidence.

use verum_ast::{FileId, ItemKind, Module};
use verum_common::Text;
use verum_fast_parser::VerumParser;
use verum_modules::{ModuleInfo, ModulePath, ModuleRegistry, SharedModuleRegistry};
use verum_types::infer::TypeChecker;

fn parse(source: &str, file: u32) -> Module {
    VerumParser::new()
        .parse_module_str(source, FileId::new(file))
        .expect("valid fixture grammar")
}

fn checked(checker: &mut TypeChecker, module: &Module) -> Vec<String> {
    let mut errors: Vec<_> = checker
        .prepare_module_declarations(module)
        .iter()
        .map(|error| format!("{error:?}"))
        .collect();
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}"));
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .iter()
            .map(|e| format!("{e:?}")),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|e| format!("{e:?}")),
    );
    errors
}

#[test]
fn forward_types_functions_protocols_and_contexts_are_prepared_before_bodies() {
    let module = parse(
        r#"
        fn caller(value: Wrapper) -> Int { later(value.item) }
        type Wrapper is { item: Later };
        fn later(value: Later) -> Int { value.amount() }
        fn clock() -> Int using [Clock] { Clock.now() }
        implement Measure for Later { fn amount(&self) -> Int { self.value } }
        type Later is { value: Int };
        type Measure is protocol { fn amount(&self) -> Int; };
        context Clock { fn now() -> Int; }
        "#,
        1,
    );
    let errors = checked(&mut TypeChecker::new(), &module);
    assert!(errors.is_empty(), "{errors:#?}");
}

fn checker_with_dependency(source: &str) -> TypeChecker {
    let mut registry = ModuleRegistry::new();
    let id = registry.allocate_id();
    let path = ModulePath::from_str("dependency");
    let ast = parse(source, 2);
    let exports = verum_modules::extract_exports_from_module(&ast, id, &path).unwrap();
    let mut info = ModuleInfo::new(id, path, ast, FileId::new(2), Text::from(source));
    info.exports = exports;
    registry.register(info);
    let mut checker =
        TypeChecker::with_shared_registry(SharedModuleRegistry::from_registry(registry));
    checker.register_builtins();
    checker.set_current_module_path("consumer");
    checker
}

#[test]
fn imported_declarations_precede_local_type_and_signature_resolution() {
    let module = parse(
        "mount dependency.{Token}; type Envelope is { value: Token }; fn read(value: Envelope) -> Int { value.value.id }",
        1,
    );
    let mut checker = checker_with_dependency("public type Token is { public id: Int };");
    let errors = checked(&mut checker, &module);
    assert!(errors.is_empty(), "{errors:#?}");
    assert_eq!(checker.current_module_path().as_str(), "consumer");
}

#[test]
fn source_without_a_file_path_keeps_the_same_owner_when_mounts_are_rechecked() {
    // An in-memory compiler input has no filesystem-derived module name. Its
    // checker retains `cog`; preparation and body checking must use that same
    // authority, including for relative imports (not an empty second owner).
    let module = parse(
        "mount self.dependency.{Token}; fn read(value: Token) -> Int { value.id }",
        1,
    );
    let mut checker = checker_with_dependency("public type Token is { public id: Int };");
    checker.set_current_module_path("cog");
    let errors = checked(&mut checker, &module);
    assert!(errors.is_empty(), "{errors:#?}");
    assert_eq!(checker.current_module_path().as_str(), "cog");
}

#[test]
fn missing_and_private_imports_are_returned_without_needing_a_body() {
    for source in ["mount absent.{Token};", "mount dependency.{Hidden};"] {
        let mut checker = checker_with_dependency("type Hidden is { id: Int };");
        let errors = checker.prepare_module_declarations(&parse(source, 1));
        assert!(!errors.is_empty(), "mount failure was suppressed: {source}");
    }
}

#[test]
fn inline_modules_declared_after_the_mount_are_available() {
    let module = parse(
        "mount shapes.{Point}; fn read(value: Point) -> Int { value.x } module shapes { public type Point is { public x: Int }; }",
        1,
    );
    let errors = checked(&mut TypeChecker::new(), &module);
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn invalid_type_and_function_signatures_remain_registration_errors() {
    for source in [
        "type Broken is { value: Absent };",
        "type Broken<T> is { value: Absent };",
        "type Broken is Present { value: Absent };",
        "fn broken(value: Absent) {}",
    ] {
        let errors = TypeChecker::new().prepare_module_declarations(&parse(source, 1));
        assert!(
            !errors.is_empty(),
            "invalid declaration was suppressed: {source}"
        );
    }
}

#[test]
fn final_record_resolution_preserves_generic_forward_and_recursive_types() {
    let module = parse(
        "type Envelope<T> is { value: Node<T> }; type Node<T> is { value: T, next: &Node<T> }; type Chain<T> is End | Link { value: T, tail: &Chain<T> };",
        1,
    );
    let errors = checked(&mut TypeChecker::new(), &module);
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn preparation_does_not_claim_to_check_function_bodies() {
    let module = parse("fn wrong() -> Int { \"not an integer\" }", 1);
    let mut checker = TypeChecker::new();
    let preparation = checker.prepare_module_declarations(&module);
    assert!(preparation.is_empty(), "{preparation:?}");
    let function = module
        .items
        .iter()
        .find(|item| matches!(item.kind, ItemKind::Function(_)))
        .unwrap();
    assert!(checker.check_item(function).is_err());
}
