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
