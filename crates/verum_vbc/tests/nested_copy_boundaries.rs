//! Source copies preserve owned children across mutation and lexical cleanup.
#![cfg(feature = "codegen")]
#[path = "fixtures/canonical_drop.rs"]
mod canonical_drop;

use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
};

fn run(source: &str) {
    for unit in [false, true] {
        let ast = Parser::new(source).parse_module().expect("source grammar");
        let mut compiler = VbcCodegen::with_config(CodegenConfig::new("nested"));
        canonical_drop::initialize(&mut compiler);
        let module = if unit {
            compiler.collect_unit_declarations(&[&ast]).unwrap();
            compiler.compile_unit_items(&[&ast], ItemFailurePolicy::Strict).unwrap();
            compiler.finalize_module_from_state().unwrap()
        } else {
            compiler.compile_additional_module(&ast).unwrap()
        };
        let decoded = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
        for (route, module) in [("source", module), ("wire", decoded)] {
            let entry = module.find_function_by_name("nested.probe").unwrap();
            let mut interpreter = Interpreter::try_new_validated(module.into()).unwrap();
            let value = interpreter.execute_function(entry).unwrap();
            assert_eq!(value.try_as_i64(), Some(73), "unit={unit}, route={route}");
        }
    }
}

#[test]
fn a_nested_sum_payload_survives_destruction_of_its_original() {
    run(r#"
        mount core.base.protocols.Drop;
        type Leaf is { value: Int };
        type Branch is { leaf: Leaf };
        implement Drop for Branch { fn drop(&mut self) { self.leaf.value = 0; } }
        type Choice<T> is Empty | Value(T);
        fn produce() -> Choice<Branch> {
            let original = Branch { leaf: Leaf { value: 73 } };
            Choice.Value(original)
        }
        fn probe() -> Int {
            match produce() { Choice.Value(branch) => branch.leaf.value, _ => 0 }
        }
    "#);
}

#[test]
fn text_payload_copies_survive_the_original_parameter_cleanup() {
    for forward in [
        "Choice.Value(text)",
        "{ let saved = text; Choice.Value(saved) }",
        "{ let record = Message { payload: text }; Choice.Value(record.payload) }",
        "Choice.Value(identity(text))",
    ] {
        run(&format!(r#"
            type Message is {{ payload: Text }};
            type Choice is Empty | Value(Text);
            fn identity(value: Text) -> Text {{ value }}
            fn forward(value: Choice) -> Choice {{
                match value {{ Choice.Value(text) => {forward}, _ => Choice.Empty }}
            }}
            fn probe() -> Int {{
                let original = Choice.Value("independent owned payload");
                let result = forward(original);
                let retained = match original {{
                    Choice.Value(text) => text == "independent owned payload", _ => false
                }};
                match result {{
                    Choice.Value(text) => if retained && text == "independent owned payload" {{73}} else {{0}},
                    _ => 0,
                }}
            }}
        "#));
    }
}

#[test]
fn a_sum_destructor_observes_its_original_field_after_matching_a_temporary() {
    run(r#"
        mount core.base.protocols.Drop;
        type Packet is Empty | Data(Text);
        implement Drop for Packet {
            fn drop(&mut self) {
                match self {
                    Packet.Data(text) => assert(text.len() == 12),
                    _ => {},
                }
            }
        }
        fn packet() -> Packet { Packet.Data("payload text") }
        fn extract() -> Text {
            match packet() { Packet.Data(text) => text, _ => "" }
        }
        fn probe() -> Int {
            let text = extract();
            if text == "payload text" { 73 } else { 0 }
        }
    "#);
}

#[test]
fn forwarding_a_field_from_a_named_sum_preserves_both_values() {
    run(r#"
        type Packet is Empty | Data(Text);
        fn probe() -> Int {
            let owner = Packet.Data("payload text");
            let result = match owner { Packet.Data(text) => text, _ => "" };
            match owner {
                Packet.Data(original) => if original == result && result == "payload text" { 73 } else { 0 },
                _ => 0,
            }
        }
    "#);
}

#[test]
fn returning_an_owned_generic_sum_transfers_only_the_selected_field() {
    run(include_str!("fixtures/return_sum_parameter.vr"));
}

#[test]
fn a_destructor_borrowing_the_match_parameter_observes_the_original_payload() {
    run(r#"
        mount core.base.protocols.Drop;
        type Packet is Empty | Data(Text);
        fn intact(input: &Packet) -> Bool {
            match input { Packet.Data(text) => text == "payload text", _ => false }
        }
        type Witness is { owner: &Packet };
        implement Drop for Witness {
            fn drop(&mut self) { assert(intact(self.owner)); }
        }
        fn extract(input: Packet) -> Text {
            let witness = Witness { owner: &input };
            match input { Packet.Data(text) => text, _ => "" }
        }
        fn probe() -> Int {
            if extract(Packet.Data("payload text")) == "payload text" { 73 } else { 0 }
        }
    "#);
}

#[test]
fn a_guard_that_publishes_a_payload_loan_preserves_its_owner_on_fallthrough() {
    let source = r#"
        type affine Ticket is { value: Int };
        type Packet is Empty | Data(Ticket);
        type Shelf is Empty | Loan(&Ticket);
        fn remember(slot: &mut Shelf, value: &Ticket) -> Bool {
            *slot = Shelf.Loan(value);
            false
        }
        fn extract(input: Packet, slot: &mut Shelf) -> Ticket {
            match input {
                Packet.Data(ticket) if remember(slot, &ticket) => Ticket { value: 0 },
                Packet.Data(ticket) => ticket,
                _ => Ticket { value: 0 },
            }
        }
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    let error = VbcCodegen::new().compile_module(&ast).expect_err("an observed owner remains live");
    assert!(error.to_string().contains("still owned by the scrutinee"), "{error}");
}

#[test]
fn assignment_preserves_nested_ownership() {
    run(r#"
        type Leaf is { value: Int };
        type Branch is { leaf: Leaf };
        fn probe() -> Int {
            let mut original = Branch { leaf: Leaf { value: 73 } };
            let mut copied = Branch { leaf: Leaf { value: 1 } };
            copied = original;
            original.leaf.value = 0;
            copied.leaf.value
        }
    "#);
}

#[test]
fn shorthand_fields_copy_their_declared_nested_value() {
    run(r#"
        type Leaf is { value: Int };
        type Branch is { leaf: Leaf };
        type Envelope is { branch: Branch };
        fn probe() -> Int {
            let mut branch = Branch { leaf: Leaf { value: 73 } };
            let envelope = Envelope { branch };
            branch.leaf.value = 0;
            envelope.branch.leaf.value
        }
    "#);
}

#[test]
fn explicit_dereference_keeps_the_owned_child_independent() {
    run(r#"
        type Leaf is { value: Int };
        type Branch is { leaf: Leaf };
        fn probe() -> Int {
            let mut original = Branch { leaf: Leaf { value: 73 } };
            let copied = *(&original);
            original.leaf.value = 0;
            copied.leaf.value
        }
    "#);
}

#[test]
fn mutable_value_parameter_preserves_the_callers_nested_value() {
    run(r#"
        type Leaf is { value: Int };
        type Branch is { leaf: Leaf };
        fn alter(mut value: Branch) { value.leaf.value = 0; }
        fn probe() -> Int {
            let original = Branch { leaf: Leaf { value: 73 } };
            alter(original);
            original.leaf.value
        }
    "#);
}

#[test]
fn record_variant_fields_follow_the_same_nested_copy_rule() {
    run(r#"
        type Leaf is { value: Int };
        type Branch is { leaf: Leaf };
        type Message is Empty | Ready { branch: Branch };
        fn probe() -> Int {
            let mut branch = Branch { leaf: Leaf { value: 73 } };
            let message = Message.Ready { branch };
            branch.leaf.value = 0;
            match message { Message.Ready { branch } => branch.leaf.value, _ => 0 }
        }
    "#);
}
