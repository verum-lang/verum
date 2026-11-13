use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(source: &str) -> Vec<String> {
    let module = Parser::new(source)
        .parse_module()
        .expect("actual source parses");
    let mut checker = TypeChecker::new();
    checker.set_generic_associated_types_enabled(true);
    checker.register_primitives();
    checker.set_current_module_path("families");
    let mut errors = checker
        .prepare_module_declarations(&module)
        .iter()
        .map(|error| format!("{error:?}"))
        .collect::<Vec<_>>();
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}"));
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .iter()
            .map(|error| format!("{error:?}")),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|d| format!("{d:?}")),
    );
    errors
}

#[test]
fn unbounded_family_domain_cannot_be_strengthened() {
    let errors = errors(
        "module families; type Mark is protocol {}; type Rel is protocol {type Out<X>;}; type Owner is {}; implement Rel for Owner {type Out<Y:Mark> = Y;}",
    );
    assert!(
        errors.iter().any(|error| error.contains("E405")),
        "{errors:?}"
    );
}

#[test]
fn declared_local_requirement_remains_usable_with_renamed_binder() {
    let errors = errors(
        "module families; type Mark is protocol {}; type Rel is protocol {type Out<X:Mark>;}; type Owner is {}; implement Rel for Owner {type Out<Y:Mark> = Y;}",
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn a_stronger_declared_requirement_entails_its_parent() {
    let errors = errors(
        "module families; type Mark is protocol {}; type Refined is protocol extends Mark {}; type Rel is protocol {type Out<X:Refined>;}; type Owner is {}; implement Rel for Owner {type Out<Y:Mark> = Y;}",
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn accepting_a_wider_local_domain_is_valid() {
    let errors = errors(
        "module families; type Mark is protocol {}; type Rel is protocol {type Out<X:Mark>;}; type Owner is {}; implement Rel for Owner {type Out<Y> = Y;}",
    );
    assert!(errors.is_empty(), "{errors:?}");
}
