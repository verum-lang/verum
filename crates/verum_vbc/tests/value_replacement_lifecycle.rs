//! Assignment prepares the next owner before publishing it and retiring the old.
#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

fn exercise(source: &str) {
    let ast = Parser::new(source).parse_module().unwrap();
    for unit in [false, true] {
        let mut compiler = VbcCodegen::with_config(CodegenConfig::new("replacement"));
        let module = if unit {
            compiler.collect_unit_declarations(&[&ast]).unwrap();
            compiler.compile_unit_items(&[&ast], ItemFailurePolicy::Strict).unwrap();
            compiler.finalize_module_from_state().unwrap()
        } else {
            compiler.compile_module(&ast).unwrap()
        };
        let wire = verum_vbc::deserialize::deserialize_module(
            &verum_vbc::serialize::serialize_module(&module).unwrap(),
        ).unwrap();
        for module in [module, wire] {
            verum_vbc::validate::validate_module(&module).unwrap();
            let entry = module.find_function_by_name("replacement.probe").unwrap();
            let mut interpreter = Interpreter::new(module.into());
            assert_eq!(interpreter.execute_function(entry).unwrap().try_as_i64(), Some(73));
            let pooled = interpreter.state.heap.object_count();
            for _ in 0..4 {
                assert_eq!(interpreter.execute_function(entry).unwrap().try_as_i64(), Some(73));
                assert_eq!(interpreter.state.heap.object_count(), pooled,
                    "replacement retained an old allocation (unit={unit})");
            }
        }
    }
}

#[test]
fn local_text_replacement_preserves_named_sources_and_self_assignment() {
    exercise(r#"
        fn probe() -> Int {
            let original = f"{123456789}";
            let mut value = original;
            let mut step = 0;
            while step < 32 {
                value = f"{987654321}";
                value = original;
                value = value;
                step = step + 1;
            }
            if original.len() == 9 && value.len() == 9 { 73 } else { 0 }
        }
    "#);
}

#[test]
fn record_and_field_replacement_use_the_declared_owned_components() {
    exercise(r#"
        type Cell is { text: Text };
        fn probe() -> Int {
            let original = Cell { text: f"{123456789}" };
            let mut changed = original;
            let mut step = 0;
            while step < 32 {
                changed.text = f"{987654321}";
                changed.text = original.text;
                changed = original;
                step = step + 1;
            }
            if original.text.len() == 9 && changed.text.len() == 9 { 73 } else { 0 }
        }
    "#);
}

#[test]
fn compound_text_replacement_retires_previous_values_and_temporary_operands() {
    exercise(r#"
        type Cell is { text: Text };
        fn probe() -> Int {
            let original = f"{123456789}";
            let mut value = original;
            let mut cell = Cell { text: original };
            let mut step = 0;
            while step < 16 {
                value += original;
                cell.text += f"{987654321}";
                step = step + 1;
            }
            if value.len() == 153 && cell.text.len() == 153 && original.len() == 9 {
                73
            } else { 0 }
        }
    "#);
}

#[test]
fn initial_assignment_does_not_read_an_uninitialized_owner() {
    exercise(r#"
        fn choose(flag: Bool) -> Text {
            let value: Text;
            if flag { value = f"{123456789}"; }
            else { value = f"{987654321}"; }
            value
        }
        fn probe() -> Int {
            let first = choose(true);
            let second = choose(false);
            if first.len() == 9 && second.len() == 9 { 73 } else { 0 }
        }
    "#);
}

#[test]
fn temporary_receivers_retire_after_observing_their_length() {
    exercise(r#"
        fn probe() -> Int {
            let mut count = 0;
            let mut total = 0;
            while count < 32 {
                total = total + f"{123456789}".len();
                count = count + 1;
            }
            if total == 288 { 73 } else { 0 }
        }
    "#);
}

#[test]
fn tuple_components_copy_their_declared_owner_before_replacement() {
    exercise(r#"
        fn probe() -> Int {
            let original = (f"{123456789}", 73);
            let copied = original.0;
            let mut changed = original;
            changed.0 = f"{9876543210}";
            if original.0.len() == 9 && copied.len() == 9 && changed.0.len() == 10 {
                changed.1
            } else { 0 }
        }
    "#);
}
