//! Source binders, rather than spelling hashes, own universe identities.
use verum_ast::{FileId, ItemKind, Module, decl::FunctionDecl};
use verum_common::List;
use verum_fast_parser::FastParser;
use verum_types::{Type, TypeChecker, ty::UniverseLevel as Level};

fn import_source(source: &str, name: &str) -> (TypeChecker, List<verum_types::TypeError>) {
    use verum_modules::{
        ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
    };
    let ast = parse(source);
    let id = ModuleId::new(0);
    let path = ModulePath::from_str("test.provider");
    let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), FileId::new(0), source.into());
    info.exports = extract_exports_from_module(&ast, id, &path).unwrap();
    let mut registry = ModuleRegistry::new();
    registry.register(info);
    let mut checker = checker();
    checker.set_current_cog("test");
    checker.set_module_registry_direct(registry.clone());
    let consumer = parse(&format!("mount test.provider.{{{name}}};"));
    let ItemKind::Mount(import) = &consumer.items[0].kind else {
        panic!("mount")
    };
    let mut errors = List::new();
    if let Err(error) = checker.process_import(import, "test.main", &registry) {
        errors.push(error);
    }
    errors.extend(checker.take_deferred_errors());
    (checker, errors)
}

fn parse(source: &str) -> Module {
    FastParser::new()
        .parse_module_str(source, FileId::new(0))
        .expect("source grammar")
}

fn checker() -> TypeChecker {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("test.main");
    checker.set_universe_poly_enabled(true);
    checker
}

fn function(module: &Module, index: usize) -> &FunctionDecl {
    let ItemKind::Function(function) = &module.items[index].kind else {
        panic!("function fixture")
    };
    function
}

fn signature(checker: &TypeChecker, name: &str) -> Type {
    checker
        .ctx_env_lookup(name)
        .expect("registered signature")
        .ty
        .clone()
}

fn params(ty: &Type) -> &List<Type> {
    let Type::Function { params, .. } = ty else {
        panic!("function type: {ty:?}")
    };
    params
}

fn level(ty: &Type) -> Level {
    let Type::Universe { level } = ty else {
        panic!("universe type: {ty:?}")
    };
    level.clone()
}

fn check_source(source: &str) {
    let ast = parse(source);
    let mut checker = checker();
    assert!(
        checker.prepare_module_declarations(&ast).is_empty(),
        "declaration registration"
    );
    for item in &ast.items {
        checker.check_item(item).expect("source type checking");
    }
    assert!(checker.take_deferred_errors().is_empty());
    assert!(
        !checker
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.is_error()),
        "emitted diagnostics: {:?}",
        checker.diagnostics()
    );
}

#[test]
fn documented_examples_parse_and_check_with_both_level_spellings() {
    check_source("fn identity<A>(value: A) -> A { value }");
    check_source("fn retain_type(value: Type(1)) -> Type(1) { value }");
    check_source("fn identity_at<universe u, A: Type(u)>(value: A) -> A { value }");
    check_source("fn identity_at<u: Level, A: Type(u)>(value: A) -> A { value }");
    check_source("fn retain_at<universe u>(value: Type(u)) -> Type(u) { value }");
    check_source("fn retain_at<u: Level>(value: Type(u)) -> Type(u) { value }");
}

#[test]
fn source_imports_resolve_declared_levels_and_preserve_binding_errors() {
    let (checker, errors) = import_source(
        "public fn retain<universe u>(value: Type(u)) -> Type(u) { value }",
        "retain",
    );
    assert!(errors.is_empty(), "{errors:?}");
    assert!(matches!(
        level(&params(&signature(&checker, "retain"))[0]),
        Level::Variable(_)
    ));
    for (source, name) in [
        (
            "public fn bad(value: Type(missing)) -> Type(missing) { value }",
            "bad",
        ),
        (
            "public type Invalid is { value: Type(missing) };",
            "Invalid",
        ),
        (
            "public fn bad<A: Type(missing)>(value: A) -> A { value }",
            "bad",
        ),
    ] {
        let (_, errors) = import_source(source, name);
        assert!(
            errors.iter().any(|error| matches!(
                error,
                verum_types::TypeError::UniverseBinding { .. }
            ) && error.is_soundness_critical()),
            "import recovery must preserve the binding error: {errors:?}"
        );
    }
}

#[test]
fn same_spelling_is_distinct_between_declarations_but_stable_across_passes() {
    let ast = parse(
        "fn first<universe u>(value: Type(u)) -> Type(u) { value }\nfn second<universe u>(value: Type(u)) -> Type(u) { value }",
    );
    let mut checker = checker();
    checker
        .register_function_signature(function(&ast, 0))
        .unwrap();
    let first = level(&params(&signature(&checker, "first"))[0]);
    checker.check_item(&ast.items[0]).unwrap();
    checker
        .register_function_signature(function(&ast, 1))
        .unwrap();
    let second = level(&params(&signature(&checker, "second"))[0]);
    assert_ne!(first, second, "sibling scopes must not recycle IDs");
    checker
        .register_function_signature(function(&ast, 0))
        .unwrap();
    assert_eq!(first, level(&params(&signature(&checker, "first"))[0]));
    assert_ne!(
        first,
        Level::Concrete(0),
        "opaque IDs are not numeric levels"
    );
}

#[test]
fn module_ownership_disambiguates_equal_source_offsets() {
    let ast = parse("fn retain<universe u>(value: Type(u)) -> Type(u) { value }");
    let mut checker = checker();
    checker.set_current_module_path("test.left");
    checker
        .register_function_signature(function(&ast, 0))
        .unwrap();
    let left = level(&params(&signature(&checker, "retain"))[0]);
    checker.set_current_module_path("test.right");
    checker
        .register_function_signature(function(&ast, 0))
        .unwrap();
    let right = level(&params(&signature(&checker, "retain"))[0]);
    assert_ne!(left, right);
    checker.set_current_module_path("test.left");
    checker
        .register_function_signature(function(&ast, 0))
        .unwrap();
    assert_eq!(left, level(&params(&signature(&checker, "retain"))[0]));
}

#[test]
fn lexical_rank_two_binders_shadow_and_unmentioned_binders_inherit() {
    let ast = parse(
        "fn outer<universe u>(value: Type(u), shadow: fn<universe u>(Type(u)) -> Type(u), inherit: fn<A>(Type(u)) -> Type(u)) -> Type(u) { value }",
    );
    let mut checker = checker();
    checker
        .register_function_signature(function(&ast, 0))
        .unwrap();
    let ty = signature(&checker, "outer");
    let parameters = params(&ty);
    let outer = level(&parameters[0]);
    let inner = level(&params(&parameters[1])[0]);
    assert_ne!(outer, inner);
    let Type::Forall { body, .. } = &parameters[2] else {
        panic!("quantified type parameter")
    };
    assert_eq!(outer, level(&params(body)[0]));
}

#[test]
fn unbound_names_and_wrong_kinds_are_refused_even_in_bounds() {
    for source in [
        "fn bad(value: Type(u)) -> Type(u) { value }",
        "fn bad<universe u>(value: Type(max(u, missing))) { }",
        "fn bad<A: Type(missing)>(value: A) -> A { value }",
        "fn bad<u>(value: Type(u)) { }",
        "fn bad<universe u>(value: u) { }",
        "fn bad<universe u>(value: fn<u>(Type(u)) -> Type(u)) { }",
    ] {
        let ast = parse(source);
        assert!(
            checker()
                .register_function_signature(function(&ast, 0))
                .is_err(),
            "{source}"
        );
    }
}

#[test]
fn failed_or_completed_declarations_do_not_leak_level_names() {
    let ast =
        parse("fn failed<universe u>(value: Type(missing)) { }\nfn later(value: Type(u)) { }");
    let mut checker = checker();
    assert!(
        checker
            .register_function_signature(function(&ast, 0))
            .is_err()
    );
    assert!(
        checker
            .register_function_signature(function(&ast, 1))
            .is_err()
    );
    let good = parse("fn good<universe u>(value: Type(u)) { }");
    checker
        .register_function_signature(function(&good, 0))
        .unwrap();
    assert!(
        checker
            .register_function_signature(function(&ast, 1))
            .is_err()
    );
}

#[test]
fn level_binders_are_not_runtime_values() {
    let ast = parse("fn bad<universe u>() -> Type(u) { u }");
    let mut checker = checker();
    checker
        .register_function_signature(function(&ast, 0))
        .unwrap();
    assert!(checker.check_item(&ast.items[0]).is_err());
}

#[test]
fn level_marker_is_grammar_even_with_an_ordinary_type_named_level() {
    check_source(
        "type Level is { value: Int };\nfn retain<u: Level>(value: Type(u)) -> Type(u) { value }",
    );
}

#[test]
fn disabled_feature_refuses_level_binders_but_accepts_concrete_universes() {
    let mut checker = checker();
    checker.set_universe_poly_enabled(false);
    for source in [
        "fn bad<universe u>() { }",
        "fn bad<u: Level>(value: Type(u)) { }",
        "fn bad(value: Type(u)) { }",
    ] {
        let ast = parse(source);
        assert!(
            checker
                .register_function_signature(function(&ast, 0))
                .is_err(),
            "{source}"
        );
    }
    let concrete = parse("fn retain(value: Type(1)) -> Type(1) { value }");
    checker
        .register_function_signature(function(&concrete, 0))
        .unwrap();
    checker.check_item(&concrete.items[0]).unwrap();
}

#[test]
fn source_type_declarations_share_the_same_binding_rules() {
    check_source("type Holder<universe u> is { value: Type(u) };");
    let ast = parse("type Invalid is { value: Type(u) };");
    assert!(!checker().prepare_module_declarations(&ast).is_empty());
}

#[test]
fn programmatic_rank_two_nesting_shares_the_validation_depth_budget() {
    use verum_ast::{decl::FunctionParamKind, ty::TypeKind};
    let mut ast = parse("fn outer(value: fn<A>(Type(0)) -> Type(0)) { }");
    let ItemKind::Function(declaration) = &mut ast.items[0].kind else {
        panic!("function")
    };
    let FunctionParamKind::Regular { ty, .. } = &mut declaration.params[0].kind else {
        panic!("parameter")
    };
    let template = ty.clone();
    for _ in 0..80 {
        let mut outer = template.clone();
        let TypeKind::Rank2Function { params, .. } = &mut outer.kind else {
            panic!("rank two")
        };
        params[0] = ty.clone();
        *ty = outer;
    }
    let mut checker = checker();
    let error = checker
        .register_function_signature(declaration)
        .expect_err("shared depth budget");
    assert!(format!("{error:?}").contains("depth budget"));
    let good = parse("fn good(value: Type(0)) -> Type(0) { value }");
    checker
        .register_function_signature(function(&good, 0))
        .unwrap();
}
