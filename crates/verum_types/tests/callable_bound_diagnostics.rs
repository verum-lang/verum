//! Callable diagnostics report requirements separately from observed properties.
use verum_ast::{FileId, Span};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_types::computational_properties::{
    ComputationalProperty as Property, PropertyRequirement as Bound, PropertySet,
};
use verum_types::{Type, TypeChecker, TypeError, Unifier};

fn callable(bound: Bound) -> Type {
    Type::function_with_requirement(List::new(), Type::Int, bound)
}

fn mismatch(actual: &Type, expected: &Type) -> (Text, Text) {
    let span = Span::new(13, 29, FileId::new(9));
    let error = Unifier::new()
        .unify(actual, expected, span)
        .expect_err("rendering must not authorize incompatible callable bounds");
    let TypeError::Mismatch {
        expected,
        actual,
        span: found,
    } = &error
    else {
        panic!("wrong refusal: {error:?}");
    };
    assert_eq!(*found, span);
    let diagnostic = error.to_diagnostic();
    assert_eq!(diagnostic.code(), Some("E400"));
    assert_eq!(
        diagnostic.message(),
        format!("Type mismatch: expected '{expected}', found '{actual}'")
    );
    (expected.clone(), actual.clone())
}

#[test]
fn unknown_is_not_rendered_as_observed_async_or_proven_sync() {
    let unknown = Type::function(List::new(), Type::Int);
    let sync = callable(Bound::source_callable(false));
    let async_bound = callable(Bound::AtMost(PropertySet::single(Property::Async)));
    assert_ne!(unknown, async_bound);
    assert_eq!(
        unknown.to_string(),
        async_bound.to_string(),
        "display text is not semantic identity"
    );
    let (expected, actual) = mismatch(&unknown, &sync);
    assert_eq!(
        expected,
        "fn() -> Int [bound: excludes {Async}; observed: unknown]"
    );
    assert_eq!(
        actual,
        "fn() -> Int [bound: unrestricted; observed: unknown]"
    );
    // The archive/spelling contract is deliberately not this diagnostic view.
    assert_eq!(unknown.to_string(), "async fn() -> Int");
    assert_eq!(sync.to_string(), "fn() -> Int");
}

#[test]
fn an_unrelated_exclusion_does_not_prove_sync() {
    let actual = callable(Bound::Excludes(PropertySet::single(Property::Mutates)));
    let (_, actual) = mismatch(&actual, &callable(Bound::source_callable(false)));
    assert_eq!(
        actual,
        "fn() -> Int [bound: excludes {Mutates}; observed: unknown]"
    );
}

#[test]
fn an_async_upper_bound_is_not_an_observed_async_body() {
    let actual = callable(Bound::AtMost(PropertySet::single(Property::Async)));
    let (_, actual) = mismatch(&actual, &callable(Bound::source_callable(false)));
    assert_eq!(
        actual,
        "fn() -> Int [bound: at most {Async}; observed: unknown]"
    );
}

#[test]
fn observed_async_remains_explicit_and_rejected_by_sync() {
    let actual = Type::function_with_properties(
        List::new(),
        Type::Int,
        PropertySet::single(Property::Async),
    );
    let (_, actual) = mismatch(&actual, &callable(Bound::source_callable(false)));
    assert_eq!(
        actual,
        "async fn() -> Int [bound: at most {Async}; observed: {Async}]"
    );
}

#[test]
fn observed_sync_is_not_unknown_even_when_its_bound_is_unrestricted() {
    let mut actual = Type::function(List::new(), Type::Int);
    let Type::Function { properties, .. } = &mut actual else {
        unreachable!()
    };
    *properties = Some(PropertySet::pure());
    // Equality of requirements is unchanged; this exercises diagnostic evidence,
    // not directional callable subtyping.
    let (_, actual) = mismatch(&actual, &callable(Bound::source_callable(false)));
    assert_eq!(actual, "fn() -> Int [bound: unrestricted; observed: Pure]");
}

#[test]
fn nested_callback_diagnostics_keep_their_own_bounds() {
    let unknown = Type::function(List::new(), Type::Int);
    let actual = Type::Tuple(
        [Type::Reference {
            mutable: false,
            inner: Box::new(unknown),
        }]
        .into_iter()
        .collect(),
    );
    let expected = Type::Tuple(List::new());
    let (_, rendered) = mismatch(&actual, &expected);
    assert_eq!(
        rendered,
        "(&fn() -> Int [bound: unrestricted; observed: unknown])"
    );
    assert_eq!(actual.to_string(), "(&async fn() -> Int)");
}

#[test]
fn bound_members_have_a_deterministic_order() {
    let set = PropertySet::from_properties([Property::Mutates, Property::IO, Property::Async]);
    let (_, actual) = mismatch(&callable(Bound::Excludes(set)), &Type::Unit);
    assert_eq!(
        actual,
        "fn() -> Int [bound: excludes {Async, IO, Mutates}; observed: unknown]"
    );
}

fn check(source: &str) -> List<Text> {
    let ast = FastParser::new()
        .parse_module_str(source, FileId::new(7))
        .expect("source grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    let mut errors = checker.prepare_module_declarations(&ast);
    errors.extend(
        ast.items
            .iter()
            .filter_map(|item| checker.check_item(item).err()),
    );
    errors.extend(checker.take_deferred_errors());
    errors
        .iter()
        .map(|error| error.to_diagnostic().message().into())
        .chain(
            checker
                .diagnostics()
                .iter()
                .filter(|error| error.is_error())
                .map(|error| error.message().into()),
        )
        .collect()
}

#[test]
fn actual_unknown_callback_and_explicit_async_have_distinct_diagnostics() {
    let unknown = check(
        "fn bridge(callback: async fn() -> Int) -> Int { let thunk: fn() -> Int = || -> Int { callback() }; thunk() }",
    );
    assert!(!unknown.is_empty());
    assert!(
        unknown
            .iter()
            .all(|message| message.contains("Type mismatch")),
        "{unknown:?}"
    );
    assert!(
        unknown
            .iter()
            .any(|message| message.contains("bound: unrestricted; observed: unknown")),
        "{unknown:?}"
    );
    assert!(
        unknown
            .iter()
            .all(|message| !message.contains("found 'async fn")),
        "{unknown:?}"
    );

    let asynchronous = check(
        "fn accept(callback: fn() -> Int) -> Int { callback() } fn bad() -> Int { accept(async || { 73 }) }",
    );
    assert!(!asynchronous.is_empty());
    assert!(
        asynchronous
            .iter()
            .any(|message| message.contains("found 'async fn")
                && message.contains("observed: {Async}")),
        "{asynchronous:?}"
    );

    let synchronous = check(
        "fn accept(callback: fn() -> Int) -> Int { callback() } fn good() -> Int { accept(|| { 73 }) }",
    );
    assert!(synchronous.is_empty(), "{synchronous:?}");
}
