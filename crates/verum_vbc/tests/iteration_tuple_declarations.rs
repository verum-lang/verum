//! Erased tuple newtypes require their declared receiver inside a loop.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    interpreter::Interpreter,
};

#[test]
fn a_singleton_tuple_retains_its_slot_across_a_function_call() {
    use verum_vbc::{deserialize::deserialize_module, serialize::serialize_module};

    let source = r#"
        module consumer;
        fn read(value: &(Int,)) -> Int { value.0 }
        fn main() -> Int {
            let single: (Int,) = (42,);
            read(&single)
        }
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new("consumer"))
        .compile_module(&ast)
        .unwrap();
    let decoded = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for image in [module, decoded] {
        let main = image.find_function_by_name("consumer.main").unwrap();
        let mut vm = Interpreter::try_new_validated(Arc::new(image)).unwrap();
        assert_eq!(vm.execute_function(main).unwrap().try_as_i64(), Some(42));
    }
}

#[test]
fn collection_tuple_bindings_resolve_each_nominal_receiver() {
    let source = r#"
        module consumer;
        type Left is (Int);
        type Right is (Int);
        implement Left { fn read(&self) -> Int { self.0 } }
        implement Right { fn read(&self) -> Int { self.0 * 10 } }
        type Rows is { items: List<(Left, Right)> };
        implement Rows {
            fn update(&mut self) {
                for (_, value) in self.items.iter_mut() { *value = Right(5); }
            }
            fn total(&self) -> Int {
                let left = Right(1);
                let mut sum = 0;
                for (left, right) in self.items.iter() {
                    sum = sum + left.read() + right.read();
                }
                sum + left.read()
            }
        }
        fn main() -> Int {
            let mut rows = Rows { items: [(Left(2), Right(4))] };
            rows.update();
            rows.total()
        }
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new("consumer"))
        .compile_module(&ast)
        .unwrap();
    let main = module.find_function_by_name("consumer.main").unwrap();
    assert_eq!(
        Interpreter::new(Arc::new(module))
            .execute_function(main)
            .unwrap()
            .try_as_i64(),
        Some(62)
    );
}

#[test]
fn a_source_collection_carries_its_tuple_items_into_value_arguments() {
    use verum_ast::{FileId, ItemKind};
    use verum_fast_parser::FastParser;
    use verum_vbc::{
        codegen::ItemFailurePolicy, deserialize::deserialize_module, serialize::serialize_module,
    };

    let parser = FastParser::new();
    let mut collection = parser.parse_module_str(
        include_str!("../../../core/collections/list.vr"), FileId::new(1),
    ).unwrap();
    collection.items.retain(|entry| match &entry.kind {
        ItemKind::Module(module) => module.items.is_none(),
        ItemKind::Type(declaration) => declaration.name.as_str() == "List",
        _ => false,
    });
    let source = r#"
        module consumer;
        mount core.collections.list.List;
        type Payload is Message(Text) | Missing;
        type Packet is { entries: List<(Int, Payload)> };
        fn size(value: Payload) -> Int {
            match value { Payload.Message(text) => text.len(), Payload.Missing => 0 }
        }
        fn total(packet: Packet) -> Int {
            let mut sum = 0;
            for (_, value) in packet.entries { sum = sum + size(value); }
            sum
        }
        fn main() -> Int { total(Packet { entries: [(1, Payload.Message("abc")), (2, Payload.Missing)] }) }
    "#;
    let consumer = parser.parse_module_str(source, FileId::new(2)).unwrap();
    let units = [&collection, &consumer];
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    codegen.initialize();
    codegen.collect_unit_declarations(&units).unwrap();
    codegen.resolve_pending_imports();
    codegen.compile_unit_items(&units, ItemFailurePolicy::Strict).unwrap();
    let module = codegen.finalize_module().unwrap();
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for checked in [module, wire] {
        let main = checked.find_function_by_name("consumer.main").unwrap();
        let mut vm = Interpreter::try_new_validated(Arc::new(checked)).unwrap();
        assert_eq!(vm.execute_function(main).unwrap().try_as_i64(), Some(3));
    }
}
