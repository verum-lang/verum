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
fn check_body(checker: &mut TypeChecker, source: &str) -> verum_common::List<verum_common::Text> {
    let start = checker.diagnostics().len();
    let module = Parser::new(source).parse_module().unwrap();
    let mut errors: verum_common::List<verum_common::Text> = checker.prepare_module_declarations(&module)
        .into_iter().map(|error| format!("{error:?}").into()).collect();
    for item in &module.items {
        if let Err(error) = checker.check_item(item) { errors.push(format!("{error:?}").into()); }
    }
    errors.extend(checker.take_deferred_errors().into_iter().map(|error| format!("{error:?}").into()));
    errors.extend(checker.diagnostics().iter().skip(start).filter(|diagnostic| diagnostic.is_error())
        .map(|diagnostic| format!("{diagnostic:?}").into()));
    errors
}

#[test]
fn fresh_failed_registration_does_not_publish_a_constructor_header() {
    for owner in ["cog", "demo.main"] {
        let mut checker = checker(owner);
        assert!(!register(&mut checker, "type Failed<T> is {value:Type(missing)};").is_empty());
        let errors = register(&mut checker,
            "type Channel<P> is protocol {};type Receiver is {};implement Channel<Failed<Int>> for Receiver {}");
        assert!(!errors.is_empty(), "{owner}: failed declaration became evidence");
        assert!(register(&mut checker, "type Failed<T> is {value:T};").is_empty());
        assert!(register(&mut checker, "implement Channel<Failed<Int>> for Receiver {}").is_empty());
    }
}

#[test]
fn failed_replacement_preserves_only_the_accepted_constructor_header() {
    // Replacement is atomic: failure preserves the accepted fields and
    // parameter roles, for generic and scalar declarations alike.
    for owner in ["cog", "demo.main"] {
        let mut checker = checker(owner);
        assert!(register(&mut checker, "type Failed<T> is {value:T};").is_empty());
        assert!(!register(&mut checker,
            "type Failed<N:meta Int, U> is {candidate:Type(missing)};").is_empty());
        let errors = register(&mut checker,
            "type Channel<P> is protocol {};type Receiver is {};implement Channel<Failed<Int>> for Receiver {}");
        assert!(errors.is_empty(), "{owner}: accepted header was not restored: {errors:?}");
        for application in ["Failed<3>", "Failed<Int, Bool>", "Failed<3, Bool>"] {
            assert!(!register(&mut checker, &format!(
                "implement Channel<{application}> for Receiver {{}}"
            )).is_empty(), "{owner}: failed replacement changed {application}");
        }
        let errors = check_body(&mut checker, "fn take(value:Failed<Int>)->Int {value.value}");
        assert!(errors.is_empty(), "{owner}: accepted field was not restored: {errors:?}");
        assert!(!check_body(&mut checker,
            "fn bad(value:Failed<Int>)->Int {value.candidate}").is_empty());
        assert!(register(&mut checker, "type Failed<N:meta Int> is {value:Bool};").is_empty());
        let errors = check_body(&mut checker, "fn replacement(value:Failed<3>)->Bool {value.value}");
        assert!(errors.is_empty(), "{owner}: valid replacement did not update header: {errors:?}");
        assert!(!check_body(&mut checker, "fn stale(value:Failed<Int>)->Int {value.value}").is_empty());
    }
}

#[test]
fn changed_source_cannot_reuse_a_previously_accepted_constructor_header() {
    use verum_modules::{ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module};
    for owner in ["cog", "demo.main"] {
        let mut checker = checker(owner);
        assert!(register(&mut checker, "type Failed<T> is {value:T};").is_empty());
        let source = "type Failed<N:meta Int, U> is {candidate:Type(missing)};";
        assert!(!register(&mut checker, source).is_empty());
        let ast = Parser::new(source).parse_module().unwrap();
        let id = ModuleId::new(1);
        let path = ModulePath::from_str(owner);
        let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), verum_ast::FileId::new(0), source.into());
        info.exports = extract_exports_from_module(&ast, id, &path).unwrap();
        let mut registry = ModuleRegistry::new();
        registry.register(info);
        checker.set_module_registry_direct(registry);
        let errors = register(&mut checker,
            "type Consumer is protocol {fn take(&self,input:Failed<Int>)->Int;};");
        assert!(errors.iter().any(|error| error.contains("conflicting source declarations")),
            "{owner}: changed source did not invalidate cached generic authority: {errors:?}");
    }
}
