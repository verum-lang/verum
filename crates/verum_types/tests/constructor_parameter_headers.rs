//! Constructor roles come from successful declarations, including direct API callers.
use verum_ast::ItemKind;
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn register(checker: &mut TypeChecker, source: &str) -> Vec<String> {
    let module = Parser::new(source).parse_module().unwrap();
    let mut errors = Vec::new();
    for item in &module.items {
        let result = match &item.kind {
            ItemKind::Type(declaration) => checker.register_type_declaration(declaration),
            ItemKind::Impl(declaration) => checker.register_impl_block(declaration),
            ItemKind::Function(declaration) => checker.register_function_signature(declaration),
            _ => continue,
        };
        if let Err(error) = result {
            errors.push(format!("{error:?}"));
        }
    }
    errors
}
fn checker(owner: &str) -> TypeChecker {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path(owner);
    checker
}
const SOURCE: &str = "type Scalar is Int; type Boxed<T> is {value:T}; type Channel<P> is protocol {}; type Receiver is {}; implement Channel<Boxed<Scalar>> for Receiver {}";

#[test]
fn direct_registration_retains_nested_alias_constructor_header() {
    for owner in ["cog", "demo.main"] {
        let errors = register(&mut checker(owner), SOURCE);
        assert!(errors.is_empty(), "{owner}: {errors:?}");
    }
}
#[test]
fn ordinary_module_preparation_retains_the_same_header() {
    let module = Parser::new(SOURCE).parse_module().unwrap();
    let mut checker = checker("demo.main");
    let errors = checker.prepare_module_declarations(&module);
    assert!(errors.is_empty(), "{errors:?}");
    for item in &module.items {
        checker.check_item(item).unwrap();
    }
    assert!(checker.take_deferred_errors().is_empty());
    assert!(
        !checker
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.is_error())
    );
}
#[test]
fn constructor_parameter_roles_are_not_guessed_from_arguments() {
    let prefix = "type Indexed<N:meta Int> is {value:Int};type Boxed<T> is {value:T};type Channel<P> is protocol {};type Receiver is {};";
    for (application, accepted) in [
        ("Indexed<3>", true),
        ("Boxed<Int>", true),
        ("Indexed<Int>", false),
        ("Boxed<3>", false),
    ] {
        let errors = register(
            &mut checker("cog"),
            &format!("{prefix} implement Channel<{application}> for Receiver {{}}"),
        );
        assert_eq!(errors.is_empty(), accepted, "{application}: {errors:?}");
        assert!(
            !errors
                .iter()
                .any(|error| error.contains("lacks its declaration-owned parameter header")),
            "real header was lost: {errors:?}"
        );
    }
}
#[test]
fn same_leaf_constructor_headers_keep_their_declaring_owner() {
    for (application, accepted) in [
        ("alpha.Boxed<Int>", true),
        ("beta.Boxed<3>", true),
        ("alpha.Boxed<3>", false),
        ("beta.Boxed<Int>", false),
    ] {
        let mut checker = checker("alpha");
        assert!(register(&mut checker, "public type Boxed<T> is {value:T};").is_empty());
        checker.set_current_module_path("beta");
        assert!(
            register(
                &mut checker,
                "public type Boxed<N:meta Int> is {value:Int};"
            )
            .is_empty()
        );
        checker.set_current_module_path("consumer");
        let errors = register(
            &mut checker,
            &format!(
                "type Channel<P> is protocol {{}};type Receiver is {{}};implement Channel<{application}> for Receiver {{}}"
            ),
        );
        assert_eq!(errors.is_empty(), accepted, "{application}: {errors:?}");
    }
}
#[test]
fn failed_registration_does_not_publish_a_constructor_header() {
    let mut checker = checker("cog");
    assert!(register(&mut checker, "type Failed<T> is {value:T};").is_empty());
    assert!(!register(&mut checker, "type Failed<T> is {value:Type(missing)};").is_empty());
    let errors = register(
        &mut checker,
        "type Channel<P> is protocol {};type Receiver is {};implement Channel<Failed<Int>> for Receiver {}",
    );
    assert!(!errors.is_empty(), "failed declaration became evidence");
    assert!(register(&mut checker, "type Failed<T> is {value:T};").is_empty());
    assert!(
        register(
            &mut checker,
            "implement Channel<Failed<Int>> for Receiver {}"
        )
        .is_empty()
    );
}
