//! Glob reexports retain the declaring type and its field visibility.
use verum_ast::FileId;
use verum_common::{List, Text};
use verum_fast_parser::VerumParser;
use verum_modules::{
    ModuleInfo, ModulePath, ModuleRegistry, SharedModuleRegistry, extract_exports_from_module,
    resolve_glob_reexports, resolve_specific_reexport_kinds,
};
use verum_types::TypeChecker;

const DEPENDENCY: &str =
    "public type Token is { public id: Int }; public fn answer() -> Int { 42 }";
const CONSUMER: &str =
    "mount facade.{Token, answer}; fn read(value: Token) -> Int { value.id + answer() }";

fn registry(modules: &[(&str, &str)]) -> ModuleRegistry {
    let mut registry = ModuleRegistry::new();
    for (owner, source) in modules {
        let id = registry.allocate_id();
        let file = FileId::new(id.as_u32());
        let ast = VerumParser::new().parse_module_str(source, file).unwrap();
        let path = ModulePath::from_str(owner);
        let exports = extract_exports_from_module(&ast, id, &path).unwrap();
        let mut module = ModuleInfo::new(id, path, ast, file, (*source).into());
        module.exports = exports;
        registry.register(module);
    }
    registry
}

fn checked(modules: &[(&str, &str)], source: &str) -> (ModuleRegistry, TypeChecker, List<Text>) {
    let mut modules: List<_> = modules.iter().copied().collect();
    modules.push(("consumer", source));
    let mut registry = registry(&modules);
    resolve_glob_reexports(&mut registry).unwrap();
    resolve_specific_reexport_kinds(&mut registry).unwrap();
    let module = registry.get_by_path("consumer").unwrap();
    let mut checker =
        TypeChecker::with_shared_registry(SharedModuleRegistry::from_registry(registry.clone()));
    checker.register_builtins();
    checker.set_current_module(module.id);
    checker.set_current_module_path("consumer");
    let mut errors: List<Text> = checker
        .prepare_module_declarations(&module.ast)
        .into_iter()
        .map(|error| format!("{error:?}").into())
        .collect();
    for item in &module.ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}").into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|e| Text::from(format!("{e:?}"))),
    );
    (registry, checker, errors)
}

#[test]
fn public_field_reexport_uses_its_real_declaring_module() {
    let (registry, checker, errors) = checked(
        &[
            ("dependency", DEPENDENCY),
            ("facade", "public mount dependency.*;"),
        ],
        CONSUMER,
    );
    assert!(errors.is_empty(), "{errors:#?}");
    let (declaration, owner) = checker
        .resolve_public_source_type("facade", "Token", &registry)
        .unwrap();
    assert_eq!(owner.as_str(), "dependency");
    assert_eq!(declaration.name.as_str(), "Token");
}

#[test]
fn private_field_stays_private_through_a_public_glob() {
    let source = DEPENDENCY.replace("public id", "id");
    let (_, _, errors) = checked(
        &[
            ("dependency", &source),
            ("facade", "public mount dependency.*;"),
        ],
        CONSUMER,
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("VisibilityError") && error.contains("dependency")),
        "{errors:#?}"
    );
    assert!(
        !errors
            .iter()
            .any(|error| error.contains("unknown declaration visibility")),
        "missing authority is not a private-field diagnosis: {errors:#?}"
    );
}

#[test]
fn glob_then_named_alias_keeps_generic_parameters_and_owner() {
    let (registry, checker, errors) = checked(
        &[
            ("dependency", "public type Token<T> is { public id: T };"),
            ("facade", "public mount dependency.*;"),
            ("bridge", "public mount facade.{Token as Ticket};"),
        ],
        "mount bridge.Ticket; fn read(value: Ticket<Int>) -> Int { value.id }",
    );
    assert!(errors.is_empty(), "{errors:#?}");
    let (declaration, owner) = checker
        .resolve_public_source_type("bridge", "Ticket", &registry)
        .unwrap();
    assert_eq!(owner.as_str(), "dependency");
    assert_eq!(declaration.name.as_str(), "Token");
}

#[test]
fn relative_globs_follow_shared_module_resolution() {
    for (facade, mount) in [
        ("pkg", "public mount .dependency.*;"),
        ("pkg.facade", "public mount super.dependency.*;"),
    ] {
        let source = format!(
            "mount {facade}.{{Token, answer}}; fn read(value: Token) -> Int {{ value.id + answer() }}"
        );
        let (registry, checker, errors) =
            checked(&[("pkg.dependency", DEPENDENCY), (facade, mount)], &source);
        assert!(errors.is_empty(), "{facade}: {errors:#?}");
        assert_eq!(
            checker
                .resolve_public_source_type(facade, "Token", &registry)
                .unwrap()
                .1
                .as_str(),
            "pkg.dependency"
        );
    }
}

#[test]
fn private_missing_and_cyclic_globs_do_not_borrow_a_homonym() {
    for modules in [
        List::from_iter([("facade", "public mount missing.*;")]),
        List::from_iter([
            ("facade", "public mount other.*;"),
            ("other", "public mount facade.*;"),
        ]),
        List::from_iter([
            ("facade", "mount dependency.*;"),
            ("dependency", DEPENDENCY),
        ]),
        List::from_iter([
            ("facade", "public mount dependency.*;"),
            ("dependency", "type Token is { public id: Int };"),
        ]),
    ] {
        let mut modules = modules;
        modules.push(("unrelated", DEPENDENCY));
        let mut registry = registry(&modules);
        // Missing imports may be reported here; declaration lookup must still
        // refuse and must not search a sibling or parent for a replacement.
        let _ = resolve_glob_reexports(&mut registry);
        let checker = TypeChecker::new();
        assert!(
            checker
                .resolve_public_source_type("facade", "Token", &registry)
                .is_none()
        );
    }
}
