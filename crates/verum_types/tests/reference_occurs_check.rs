//! T2848: references cannot turn an impossible type equation into fresh holes.
use verum_ast::{FileId, Span};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_types::{Type, TypeChecker, TypeError, Unifier, ty::TypeVar};

#[derive(Clone, Copy, Debug)]
enum Tier {
    Managed,
    Checked,
    Unsafe,
}

const TIERS: [Tier; 3] = [Tier::Managed, Tier::Checked, Tier::Unsafe];

fn reference(tier: Tier, mutable: bool, inner: Type) -> Type {
    let inner = Box::new(inner);
    match tier {
        Tier::Managed => Type::Reference { mutable, inner },
        Tier::Checked => Type::CheckedReference { mutable, inner },
        Tier::Unsafe => Type::UnsafeReference { mutable, inner },
    }
}

fn refuses_without_binding(unifier: &mut Unifier, left: &Type, right: &Type) {
    let previous = unifier.save_substitution();
    let span = Span::new(13, 29, FileId::new(9));
    let error = unifier
        .unify(left, right, span)
        .expect_err("a recursive reference equation has no finite solution");
    assert!(
        matches!(error, TypeError::InfiniteType { span: found, .. } if found == span),
        "wrong refusal for {left:?} = {right:?}: {error:?}"
    );
    assert_eq!(unifier.get_substitution(), &previous);
}

#[test]
fn every_reference_tier_rejects_its_own_type_variable_in_both_directions() {
    for tier in TIERS {
        for mutable in [false, true] {
            for reverse in [false, true] {
                let mut unifier = Unifier::new();
                let established = Type::Var(TypeVar::fresh());
                unifier
                    .unify(&established, &Type::Bool, Span::dummy())
                    .unwrap();
                let variable = Type::Var(TypeVar::fresh());
                let borrowed = reference(tier, mutable, variable.clone());
                let (left, right) = if reverse {
                    (&borrowed, &variable)
                } else {
                    (&variable, &borrowed)
                };
                refuses_without_binding(&mut unifier, left, right);
                assert_eq!(unifier.apply(&variable), variable);
                assert_eq!(unifier.apply(&established), Type::Bool);
            }
        }
    }
}

#[test]
fn an_indirect_cycle_retains_the_prior_reference_binding() {
    for outer in TIERS {
        for inner in TIERS {
            let mut unifier = Unifier::new();
            let first = Type::Var(TypeVar::fresh());
            let second = Type::Var(TypeVar::fresh());
            let borrowed = reference(outer, false, second.clone());
            unifier.unify(&first, &borrowed, Span::dummy()).unwrap();
            refuses_without_binding(
                &mut unifier,
                &second,
                &reference(inner, false, first.clone()),
            );
            assert_eq!(unifier.apply(&first), borrowed);
            assert_eq!(unifier.apply(&second), second);
        }
    }
}

#[test]
fn a_nested_reference_cycle_does_not_publish_an_earlier_tuple_binding() {
    for tier in TIERS {
        let mut unifier = Unifier::new();
        let first = Type::Var(TypeVar::fresh());
        let recursive = Type::Var(TypeVar::fresh());
        let left = Type::Tuple(List::from_iter([first.clone(), recursive.clone()]));
        let right = Type::Tuple(List::from_iter([
            Type::Int,
            reference(
                tier,
                false,
                Type::Tuple(List::from_iter([Type::Bool, recursive.clone()])),
            ),
        ]));
        refuses_without_binding(&mut unifier, &left, &right);
        assert_eq!(unifier.apply(&first), first);
        assert_eq!(unifier.apply(&recursive), recursive);
    }
}

#[test]
fn distinct_reference_variables_can_still_acquire_a_concrete_element() {
    for tier in TIERS {
        for mutable in [false, true] {
            let mut unifier = Unifier::new();
            let handle = Type::Var(TypeVar::fresh());
            let element = Type::Var(TypeVar::fresh());
            unifier
                .unify(
                    &handle,
                    &reference(tier, mutable, element.clone()),
                    Span::dummy(),
                )
                .unwrap();
            unifier.unify(&element, &Type::Int, Span::dummy()).unwrap();
            assert_eq!(unifier.apply(&handle), reference(tier, mutable, Type::Int));
            assert_eq!(unifier.apply(&element), Type::Int);
        }
    }
}

fn source_errors(source: &str) -> List<Text> {
    let module = FastParser::new()
        .parse_module_str(source, FileId::new(17))
        .expect("source grammar");
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path("fixture.reference_equations");
    let mut errors = checker.prepare_module_declarations(&module);
    if errors.is_empty() {
        checker.with_module_declarations(&module, |checker| {
            errors.extend(
                module
                    .items
                    .iter()
                    .filter_map(|item| checker.check_item(item).err()),
            );
        });
    }
    errors.extend(checker.take_deferred_errors());
    let mut messages: List<Text> = errors
        .iter()
        .map(|error| error.to_diagnostic().message().into())
        .collect();
    messages.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|error| error.to_diagnostic().message().into()),
    );
    messages.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|error| error.is_error())
            .map(|error| error.message().into()),
    );
    messages
}

#[test]
fn source_shadowing_borrows_the_previous_binding() {
    let errors = source_errors("fn probe(value: Int) -> Int { let value = &value; *value }");
    assert!(
        errors.is_empty(),
        "a new source binding is not a recursive type: {errors:?}"
    );
}

#[test]
fn a_generic_method_cannot_turn_its_value_parameter_into_a_reference() {
    let errors = source_errors(
        r#"
        type Holder<T> is { item: T };
        implement<T> Holder<T> {
            fn accept(&self, value: T) {}
            fn invalid(&self, value: T) { self.accept(&value); }
        }
    "#,
    );
    assert!(
        errors.iter().any(|message| message.contains("Infinite type:")),
        "the invalid call must retain its type equation: {errors:?}"
    );
    assert!(
        !errors
            .iter()
            .any(|message| message.contains("no method named")),
        "the declared method is available; its argument is invalid: {errors:?}"
    );
}

#[test]
fn a_declared_borrowed_generic_argument_preserves_the_value_parameter() {
    let errors = source_errors(
        r#"
        type Holder<T> is { item: T };
        implement<T> Holder<T> {
            fn accept(&self, value: &T) {}
            fn valid(&self, value: T) { self.accept(&value); }
        }
    "#,
    );
    assert!(
        errors.is_empty(),
        "a distinct reference formal must remain valid: {errors:?}"
    );
}
