//! T1761: constants belong to their declaring source module, including methods.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{ItemFailurePolicy, VbcCodegen};
use verum_vbc::interpreter::Interpreter;

fn check(reverse: bool, entry: &str, expected: i64) {
    check_registered(reverse, entry, expected, false);
}
fn check_registered(reverse: bool, entry: &str, expected: i64, repeated: bool) {
    let first = Parser::new(
        r#"
module first;
const TABLE: [Int; 2] = [11, 13];
fn read() -> Int { TABLE[0] }
"#,
    )
    .parse_module()
    .expect("first grammar");
    let second = Parser::new(
        r#"
module second;
const TABLE: [Int; 2] = [17, 19];
type Reader is {};
implement Reader { fn read(&self) -> Int { TABLE[1] } }
fn read() -> Int { TABLE[0] }
fn method() -> Int { let reader = Reader {}; reader.read() }
"#,
    )
    .parse_module()
    .expect("second grammar");
    let modules = if reverse {
        [&second, &first]
    } else {
        [&first, &second]
    };
    let mut codegen = VbcCodegen::new();
    codegen.register_builtin_variants();
    if repeated {
        codegen
            .collect_unit_declarations(&[&second])
            .expect("dependency declarations");
    }
    codegen
        .collect_unit_declarations(&modules)
        .expect("declarations");
    codegen
        .compile_unit_items(&modules, ItemFailurePolicy::Strict)
        .expect("source bodies");
    let module = codegen.finalize_module().expect("source VBC");
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).expect("serialize"),
    )
    .expect("decode");
    for (route, module) in [("source", module), ("wire", wire)] {
        let id = module.find_function_by_name(entry).expect("source entry");
        let mut interpreter = Interpreter::new(Arc::new(module));
        let actual = interpreter.execute_function(id).expect("execute").as_i64();
        assert_eq!(actual, expected, "{route} {entry}, reversed={reverse}");
    }
}
#[test]
fn first_constant_is_not_shadowed_by_later_module() {
    check(false, "first.read", 11);
}
#[test]
fn later_constant_keeps_its_declaring_module() {
    check(false, "second.read", 17);
}
#[test]
fn method_reads_its_declaring_module_constant() {
    check(false, "second.method", 19);
}
#[test]
fn first_constant_survives_reversed_declaration_order() {
    check(true, "first.read", 11);
}
#[test]
fn later_constant_survives_reversed_declaration_order() {
    check(true, "second.read", 17);
}
#[test]
fn method_constant_survives_reversed_declaration_order() {
    check(true, "second.method", 19);
}

#[test]
fn repeated_source_declaration_keeps_constant_body_identity() {
    check_registered(false, "second.read", 17, true);
}
#[test]
fn repeated_method_declaration_keeps_constant_body_identity() {
    check_registered(false, "second.method", 19, true);
}
