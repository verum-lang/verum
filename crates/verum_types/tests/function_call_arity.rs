//! Generic arguments, value spellings and unsafe blocks do not change runtime arity.
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn check(source: &str, stdlib_source: bool) -> List<Text> {
    let module = Parser::new(source).parse_module().expect("source grammar");
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.stdlib_single_file_mode = stdlib_source;
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
            .diagnostic_sources()
            .iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors
}

fn rejects_arity(source: &str) {
    for stdlib_source in [false, true] {
        let errors = check(source, stdlib_source);
        assert!(
            errors.iter().any(|error| error.contains("E102")),
            "stdlib_source={stdlib_source}, {source}: {errors:?}"
        );
    }
}

#[test]
fn generic_arguments_do_not_supply_missing_values() {
    rejects_arity("fn identity<T>(value:T)->T {value} fn probe()->Int {identity<Int>()}");
}

#[test]
fn generic_arguments_do_not_allow_extra_values() {
    rejects_arity("fn identity<T>(value:T)->T {value} fn probe()->Int {identity<Int>(7,8)}");
}

#[test]
fn unsafe_body_preserves_value_arity() {
    rejects_arity("fn single(value:Int)->Int {value} fn probe()->Int {unsafe {single(7,8)}}");
}

#[test]
fn uppercase_values_preserve_value_arity() {
    rejects_arity(
        "fn single(value:Int)->Int {value} fn probe()->Int {let Value=7; single(Value,Value)}",
    );
}

#[test]
fn stdlib_source_preserves_missing_and_extra_value_checks() {
    rejects_arity("fn single(value:Int)->Int {value} fn probe()->Int {single()}");
    rejects_arity("fn single(value:Int)->Int {value} fn probe()->Int {single(7,8)}");
}

#[test]
fn exact_generic_values_and_declared_defaults_remain_valid() {
    for source in [
        "fn identity<T>(value:T)->T {value} fn probe()->Int {identity<Int>(7)}",
        "fn choose<T>(value:T, extra:Int=5)->T {value} fn probe()->Int {choose<Int>(7)}",
        "fn choose<T>(value:T, extra:Int=5)->T {value} fn probe()->Int {choose<Int>(7,8)}",
    ] {
        for stdlib_source in [false, true] {
            let errors = check(source, stdlib_source);
            assert!(
                errors.is_empty(),
                "stdlib_source={stdlib_source}, {source}: {errors:?}"
            );
        }
    }
}
