use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(source: &str) -> List<Text> {
    let ast = Parser::new(source).parse_module().unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    let mut errors = checker.prepare_module_declarations(&ast);
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error);
        }
    }
    errors.extend(checker.take_deferred_errors());
    let mut messages: List<Text> = errors.iter().map(|e| e.to_string().into()).collect();
    messages.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|d| d.message().into()),
    );
    messages
}

#[test]
fn inferred_constructor_values_keep_their_nominal_protocol_evidence() {
    let declarations = "type Event is Empty | Value(Int) | Pair(Int, Bool);
        type Other is Empty | Value(Int) | Pair(Int, Bool);
        type Same<T> is protocol {};
        implement<T> Same<T> for T {}
        fn same<A, B: Same<A>>(a: A, b: B) {}";
    for value in ["Event.Empty", "Event.Value(17)", "Event.Pair(17, true)"] {
        let source = format!("{declarations} fn run(a: Event) {{ let b = {value}; same(a, b); }}");
        let result = errors(&source);
        assert!(result.is_empty(), "{source}: {result:?}");
        let source = source.replace(value, &value.replace("Event.", "Other."));
        assert!(
            !errors(&source).is_empty(),
            "a different constructor owner was accepted: {source}"
        );
    }
}

#[test]
fn constructor_values_freshen_parameters_without_erasing_nominal_arguments() {
    let source = "type Event<T> is Empty | Value(T) | Pair(T, T);
        type Same<T> is protocol {};
        implement<T> Same<T> for T {}
        fn same<A, B: Same<A>>(a: A, b: B) {}
        fn expect_integer(value: Event<Int>) {}
        fn run(a: Event<Int>, b: Event<Bool>) {
            let integers = Event.Value(17); let booleans = Event.Value(true);
            same(a, integers); same(b, booleans);
            let empty = Event.Empty; expect_integer(empty);
            let pair = Event.Pair(true, false); same(b, pair);
        }";
    assert!(errors(source).is_empty(), "{:?}", errors(source));
    let wrong = source.replace("same(b, booleans)", "same(a, booleans)");
    assert!(
        !errors(&wrong).is_empty(),
        "different nominal arguments were accepted"
    );
}

#[test]
fn constructor_function_values_preserve_synchronous_callable_contracts() {
    let source = "type Event<T> is Empty | Value(T);
        fn apply(f: fn(Int) -> Event<Int>) -> Event<Int> { f(17) }
        fn run() -> Event<Int> { let constructor = Event.Value; apply(constructor) }";
    assert!(errors(source).is_empty(), "{:?}", errors(source));
}
