//! Sum payloads share the ordinary value-copy and affine-transfer boundary.
#![cfg(feature = "codegen")]
#[path = "fixtures/canonical_drop.rs"]
mod canonical_drop;

use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, CodegenResult, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

fn compile(source: &str, unit: bool) -> CodegenResult<VbcModule> {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("payload"));
    canonical_drop::initialize(&mut codegen);
    if unit {
        codegen.collect_unit_declarations(&[&ast])?;
        codegen.compile_unit_items(&[&ast], ItemFailurePolicy::Strict)?;
        codegen.finalize_module_from_state()
    } else {
        codegen.compile_additional_module(&ast)
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
        mount core.base.protocols.Drop;
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

#[test]
fn an_inferred_conditional_sum_preserves_its_payload_declaration() {
    run(r#"
        type Signal<T> is NoSignal | Finished | Sample(T);
        fn sample(value: Int) -> Signal<Int> {
            let selected = if value < 0 {
                if value < -8 { Signal.Finished } else { Signal.NoSignal }
            } else if value < 10 {
                Signal.Sample(value)
            } else {
                Signal.NoSignal
            };
            match selected {
                Signal.Sample(item) => Signal.Sample(item),
                Signal.Finished => Signal.Finished,
                _ => Signal.NoSignal,
            }
        }
        fn probe() -> Int {
            let valid = match sample(3) { Signal.Sample(item) => item, _ => 0 };
            let empty = match sample(15) { Signal.NoSignal => 30, _ => 0 };
            let finished = match sample(-9) { Signal.Finished => 40, _ => 0 };
            valid + empty + finished
        }
    "#);
}

#[test]
fn an_inferred_conditional_sum_retains_its_borrowed_affine_payload() {
    let source = r#"
        type affine Ticket is { number: Int };
        type Signal<T> is NoSignal | Sample(T);
        fn duplicate(input: &Ticket, choose: Bool) -> Signal<Ticket> {
            let selected = if choose { Signal.NoSignal } else { Signal.Sample(input) };
            match selected {
                Signal.Sample(borrowed) => Signal.Sample(*borrowed),
                Signal.NoSignal => Signal.NoSignal,
            }
        }
    "#;
    for unit in [false, true] {
        let error = compile(source, unit).expect_err("the selected payload is still borrowed");
        assert!(error.to_string().contains("exclusive borrowed owner"), "{error}");
    }
}

#[test]
fn a_nullary_variant_of_another_sum_cannot_supply_ownership_evidence() {
    let source = r#"
        type Signal<T> is Empty | Sample(T);
        type Different<T> is Empty | Other(T);
        type Container<T> is One(T);
        fn mixed(choose: Bool) -> Container<Signal<Int>> {
            let selected = if choose { Different.Empty } else { Signal.Sample(73) };
            Container.One(selected)
        }
    "#;
    for unit in [false, true] {
        let error = compile(source, unit).expect_err("the empty variant belongs to a different sum");
        assert!(error.to_string().contains("retained source declaration"), "{error}");
    }
}
