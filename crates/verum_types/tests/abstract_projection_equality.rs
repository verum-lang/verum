//! An unresolved associated projection is a type expression, not a wildcard.
use verum_ast::Span;
use verum_common::List;
use verum_fast_parser::Parser;
use verum_types::{Type, TypeChecker, ty::TypeVar, unify::Unifier};
fn projection(name: &str, receiver: Type) -> Type {
    Type::Generic {
        name: format!("::{name}").into(),
        args: [receiver].into_iter().collect::<List<_>>(),
    }
}
fn errors(source: &str) -> Vec<String> {
    let ast = Parser::new(source).parse_module().unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("example");
    let mut errors = checker.prepare_module_declarations(&ast);
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error);
        }
    }
    errors.extend(checker.take_deferred_errors());
    errors
        .iter()
        .chain(checker.diagnostic_sources())
        .map(ToString::to_string)
        .collect()
}
const RULE: &str = "type Rule is protocol { type Item; type Other; };";
#[test]
fn abstract_projection_is_not_any_concrete_type() {
    let item = projection("Item", Type::Var(TypeVar::fresh()));
    for other in [Type::Bool, Type::Int, Type::Text, Type::Unit] {
        for (left, right) in [(&item, &other), (&other, &item)] {
            assert!(Unifier::new().unify(left, right, Span::dummy()).is_err());
        }
    }
}
#[test]
fn different_members_are_not_equal_even_with_the_same_receiver() {
    let receiver = Type::Var(TypeVar::fresh());
    let item = projection("Item", receiver.clone());
    let other = projection("Other", receiver);
    assert!(Unifier::new().unify(&item, &other, Span::dummy()).is_err());
}
#[test]
fn inference_variables_retain_the_entire_projection() {
    let item = projection("Item", Type::Var(TypeVar::fresh()));
    let variable = Type::Var(TypeVar::fresh());
    let mut unifier = Unifier::new();
    unifier.unify(&variable, &item, Span::dummy()).unwrap();
    assert_eq!(unifier.apply(&variable), item);
    assert!(
        unifier
            .unify(&variable, &Type::Bool, Span::dummy())
            .is_err()
    );
}
#[test]
fn equal_projections_remain_equal() {
    let item = projection("Item", Type::Var(TypeVar::fresh()));
    Unifier::new().unify(&item, &item, Span::dummy()).unwrap();
}
#[test]
fn generic_bodies_cannot_invent_associated_equalities() {
    for body in [
        "fn wrong<T:Rule>(value:T.Item)->Bool{value}",
        "fn wrong<T:Rule>(value:Bool)->T.Item{value}",
        "fn wrong<T:Rule>(value:(T.Item,Int))->(Bool,Int){value}",
        "fn wrong<T:Rule>(value:T.Item)->T.Other{value}",
    ] {
        let source = format!("{RULE} {body}");
        assert!(!errors(&source).is_empty(), "invented equality: {source}");
    }
}
#[test]
fn declared_equalities_and_identical_projections_remain_usable() {
    for body in [
        "fn same<T:Rule>(value:T.Item)->T.Item{value}",
        "fn declared<T:Rule<Item=Bool>>(value:T.Item)->Bool{value}",
        "fn reverse<T:Rule<Item=Bool>>(value:Bool)->T.Item{value}",
    ] {
        let source = format!("{RULE} {body}");
        let errors = errors(&source);
        assert!(errors.is_empty(), "{source}\n{errors:#?}");
    }
}
#[test]
fn concrete_projection_uses_its_actual_implementation() {
    let prefix = format!(
        "{RULE} type Concrete is {{}}; implement Rule for Concrete {{type Item=Bool; type Other=Int;}}"
    );
    let valid = format!("{prefix} fn yes(value:Concrete.Item)->Bool{{value}}");
    assert!(errors(&valid).is_empty(), "{:?}", errors(&valid));
    assert!(
        !errors(&format!(
            "{prefix} fn no(value:Concrete.Item)->Int{{value}}"
        ))
        .is_empty()
    );
}

#[test]
fn overridable_associated_default_is_not_a_universal_equality() {
    let prefix = "type Rule is protocol {type Item=Int;};";
    assert!(
        !errors(&format!(
            "{prefix} fn wrong<T:Rule>(value:T.Item)->Int{{value}}"
        ))
        .is_empty()
    );
    let valid = format!("{prefix} fn declared<T:Rule<Item=Int>>(value:T.Item)->Int{{value}}");
    assert!(errors(&valid).is_empty(), "{:?}", errors(&valid));
    let override_source = format!(
        "{prefix} type Concrete is {{}}; implement Rule for Concrete {{type Item=Text;}} fn actual(value:Concrete.Item)->Text{{value}}"
    );
    assert!(
        errors(&override_source).is_empty(),
        "{:?}",
        errors(&override_source)
    );
}

#[test]
fn repeated_bounds_keep_the_stronger_consistent_equality() {
    for bounds in ["Rule + Rule<Item=Bool>", "Rule<Item=Bool> + Rule"] {
        let source = format!("{RULE} fn same<T:{bounds}>(value:T.Item)->Bool{{value}}");
        assert!(
            errors(&source).is_empty(),
            "{source}\n{:?}",
            errors(&source)
        );
    }
    assert!(
        !errors(&format!(
            "{RULE} fn wrong<T:Rule<Item=Bool> + Rule<Item=Int>>(value:T.Item)->Bool{{value}}"
        ))
        .is_empty()
    );
}

#[test]
fn inherited_member_equalities_keep_the_declaring_protocol() {
    let source = format!(
        "{RULE} type Child is protocol extends Rule {{}}; fn inherited<T:Child<Item=Bool>>(value:T.Item)->Bool{{value}}"
    );
    assert!(errors(&source).is_empty(), "{:?}", errors(&source));
}
