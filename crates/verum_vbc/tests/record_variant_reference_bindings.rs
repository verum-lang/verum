#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::interpreter::Interpreter;

fn run(source: &str) -> i64 {
    let ast = Parser::new(source).parse_module().expect("parse");
    let module = VbcCodegen::with_config(CodegenConfig::new("record_refs"))
        .compile_module(&ast)
        .expect("compile");
    let entry = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|n| n == "probe" || n.ends_with(".probe"))
        })
        .expect("probe")
        .id;
    let value = Interpreter::new(Arc::new(module))
        .execute_function(entry)
        .expect("execute");
    assert!(value.is_int(), "expected Int, got {value:?}");
    value.as_i64()
}

#[test]
fn mutable_record_variant_binding_writes_through_the_second_field() {
    assert_eq!(
        run(r#"
type Sum is Pair { first: Int, second: Int } | Empty;
fn probe() -> Int {
    let mut s = Sum.Pair { first: 41, second: 1 };
    match &mut s {
        Sum.Pair { first: _, second: ref mut item } => { *item = *item + 2; },
        Sum.Empty => {},
    }
    match s { Sum.Pair { first, second } => first + second, Sum.Empty => 0 }
}
"#),
        44
    );
}

#[test]
fn immutable_record_variant_binding_reads_the_field() {
    assert_eq!(
        run(r#"
type Sum is Pair { first: Int, second: Int } | Empty;
fn probe() -> Int {
    let s = Sum.Pair { first: 41, second: 3 };
    match &s {
        Sum.Pair { first: _, second: ref item } => *item,
        Sum.Empty => 0,
    }
}
"#),
        3
    );
}

#[test]
fn a_value_binding_does_not_alias_the_variant_payload() {
    assert_eq!(
        run(r#"
type Sum is Pair { first: Int, second: Int } | Empty;
fn probe() -> Int {
    let s = Sum.Pair { first: 41, second: 3 };
    let read = match s { Sum.Pair { first: _, second } => second, Sum.Empty => 0 };
    let original = match s { Sum.Pair { first: _, second } => second, Sum.Empty => 0 };
    read + original
}
"#),
        6
    );
}

#[test]
fn tuple_variant_reference_binding_keeps_working() {
    assert_eq!(
        run(r#"
type Sum is Pair(Int, Int) | Empty;
fn probe() -> Int {
    let mut s = Sum.Pair(41, 1);
    match &mut s {
        Sum.Pair(_, ref mut item) => { *item = *item + 2; },
        Sum.Empty => {},
    }
    match s { Sum.Pair(first, second) => first + second, Sum.Empty => 0 }
}
"#),
        44
    );
}
