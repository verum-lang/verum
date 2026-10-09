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
