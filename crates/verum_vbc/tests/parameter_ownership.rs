//! By-value calls establish an owner before parameter cleanup can run.
#![cfg(feature = "codegen")]
#[path = "fixtures/canonical_drop.rs"]
mod canonical_drop;

use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, CodegenResult, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

const DECLARATIONS: &str = r#"
mount core.base.protocols.Drop;
type Count is { value: Int };
type affine Ticket is { counter: &mut Count };
implement Drop for Ticket {
    fn drop(&mut self) { self.counter.value = self.counter.value + 1; }
}
type Copyable is { counter: &mut Count, value: Int };
implement Drop for Copyable {
    fn drop(&mut self) { self.counter.value = self.counter.value + 1; self.value = 0; }
}
"#;

fn compile(source: &str, unit: bool) -> CodegenResult<VbcModule> {
    let text = format!("{DECLARATIONS}\n{source}");
    let ast = Parser::new(&text).parse_module().expect("parameter source grammar");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("parameter_ownership"));
    canonical_drop::initialize(&mut codegen);
    if unit {
        codegen.collect_unit_declarations(&[&ast])?;
        codegen.compile_unit_items(&[&ast], ItemFailurePolicy::Strict)?;
        codegen.finalize_module_from_state()
    } else {
        codegen.compile_additional_module(&ast)
    }
}

fn run(source: &str) {
    for unit in [false, true] {
        let original = compile(source, unit).expect("declared call compilation");
        let wire = verum_vbc::deserialize::deserialize_module(
            &verum_vbc::serialize::serialize_module(&original).expect("wire"),
        ).expect("wire decode");
        for (route, module) in [("source", original), ("wire", wire)] {
            verum_vbc::validate::validate_module(&module).expect("valid archive");
            let entry = module.find_function_by_name("parameter_ownership.probe").unwrap();
            let result = Interpreter::new(module.into()).execute_function(entry);
            assert_eq!(result.unwrap().try_as_i64(), Some(73), "unit={unit}, {route}");
        }
    }
}

#[test]
fn unused_temporary_parameter_is_cleaned_at_normal_return() {
    run(r#"
        fn take(value: Ticket) {}
        fn probe() -> Int {
            let mut count = Count { value: 0 };
            take(Ticket { counter: &mut count });
            if count.value == 1 { 73 } else { count.value }
        }
    "#);
}

#[test]
fn generic_temporary_parameter_uses_the_instantiated_drop_witness() {
    run(r#"
        fn take<T>(value: T) {}
        fn probe() -> Int {
            let mut count = Count { value: 0 };
            take(Ticket { counter: &mut count });
            if count.value == 1 { 73 } else { count.value }
        }
    "#);
}

#[test]
fn a_named_copyable_argument_keeps_its_caller_owner() {
    run(r#"
        fn take(value: Copyable) {}
        fn probe() -> Int {
            let mut count = Count { value: 0 };
            {
                let source = Copyable { counter: &mut count, value: 73 };
                take(source);
                if count.value != 1 { return 10 + count.value; }
                if source.value != 73 { return 20; }
            }
            if count.value == 2 { 73 } else { 30 + count.value }
        }
    "#);
}

#[test]
fn a_mutable_parameter_needs_only_the_call_boundary_copy() {
    run(r#"
        fn take(mut value: Copyable) { value.value = 9; }
        fn probe() -> Int {
            let mut count = Count { value: 0 };
            {
                let source = Copyable { counter: &mut count, value: 73 };
                take(source);
                if count.value != 1 { return 10 + count.value; }
                if source.value != 73 { return 20; }
            }
            if count.value == 2 { 73 } else { 30 + count.value }
        }
    "#);
}

#[test]
fn ordinary_function_values_obey_the_same_argument_boundary() {
    run(r#"
        fn take(value: Ticket) {}
        fn probe() -> Int {
            let mut count = Count { value: 0 };
            let selected: fn(Ticket) = take;
            selected(Ticket { counter: &mut count });
            if count.value == 1 { 73 } else { count.value }
        }
    "#);
}

#[test]
fn closure_parameters_obey_the_same_argument_boundary() {
    run(r#"
        fn probe() -> Int {
            let mut count = Count { value: 0 };
            let selected: fn(Ticket) = |value: Ticket| {};
            selected(Ticket { counter: &mut count });
            if count.value == 1 { 73 } else { count.value }
        }
    "#);
}

#[test]
fn consuming_method_receivers_are_cleaned_at_return() {
    run(r#"
        implement Ticket { fn finish(self) {} }
        fn probe() -> Int {
            let mut count = Count { value: 0 };
            Ticket { counter: &mut count }.finish();
            if count.value == 1 { 73 } else { count.value }
        }
    "#);
}

#[test]
fn receiver_handoffs_preserve_the_exact_owner_until_the_destination_drops() {
    for (declarations, projection) in [
        ("type Envelope is { ticket: Ticket }; implement Ticket { fn forward(self) -> Envelope { Envelope { ticket: (self) } } }", "result.ticket.counter.value"),
        ("type Envelope is Present { ticket: Ticket }; implement Ticket { fn forward(self) -> Envelope { Envelope.Present { ticket: self } } }", "match result { Envelope.Present { ticket } => ticket.counter.value }"),
        ("type Envelope<T> is Present(T); implement Ticket { fn forward(self) -> Envelope<Ticket> { Envelope.Present(self) } }", "match result { Envelope.Present(ticket) => ticket.counter.value }"),
        ("implement Ticket { fn forward(self) -> Ticket { let value = self; value } }", "result.counter.value"),
        ("fn receive<T>(value: T) -> T { value } implement Ticket { fn forward(self) -> Ticket { receive(self) } }", "result.counter.value"),
    ] {
        run(&format!(r#"
            {declarations}
            fn probe() -> Int {{
                let mut count = Count {{ value: 0 }};
                {{
                    let result = Ticket {{ counter: &mut count }}.forward();
                    if count.value != 0 {{ return 11; }}
                    if ({projection}) != 0 {{ return 12; }}
                }}
                if count.value == 1 {{ 73 }} else {{ 20 + count.value }}
            }}
        "#));
    }
}

#[test]
fn a_borrowed_receiver_stored_in_a_record_does_not_transfer_its_referent() {
    run(r#"
        type Borrowed is { ticket: &Ticket };
        implement Ticket { fn borrow(&self) -> Borrowed { Borrowed { ticket: self } } }
        fn probe() -> Int {
            let mut count = Count { value: 0 };
            {
                let ticket = Ticket { counter: &mut count };
                { let borrowed = ticket.borrow(); if borrowed.ticket.counter.value != 0 { return 11; } }
                if count.value != 0 { return 12; }
            }
            if count.value == 1 { 73 } else { 20 + count.value }
        }
    "#);
}

#[test]
fn dynamic_method_arguments_use_the_declared_parameter_contract() {
    run(r#"
        type Sink is protocol { fn accept(&self, value: Ticket); };
        type Receiver is ();
        implement Sink for Receiver { fn accept(&self, value: Ticket) {} }
        fn invoke(sink: &dyn Sink, value: Ticket) { sink.accept(value); }
        fn probe() -> Int {
            let mut count = Count { value: 0 };
            let receiver = Receiver();
            invoke(&receiver, Ticket { counter: &mut count });
            if count.value == 1 { 73 } else { count.value }
        }
    "#);
}

#[test]
fn early_return_cleans_only_the_owned_parameter_on_the_taken_edge() {
    run(r#"
        fn take(value: Ticket, early: Bool) { if early { return; } }
        fn probe() -> Int {
            let mut count = Count { value: 0 };
            take(Ticket { counter: &mut count }, true);
            take(Ticket { counter: &mut count }, false);
            if count.value == 2 { 73 } else { count.value }
        }
    "#);
}

#[test]
fn a_returned_parameter_retains_its_obligation_until_the_caller_scope_ends() {
    run(r#"
        fn identity<T>(value: T) -> T { value }
        fn probe() -> Int {
            let mut count = Count { value: 0 };
            {
                let value = identity(Ticket { counter: &mut count });
                if count.value != 0 { return 10 + count.value; }
            }
            if count.value == 1 { 73 } else { count.value }
        }
    "#);
}

#[test]
fn borrowed_and_raw_parameters_never_receive_the_referent_obligation() {
    run(r#"
        fn borrow(value: &Ticket) -> Int { value.counter.value }
        unsafe fn raw(value: *const Int) -> Int { *value }
        fn probe() -> Int {
            let mut count = Count { value: 0 };
            {
                let ticket = Ticket { counter: &mut count };
                if borrow(&ticket) != 0 { return 11; }
                if unsafe { raw(&count.value as *const Int) } != 0 { return 12; }
                if count.value != 0 { return 13; }
            }
            if count.value == 1 { 73 } else { count.value }
        }
    "#);
}

#[test]
fn a_borrowed_affine_place_cannot_supply_an_owned_parameter() {
    let source = "fn take(value: Ticket) {} fn invalid(value: &Ticket) { take(*value); }";
    for unit in [false, true] {
        let result = compile(source, unit);
        assert!(result.is_err(), "borrowed affine source cannot become a parameter owner, unit={unit}");
    }
}

#[test]
fn a_named_affine_argument_is_transferred_before_parameter_cleanup() {
    run(r#"
        fn take<T>(value: T) {}
        fn probe() -> Int {
            let mut count = Count { value: 0 };
            {
                let ticket = Ticket { counter: &mut count };
                take(ticket);
                if count.value != 1 { return 11; }
            }
            if count.value == 1 { 73 } else { 12 }
        }
    "#);
}

#[test]
fn a_named_affine_replacement_retires_only_the_replaced_owner() {
    run(r#"
        fn probe() -> Int {
            let mut count = Count { value: 0 };
            {
                let mut selected = Ticket { counter: &mut count };
                let incoming = Ticket { counter: &mut count };
                selected = incoming;
                if count.value != 1 { return 11; }
            }
            if count.value == 2 { 73 } else { 12 }
        }
    "#);
}

#[test]
fn a_generic_replacement_uses_the_actual_copy_or_transfer_discipline() {
    run(r#"
        fn replace<T>(first: T, second: T) {
            let mut selected = first;
            selected = second;
        }
        fn probe() -> Int {
            let mut count = Count { value: 0 };
            {
                let first = Ticket { counter: &mut count };
                let second = Ticket { counter: &mut count };
                replace(first, second);
                if count.value != 2 { return 11; }
            }
            if count.value == 2 { 73 } else { 12 }
        }
    "#);
}

#[test]
fn a_tuple_takes_a_named_affine_element_once() {
    run(r#"
        fn probe() -> Int {
            let mut count = Count { value: 0 };
            {
                let ticket = Ticket { counter: &mut count };
                let pair = (ticket, 73);
                if pair.1 != 73 || count.value != 0 { return 11; }
            }
            if count.value == 1 { 73 } else { 12 }
        }
    "#);
}

#[test]
fn generic_callable_bounds_keep_the_argument_contract() {
    run(r#"
        fn invoke<F: fn(Ticket)>(selected: F, value: Ticket) { selected(value); }
        fn take(value: Ticket) {}
        fn probe() -> Int {
            let mut count = Count { value: 0 };
            invoke(take, Ticket { counter: &mut count });
            if count.value == 1 { 73 } else { count.value }
        }
    "#);
}

#[test]
fn returned_parameters_survive_normal_and_early_exit_without_double_drop() {
    run(r#"
        fn identity<T>(value: T, early: Bool) -> T {
            if early { return value; }
            value
        }
        fn probe() -> Int {
            let mut count = Count { value: 0 };
            {
                let first = identity(Ticket { counter: &mut count }, true);
                let second = identity(Ticket { counter: &mut count }, false);
                if count.value != 0 { return 11; }
            }
            if count.value == 2 { 73 } else { count.value }
        }
    "#);
}

#[test]
fn nested_owned_parameter_copy_preserves_the_callers_component() {
    run(r#"
        type Envelope is { payload: Copyable };
        fn take(value: Envelope) {}
        fn probe() -> Int {
            let mut count = Count { value: 0 };
            {
                let source = Envelope { payload: Copyable { counter: &mut count, value: 73 } };
                take(source);
                if count.value != 1 { return 10 + count.value; }
                if source.payload.value != 73 { return 20; }
            }
            if count.value == 2 { 73 } else { 30 + count.value }
        }
    "#);
}

#[test]
fn projected_argument_cannot_destroy_its_enclosing_owner() {
    run(r#"
        type Envelope is { payload: Copyable };
        fn take(value: Copyable) {}
        fn probe() -> Int {
            let mut count = Count { value: 0 };
            {
                let source = Envelope { payload: Copyable { counter: &mut count, value: 73 } };
                take(source.payload);
                if count.value != 1 { return 10 + count.value; }
                if source.payload.value != 73 { return 20; }
            }
            if count.value == 2 { 73 } else { 30 + count.value }
        }
    "#);
}

#[test]
fn a_parameter_projected_result_must_survive_parameter_cleanup() {
    run(r#"
        type Envelope is { payload: Copyable };
        fn extract(value: Envelope) -> Copyable { value.payload }
        fn probe() -> Int {
            let mut count = Count { value: 0 };
            {
                let result = extract(Envelope { payload: Copyable { counter: &mut count, value: 73 } });
                if result.value != 73 { return 20; }
            }
            if count.value == 2 { 73 } else { 30 + count.value }
        }
    "#);
}
