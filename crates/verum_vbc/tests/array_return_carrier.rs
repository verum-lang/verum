//! T1726: packed returns use the declaring collection's runtime operation.
#![cfg(feature = "codegen")]

use verum_ast::ItemKind;
use verum_common::{List, Shared};
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module,
    instruction::Instruction,
    interpreter::Interpreter,
    serialize::serialize_module,
    types::{TypeId, TypeRef},
};

#[test]
fn source_declared_list_materializes_packed_returns_after_id_assignment() {
    let mut declaration = Parser::new(include_str!("../../../core/collections/list.vr"))
        .parse_module().expect("actual List grammar");
    // Keep the production record and module header intact. The operation under
    // test is callable return conversion, not any of List's method bodies.
    declaration.items.retain(|item| match &item.kind {
        ItemKind::Module(header) => header.items.is_none(),
        ItemKind::Type(declaration) => declaration.name.name == "List",
        _ => false,
    });
    assert_eq!(declaration.items.len(), 2);
    let consumer = Parser::new(r#"
        module consumer;
        mount core.collections.list.List as Buffer;
        fn probe() -> Buffer<Int16> {
            let values: [Int16; 3] = [-32768, -1, 1];
            let first = values[0];
            values
        }
    "#).parse_module().expect("consumer grammar");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    codegen.initialize();
    let units = [&declaration, &consumer];
    codegen.collect_unit_declarations(&units).expect("source declarations");
    codegen.compile_unit_items(&units, ItemFailurePolicy::Strict).expect("compile source");
    let source = codegen.finalize_module().expect("completed source module");
    let wire = deserialize_module(&serialize_module(&source).expect("encode")).expect("decode");
    for module in [source, wire] {
        let entry = module.find_function_by_name("consumer.probe").expect("exact callable");
        let function = module.get_function(entry).unwrap();
        let TypeRef::Instantiated { base, .. } = function.return_type else {
            panic!("declared collection return");
        };
        assert_ne!(base, TypeId::LIST, "exercise a source-assigned descriptor ID");
        let start = function.bytecode_offset as usize;
        let code = verum_vbc::bytecode::decode_instructions(
            &module.bytecode[start..start + function.bytecode_length as usize],
        ).unwrap();
        assert!(code.iter().any(|op| matches!(op, Instruction::NewList { .. })),
            "the callable must materialize the packed producer: {code:#?}");
        let mut vm = Interpreter::new(Shared::new(module).into_arc());
        let value = vm.execute_function(entry).expect("execute");
        let actual: List<_> = vm.state.list_elements(value).expect("List storage")
            .iter().map(|value| value.try_as_i64()).collect();
        assert_eq!(actual.as_slice(), [Some(-32768), Some(-1), Some(1)]);
    }
}
