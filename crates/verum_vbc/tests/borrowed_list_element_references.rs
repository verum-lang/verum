//! T1684: borrowing an element follows the same container as length/value reads.
//! Parsed source is serialized before execution; no stdlib archive is injected.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_vbc::bytecode::decode_instructions;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::instruction::{CbgrSubOpcode, Instruction, Reg};
use verum_vbc::interpreter::{Interpreter, InterpreterError};
use verum_vbc::module::{FunctionDescriptor, FunctionId, VbcModule};
use verum_vbc::types::{StringId, TypeId};
use verum_vbc::value::{Capabilities, ThinRef, Value};

const ENV: &str = r#"
type Holder is { marker: Int, values: List<Int> };
type Packet is Empty | Items(List<Int>);
fn sample() -> List<Int> {
    let mut values: List<Int> = List<Int>.new();
    values.push(11);
    values.push(42);
    values.push(99);
    values
}
fn record_values(holder: &Holder) -> Maybe<&List<Int>> {
    Maybe.Some(&holder.values)
}
fn variant_values(packet: &Packet) -> Maybe<&List<Int>> {
    match packet {
        Packet.Items(ref values) => Maybe.Some(values),
        Packet.Empty => Maybe.None,
    }
}
fn record_values_mut(holder: &mut Holder) -> Maybe<&mut List<Int>> {
    Maybe.Some(&mut holder.values)
}
fn variant_values_mut(packet: &mut Packet) -> Maybe<&mut List<Int>> {
    match packet {
        Packet.Items(ref mut values) => Maybe.Some(values),
        Packet.Empty => Maybe.None,
    }
}
fn read_value(values: &List<Int>, index: Int) -> Int { values[index] }
fn read_ref(values: &List<Int>, index: Int) -> Int {
    let item = &values[index];
    *item
}
fn forward(values: &List<Int>, index: Int) -> Int { read_ref(values, index) }
fn write_ref(values: &mut List<Int>, index: Int) {
    let item = &mut values[index];
    *item = 73;
}
"#;

fn body_has(module: &VbcModule, name: &str, predicate: impl Fn(&Instruction) -> bool) {
    let function = module
        .functions
        .iter()
        .find(|function| {
            module.get_string(function.name).is_some_and(|actual| {
                actual == name
                    || actual
                        .strip_suffix(name)
                        .is_some_and(|prefix| prefix.ends_with('.'))
            })
        })
        .unwrap_or_else(|| panic!("missing source function {name}"));
    let start = function.bytecode_offset as usize;
    let end = start + function.bytecode_length as usize;
    let instructions = decode_instructions(&module.bytecode[start..end]).expect("decode body");
    assert!(
        instructions.iter().any(predicate),
        "expected carrier instruction in {:?}: {instructions:?}",
        module.get_string(function.name)
    );
}

fn execute(body: &str) -> Result<Value, InterpreterError> {
    let source: Text = format!("{ENV}\n{body}").into();
    let ast = Parser::new(source.as_str())
        .parse_module()
        .expect("source grammar");
    let module = VbcCodegen::with_config(CodegenConfig::new("borrowed_lists"))
        .compile_module(&ast)
        .expect("source VBC");
    let bytes = verum_vbc::serialize::serialize_module(&module).expect("serialize VBC");
    let module = verum_vbc::deserialize::deserialize_module(&bytes).expect("decode VBC");
    body_has(&module, "read_ref", |instruction| {
        matches!(instruction, Instruction::CbgrExtended { sub_op, .. }
            if *sub_op == CbgrSubOpcode::RefListElement as u8)
    });
    body_has(&module, "read_value", |instruction| {
        matches!(instruction, Instruction::GetE { .. })
    });
    body_has(&module, "record_values", |instruction| {
        matches!(instruction, Instruction::CbgrExtended { sub_op, .. }
            if *sub_op == CbgrSubOpcode::RefField as u8)
    });
    body_has(&module, "variant_values", |instruction| {
        matches!(instruction, Instruction::GetVariantDataRef { .. })
    });
    let entry = module
        .functions
        .iter()
        .find(|function| {
            module
                .get_string(function.name)
                .is_some_and(|name| name == "probe" || name.ends_with(".probe"))
        })
        .expect("qualified probe")
        .id;
    Interpreter::new(Arc::new(module)).execute_function(entry)
}

fn value(body: &str, expected: i64) {
    let actual = execute(body).expect("source must execute");
    assert!(actual.is_int(), "expected Int, got {actual:?}");
    assert_eq!(actual.as_i64(), expected);
}

fn bounds(body: &str, index: i64, length: usize) {
    let error =
        execute(body).expect_err("invalid index must be refused before exposing a reference");
    assert!(
        matches!(error, InterpreterError::IndexOutOfBounds { index: actual, length: actual_length }
            if actual == index && actual_length == length),
        "expected index {index} for length {length}, got {error:?}"
    );
}

#[test]
fn a_direct_list_element_borrow_reads_the_selected_element() {
    value(
        "fn probe() -> Int { let values = sample(); let item = &values[1]; *item }",
        42,
    );
}

#[test]
fn an_ordinary_list_reference_can_be_forwarded_to_element_borrowing() {
    value(
        "fn probe() -> Int { let values = sample(); forward(&values, 1) }",
        42,
    );
}

#[test]
fn a_record_payload_returned_through_maybe_keeps_length_and_value_indexing() {
    value(
        r#"
fn probe() -> Int {
    let holder = Holder { marker: 700, values: sample() };
    match record_values(&holder) {
        Maybe.Some(values) => values.len() * 100 + read_value(values, 1),
        Maybe.None => -1,
    }
}
"#,
        342,
    );
}

#[test]
fn a_record_payload_returned_through_maybe_supports_element_borrowing() {
    value(
        r#"
fn probe() -> Int {
    let holder = Holder { marker: 700, values: sample() };
    match record_values(&holder) {
        Maybe.Some(values) => forward(values, 1),
        Maybe.None => -1,
    }
}
"#,
        42,
    );
}

#[test]
fn a_variant_payload_returned_through_maybe_keeps_length_and_value_indexing() {
    value(
        r#"
fn probe() -> Int {
    let packet = Packet.Items(sample());
    match variant_values(&packet) {
        Maybe.Some(values) => values.len() * 100 + read_value(values, 1),
        Maybe.None => -1,
    }
}
"#,
        342,
    );
}

#[test]
fn a_variant_payload_returned_through_maybe_supports_element_borrowing() {
    value(
        r#"
fn probe() -> Int {
    let packet = Packet.Items(sample());
    match variant_values(&packet) {
        Maybe.Some(values) => forward(values, 1),
        Maybe.None => -1,
    }
}
"#,
        42,
    );
}

#[test]
fn a_borrowed_record_element_updates_the_original_list() {
    value(
        r#"
fn probe() -> Int {
    let mut holder = Holder { marker: 700, values: sample() };
    match record_values_mut(&mut holder) {
        Maybe.Some(values) => write_ref(values, 1),
        Maybe.None => {},
    }
    holder.values[0] * 10000 + holder.values[1] * 100 + holder.values[2]
}
"#,
        117399,
    );
}

#[test]
fn a_borrowed_variant_element_updates_the_original_list() {
    value(
        r#"
fn probe() -> Int {
    let mut packet = Packet.Items(sample());
    match variant_values_mut(&mut packet) {
        Maybe.Some(values) => write_ref(values, 1),
        Maybe.None => {},
    }
    match packet {
        Packet.Items(values) => values[0] * 10000 + values[1] * 100 + values[2],
        Packet.Empty => -1,
    }
}
"#,
        117399,
    );
}

#[test]
fn an_absent_variant_does_not_invent_a_list_reference() {
    value(
        r#"
fn probe() -> Int {
    let packet = Packet.Empty;
    match variant_values(&packet) { Maybe.Some(_) => -1, Maybe.None => 7 }
}
"#,
        7,
    );
}

#[test]
fn an_empty_returned_list_retains_zero_length() {
    value(
        r#"
fn probe() -> Int {
    let packet = Packet.Items(List<Int>.new());
    match variant_values(&packet) { Maybe.Some(values) => values.len(), Maybe.None => -1 }
}
"#,
        0,
    );
}

#[test]
fn an_empty_returned_list_refuses_element_borrowing() {
    bounds(
        r#"
fn probe() -> Int {
    let packet = Packet.Items(List<Int>.new());
    match variant_values(&packet) { Maybe.Some(values) => read_ref(values, 0), Maybe.None => -1 }
}
"#,
        0,
        0,
    );
}

#[test]
fn a_direct_list_refuses_an_index_at_its_length() {
    bounds(
        "fn probe() -> Int { let values = sample(); read_ref(&values, 3) }",
        3,
        3,
    );
}

#[test]
fn a_returned_record_list_refuses_an_index_at_its_actual_length() {
    bounds(
        r#"
fn probe() -> Int {
    let holder = Holder { marker: 700, values: sample() };
    match record_values(&holder) { Maybe.Some(values) => read_ref(values, 3), Maybe.None => -1 }
}
"#,
        3,
        3,
    );
}

#[test]
fn a_returned_variant_list_refuses_a_negative_index_with_its_actual_length() {
    bounds(
        r#"
fn probe() -> Int {
    let packet = Packet.Items(sample());
    match variant_values(&packet) { Maybe.Some(values) => read_ref(values, -1), Maybe.None => -1 }
}
"#,
        -1,
        3,
    );
}

fn element_interpreter(register_layers: usize) -> Interpreter {
    let mut module = VbcModule::new("element_reference_opcode".into());
    let mut instructions = List::new();
    let mut receiver = Reg(0);
    for layer in 0..register_layers {
        let next = Reg(4 + layer as u16);
        instructions.push(Instruction::Ref {
            dst: next,
            src: receiver,
        });
        receiver = next;
    }
    instructions.push(Instruction::LoadI {
        dst: Reg(1),
        value: 0,
    });
    let operands: List<u8> = [2, receiver.0 as u8, 1].into_iter().collect();
    instructions.push(Instruction::CbgrExtended {
        sub_op: CbgrSubOpcode::RefListElement as u8,
        operands: operands.into(),
    });
    instructions.push(Instruction::Deref {
        dst: Reg(3),
        ref_reg: Reg(2),
    });
    instructions.push(Instruction::Ret { value: Reg(3) });
    for instruction in &instructions {
        verum_vbc::bytecode::encode_instruction(instruction, &mut module.bytecode);
    }
    let mut function = FunctionDescriptor::new(StringId::EMPTY);
    function.id = FunctionId(0);
    function.register_count = 4 + register_layers as u16;
    function.bytecode_length = module.bytecode.len() as u32;
    module.functions.push(function);
    Interpreter::new(Arc::new(module))
}

#[test]
fn a_null_thin_reference_is_refused_before_reading_a_container_header() {
    for register_layers in [0, 2] {
        let error = element_interpreter(register_layers)
            .execute_function_with_args(FunctionId(0), &[Value::from_thin_ref(ThinRef::null())])
            .expect_err("null ThinRef must be refused");
        assert!(matches!(error, InterpreterError::NullPointer), "{error:?}");
    }
}

#[test]
fn a_thin_reference_and_register_chain_reach_the_same_element() {
    for register_layers in [0, 2] {
        let mut interpreter = element_interpreter(register_layers);
        let array = interpreter
            .state
            .heap
            .alloc_array(TypeId::INT, 1)
            .expect("inline array");
        // SAFETY: the live allocation has one initialized Value slot, and the
        // local carrier slot remains alive until this interpreter call returns.
        unsafe {
            *array.data_ptr().cast::<Value>() = Value::from_i64(37);
        }
        let mut slot = Value::from_ptr(array.as_ptr().cast::<u8>());
        let reference = Value::from_thin_ref(ThinRef::new(
            (&mut slot as *mut Value).cast::<u8>(),
            0,
            0,
            Capabilities::FULL,
        ));
        let result = interpreter
            .execute_function_with_args(FunctionId(0), &[reference])
            .expect("ThinRef/register chain element");
        assert_eq!(result.as_i64(), 37);
    }
}
