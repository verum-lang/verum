use verum_ast::FileId;
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_types::{
    TypeChecker,
    context::TypeScheme,
    ty::Type,
    universe_inference::UniverseInference,
    universe_level::{UniverseLevel as Level, UniverseMeta},
};

fn complaints(source: &str) -> List<Text> {
    let ast = FastParser::new()
        .parse_module_str(source, FileId::new(1))
        .expect("valid grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_universe_poly_enabled(true);
    let mut errors: List<_> = checker
        .prepare_module_declarations(&ast)
        .into_iter()
        .map(|e| Text::from(format!("{e}")))
        .collect();
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(Text::from(format!("{error}")));
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .iter()
            .map(|e| Text::from(format!("{e}"))),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|d| Text::from(format!("{d:?}"))),
    );
    errors
}

#[test]
fn separate_calls_instantiate_the_same_level_binder_independently() {
    let source = "fn retain<universe u>(x: Type(u)) -> Type(u) { x }\nfn one(x: Type(1)) -> Type(1) { retain(x) }\nfn two(x: Type(2)) -> Type(2) { retain(x) }";
    let errors = complaints(source);
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn source_parameters_remain_rigid_and_one_call_cannot_use_two_levels_for_one_binder() {
    for source in [
        "fn bad<universe u, universe v>(x: Type(u)) -> Type(v) { x }",
        "fn retain<universe u>(x: Type(u)) -> Type(u) { x } fn bad(x: Type(2)) -> Type(1) { retain(x) }",
        "fn pair<universe u>(x: Type(u), y: Type(u)) -> Type(u) { x } fn bad(x: Type(1), y: Type(2)) -> Type(1) { pair(x, y) }",
    ] {
        assert!(
            !complaints(source).is_empty(),
            "accepted invalid source: {source}"
        );
    }
}

#[test]
fn instantiation_preserves_unquantified_levels_and_uses_one_replacement_per_binder() {
    let scheme = TypeScheme::mono(Type::tuple(
        vec![
            Type::universe(Level::Variable(7)),
            Type::universe(Level::Variable(7)),
            Type::universe(Level::Variable(8)),
        ]
        .into(),
    ))
    .with_universe_parameters(vec![7].into())
    .unwrap();
    let first = scheme.instantiate();
    let second = scheme.instantiate();
    let Type::Tuple(first) = first else {
        panic!("tuple");
    };
    let Type::Tuple(second) = second else {
        panic!("tuple");
    };
    assert_eq!(first[0], first[1]);
    assert_ne!(first[0], second[0]);
    assert_eq!(first[2], Type::universe(Level::Variable(8)));
    assert_eq!(first[2], second[2]);
}

#[test]
fn only_metas_can_be_solved_and_failed_equations_leave_no_partial_bindings() {
    let a = Level::Meta(UniverseMeta::fresh());
    let b = Level::Meta(UniverseMeta::fresh());
    let mut inference = UniverseInference::default();
    assert!(inference.unify(&Level::Variable(0), &Level::TYPE1).is_err());
    inference.unify(&a, &b.succ()).unwrap();
    let prior = inference.clone();
    assert!(inference.unify(&b, &a).is_err());
    assert_eq!(inference, prior);
    inference.unify(&b, &Level::TYPE1).unwrap();
    assert!(inference.apply(&a).unwrap().equivalent(&Level::TYPE2));
    assert!(inference.unify(&a, &Level::TYPE).is_err());
    assert!(inference.apply(&a).unwrap().equivalent(&Level::TYPE2));
}

#[test]
fn unresolved_inference_cannot_be_used_as_universal_order_or_smt_model_evidence() {
    let meta = Level::Meta(UniverseMeta::fresh());
    assert!(meta.normalize().is_none());
    assert!(!meta.is_less_or_equal(&Level::TYPE));
    assert!(
        verum_types::universe_solver::solve_universe_constraints(&[
            verum_types::context::UniverseConstraint::Equal(meta, Level::TYPE)
        ])
        .is_err()
    );
}

#[test]
fn scheme_body_bounds_and_inductive_universe_share_one_instantiation() {
    use verum_common::Map;
    use verum_types::ty::TypeVar;
    let parameter = TypeVar::fresh();
    let universe = Type::universe(Level::Variable(7));
    let mut bounds = Map::new();
    bounds.insert(parameter, List::from_single(universe.clone()));
    let inductive = Type::Inductive {
        name: "Witness".into(),
        params: List::new(),
        indices: List::new(),
        universe: Level::Variable(7),
        constructors: List::new(),
    };
    let scheme = TypeScheme::poly(
        List::from_single(parameter),
        Type::tuple(vec![universe, inductive].into()),
    )
    .with_type_bounds(bounds)
    .with_universe_parameters(vec![7].into())
    .unwrap();
    let (body, fresh, bounds) = scheme.instantiate_with_type_bounds();
    let Type::Tuple(fields) = body else {
        panic!("tuple")
    };
    let Type::Universe { level } = &fields[0] else {
        panic!("universe")
    };
    let Type::Inductive { universe, .. } = &fields[1] else {
        panic!("inductive")
    };
    assert_eq!(universe, level);
    assert_eq!(bounds[&fresh[0]][0], fields[0]);
    assert!(matches!(level, Level::Meta(_)));
}

#[test]
fn callable_result_bounds_are_checked_without_becoming_kind_bounds() {
    let declaration = "fn apply<universe u, F: fn() -> Type(u)>(f: F) -> Type(u) { f() }";
    let valid = format!(
        "{declaration} fn one(f: fn() -> Type(1)) -> Type(1) {{ apply(f) }} fn two(f: fn() -> Type(2)) -> Type(2) {{ apply(f) }}"
    );
    let errors = complaints(&valid);
    assert!(errors.is_empty(), "{errors:?}");
    let invalid = format!("{declaration} fn bad(f: fn() -> Type(2)) -> Type(1) {{ apply(f) }}");
    assert!(!complaints(&invalid).is_empty());
}

#[test]
fn callable_bounds_survive_forwarding_and_reject_named_wrong_results() {
    let source = "fn apply<universe u, F: fn() -> Type(u)>(f: F) -> Type(u) { f() }
        fn forward<universe v, F: fn() -> Type(v)>(f: F) -> Type(v) { apply(f) }";
    assert!(complaints(source).is_empty(), "{:?}", complaints(source));
    let source = "fn apply<F: fn() -> Int, G: fn() -> F>(g: G) -> F { g() }
        fn forward<F: fn() -> Int, G: fn() -> F>(g: G) -> F { apply(g) }";
    assert!(complaints(source).is_empty(), "{:?}", complaints(source));
    assert!(
        !complaints(
            "fn apply<F: fn() -> Int>(f: F) -> Int { f() }
        fn text() -> Text { \"wrong\" } fn bad() -> Int { apply(text) }"
        )
        .is_empty()
    );
    let source = "fn keep<F: fn()>(f: F) -> F { f }
        fn number() -> Int { 1 } fn run() -> Int { let f = keep(number); f() }";
    assert!(complaints(source).is_empty(), "{:?}", complaints(source));
}

#[test]
fn callable_constraints_cannot_be_erased_by_local_aliases() {
    let valid = "fn apply<universe u, F: fn() -> Type(u)>(f: F) -> Type(u) { f() }
        fn good(f: fn() -> Type(1)) -> Type(1) { let alias = apply; alias(f) }";
    assert!(complaints(valid).is_empty(), "{:?}", complaints(valid));
    let invalid = "fn apply<universe u, F: fn() -> Type(u)>(f: F) -> Type(u) { f() }
        fn bad(f: fn() -> Type(2)) -> Type(1) { let alias = apply; alias(f) }";
    assert!(!complaints(invalid).is_empty());
}

#[test]
fn callable_constraints_survive_value_position_coercions() {
    let declaration = "fn apply<universe u, F: fn() -> Type(u)>(f: F) -> Type(u) { f() }";
    for level in [1, 2] {
        for expression in [
            format!("let alias: fn(fn() -> Type({level})) -> Type(1) = apply; alias(f)"),
            "(apply)(f)".into(),
            format!(
                "fn invoke(g: fn(fn() -> Type({level})) -> Type(1), h: fn() -> Type({level})) -> Type(1) {{ g(h) }} invoke(apply, f)"
            ),
        ] {
            let source = format!(
                "{declaration} fn use_it(f: fn() -> Type({level})) -> Type(1) {{ {expression} }}"
            );
            let errors = complaints(&source);
            if level == 1 {
                assert!(errors.is_empty(), "{source}: {errors:?}");
            } else {
                assert!(!errors.is_empty(), "bound lost: {source}");
            }
        }
    }
}

#[test]
fn callable_bounds_do_not_hide_free_level_names() {
    let errors = complaints("fn invalid<F: fn() -> Type(missing)>(f: F) { }");
    assert!(
        errors
            .iter()
            .any(|error| error.contains("unbound") || error.contains("undeclared")),
        "{errors:?}"
    );
}
