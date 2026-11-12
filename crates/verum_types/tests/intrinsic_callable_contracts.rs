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
fn a_checked_intrinsic_body_cannot_invent_purity() {
    for source in [
        "pure fn probe(number: Int) -> Int { @intrinsic(\"syscall0\", number) }",
        "fn invoke(number: Int) -> Int { @intrinsic(\"syscall0\", number) }
         pure fn probe(number: Int) -> Int { invoke(number) }",
    ] {
        let rejected = errors(source);
        assert!(
            rejected
                .iter()
                .any(|error| error.contains("ImpurePureFunction")),
            "accepted an unproved operation footprint: {source}: {rejected:?}"
        );
    }
}

#[test]
fn an_unrestricted_body_can_still_use_an_intrinsic() {
    let source = "fn invoke(number: Int) -> Int { @intrinsic(\"syscall0\", number) }
        fn probe(number: Int) -> Int { invoke(number) }";
    let accepted = errors(source);
    assert!(accepted.is_empty(), "{accepted:?}");
}

#[test]
fn unknown_intrinsic_properties_do_not_satisfy_a_callback_bound() {
    let source = "fn invoke(number: Int) -> Int { @intrinsic(\"syscall0\", number) }
        static mut CALLBACK: fn(Int) -> Int = invoke;";
    let rejected = errors(source);
    assert!(
        rejected
            .iter()
            .any(|error| error.contains("Mismatch") && error.contains("async")),
        "callback lost its missing-property obligation: {rejected:?}"
    );
}

#[test]
fn registered_constants_satisfy_meta_purity() {
    for source in [
        "meta fn mode() -> Bool { @intrinsic(\"is_debug\") }",
        "meta fn width() -> UInt32 { @intrinsic(\"target_pointer_width\") }",
        "meta fn infinity() -> Float { @intrinsic(\"f64_infinity\") }",
        "fn mode() -> Bool { @intrinsic(\"is_debug\") }
         pure fn probe() -> Bool { mode() }",
        "fn mode() -> Bool { @intrinsic(\"is_debug\") }
         static mut CALLBACK: fn() -> Bool = mode;",
    ] {
        let accepted = errors(source);
        assert!(accepted.is_empty(), "{source}: {accepted:?}");
    }
}

#[test]
fn registered_target_queries_satisfy_meta_purity() {
    for source in [
        "meta fn target_has_atomic<T>() -> Bool { @intrinsic(\"target_has_atomic\", T) }",
        "meta fn target_has_feature(name: Text) -> Bool { @intrinsic(\"target_has_feature\", name) }",
    ] {
        let accepted = errors(source);
        assert!(accepted.is_empty(), "{source}: {accepted:?}");
    }
}

#[test]
fn registered_layout_queries_satisfy_meta_purity() {
    for name in ["size_of", "align_of", "stride_of", "bits_of", "type_id"] {
        let source = format!("meta fn probe<T>() -> USize {{ @intrinsic(\"{name}\", T) }}");
        let accepted = errors(&source);
        assert!(accepted.is_empty(), "{source}: {accepted:?}");
    }
}

#[test]
fn retired_reflection_functions_require_an_actual_declaration() {
    for name in ["size_of", "align_of", "stride_of", "bits_of", "type_id", "type_name"] {
        for call in [format!("{name}<Int>()"), format!("{name}(Int)")] {
            let rejected = errors(&format!("fn probe() {{ let value = {call}; }}"));
            assert!(rejected.iter().any(|error| error.contains(name)), "ambient {call} was accepted: {rejected:?}");
        }
        let source = format!("fn {name}(value: Int) -> Int {{ value + 1 }} fn probe() -> Int {{ {name}(72) }}");
        let accepted = errors(&source);
        assert!(accepted.is_empty(), "{name} declaration was intercepted: {accepted:?}");
        let rejected = errors(&source.replace(&format!("{name}(72)"), &format!("{name}(72, 1)")));
        assert!(!rejected.is_empty(), "{name} acquired a variadic builtin contract");
    }
    for name in ["size_of", "align_of", "type_id", "type_name"] {
        assert!(!errors(&format!("fn probe() {{ let value = @{name}(Bool); }}")).is_empty(), "@{name} accepted an obsolete query");
    }
}

#[test]
fn identity_surfaces_infer_the_same_unsigned_carrier() {
    for source in ["Int.id", "@intrinsic(\"type_id\", Int)", "@vbc(TYPE_ID, Int)"] {
        let expression = Parser::new(source).parse_expr().unwrap();
        let mut checker = TypeChecker::new();
        checker.register_builtins();
        assert_eq!(checker.synth_expr(&expression).unwrap().ty, verum_types::Type::uint64(), "{source}");
    }
    assert!(errors("meta fn identity<T>() -> UInt64 { @vbc(TYPE_ID, T) }").is_empty());
    assert!(errors("meta fn unsound<T>() -> UInt64 { @vbc(NO_SUCH_TYPE_QUERY, T) }")
        .iter().any(|error| error.contains("ImpureMetaFunction")));
}

#[test]
fn inspection_result_and_purity_follow_the_selected_operation() {
    let alignment = errors("meta fn min_align() -> USize { UInt8.alignment as USize }");
    assert!(alignment.is_empty(), "{alignment:?}");
    for (operation, result) in [("type_name", "Text"), ("needs_drop", "Bool")] {
        for body in [format!("@intrinsic(\"{operation}\", T)"), format!("@vbc({}, T)", operation.to_uppercase())] {
            let source = format!("meta fn probe<T>() -> {result} {{ {body} }}");
            let accepted = errors(&source);
            assert!(accepted.is_empty(), "{source}: {accepted:?}");
        }
    }
    assert!(errors("meta fn wrong<T>() -> Bool { @intrinsic(\"type_name\", T) }")
        .iter().any(|error| error.contains("Mismatch")));
}

#[test]
fn runtime_and_unknown_operations_do_not_acquire_constant_contracts() {
    for source in [
        "meta fn probe(number: Int) -> Int { @intrinsic(\"syscall0\", number) }",
        "meta fn probe() -> Int { @intrinsic(\"num_cpus\") }",
        "meta fn probe() -> Bool { @intrinsic(\"unregistered_constant\") }",
        // Even a registered constant must retain evaluation of its operands.
        "fn work() -> Int { @intrinsic(\"syscall0\", 1) }
         pure fn probe() -> Bool { @intrinsic(\"is_debug\", work()) }",
    ] {
        let rejected = errors(source);
        assert!(
            rejected.iter().any(|error| error.contains("ImpureMetaFunction")
                || error.contains("ImpurePureFunction")),
            "operation acquired an unproved constant contract: {source}: {rejected:?}"
        );
    }
}
