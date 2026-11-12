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
