//! Both protocol spellings retain the same declaring-source authority.
use crate::owned_source_semantics::{check_source_module, source_registry};
use verum_ast::FileId;
use verum_common::List;
use verum_fast_parser::FastParser;
use verum_types::TypeChecker;

#[test]
fn source_context_kind_is_explicit_for_local_and_imported_implementations() {
    for (declaration, accepted) in [
        ("context Signal { fn read() -> Int; }", false),
        ("context protocol Signal { fn read() -> Int; }", true),
        (
            "context type Signal is protocol { fn read() -> Int; };",
            true,
        ),
    ] {
        let provider = FastParser::new().parse_module_str(&format!(
            "module realm.driver; public {declaration} public type Device is (); implement Signal for Device {{ fn read() -> Int {{ 42 }} }}"
        ), FileId::new(0)).unwrap();
        let consumer = FastParser::new().parse_module_str(
            "module realm.client; mount realm.driver.{Device, Signal}; fn probe(device: &Device) -> Int using [Signal] { Signal.read() }",
            FileId::new(1)).unwrap();
        let registry = source_registry([
            ("realm.driver", "driver.vr", &provider),
            ("realm.client", "client.vr", &consumer),
        ])
        .unwrap();
        for (owner, ast) in [("realm.driver", &provider), ("realm.client", &consumer)] {
            let mut checker = TypeChecker::with_minimal_context();
            checker.register_builtins();
            let result = check_source_module(&mut checker, owner, ast, &registry);
            assert_eq!(
                result.is_ok(),
                accepted,
                "{owner} {declaration}: {:?}",
                result.as_ref().err().map(|errors| errors
                    .iter()
                    .map(|error| error.message())
                    .collect::<List<_>>())
            );
            if let Err(errors) = result {
                assert!(
                    errors
                        .iter()
                        .any(|error| error.code() == Some("E101")
                            && error.message().contains("Signal")),
                    "wrong kind refusal: {errors:?}"
                );
            }
        }
    }
}

#[test]
fn source_protocol_parameters_and_method_results_stay_owned() {
    for declaration in [
        "context protocol Signal<T> { fn read(value: T) -> T; }",
        "context type Signal<T> is protocol { fn read(value: T) -> T; };",
    ] {
        for (result, accepted) in [("42", true), ("true", false)] {
            let provider = FastParser::new().parse_module_str(&format!(
                "module realm.driver; public {declaration} public type Device is (); implement Signal<Int> for Device {{ fn read(value: Int) -> Int {{ {result} }} }}"
            ), FileId::new(0)).unwrap();
            let registry = source_registry([("realm.driver", "driver.vr", &provider)]).unwrap();
            let mut checker = TypeChecker::with_minimal_context();
            checker.register_builtins();
            let checked = check_source_module(&mut checker, "realm.driver", &provider, &registry);
            assert_eq!(
                checked.is_ok(),
                accepted,
                "{declaration} result={result}: {checked:?}"
            );
            if let Err(errors) = checked {
                assert!(
                    errors.iter().any(|error| error.code() == Some("E400")
                        && error.message().contains("expected 'Int', found 'Bool'")),
                    "wrong return-type refusal: {errors:?}"
                );
            }
        }
    }
}

#[test]
fn imported_context_methods_keep_their_owner_arguments_and_dependencies() {
    for declaration in [
        "context protocol Signal<T> { fn read(value: T) -> T; }",
        "context type Signal<T> is protocol { fn read(value: T) -> T; };",
    ] {
        let provider = FastParser::new()
            .parse_module_str(
                &format!("module realm.driver; public {declaration}"),
                FileId::new(0),
            )
            .unwrap();
        for (body, requirements, accepted) in [
            ("Wire.read(42)", "using [Wire<Int>]", true),
            ("Wire.read(true)", "using [Wire<Int>]", false),
            ("Wire.read(42)", "", false),
        ] {
            let consumer = FastParser::new().parse_module_str(&format!(
                "module realm.client; mount realm.driver.{{Signal as Wire}}; fn probe() -> Int {requirements} {{ {body} }}"
            ), FileId::new(1)).unwrap();
            let registry = source_registry([
                ("realm.driver", "driver.vr", &provider),
                ("realm.client", "client.vr", &consumer),
            ])
            .unwrap();
            let mut checker = TypeChecker::with_minimal_context();
            checker.register_builtins();
            let result = check_source_module(&mut checker, "realm.client", &consumer, &registry);
            assert_eq!(
                result.is_ok(),
                accepted,
                "{declaration} {body} {requirements}: {result:?}"
            );
        }
    }
}

#[test]
fn protocol_spellings_share_manifest_and_context_admission() {
    for (short, canonical, reason) in [
        (
            "protocol Signal<F<_>> {}",
            "type Signal<F<_>> is protocol {};",
            "higher_kinded_protocols",
        ),
        (
            "protocol Signal { type Item<T>; }",
            "type Signal is protocol { type Item<T>; };",
            "generic_associated_types",
        ),
        (
            "context protocol Signal { fn read<T>(value: T) -> T; }",
            "context type Signal is protocol { fn read<T>(value: T) -> T; };",
            "not object-safe",
        ),
        (
            "context protocol Signal { fn read() -> Self; }",
            "context type Signal is protocol { fn read() -> Self; };",
            "not object-safe",
        ),
    ] {
        for declaration in [short, canonical] {
            let module = FastParser::new()
                .parse_module_str(
                    &format!("module realm.driver; public {declaration}"),
                    FileId::new(0),
                )
                .unwrap();
            let registry = source_registry([("realm.driver", "driver.vr", &module)]).unwrap();
            let mut checker = TypeChecker::with_minimal_context();
            checker.register_builtins();
            let result = check_source_module(&mut checker, "realm.driver", &module, &registry);
            let errors = result.expect_err(declaration);
            assert!(
                errors.iter().any(|error| error.message().contains(reason)),
                "{declaration} must retain {reason}: {errors:?}"
            );
            assert!(
                checker
                    .protocol_checker
                    .read()
                    .get_protocol_definition("realm.driver.Signal")
                    .is_none(),
                "failed context protocol was published"
            );
            if reason != "not object-safe" {
                let mut enabled = TypeChecker::with_minimal_context();
                enabled.register_builtins();
                enabled.set_higher_kinded_enabled(true);
                enabled.set_higher_kinded_protocols_enabled(true);
                enabled.set_generic_associated_types_enabled(true);
                let result = check_source_module(&mut enabled, "realm.driver", &module, &registry);
                assert!(result.is_ok(), "enabled {declaration}: {result:?}");
            }
        }
    }
}

#[test]
fn imported_context_result_keeps_the_declaring_nominal_owner() {
    let provider = FastParser::new().parse_module_str(
        "module realm.driver; public type Source is (Int); public context protocol Signal { fn read() -> Source; }",
        FileId::new(0)).unwrap();
    for (output, accepted) in [("Origin", true), ("Source", false)] {
        let consumer = FastParser::new().parse_module_str(&format!(
            "module realm.client; mount realm.driver.{{Source as Origin, Signal as Wire}}; type Source is (Bool); fn probe() -> {output} using [Wire] {{ Wire.read() }}"
        ), FileId::new(1)).unwrap();
        let registry = source_registry([
            ("realm.driver", "driver.vr", &provider),
            ("realm.client", "client.vr", &consumer),
        ])
        .unwrap();
        let mut checker = TypeChecker::with_minimal_context();
        checker.register_builtins();
        let result = check_source_module(&mut checker, "realm.client", &consumer, &registry);
        assert_eq!(
            result.is_ok(),
            accepted,
            "selected result {output}: {result:?}"
        );
    }
}

#[test]
fn context_methods_retain_inherited_contracts_and_nested_dependencies() {
    for (source, accepted) in [
        (
            "type Base<T> is protocol { fn read(value: T) -> T; }; context protocol Signal extends Base<Int> {} fn probe() -> Int using [Signal] { Signal.read(42) }",
            true,
        ),
        (
            "type Base<T> is protocol { fn read(value: T) -> T; }; context protocol Signal extends Base<Int> {} fn probe() -> Int using [Signal] { Signal.read(true) }",
            false,
        ),
        (
            "context Clock { fn tick(); } context protocol Signal { fn read() -> Int using [Clock]; } fn probe() -> Int using [Signal, Clock] { Signal.read() }",
            true,
        ),
        (
            "context Clock { fn tick(); } context protocol Signal { fn read() -> Int using [Clock]; } fn probe() -> Int using [Signal] { Signal.read() }",
            false,
        ),
    ] {
        let module = FastParser::new()
            .parse_module_str(source, FileId::new(0))
            .unwrap();
        let registry = source_registry([("realm.driver", "driver.vr", &module)]).unwrap();
        let mut checker = TypeChecker::with_minimal_context();
        checker.register_builtins();
        let result = check_source_module(&mut checker, "realm.driver", &module, &registry);
        assert_eq!(result.is_ok(), accepted, "{source}: {result:?}");
    }
}

#[test]
fn loading_a_protocol_dependency_does_not_mount_its_context_name() {
    let provider = FastParser::new().parse_module_str(
        "module realm.driver; public context protocol Signal { fn read() -> Int; } public type Device is { capability: Signal };",
        FileId::new(0)).unwrap();
    let consumer = FastParser::new().parse_module_str(
        "module realm.client; mount realm.driver.{Device}; fn probe(device: &Device) -> Int using [Signal] { Signal.read() }",
        FileId::new(1)).unwrap();
    let registry = source_registry([
        ("realm.driver", "driver.vr", &provider),
        ("realm.client", "client.vr", &consumer),
    ])
    .unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_builtins();
    let result = check_source_module(&mut checker, "realm.client", &consumer, &registry);
    let errors =
        result.expect_err("a transitive type dependency became an implicit context import");
    assert!(
        errors
            .iter()
            .any(|error| error.code() == Some("E605") && error.message().contains("Signal")),
        "wrong refusal: {errors:?}"
    );
}
