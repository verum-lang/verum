//! T1698: a mutable method call must preserve the owned receiver for consumption.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    module::VbcModule,
    serialize::serialize_module,
};

const STATE: &str = r#"
module consuming_state;
type State is { a: Int, b: Int, c: Int, d: Int };
implement State {
    fn new() -> State { State { a: 11, b: 22, c: 33, d: 44 } }
    fn update(&mut self, amount: Int) { self.d = self.d + amount; }
    fn replace(&mut self, amount: Int) { *self = State { a: 1, b: 2, c: 3, d: amount }; }
    fn finish(self) -> Int { self.a + self.b + self.c + self.d }
    fn borrowed(&self) -> Int { self.a + self.b + self.c + self.d }
}
"#;

fn compile(body: &str) -> VbcModule {
    let source = format!("{STATE}\nfn probe() -> Int {{ {body} }}");
    let ast = Parser::new(&source).parse_module().expect("parse control");
    VbcCodegen::with_config(CodegenConfig::new("consuming_state"))
        .compile_module(&ast).expect("compile control")
}

fn assert_source_and_wire(body: &str, expected: i64) {
    let source = compile(body);
    let wire = deserialize_module(&serialize_module(&source).unwrap()).unwrap();
    for (route, module) in [("source", source), ("serialized", wire)] {
        let entry = module.functions.iter().find(|f|
            module.get_string(f.name) == Some("consuming_state.probe")
        ).expect("exact entry descriptor").id;
        let result = Interpreter::new(Arc::new(module)).execute_function(entry);
        assert_eq!(result.unwrap_or_else(|error| panic!("{route}, {body}: {error:?}")).as_i64(), expected, "{route}, {body}");
    }
}

#[test]
fn consuming_receiver_without_mutable_call_is_the_positive_control() {
    assert_source_and_wire("let state = State.new(); state.finish()", 110);
}

#[test]
fn mutable_update_then_consuming_method_keeps_the_owned_state() {
    assert_source_and_wire("let mut state = State.new(); state.update(7); state.finish()", 117);
}

#[test]
fn repeated_mutable_updates_then_consumption_preserve_all_fields() {
    assert_source_and_wire("let mut state = State.new(); state.update(7); state.update(9); state.finish()", 126);
}

#[test]
fn replacing_through_mutable_self_preserves_the_callers_value() {
    assert_source_and_wire("let mut state = State.new(); state.replace(70); state.finish()", 76);
}

#[test]
fn mutable_call_then_borrowed_read_has_the_same_state() {
    assert_source_and_wire("let mut state = State.new(); state.update(7); state.borrowed()", 117);
}
