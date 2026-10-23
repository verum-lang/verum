//! Decimal TOML numbers consume the declared Result-returning Text parsers.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{ItemFailurePolicy, VbcCodegen};
use verum_vbc::interpreter::Interpreter;

#[test]
fn decimal_numbers_and_numeric_failures_use_the_real_parser() {
    let toml = Parser::new(include_str!("../../../core/configuration/toml.vr"))
        .parse_module().expect("TOML grammar");
    let maybe = Parser::new(include_str!("../../../core/base/maybe.vr"))
        .parse_module().expect("Maybe grammar");
    let result = Parser::new(include_str!("../../../core/base/result.vr"))
        .parse_module().expect("Result grammar");
    let text = Parser::new(include_str!("../../../core/text/text.vr"))
        .parse_module().expect("Text grammar");
    let caller = Parser::new(r#"
module core.configuration.toml;
fn numeric_control(source: &Text) -> Result<TomlValue, TomlError> {
    let mut parser = new_parser(source.as_bytes());
    let result = parse_number(&mut parser);
    assert_eq(parser.pos, source.len());
    result
}
fn integer_control(source: &Text, expected: Int) {
    match numeric_control(source) {
        Result.Ok(TomlValue.TomlInt(value)) => assert_eq(value, expected),
        _ => panic("valid decimal integer refused or misclassified"),
    }
}
fn float_control(source: &Text, expected: Float) {
    match numeric_control(source) {
        Result.Ok(TomlValue.TomlFloat(value)) => assert_eq(value, expected),
        _ => panic("valid decimal float refused or misclassified"),
    }
}
fn number_controls() -> Int {
    integer_control(&"0", 0);
    integer_control(&"1", 1);
    integer_control(&"-42", -42);
    integer_control(&"+42", 42);
    integer_control(&"1_024", 1024);
    float_control(&"1.5", 1.5);
    float_control(&"-0.5", -0.5);
    float_control(&"2e3", 2000.0);
    float_control(&"1_000.25", 1000.25);
    assert(numeric_control(&"9223372036854775808") is Result.Err(_));
    assert(numeric_control(&"1.") is Result.Err(_));
    assert(numeric_control(&"1e+") is Result.Err(_));
    12
}
"#).parse_module().expect("numeric controls grammar");
    let mut codegen = VbcCodegen::new();
    codegen.compile_module_with_mounts(&toml,
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/configuration/toml.vr"),
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../core"))
        .expect("complete production TOML module");
    let modules = [&maybe, &result, &text, &caller];
    codegen.collect_unit_declarations(&modules).expect("source declarations");
    codegen.compile_unit_items(&modules, ItemFailurePolicy::Strict).expect("source bodies");
    let mut module = codegen.finalize_module().expect("source bytecode");
    module.resolve_protocol_dispatch();
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).expect("serialize"))
        .expect("deserialize");
    for (route, module) in [("source", module), ("wire", wire)] {
        let entry = module.find_function_by_name("core.configuration.toml.number_controls")
            .expect("numeric control entry");
        let actual = Interpreter::new(Arc::new(module)).execute_function(entry)
            .unwrap_or_else(|error| panic!("{route}: {error:?}"));
        assert_eq!(actual.as_i64(), 12, "{route}");
    }
}
