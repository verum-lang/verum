use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(body: &str) -> List<Text> {
    let source = format!(
        "type Carrier is (Int);
         type Positive is Carrier where |value| value.0 > 0;
         {body}"
    );
    let module = Parser::new(&source).parse_module().expect("valid grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    let mut errors = checker.prepare_module_declarations(&module);
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error);
        }
    }
    errors.extend(checker.take_deferred_errors());
    let mut result: List<Text> = errors.iter().map(|error| error.to_string().into()).collect();
    result.extend(checker.diagnostics().iter().filter(|error| error.is_error())
        .map(|error| error.message().into()));
    result
}

#[test]
fn selected_alias_methods_retain_their_actual_callable_contracts() {
    let accepted = errors("implement Positive { pure fn read(&self) -> Int { self.0 } }
        pure fn use_value(value: &Positive) -> Int { value.read() }");
    assert!(accepted.is_empty(), "{accepted:?}");

    let rejected = errors("static mut observed: Int = 1;
        implement Positive { fn read(&self) -> Int { observed } }
        pure fn use_value(value: &Positive) -> Int { value.read() }");
    assert!(rejected.iter().any(|error| error.contains("pure function")), "{rejected:?}");
}

#[test]
fn selected_alias_methods_enforce_protocol_and_explicit_type_arguments() {
    let declarations = "type Mark is protocol {};
        implement Mark for Int {}
        implement Positive { fn accept<T: Mark>(&self, item: T) -> T { item } }";
    let accepted = errors(&format!("{declarations}
        fn use_value(value: &Positive) -> Int {{ value.accept<Int>(7) }}"));
    assert!(accepted.is_empty(), "{accepted:?}");
    for body in [
        "fn bad(value: &Positive) -> Bool { value.accept(true) }",
        "fn bad(value: &Positive) -> Bool { value.accept<Bool>(7) }",
    ] {
        let rejected = errors(&format!("{declarations} {body}"));
        assert!(!rejected.is_empty(), "accepted {body}");
    }
}

#[test]
fn selected_alias_methods_do_not_erase_callable_argument_bounds() {
    let declarations = "implement Positive {
        fn apply<F: fn(Int) -> Int>(&self, function: F) -> Int { function(7) }
    }";
    let accepted = errors(&format!("{declarations}
        fn add(value: Int) -> Int {{ value + 1 }}
        fn use_value(value: &Positive) -> Int {{ value.apply(add) }}"));
    assert!(accepted.is_empty(), "{accepted:?}");
    let rejected = errors(&format!("{declarations}
        async fn pending(value: Int) -> Int {{ value }}
        fn bad(value: &Positive) -> Int {{ value.apply(pending) }}"));
    assert!(!rejected.is_empty(), "accepted an asynchronous callback");
}
