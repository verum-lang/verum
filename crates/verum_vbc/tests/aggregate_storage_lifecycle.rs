//! Declared structural owners retire their allocation after their components.
#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

fn exercise(source: &str) {
    let ast = Parser::new(source).parse_module().unwrap();
    for unit in [false, true] {
        let mut compiler = VbcCodegen::with_config(CodegenConfig::new("aggregate_lifetime"));
        let module = if unit {
            compiler.collect_unit_declarations(&[&ast]).unwrap();
            compiler
                .compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
                .unwrap();
            compiler.finalize_module_from_state().unwrap()
        } else {
            compiler.compile_module(&ast).unwrap()
        };
        let wire = verum_vbc::deserialize::deserialize_module(
            &verum_vbc::serialize::serialize_module(&module).unwrap(),
        )
        .unwrap();
        for module in [module, wire] {
            verum_vbc::validate::validate_module(&module).unwrap();
            let probe = module
                .find_function_by_name("aggregate_lifetime.probe")
                .unwrap();
            let mut interpreter = Interpreter::new(module.into());
            for _ in 0..4 {
                assert_eq!(
                    interpreter.execute_function(probe).unwrap().try_as_i64(),
                    Some(73)
                );
                assert_eq!(
                    interpreter.state.heap.object_count(),
                    0,
                    "structural ownership must retire every allocation, unit={unit}"
                );
            }
        }
    }
}

#[test]
fn nested_records_and_tuples_retire_all_owned_allocations() {
    exercise(
        r#"
        type Cell is { value: Int };
        type Packet is { cell: Cell, pair: (Cell, Int) };
        fn probe() -> Int {
            let cell = Cell { value: 31 };
            let packet = Packet { cell: cell, pair: (cell, 11) };
            let duplicate = packet;
            packet.cell.value + duplicate.pair.0.value + duplicate.pair.1
        }
    "#,
    );
}

#[test]
fn sum_payloads_and_borrowed_fields_keep_distinct_ownership() {
    exercise(
        r#"
        type Cell is { value: Int };
        type Choice is Empty | Item(Cell);
        type View is { cell: &Cell };
        fn read(cell: &Cell) -> Int {
            let borrowed = View { cell: cell };
            borrowed.cell.value
        }
        fn probe() -> Int {
            let cell = Cell { value: 73 };
            let owned = Choice.Item(cell);
            let empty = Choice.Empty;
            let observed = read(&cell);
            match owned { Choice.Item(payload) => payload.value, Choice.Empty => observed }
        }
    "#,
    );
}

#[test]
fn a_shadowing_value_field_still_copies_its_owner() {
    exercise(
        r#"
        type Selection is Absent | Present(Int);
        type Cell is { value: Int };
        type Holder is { Absent: Cell };
        fn probe() -> Int {
            let Selection = Holder { Absent: Cell { value: 73 } };
            let copied = Selection.Absent;
            let mut changed = Selection;
            changed.Absent.value = 0;
            copied.value
        }
    "#,
    );
}
