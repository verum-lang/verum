use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(source: &str) -> List<Text> {
    let module = Parser::new(source).parse_module().unwrap();
    let mut checker = TypeChecker::new();
    checker.set_current_module_path("operand");
    checker.register_primitives();
    let mut errors = checker.prepare_module_declarations(&module);
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error);
        }
    }
    errors.extend(checker.take_deferred_errors());
    let mut messages: List<Text> = errors
        .iter()
        .map(|error| error.to_string().into())
        .collect();
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
fn admitted_reference_coercions_keep_the_declared_pointee() {
    for pointer in ["&Int", "&mut Int", "&unsafe Int", "&unsafe mut Int"] {
        let source = format!(
            r#"
            unsafe fn inspect<T>(value: &unsafe T) -> Int {{ T.size as Int }}
            fn consume(value: {pointer}) -> Int {{ unsafe {{ inspect(value) }} }}
        "#
        );
        assert!(
            errors(&source).is_empty(),
            "{pointer}: {:?}",
            errors(&source)
        );
    }
}

#[test]
fn call_coercion_refuses_write_escalation_and_a_different_pointee() {
    for source in [
        "unsafe fn write<T>(value: &unsafe mut T) {} fn consume(value: &Int) { unsafe { write(value); } }",
        "unsafe fn inspect<T>(value: &unsafe T) {} fn consume(value: &mut Int) { unsafe { inspect<Bool>(value); } }",
        "type First is { value: Int }; type Second is { value: Int }; unsafe fn inspect(value: &unsafe First) {} fn consume(value: &mut Second) { unsafe { inspect(value); } }",
    ] {
        let errors = errors(source);
        assert!(errors.iter().any(|error| error.contains("mismatch")), "{source}: {errors:?}");
    }
}
