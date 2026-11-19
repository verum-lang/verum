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

#[test]
fn retained_handle_keeps_the_payload_loan_without_borrowing_the_wrapper() {
    let declarations = r#"
        type Envelope<T> is { stored: T };
        fn retained<T>(handle: &Envelope<T>, count: &mut UInt32) -> Envelope<T> {
            unsafe { @intrinsic("retain_copy", handle, count) }
        }
    "#;
    let valid = errors(&format!(r#"{declarations}
        fn keep(input: &Int, count: &mut UInt32) -> Envelope<&Int> {{
            let wrapper = Envelope {{ stored: input }};
            retained(&wrapper, count)
        }}"#));
    assert!(valid.is_empty(), "{valid:?}");
    let invalid = errors(&format!(r#"{declarations}
        fn dangling(count: &mut UInt32) -> Envelope<&Int> {{
            let local = 7;
            let wrapper = Envelope {{ stored: &local }};
            retained(&wrapper, count)
        }}"#));
    assert!(invalid.iter().any(|error| error.contains("DanglingReference") && error.contains("local")), "{invalid:?}");
}

#[test]
fn concrete_protocol_retain_reads_the_handle_and_only_writes_the_counter() {
    let declarations = r#"
        type Allocation<T> is { count: UInt32, payload: T };
        type Envelope<T> is { payload: T, ptr: &unsafe mut Allocation<T> };
        type Duplicate is protocol { fn duplicate(&self) -> Self; };
        implement<T> Duplicate for Envelope<T> {
            fn duplicate(&self) -> Envelope<T> {
                let allocation = self.ptr;
                unsafe { @intrinsic("retain_copy", self, &mut (*allocation).count) }
            }
        }
    "#;
    let valid = errors(&format!(r#"{declarations}
        fn keep(input: &Int, storage: &unsafe mut Allocation<&Int>) -> Envelope<&Int> {{
            let local = Envelope {{ payload: input, ptr: storage }};
            local.duplicate()
        }}"#));
    assert!(valid.is_empty(), "{valid:?}");
    let dangling = errors(&format!(r#"{declarations}
        fn dangling(storage: &unsafe mut Allocation<&Int>) -> Envelope<&Int> {{
            let short = 29;
            let holder = Envelope {{ payload: &short, ptr: storage }};
            holder.duplicate()
        }}"#));
    assert!(dangling.iter().any(|error| error.contains("DanglingReference") && error.contains("short")), "{dangling:?}");
}

#[test]
fn consuming_the_retained_copy_does_not_invalidate_its_source_handle() {
    let source = r#"
        type Cell is { count: UInt32 };
        type Handle is { pointer: &unsafe mut Cell };
        type Duplicate is protocol { fn duplicate(&self) -> Self; };
        implement Duplicate for Handle {
            fn duplicate(&self) -> Handle {
                let allocation = self.pointer;
                unsafe { @intrinsic("retain_copy", self, &mut (*allocation).count) }
            }
        }
        fn make_handle() -> Handle;
        fn consume(handle: Handle);
        fn keep_using_the_original() {
            let original = make_handle();
            consume(original.duplicate());
            consume(original.duplicate());
        }
    "#;
    let accepted = errors(source);
    assert!(accepted.is_empty(), "{accepted:?}");
    let opaque = source.replace(
        "let allocation = self.pointer;\n                unsafe { @intrinsic(\"retain_copy\", self, &mut (*allocation).count) }",
        "unknown_result(self)",
    ).replace("fn make_handle()", "fn unknown_result(input: &Handle) -> Handle;\n        fn make_handle()");
    let refused = errors(&opaque);
    assert!(refused.iter().any(|error| error.contains("cannot establish the lifetime")
        && error.contains("original")), "{refused:?}");
}

#[test]
fn shared_storage_without_counter_write_permission_stays_read_only() {
    let storage = r#"
        type Allocation is { count: UInt32 };
        type Envelope is { ptr: &unsafe Allocation };
    "#;
    for operation in [
        "(*handle.ptr).count = 4;",
        "@intrinsic(\"retain_copy\", handle, &(*handle.ptr).count);",
        "@intrinsic(\"retain_copy\", handle, &mut (*handle.ptr).count);",
    ] {
        let refused = errors(&format!("{storage} fn change(handle: &Envelope) {{ unsafe {{ {operation} }} }}"));
        assert!(refused.iter().any(|error| error.contains("SharedReferenceWrite")
            || error.contains("cannot write through a shared reference")), "{operation}: {refused:?}");
    }
}
