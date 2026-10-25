//! Generic method calls forward the caller's declaration-owned parameter IDs.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
};

const TYPES: &str = r#"
type Count is { value: Int };
type affine Ticket is { count: &mut Count };
implement Drop for Ticket { fn drop(&mut self) { self.count.value = self.count.value + 1; } }
type Sink<T> is { marker: Int };
implement<T> Sink<T> {
    fn consume(&self, value: T) { let owned: T = value; drop(owned); }
}
"#;

fn check(source: &str, unresolved: bool) {
    let ast = Parser::new(&format!("{TYPES}\n{source}"))
        .parse_module()
        .unwrap();
    for bootstrap in [false, true] {
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new("forwarding"));
        let mut module = if bootstrap {
            codegen.collect_unit_declarations(&[&ast]).unwrap();
            codegen
                .compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
                .unwrap();
            codegen.finalize_module_from_state().unwrap()
        } else {
            codegen.compile_module(&ast).unwrap()
        };
        // The VBC handoff must work independently of optional analysis metadata.
        for function in &mut module.functions {
            function.value_uses = None;
        }
        let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
        let entry = module.find_function_by_name("forwarding.probe").unwrap();
        let result = Interpreter::new(Arc::new(module)).execute_function(entry);
        if unresolved {
            let error = result.unwrap_err().to_string();
            assert!(
                error.contains("generic ownership requires a resolved declaration witness"),
                "{error}"
            );
        } else {
            assert_eq!(
                result.unwrap().as_i64(),
                1,
                "bootstrap={bootstrap}: exactly one resource owner"
            );
        }
    }
}

fn run(source: &str) {
    check(source, false);
}

#[test]
fn a_generic_function_forwards_its_witness_to_a_borrowed_receiver() {
    run(r#"
fn forward<T>(sink: &Sink<T>, value: T) { sink.consume(value); }
fn probe() -> Int {
    let mut count = Count { value: 0 };
    {
        let sink: Sink<Ticket> = Sink { marker: 0 };
        let value = Ticket { count: &mut count };
        forward<Ticket>(&sink, value);
    }
    count.value
}
"#);
}

#[test]
fn a_field_receiver_maps_the_callers_second_parameter_to_the_callees_first() {
    run(r#"
type Relay<A, B> is { left: &Sink<A>, right: &Sink<B> };
implement<A, B> Relay<A, B> {
    fn consume_right(&self, value: B) { self.right.consume(value); }
}

fn probe() -> Int {
    let mut count = Count { value: 0 };
    {
        let left: Sink<Int> = Sink { marker: 0 };
        let right: Sink<Ticket> = Sink { marker: 1 };
        let relay: Relay<Int, Ticket> = Relay { left: &left, right: &right };
        let value = Ticket { count: &mut count };
        relay.consume_right(value);
    }
    count.value
}
"#);
}

#[test]
fn an_unknown_argument_cannot_borrow_an_unrelated_callers_witness() {
    check(
        r#"
fn forward<T>(sink: &Sink<UnboundSlot>, value: T) { sink.consume(value); }
fn probe() -> Int {
    let mut count = Count { value: 0 };
    let sink: Sink<UnboundSlot> = Sink { marker: 0 };
    let value = Ticket { count: &mut count };
    forward<Ticket>(&sink, value);
    count.value
}
"#,
        true,
    );
}

#[test]
fn method_level_parameters_keep_their_declared_ids() {
    run(r#"
type Relay<A> is { first: A };
implement<A> Relay<A> {
    fn forward<B>(&self, sink: &Sink<B>, value: B) { sink.consume(value); }
}
fn probe() -> Int {
    let mut count = Count { value: 0 };
    {
        let relay: Relay<Int> = Relay { first: 0 };
        let sink: Sink<Ticket> = Sink { marker: 0 };
        let value = Ticket { count: &mut count };
        relay.forward<Ticket>(&sink, value);
    }
    count.value
}
"#);
}

#[test]
fn a_borrowed_method_argument_remains_owned_by_its_caller() {
    run(r#"
type Reader is {};
implement Reader {
    fn inspect(&self, value: &Ticket) -> Int { value.count.value }
}
fn probe() -> Int {
    let mut count = Count { value: 0 };
    {
        let reader = Reader {};
        let value = Ticket { count: &mut count };
        assert_eq(reader.inspect(&value), 0);
        assert_eq(count.value + 0, 0);
    }
    count.value
}
"#);
}
