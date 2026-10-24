#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::interpreter::Interpreter;

fn run(source: &str) -> i64 {
    let ast = Parser::new(source).parse_module().expect("parse");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("cloning"));
    let module = codegen.compile_module(&ast).expect("compile");
    let module = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    let entry = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|name| name.ends_with(".probe") || name == "probe")
        })
        .unwrap()
        .id;
    Interpreter::new(Arc::new(module))
        .execute_function(entry)
        .expect("execute")
        .as_i64()
}

const FOREIGN_CLONE: &str = r#"
type Clone is protocol { fn clone(&self) -> Self; };
type Foreign is { first: Int, second: Int, third: Int, fourth: Int };
implement Clone for Foreign {
    fn clone(&self) -> Foreign { panic("foreign clone must not run"); }
}
fn copied<T: Clone>(value: &T) -> T { value.clone() }
"#;

#[test]
fn generic_record_clone_does_not_select_an_unrelated_body() {
    assert_eq!(
        run(&format!(
            "{FOREIGN_CLONE}{}",
            r#"
type Authority is { id: Int };
fn probe() -> Int {
    let original = Authority { id: 42 };
    let copy = copied(&original);
    copy.id
}
"#
        )),
        42
    );
}

#[test]
fn generic_clone_uses_concrete_semantics_when_declared() {
    assert_eq!(
        run(&format!(
            "{FOREIGN_CLONE}{}",
            r#"
type Authority is { id: Int };
implement Clone for Authority {
    fn clone(&self) -> Authority { Authority { id: self.id + 1 } }
}
fn probe() -> Int {
    let original = Authority { id: 41 };
    let copy = copied(&original);
    copy.id
}
"#
        )),
        42
    );
}

#[test]
fn generic_record_clone_keeps_mutations_independent() {
    assert_eq!(
        run(&format!(
            "{FOREIGN_CLONE}{}",
            r#"
type Authority is { id: Int };
fn probe() -> Int {
    let original = Authority { id: 42 };
    let mut copy = copied(&original);
    copy.id = 99;
    original.id
}
"#
        )),
        42
    );
}
