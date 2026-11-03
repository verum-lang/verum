#![cfg(feature = "codegen")]
//! Nested patterns retain their binding's declared receiver, not a display hint.
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};

fn execute(body: &str) {
    let source = format!(r#"
        module pattern_receiver;
        type Deref is protocol {{ type Target; fn deref(&self) -> &Self.Target; }};
        type Guard<T> is {{ marker: Int, value: T }};
        implement<T> Deref for Guard<T> {{
            type Target = T;
            fn deref(&self) -> &T {{ &self.value }}
        }}
        type Slot<T> is End | Item(T);
        type Queue<T> is {{ value: Slot<T> }};
        implement<T> Queue<T> {{ fn pop(&mut self) -> Slot<T> {{ self.value }} }}
        type Node<K, V> is {{ keys: [K; 2], values: [V; 2] }};
        fn fetch<K, V>(slot: Slot<(&Guard<Node<K, V>>, Int)>) -> V {{ {body} }}
        fn main() -> Int {{
            let node: Int = 101;
            let guarded: Guard<Node<Bool, Int>> = Guard {{ marker: 99,
                value: Node {{ keys: [true, false], values: [29, 31] }} }};
            let selected = fetch(Slot.Item((&guarded, 1)));
            selected + node
        }}
    "#);
    let ast = Parser::new(&source).parse_module().expect("pattern grammar");
    let module = VbcCodegen::with_config(CodegenConfig::new("pattern_receiver"))
        .compile_module(&ast).expect("declared nested receiver");
    let restored = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    ).unwrap();
    for module in [module, restored] {
        let entry = module.find_function_by_name("pattern_receiver.main").unwrap();
        let value = verum_vbc::interpreter::Interpreter::new(Arc::new(module))
            .execute_function(entry).expect("actual pattern execution");
        assert_eq!(value.try_as_i64(), Some(132));
    }
}

#[test]
fn match_tuple_payload_preserves_generic_receiver_and_reference() {
    execute("match slot { Slot.Item((node, index)) => node.values[index], Slot.End => panic(\"empty\") }");
}

#[test]
fn if_let_tuple_payload_preserves_generic_receiver_and_reference() {
    execute("if let Slot.Item((node, index)) = slot { return node.values[index]; } panic(\"empty\")");
}

#[test]
fn while_let_tuple_payload_preserves_generic_receiver_and_reference() {
    execute("while let Slot.Item((node, index)) = slot { return node.values[index]; } panic(\"empty\")");
}

#[test]
fn method_payload_substitution_keeps_caller_generic_parameters() {
    execute(r#"
        let mut queue: Queue<(&Guard<Node<K, V>>, Int)> = Queue { value: slot };
        while let Slot.Item((node, index)) = queue.pop() { return node.values[index]; }
        panic("empty")
    "#);
}

#[test]
fn mutable_binding_cannot_upgrade_an_immutable_payload_reference() {
    let source = r#"
        module pattern_reference;
        type Slot<T> is Empty | Item(T);
        type Node is { values: [Int; 2] };
        fn update(slot: Slot<(&Node, Int)>) {
            if let Slot.Item((mut node, index)) = slot { node.values[index] = 73; }
        }
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    let error = VbcCodegen::with_config(CodegenConfig::new("pattern_reference"))
        .compile_module(&ast).expect_err("immutable payload cannot authorize mutation");
    assert!(error.to_string().contains("no declared mutable projection"), "{error}");
}
