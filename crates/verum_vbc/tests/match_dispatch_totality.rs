//! The backend must respect every arm even when source coverage is not proved.
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::{Interpreter, InterpreterError},
    serialize::serialize_module,
    validate::validate_module,
};

fn check(body: &str, expected: Option<i64>) {
    let source = format!("module dispatch; fn probe() -> Int {{{body}}}");
    let ast = Parser::new(&source).parse_module().expect("source grammar");
    let module = VbcCodegen::with_config(CodegenConfig::new("dispatch").with_validation())
        .compile_module(&ast)
        .expect("source lowering");
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for (route, module) in [("produced", module), ("wire", wire)] {
        validate_module(&module).expect("strict VBC validation");
        let probe = module.find_function_by_name("dispatch.probe").unwrap();
        let value = Interpreter::new(Arc::new(module)).execute_function(probe);
        if let Some(expected) = expected {
            assert_eq!(
                value
                    .unwrap_or_else(|error| panic!("{route}: {error:?}"))
                    .try_as_i64(),
                Some(expected),
                "{route}: {body}"
            );
        } else {
            assert!(
                matches!(value, Err(InterpreterError::Panic { ref message })
                if message.contains("non-exhaustive match")),
                "{route}: {body}: {value:?}"
            );
        }
    }
}

#[test]
fn a_false_final_guard_never_executes_its_body() {
    check("match 7 {0 => 5, _ if false => 99}", None);
    check("match true {_ if false => 99}", None);
}

#[test]
fn a_mismatched_final_pattern_never_executes_its_body() {
    check("match 7 {0 => 5, 1 => 99}", None);
    check("match true {false => 99}", None);
    check("match (1, 2) {(1, 3) => 99}", None);
}

#[test]
fn valid_guards_and_patterns_still_select_the_correct_arm() {
    for (body, expected) in [
        ("match 7 {0 => 5, value if value == 7 => value + 1}", 8),
        ("match 7 {value if false => 99, value => value + 1}", 8),
        ("match 0 {0 => 5, _ if false => 99}", 5),
        ("match true {false => 99, true => 8}", 8),
        (
            "let mut visits = 0; let value = match 7 {
            x if {visits += 1; false} => 99, x => x}; visits * 10 + value",
            17,
        ),
    ] {
        check(body, Some(expected));
    }
}

#[test]
fn numeric_patterns_observe_full_width_and_ieee_comparisons() {
    for (body, expected) in [
        (
            "match 9223372036854775807 {0..9223372036854775807 => 99, _ => 8}",
            8,
        ),
        (
            "match 9223372036854775807 {0..=9223372036854775807 => 8, _ => 99}",
            8,
        ),
        ("match -0.0 {0.0 => 8, _ => 99}", 8),
        ("match 0.25 {0.0 => 99, 0.25 => 8, _ => 99}", 8),
        ("let value = 0.0 / 0.0; match value {0.0 => 99, _ => 8}", 8),
    ] {
        check(body, Some(expected));
    }
}
