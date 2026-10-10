//! T1706: physical storage proof must retain declared integer element meaning.
//! Parsed source and independently decoded VBC; no typechecker or stdlib bake.
#![cfg(feature = "codegen")]

use verum_common::{List, Shared, Text};
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::interpreter::Interpreter;

#[derive(Clone, Copy, Debug)]
enum Boundary {
    Local,
    ArrayReturn,
    ListReturn,
}

fn check_elements(element: &str, first: i64, second: i64, boundary: Boundary) {
    check_declared_elements("", element, first, second, boundary);
}

fn check_declared_elements(prelude: &str, element: &str, first: i64, second: i64, boundary: Boundary) {
    let producer = format!(
        "let values: [{element}; 3] = [{first}, {second}, 1];"
    );
    let checks = format!(
        "assert_eq(values[0], {first}); assert_eq(values[1], {second}); assert_eq(values[2], 1); 1"
    );
    let source = match boundary {
        Boundary::Local => format!("fn probe() -> Int {{ {producer} {checks} }}"),
        Boundary::ArrayReturn => format!(
            "fn make() -> [{element}; 3] {{ {producer} values }} fn probe() -> Int {{ let values = make(); {checks} }}"
        ),
        Boundary::ListReturn => format!(
            "fn make() -> List<{element}> {{ {producer} values }} fn probe() -> Int {{ let values = make(); {checks} }}"
        ),
    };
    let source = format!("{prelude}\n{source}");
    let parsed = Parser::new(&source).parse_module().expect("source grammar");
    let original = VbcCodegen::with_config(CodegenConfig::new("signed_array_elements"))
        .compile_module(&parsed).expect("source lowering");
    let encoded = verum_vbc::serialize::serialize_module(&original).expect("encode VBC");
    let decoded = verum_vbc::deserialize::deserialize_module(&encoded).expect("decode VBC");
    let mut failures: List<Text> = List::new();
    for (route, module) in [("source", original), ("wire", decoded)] {
        let probe = module.functions.iter()
            .find(|function| module.get_string(function.name) == Some("signed_array_elements.probe"))
            .expect("exact probe declaration").id;
        match Interpreter::new(Shared::new(module).into_arc()).execute_function(probe) {
            Ok(value) if value.is_int() && value.as_i64() == 1 => {}
            Ok(value) => failures.push(format!("{element} {boundary:?} {route}: {value:?}").into()),
            Err(error) => failures.push(format!("{element} {boundary:?} {route}: {error}").into()),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

macro_rules! case {
    ($name:ident, $element:literal, $first:expr, $second:expr, $boundary:ident) => {
        #[test]
        fn $name() { check_elements($element, $first, $second, Boundary::$boundary); }
    };
}

case!(signed_byte_local, "Int8", -128, -1, Local);
case!(signed_byte_array_return, "Int8", -128, -1, ArrayReturn);
case!(signed_byte_list_return, "Int8", -128, -1, ListReturn);
case!(signed_short_local, "Int16", -32768, -1, Local);
case!(signed_short_array_return, "Int16", -32768, -1, ArrayReturn);
case!(signed_short_list_return, "Int16", -32768, -1, ListReturn);
case!(signed_word_local, "Int32", -2147483648, -1, Local);
case!(signed_word_array_return, "Int32", -2147483648, -1, ArrayReturn);
case!(signed_word_list_return, "Int32", -2147483648, -1, ListReturn);
case!(unsigned_byte_local, "Byte", 128, 255, Local);
case!(unsigned_byte_array_return, "Byte", 128, 255, ArrayReturn);
case!(unsigned_byte_list_return, "Byte", 128, 255, ListReturn);
case!(unsigned_short_local, "UInt16", 32768, 65535, Local);
case!(unsigned_short_array_return, "UInt16", 32768, 65535, ArrayReturn);
case!(unsigned_short_list_return, "UInt16", 32768, 65535, ListReturn);
case!(unsigned_word_local, "UInt32", 2147483648, 4294967295, Local);
case!(unsigned_word_array_return, "UInt32", 2147483648, 4294967295, ArrayReturn);
case!(unsigned_word_list_return, "UInt32", 2147483648, 4294967295, ListReturn);

#[test]
fn signed_short_alias_local() {
    check_declared_elements("type SignedShort = Int16;", "SignedShort", -32768, -1, Boundary::Local);
}

#[test]
fn signed_short_alias_list_return() {
    check_declared_elements("type SignedShort = Int16;", "SignedShort", -32768, -1, Boundary::ListReturn);
}

// These controls force a canonical packed producer before applying an alias
// contract. The older alias fixtures above can legitimately use boxed Lists.
fn check_packed_elements(
    prelude: &str,
    element: &str,
    producer: &str,
    width: usize,
    first: i64,
    second: i64,
    boundary: Boundary,
) {
    use verum_vbc::array_storage::{ArrayResultFact, ArrayResultFacts};
    use verum_vbc::instruction::{Instruction, MemSubOpcode};

    let checks = format!(
        "assert_eq(values[0], {first}); assert_eq(values[1], {second}); assert_eq(values[2], 1); 1"
    );
    let body = match boundary {
        Boundary::Local => format!("fn probe() -> Int {{ {producer} {checks} }}"),
        Boundary::ArrayReturn => format!(
            "fn make() -> [{element}; 3] {{ {producer} values }} fn probe() -> Int {{ let values = make(); {checks} }}"
        ),
        Boundary::ListReturn => format!(
            "fn make() -> List<{element}> {{ {producer} values }} fn probe() -> Int {{ let values = make(); {checks} }}"
        ),
    };
    let source = format!("{prelude}\n{body}");
    let original = VbcCodegen::with_config(CodegenConfig::new("packed_signed_array_elements"))
        .compile_module(&Parser::new(&source).parse_module().expect("source grammar"))
        .expect("source lowering");
    let encoded = verum_vbc::serialize::serialize_module(&original).expect("encode VBC");
    let decoded = verum_vbc::deserialize::deserialize_module(&encoded).expect("decode VBC");
    let owner = match boundary {
        Boundary::Local => "packed_signed_array_elements.probe",
        _ => "packed_signed_array_elements.make",
    };
    let mut failures: List<Text> = List::new();
    for (route, module) in [("source", original), ("wire", decoded)] {
        let function = module.functions.iter()
            .find(|function| module.get_string(function.name) == Some(owner))
            .expect("exact packed producer");
        let start = function.bytecode_offset as usize;
        let end = start + function.bytecode_length as usize;
        let instructions = verum_vbc::bytecode::decode_instructions(&module.bytecode[start..end])
            .expect("independent producer decode");
        let mut facts = ArrayResultFacts::default();
        let packed = instructions.iter().any(|instruction| {
            facts.observe(instruction);
            let Instruction::MemExtended { sub_op, operands } = instruction else { return false };
            if !matches!(MemSubOpcode::from_byte(*sub_op),
                Some(MemSubOpcode::NewByteArray | MemSubOpcode::NewTypedArray)) {
                return false;
            }
            let mut cursor = 0;
            let destination = verum_vbc::encoding::decode_reg(operands, &mut cursor)
                .expect("canonical allocation register");
            facts.get(destination) == Some(ArrayResultFact::Packed { width, float: false, count: 3 })
        });
        if !packed {
            failures.push(format!("{element} {boundary:?} {route}: actual {owner} body did not establish packed width {width}, count 3; no packed-alias coverage").into());
            continue;
        }
        let probe = module.functions.iter()
            .find(|function| module.get_string(function.name) == Some("packed_signed_array_elements.probe"))
            .expect("exact probe declaration").id;
        match Interpreter::new(Shared::new(module).into_arc()).execute_function(probe) {
            Ok(value) if value.is_int() && value.as_i64() == 1 => {}
            Ok(value) => failures.push(format!("packed {element} {boundary:?} {route}: {value:?}").into()),
            Err(error) => failures.push(format!("packed {element} {boundary:?} {route}: {error}").into()),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

const PACKED_SIGNED_BYTE: &str =
    "let mut values: [Int8; 3] = [0_i8; 3]; values[0] = -128; values[1] = -1; values[2] = 1;";

#[test]
fn packed_signed_byte_local() {
    check_packed_elements("", "Int8", PACKED_SIGNED_BYTE, 1, -128, -1, Boundary::Local);
}

#[test]
fn packed_signed_byte_array_return() {
    check_packed_elements("", "Int8", PACKED_SIGNED_BYTE, 1, -128, -1, Boundary::ArrayReturn);
}

#[test]
fn packed_signed_byte_list_return() {
    check_packed_elements("", "Int8", PACKED_SIGNED_BYTE, 1, -128, -1, Boundary::ListReturn);
}

#[test]
fn packed_signed_alias_array_return() {
    check_packed_elements("type SignedShort is Int16;", "SignedShort",
        "let values: [Int16; 3] = [-32768, -1, 1];", 2, -32768, -1, Boundary::ArrayReturn);
}

#[test]
fn packed_signed_alias_list_return() {
    check_packed_elements("type SignedShort is Int16;", "SignedShort",
        "let values: [Int16; 3] = [-32768, -1, 1];", 2, -32768, -1, Boundary::ListReturn);
}

#[test]
fn packed_unsigned_alias_array_return() {
    check_packed_elements("type UnsignedShort is UInt16;", "UnsignedShort",
        "let values: [UInt16; 3] = [32768, 65535, 1];", 2, 32768, 65535, Boundary::ArrayReturn);
}

#[test]
fn packed_unsigned_alias_list_return() {
    check_packed_elements("type UnsignedShort is UInt16;", "UnsignedShort",
        "let values: [UInt16; 3] = [32768, 65535, 1];", 2, 32768, 65535, Boundary::ListReturn);
}

fn check_list_payload(source: &str, expected: &[i64]) {
    let original = VbcCodegen::with_config(CodegenConfig::new("signed_list_payload"))
        .compile_module(&Parser::new(source).parse_module().expect("source grammar"))
        .expect("source lowering");
    let encoded = verum_vbc::serialize::serialize_module(&original).expect("encode VBC");
    let decoded = verum_vbc::deserialize::deserialize_module(&encoded).expect("decode VBC");
    let mut failures: List<Text> = List::new();
    for (route, module) in [("source", original), ("wire", decoded)] {
        let make = module.functions.iter()
            .find(|function| module.get_string(function.name) == Some("signed_list_payload.make"))
            .expect("exact List producer");
        let start = make.bytecode_offset as usize;
        let instructions = verum_vbc::bytecode::decode_instructions(
            &module.bytecode[start..start + make.bytecode_length as usize]
        ).expect("independent List producer decode");
        let make = make.id;
        let mut interpreter = Interpreter::new(Shared::new(module).into_arc());
        let value = match interpreter.execute_function(make) {
            Ok(value) => value,
            Err(error) => { failures.push(format!("{route}: {error}").into()); continue; }
        };
        // Inspect the actual boxed payload before any typed index expression
        // could normalize a still-unsigned element and hide a broken conversion.
        if !value.is_ptr() || !interpreter.state.heap.contains(value.as_ptr()) {
            failures.push(format!("{route}: producer did not return an owned List object").into());
            continue;
        }
        let elements = interpreter.state.list_elements(value);
        let actual = elements.as_ref().and_then(|elements| elements.iter()
            .map(|value| value.is_int().then(|| value.as_i64())).collect::<Option<List<_>>>());
        if actual.as_deref() != Some(expected) {
            failures.push(format!("{route}: expected boxed {expected:?}, got {elements:?}; producer: {instructions:?}").into());
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn boxed_signed_list_payload_is_already_signed() {
    check_list_payload("fn make() -> List<Int16> { [-32768, -1, 1] }", &[-32768, -1, 1]);
}

#[test]
fn boxed_unsigned_list_payload_keeps_high_bits() {
    check_list_payload("fn make() -> List<UInt16> { [32768, 65535, 1] }", &[32768, 65535, 1]);
}

#[test]
fn packed_signed_byte_list_payload_is_normalized_before_return() {
    check_list_payload(&format!("fn make() -> List<Int8> {{ {PACKED_SIGNED_BYTE} values }}"), &[-128, -1, 1]);
}

#[test]
fn packed_signed_short_list_payload_is_normalized_before_return() {
    check_list_payload("fn make() -> List<Int16> { let values: [Int16; 3] = [-32768, -1, 1]; values }", &[-32768, -1, 1]);
}

#[test]
fn packed_signed_word_list_payload_is_normalized_before_return() {
    check_list_payload("fn make() -> List<Int32> { let values: [Int32; 3] = [-2147483648, -1, 1]; values }", &[-2147483648, -1, 1]);
}

#[test]
fn packed_signed_alias_list_payload_is_normalized_before_return() {
    check_list_payload("type SignedShort is Int16; fn make() -> List<SignedShort> { let values: [Int16; 3] = [-32768, -1, 1]; values }", &[-32768, -1, 1]);
}

#[test]
fn packed_unsigned_alias_list_payload_keeps_high_bits() {
    check_list_payload("type UnsignedShort is UInt16; fn make() -> List<UnsignedShort> { let values: [UInt16; 3] = [32768, 65535, 1]; values }", &[32768, 65535, 1]);
}

#[test]
fn signed_read_before_list_return_keeps_the_packed_producer() {
    check_list_payload("fn make() -> List<Int16> { let values: [Int16; 3] = [-32768, -1, 1]; let first = values[0]; values }", &[-32768, -1, 1]);
}

#[test]
fn named_field_unpack_preserves_signed_payloads() {
    check_list_payload("type Holder is { values: [Int16; 3] }; fn make() -> List<Int16> { let input: [Int16; 3] = [-32768, -1, 1]; let holder = Holder { values: input }; holder.values }", &[-32768, -1, 1]);
}

#[test]
fn shorthand_field_unpack_preserves_signed_payloads() {
    check_list_payload("type Holder is { values: [Int16; 3] }; fn make() -> List<Int16> { let values: [Int16; 3] = [-32768, -1, 1]; let holder = Holder { values }; holder.values }", &[-32768, -1, 1]);
}

#[test]
fn generic_element_named_like_a_primitive_does_not_gain_signed_normalization() {
    let parsed = Parser::new("fn read<Int16>(values: List<Int16>) -> Int16 { values[0] }")
        .parse_module().expect("generic source grammar");
    let original = VbcCodegen::with_config(CodegenConfig::new("generic_element_owner"))
        .compile_module(&parsed).expect("generic source lowering");
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&original).expect("encode generic VBC")
    ).expect("decode generic VBC");
    for (route, module) in [("source", original), ("wire", wire)] {
        let function = module.functions.iter().find(|function|
            module.get_string(function.name) == Some("generic_element_owner.read"))
            .expect("exact generic reader");
        let start = function.bytecode_offset as usize;
        let instructions = verum_vbc::bytecode::decode_instructions(
            &module.bytecode[start..start + function.bytecode_length as usize]
        ).expect("decode actual generic reader");
        assert!(instructions.iter().any(|instruction| matches!(instruction,
            verum_vbc::instruction::Instruction::GetE { .. })), "{route}: no element reader");
        assert!(!instructions.iter().any(|instruction| matches!(instruction,
            verum_vbc::instruction::Instruction::ArithExtended { sub_op, .. }
                if *sub_op == verum_vbc::instruction::ArithSubOpcode::SextI.to_byte()
        )), "{route}: a generic parameter acquired primitive sign semantics");
    }
}
