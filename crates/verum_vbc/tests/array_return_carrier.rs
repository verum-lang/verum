//! T1726: packed returns use the declaring collection's runtime operation.
#![cfg(feature = "codegen")]

#[path = "fixtures/declared_list.rs"]
mod declared_list;

use verum_common::{List, Shared, well_known_types::WellKnownType};
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    instruction::Instruction,
    interpreter::{Interpreter, InterpreterError},
    types::{TypeId, TypeRef},
};

#[test]
fn source_declared_list_materializes_packed_returns_after_id_assignment() {
    let source = declared_list::compile_consumer(r#"
        module consumer;
        mount core.collections.list.List as Buffer;
        fn probe() -> Buffer<Int16> {
            let values: [Int16; 3] = [-32768, -1, 1];
            let first = values[0];
            values
        }
    "#);
    for module in declared_list::source_and_wire(source) {
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

#[test]
fn forwarding_a_declared_byte_list_copies_its_elements() {
    let source = declared_list::compile_consumer(r#"
        module consumer;
        mount core.collections.list.List;
        fn forward(xs: List<Byte>) -> List<Byte> { xs }
        fn probe() -> Int {
            let input: List<Byte> = [1, 2];
            let mut xs = forward(input);
            xs[0] = 9 as Byte;
            assert_eq(input[0], 1);
            assert_eq(xs[0], 9);
            assert_eq(xs[1], 2);
            1
        }
    "#);
    for module in declared_list::source_and_wire(source) {
        let forward = module.find_function_by_name("consumer.forward").expect("forward");
        let ty = &module.get_function(forward).unwrap().return_type;
        let TypeRef::Instantiated { base, args } = ty else { panic!("declared List<Byte>") };
        assert_eq!(module.declared_runtime_carrier(*base), Some(WellKnownType::List));
        assert_eq!(args.as_slice(), [TypeRef::Concrete(TypeId::U8)]);
        let entry = module.find_function_by_name("consumer.probe").expect("probe");
        let caller = module.get_function(entry).unwrap();
        let start = caller.bytecode_offset as usize;
        let code = verum_vbc::bytecode::decode_instructions(
            &module.bytecode[start..start + caller.bytecode_length as usize],
        ).unwrap();
        assert!(code.iter().any(|op| matches!(op, Instruction::CloneTyped { type_ref, .. } if type_ref == ty)));
        let value = Interpreter::new(Shared::new(module).into_arc()).execute_function(entry).expect("copy the List");
        assert_eq!(value.try_as_i64(), Some(1));
    }
}

#[test]
fn forwarding_without_a_list_declaration_rejects_the_typed_copy() {
    let ast = Parser::new(r#"
        fn forward(xs: List<Byte>) -> List<Byte> { xs }
        fn probe() -> Int {
            let input: List<Byte> = [1, 2];
            let mut xs = forward(input);
            xs.push(3 as Byte);
            assert_eq(xs.len(), 3);
            assert_eq(xs[0], 1);
            assert_eq(xs[2], 3);
            1
        }
    "#).parse_module().expect("standalone consumer grammar");
    let source = VbcCodegen::with_config(CodegenConfig::new("consumer"))
        .compile_module(&ast).expect("standalone consumer");
    for module in declared_list::source_and_wire(source) {
        let forward = module.find_function_by_name("consumer.forward").expect("forward");
        let ty = &module.get_function(forward).unwrap().return_type;
        let TypeRef::Instantiated { base, .. } = ty else { panic!("List type operand") };
        assert!(module.get_type(*base).is_none());
        let expected = format!(
            "typed copy requires valid declared owned storage: missing VBC declaration for List (type #{})",
            base.0,
        );
        let entry = module.find_function_by_name("consumer.probe").expect("probe");
        let error = Interpreter::new(Shared::new(module).into_arc()).execute_function(entry).unwrap_err();
        assert!(matches!(error, InterpreterError::InvalidOperand { ref message }
            if message == &expected),
            "{error:?}");
    }
}
