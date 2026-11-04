//! Bottom elimination is directed; type equality cannot manufacture bottom.
use verum_ast::span::Span;
use verum_fast_parser::Parser;
use verum_types::{TypeChecker, ty::{Type, TypeVar}, unify::Unifier};

fn errors(source: &str) -> Vec<String> {
    let module = Parser::new(source).parse_module().expect("valid syntax");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("bottom_test");
    let mut errors: Vec<_> = checker.prepare_module_declarations(&module)
        .iter().map(|error| format!("{error:?}")).collect();
    for item in &module.items {
        if let Err(error) = checker.check_item(item) { errors.push(format!("{error:?}")); }
    }
    errors.extend(checker.take_deferred_errors().iter().map(|error| format!("{error:?}")));
    errors.extend(checker.diagnostics().iter().filter(|d| d.is_error()).map(|d| format!("{d:?}")));
    errors
}

#[test]
fn equality_does_not_coerce_bottom_in_either_direction() {
    for inhabited in [Type::Unit, Type::Int, Type::Bool, Type::Unknown] {
        for (left, right) in [(&inhabited, &Type::Never), (&Type::Never, &inhabited)] {
            assert!(Unifier::new().unify(left, right, Span::dummy()).is_err());
        }
    }
    assert!(Unifier::new().unify(&Type::Never, &Type::Never, Span::dummy()).is_ok());
    let variable = TypeVar::fresh();
    let mut unifier = Unifier::new();
    unifier.unify(&Type::Var(variable), &Type::Never, Span::dummy()).unwrap();
    assert_eq!(unifier.apply(&Type::Var(variable)), Type::Never);
    assert!(unifier.unify(&Type::Var(variable), &Type::Int, Span::dummy()).is_err());
}

#[test]
fn nested_carriers_do_not_identify_bottom_with_a_value_type() {
    let bottom = Type::function(vec![Type::Never].into(), Type::Unit);
    let inhabited = Type::function(vec![Type::Int].into(), Type::Unit);
    for (left, right) in [(&bottom, &inhabited), (&inhabited, &bottom)] {
        assert!(Unifier::new().unify(left, right, Span::dummy()).is_err());
    }
}

#[test]
fn inhabited_expressions_cannot_supply_bottom() {
    for source in [
        "fn forge() -> ! { () }",
        "type Empty is !; fn forge() -> Empty { () }",
        "type Empty is !; type Alias is Empty; fn forge() -> Alias { 42 }",
        "fn consume(x: !) -> Int { x } fn forge() -> Int { consume(7) }",
        "type Holder is { value: ! }; fn forge() -> Holder { Holder { value: () } }",
        "fn forge() -> fn() -> ! { || { () } }",
        "fn forge(x: unknown) -> ! { x }",
        "type Empty is !; fn forge(x: unknown) -> Empty { x }",
        "fn forge(b: Bool, x: !) -> ! { if b { x } }",
        "fn forge(b: Bool) -> Int { if b { 42 } }",
    ] {
        assert!(!errors(source).is_empty(), "accepted: {source}");
    }
}

#[test]
fn bottom_elimination_resolves_real_aliases() {
    for source in [
        "fn eliminate<A>(x: !) -> A { x }",
        "type Empty is !; fn eliminate<A>(x: Empty) -> A { x }",
        "type Empty is !; type Alias is Empty; fn eliminate<A>(x: Alias) -> A { x }",
        "fn select(b: Bool, x: !) -> Int { let y = if b { x } else { 7 }; y }",
        "fn select(b: Bool, x: !) -> Int { let y = if b { 7 } else { x }; y }",
    ] {
        assert!(errors(source).is_empty(), "{source}: {:?}", errors(source));
    }
}

#[test]
fn a_diverging_branch_does_not_hide_the_other_branch_type() {
    for source in [
        "fn forge(b: Bool, x: !) -> Bool { let y = if b { x } else { 7 }; y }",
        "fn forge(b: Bool, x: !) -> ! { let y = if b { x } else { 7 }; y }",
    ] {
        assert!(!errors(source).is_empty(), "accepted: {source}");
    }
}

#[test]
fn a_nominal_name_does_not_authorize_bottom_elimination() {
    assert!(errors("type Never is { value: Int }; fn good(x: Never) -> Never { x }").is_empty());
    assert!(!errors("type Never is { value: Int }; fn forge<A>(x: Never) -> A { x }").is_empty());
}

#[test]
fn record_literals_require_a_declared_record_constructor() {
    for source in [
        "type Empty is !; fn forge() -> Empty { Empty {} }",
        "type Number is Int; fn forge() -> Number { Number {} }",
        "type Callback is fn() -> Int; fn forge() -> Callback { Callback {} }",
        "fn forge<T>() -> T { T {} }",
    ] {
        assert!(!errors(source).is_empty(), "accepted: {source}");
    }
}

#[test]
fn actual_empty_records_and_record_aliases_keep_their_constructors() {
    for source in [
        "type Empty is {}; fn make() -> Empty { Empty {} }",
        "type Value is { n: Int }; type Alias is Value; fn make() -> Alias { Alias { n: 7 } }",
        "type Value<T> is { n: T }; type Alias<T> is Value<T>; fn make() -> Alias<Int> { Alias { n: 7 } }",
    ] {
        let errors = errors(source);
        assert!(errors.is_empty(), "{source}: {errors:?}");
    }
}
