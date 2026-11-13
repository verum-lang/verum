//! Sum payloads share the ordinary value-copy and affine-transfer boundary.
#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, CodegenResult, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

fn compile(source: &str, unit: bool) -> CodegenResult<VbcModule> {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("payload"));
    if unit {
        codegen.collect_unit_declarations(&[&ast])?;
        codegen.compile_unit_items(&[&ast], ItemFailurePolicy::Strict)?;
        codegen.finalize_module_from_state()
    } else {
        codegen.compile_module(&ast)
    }
}

fn run(source: &str) {
    for unit in [false, true] {
        let module = compile(source, unit).expect("payload compilation");
        let wire = verum_vbc::serialize::serialize_module(&module).expect("wire encode");
        let decoded = verum_vbc::deserialize::deserialize_module(&wire).expect("wire decode");
        for (route, module) in [("source", module), ("wire", decoded)] {
            verum_vbc::validate::validate_module(&module).expect("declaration validation");
            let entry = module.find_function_by_name("payload.probe").unwrap();
            let result = Interpreter::new(module.into()).execute_function(entry);
            assert_eq!(result.unwrap().try_as_i64(), Some(73), "unit={unit}, {route}");
        }
    }
}

#[test]
fn named_payload_copies_the_place_before_later_mutation() {
    run(r#"
        type Cell is { value: Int };
        type Choice<T> is Empty | Value(T);
        fn probe() -> Int {
            let mut source = Cell { value: 73 };
            let result = Choice.Value(source);
            source.value = 0;
            match result { Choice.Value(value) => value.value, _ => 0 }
        }
    "#);
}

#[test]
fn a_returned_nested_payload_survives_its_original_local_cleanup() {
    run(r#"
        type Cell is { value: Int };
        implement Drop for Cell { fn drop(&mut self) { self.value = 0; } }
        type Choice<T> is Empty | Value(T);
        fn produce() -> Choice<Choice<Cell>> {
            let source = Cell { value: 73 };
            Choice.Value(Choice.Value(source))
        }
        fn probe() -> Int {
            match produce() {
                Choice.Value(Choice.Value(value)) => value.value,
                _ => 0,
            }
        }
    "#);
}

#[test]
fn explicit_dereference_payload_uses_its_declared_copy_discipline() {
    run(r#"
        type Cell is { value: Int };
        type Choice<T> is Empty | Value(T);
        fn probe() -> Int {
            let mut source = Cell { value: 73 };
            let result = {
                let borrowed = &source;
                Choice.Value(*borrowed)
            };
            source.value = 0;
            match result { Choice.Value(value) => value.value, _ => 0 }
        }
    "#);
}

#[test]
fn reference_payload_keeps_the_original_referent() {
    let source = r#"
        type Cell is { value: Int };
        type Choice<T> is Empty | Value(T);
        fn probe() -> Int {
            let source = Cell { value: 73 };
            let borrowed = &source;
            let result: Choice<&Cell> = Choice.Value(borrowed);
            match result { Choice.Value(value) => value.value, _ => 0 }
        }
    "#;
    run(source);
    for unit in [false, true] {
        let module = compile(source, unit).unwrap();
        let entry = module.find_function_by_name("payload.probe").unwrap();
        let function = module.get_function(entry).unwrap();
        let start = function.bytecode_offset as usize;
        let body = verum_vbc::bytecode::decode_instructions(
            &module.bytecode[start..start + function.bytecode_length as usize],
        ).unwrap();
        assert!(!body.iter().any(|instruction| matches!(instruction,
            verum_vbc::Instruction::Clone { .. })), "reference cannot copy its referent: {body:?}");
    }
}

#[test]
fn borrowed_affine_payload_cannot_be_copied_into_a_sum() {
    let source = r#"
        type affine Cell is { value: Int };
        type Choice<T> is Empty | Value(T);
        fn copy(source: &Cell) -> Choice<Cell> { Choice.Value(*source) }
    "#;
    for unit in [false, true] {
        let error = match compile(source, unit) {
            Err(error) => error,
            Ok(_) => panic!("borrow cannot acquire payload ownership, unit={unit}"),
        };
        assert!(error.to_string().contains("exclusive borrowed owner"), "{error}");
    }
}
