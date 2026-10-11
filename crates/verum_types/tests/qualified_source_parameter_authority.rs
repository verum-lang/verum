//! T1733: unmounted qualified source templates retain declaration authority.
use verum_ast::{FileId, ItemKind};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::TypeChecker;

fn check(caller: &str, consumer: &str, modules: &[(&str, &str)]) -> List<Text> {
    let mut registry = ModuleRegistry::new();
    let mut consumer_ast = None;
    for (index, (owner, source)) in modules
        .iter()
        .copied()
        .chain(std::iter::once((caller, consumer)))
        .enumerate()
    {
        let file = FileId::new(index as u32);
        let ast = FastParser::new()
            .parse_module_str(source, file)
            .expect("source grammar");
        let id = ModuleId::new(index as u32);
        let path = ModulePath::from_str(owner);
        let mut module = ModuleInfo::new(id, path.clone(), ast.clone(), file, source.into());
        module.exports = extract_exports_from_module(&ast, id, &path).expect("source exports");
        registry.register(module);
        if owner == caller {
            consumer_ast = Some(ast);
        }
    }
    let ast = consumer_ast.expect("consumer module");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_cog(caller.split('.').next().expect("cog"));
    checker.set_current_module_path(caller);
    checker.set_module_registry_direct(registry.clone());
    checker.register_stdlib_types_for_module(&ast);
    let mut errors = List::new();
    for item in &ast.items {
        if let ItemKind::Mount(import) = &item.kind {
            if let Err(error) = checker.process_import(import, caller, &registry) {
                errors.push(format!("{error:?}").into());
            }
        }
    }
    for item in &ast.items {
        let result = match &item.kind {
            ItemKind::Type(declaration) => checker.register_type_declaration(declaration),
            ItemKind::Function(declaration) => checker.register_function_signature(declaration),
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
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors
}

#[test]
fn qualified_private_template_cannot_borrow_a_public_homonym() {
    let modules = [
        ("demo.alpha", "type Hidden<T> is | Wrap(T);"),
        ("demo.beta", "public type Hidden<T> is | Wrap(T);"),
    ];
    for reverse in [false, true] {
        let modules = if reverse {
            [modules[1], modules[0]]
        } else {
            modules
        };
        let errors = check(
            "demo.main",
            "public type Hidden<T> is | Wrap(T); fn probe(value: demo.alpha.Hidden<Int>) {}",
            &modules,
        );
        assert!(
            errors.iter().any(|error| error.contains("VisibilityError")
                && error.contains("Private")
                && error.contains("demo.alpha")
                && error.contains("Hidden")),
            "{errors:?}"
        );
    }
}

#[test]
fn qualified_missing_template_cannot_borrow_a_sibling_or_local_homonym() {
    let errors = check(
        "demo.main",
        "type Missing<T> is | Wrap(T); fn probe(value: demo.alpha.Missing<Int>) {}",
        &[
            ("demo.alpha", "public type Other<T> is | Wrap(T);"),
            ("demo.beta", "public type Missing<T> is | Wrap(T);"),
        ],
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("TypeNotFound") && error.contains("demo.alpha.Missing")),
        "{errors:?}"
    );
}

#[test]
fn qualified_private_template_remains_available_to_its_owner() {
    let errors = check(
        "demo.alpha",
        "type Hidden<T> is | Wrap(T); fn probe(value: demo.alpha.Hidden<Int>) {}",
        &[],
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn qualified_cog_visible_template_checks_the_actual_caller() {
    let modules = [("demo.alpha", "public(cog) type Shared<T> is | Wrap(T);")];
    let source = "fn probe(value: demo.alpha.Shared<Int>) {}";
    let errors = check("demo.main", source, &modules);
    assert!(errors.is_empty(), "same cog: {errors:?}");
    let errors = check("other.main", source, &modules);
    assert!(
        errors.iter().any(|error| error.contains("VisibilityError")
            && error.contains("PublicCrate")
            && error.contains("demo.alpha")
            && error.contains("Shared")),
        "foreign cog: {errors:?}"
    );
}

#[test]
fn qualified_renamed_reexport_retains_the_alias_declarations_default() {
    let modules = [
        (
            "demo.alpha",
            "public type Target<Value, Phantom = Bool> is fn(Value) -> Value;",
        ),
        (
            "demo.bridge",
            "public mount demo.alpha.{Target as Published};",
        ),
    ];
    let errors = check(
        "demo.main",
        "type Published<A, B, C> is | Ambient(A); fn probe(value: demo.bridge.Published<Int>, input: Int) -> Int { value(input) }",
        &modules,
    );
    assert!(errors.is_empty(), "{errors:?}");
}
