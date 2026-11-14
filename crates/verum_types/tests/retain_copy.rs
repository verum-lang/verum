//! Missing operation evidence must never manufacture a pure callable.
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(source: &str) -> List<Text> {
    let module = Parser::new(source).parse_module().unwrap();
    let mut checker = TypeChecker::new();
    checker.register_builtins();
    let mut errors: List<Text> = checker
        .prepare_module_declarations(&module)
        .into_iter()
        .map(|error| format!("{error:?}").into())
        .collect();
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}").into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|d| d.message().into()),
    );
    errors
}

#[test]
fn retain_copy_result_is_the_borrowed_handle_and_counter_is_exact() {
    let accepted = errors(
        r#"type Handle is { value: Int };
        fn retain(handle: &Handle, counter: &mut UInt32) -> Handle {
            unsafe { @intrinsic("retain_copy", handle, counter) }
        }"#,
    );
    assert!(accepted.is_empty(), "{accepted:?}");
    for counter in ["&UInt32", "&mut UInt64", "&mut Int"] {
        let source = format!(
            r#"type Handle is {{ value: Int }};
            fn retain(handle: &Handle, counter: {counter}) -> Handle {{
                unsafe {{ @intrinsic("retain_copy", handle, counter) }}
            }}"#
        );
        let rejected = errors(&source);
        assert!(rejected.iter().any(|e| e.contains("SharedReferenceWrite") || e.contains("counter must be UInt32")), "{counter}: {rejected:?}");
    }
    assert!(
        !errors(
            r#"type Handle is { value: Int };
        fn wrong(handle: &Handle, counter: &mut UInt32) -> Bool {
            unsafe { @intrinsic("retain_copy", handle, counter) }
        }"#
        )
        .is_empty()
    );
}
#[test]
fn retain_is_not_a_pure_operation() {
    let rejected = errors(
        r#"type Handle is { value: Int };
        pure fn retain(handle: &Handle, counter: &mut UInt32) -> Handle {
            unsafe { @intrinsic("retain_copy", handle, counter) }
        }"#,
    );
    assert!(
        rejected.iter().any(|e| e.contains("ImpurePureFunction")),
        "{rejected:?}"
    );
}

#[test]
fn retain_copy_requires_an_explicit_unsafe_contract() {
    let refused = errors(
        r#"type Handle is { value: Int };
        fn retain(handle: &Handle, counter: &mut UInt32) -> Handle {
            @intrinsic("retain_copy", handle, counter)
        }"#,
    );
    assert!(
        refused
            .iter()
            .any(|error| error.contains("requires unsafe")),
        "{refused:?}"
    );
}
