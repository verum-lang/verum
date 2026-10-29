//! Pooled resources retain exactly one owner across extraction and return.
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
type Choice<T> is Held(T) | Empty;
implement<T> Drop for Choice<T> {
    fn drop(&mut self) {
        match &mut self { Choice.Held(value) => value.drop(), Choice.Empty => {} }
    }
}
"#;

fn run(source: &str) {
    let ast = Parser::new(&format!("{TYPES}\n{source}"))
        .parse_module()
        .unwrap();
    for bootstrap in [false, true] {
        let mut cg = VbcCodegen::with_config(CodegenConfig::new("pooled_handoff"));
        let mut module = if bootstrap {
            cg.collect_unit_declarations(&[&ast]).unwrap();
            cg.compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
                .unwrap();
            cg.finalize_module_from_state().unwrap()
        } else {
            cg.compile_module(&ast).unwrap()
        };
        for function in &mut module.functions {
            function.value_uses = None;
        }
        let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
        let entry = module
            .find_function_by_name("pooled_handoff.probe")
            .unwrap();
        let result = Interpreter::new(Arc::new(module)).execute_function(entry);
        assert_eq!(result.unwrap().as_i64(), 1, "bootstrap={bootstrap}");
    }
}

fn returned_resource(annotation: &str) {
    for selected in [
        "match self.value { Choice.Held(value) => value, Choice.Empty => return }",
        "(match self.value { Choice.Held(value) => value, Choice.Empty => return })",
        "match self.value { Choice.Held(value) => { let value = value; value }, Choice.Empty => { return; } }",
        "match self.value { Choice.Held(value) if false => value, Choice.Held(value) => { if true { value } else { return; } }, Choice.Empty => return }",
    ] {
        run(&format!(
            r#"
type Sink<T> is {{ slot: &mut Choice<T> }};
implement<T> Sink<T> {{
    fn store(&self, value: T) {{
        let owned: T = value;
        *self.slot = Choice.Held(owned);
    }}
}}
type Lease<T> is {{ value: Choice<T> }};
implement<T> Lease<T> {{
    fn release(mut self, sink: &Sink<T>) {{
        let value{annotation} = {selected};
        self.value = Choice.Empty;
        sink.store(value);
    }}
}}
fn probe() -> Int {{
    let mut count = Count {{ value: 0 }};
    {{
        let mut slot: Choice<Ticket> = Choice.Empty;
        {{
            let sink: Sink<Ticket> = Sink {{ slot: &mut slot }};
            let lease: Lease<Ticket> = Lease {{ value: Choice.Held(Ticket {{ count: &mut count }}) }};
            lease.release(&sink);
            assert_eq(count.value + 0, 0);
        }}
        assert_eq(count.value + 0, 0);
    }}
    count.value
}}
"#
        ));
    }
}

#[test]
fn inferred_match_payload_can_be_returned_to_another_owner() {
    returned_resource("");
}

#[test]
fn annotated_match_payload_can_be_returned_to_another_owner() {
    returned_resource(": T");
}

#[test]
fn record_payload_uses_its_declared_generic_slot_in_the_callers_scope() {
    run(r#"
type Packet<A, B> is Held { payload: B, note: A } | Empty;
type Sink<T> is { slot: &mut Choice<T> };
implement<T> Sink<T> {
    fn store(&self, value: T) { let owned: T = value; *self.slot = Choice.Held(owned); }
}
type Lease<L, R> is { value: Packet<L, R> };
implement<L, R> Lease<L, R> {
    fn release(mut self, sink: &Sink<R>) {
        let value = match self.value { Packet.Held { payload, .. } => payload, Packet.Empty => return };
        self.value = Packet.Empty;
        sink.store(value);
    }
}
fn probe() -> Int {
    let mut count = Count { value: 0 };
    {
        let mut slot: Choice<Ticket> = Choice.Empty;
        {
            let sink: Sink<Ticket> = Sink { slot: &mut slot };
            let lease: Lease<Int, Ticket> = Lease {
                value: Packet.Held { payload: Ticket { count: &mut count }, note: 7 },
            };
            lease.release(&sink);
            assert_eq(count.value + 0, 0);
        }
        assert_eq(count.value + 0, 0);
    }
    count.value
}
"#);
}

#[test]
fn a_reference_payload_keeps_its_borrow_through_a_match_result() {
    run(r#"
type View<T> is Held(&T) | Empty;
fn inspect<T>(borrower: View<T>) {
    let value = match borrower { View.Held(value) => value, View.Empty => return };
    let borrowed = value;
}
fn probe() -> Int {
    let mut count = Count { value: 0 };
    {
        let ticket = Ticket { count: &mut count };
        let borrower: View<Ticket> = View.Held(&ticket);
        inspect<Ticket>(borrower);
        assert_eq(count.value + 0, 0);
    }
    count.value
}
"#);
}

#[test]
fn generic_field_extraction_keeps_payload_alive_when_the_slot_is_cleared() {
    run(r#"
type Lease<T> is { value: Choice<T> };
implement<T> Lease<T> {
    fn release(mut self, count: &Count) {
        let value: T = match self.value { Choice.Held(value) => value, Choice.Empty => return };
        assert_eq(count.value + 0, 0);
        self.value = Choice.Empty;
        assert_eq(count.value + 0, 0);
        drop(value);
        assert_eq(count.value + 0, 1);
    }
}
fn probe() -> Int {
    let mut count = Count { value: 0 };
    {
        let lease: Lease<Ticket> = Lease { value: Choice.Held(Ticket { count: &mut count }) };
        lease.release(&count);
        assert_eq(count.value + 0, 1);
    }
    count.value
}
"#);
}
