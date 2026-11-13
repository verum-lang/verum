//! Record projections use the allocation that owns their inline storage.
#![cfg(feature = "codegen")]

use verum_common::Shared;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
    validate::validate_module,
    value::Value,
};

fn interpreter(body: &str) -> Interpreter {
    let source = format!(
        "module raw_fields;
         type Record is {{ first: Int, second: Int, third: Int }};
         {body}"
    );
    let ast = Parser::new(&source).parse_module().unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new("raw_fields"))
        .compile_module(&ast)
        .unwrap();
    let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    validate_module(&module).unwrap();
    Interpreter::new(Shared::new(module).into_arc())
}

#[test]
fn inline_record_reads_and_writes_preserve_adjacent_records() {
    for integer_address in [false, true] {
        let mut vm = interpreter(
            "fn probe(pointer: &unsafe Record) -> Int {
                let record = unsafe { &mut *pointer };
                let old = record.third;
                record.second = 73;
                old + record.second
            }",
        );
        let mut slots = Box::new([1, 2, 3, 4, 5, 6].map(Value::from_i64));
        let base = slots.as_mut_ptr() as usize;
        vm.state.cbgr_bridge_extents.insert(base, std::mem::size_of_val(slots.as_ref()));
        let address = base + 3 * std::mem::size_of::<Value>();
        let pointer = if integer_address {
            Value::from_i64(address as i64)
        } else {
            Value::from_ptr(address as *mut u8)
        };
        let entry = vm.state.module.find_function_by_name("raw_fields.probe").unwrap();
        let result = vm.execute_function_with_args(entry, &[pointer]).unwrap();
        assert_eq!(result.try_as_i64(), Some(79));
        assert_eq!(slots.map(|value| value.as_i64()), [1, 2, 3, 4, 73, 6]);
        vm.state.cbgr_bridge_extents.remove(&base);
    }
}

#[test]
fn inline_record_projection_rejects_truncated_and_misaligned_storage() {
    for body in [
        "fn probe(pointer: &unsafe Record) -> Int { unsafe { (*pointer).third } }",
        "fn probe(pointer: &unsafe mut Record) -> Int { unsafe { (*pointer).third = 73; } 73 }",
    ] {
        for offset in [1, 16] {
            let mut vm = interpreter(body);
            let mut slots = Box::new([1, 2, 3].map(Value::from_i64));
            let base = slots.as_mut_ptr() as usize;
            vm.state.cbgr_bridge_extents.insert(base, std::mem::size_of_val(slots.as_ref()));
            let pointer = Value::from_ptr((base + offset) as *mut u8);
            let entry = vm.state.module.find_function_by_name("raw_fields.probe").unwrap();
            assert!(vm.execute_function_with_args(entry, &[pointer]).is_err());
            assert_eq!(slots.map(|value| value.as_i64()), [1, 2, 3]);
            vm.state.cbgr_bridge_extents.remove(&base);
        }
    }
}

#[test]
fn inline_record_projection_requires_a_live_allocation() {
    let mut vm = interpreter(
        "fn probe(pointer: &unsafe Record) -> Int { unsafe { (*pointer).third } }",
    );
    let mut slots = Box::new([1, 2, 3].map(Value::from_i64));
    let base = slots.as_mut_ptr() as usize;
    vm.state.cbgr_bridge_extents.insert(base, std::mem::size_of_val(slots.as_ref()));
    vm.state.cbgr_bridge_extents.remove(&base);
    let entry = vm.state.module.find_function_by_name("raw_fields.probe").unwrap();
    let result = vm.execute_function_with_args(entry, &[Value::from_ptr(base as *mut u8)]);
    assert!(result.is_err(), "stale record read returned {result:?}");
}
