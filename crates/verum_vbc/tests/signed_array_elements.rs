//! T1706: physical storage proof must retain declared integer element meaning.
//! Parsed source and independently decoded VBC; no typechecker or stdlib bake.
#![cfg(feature = "codegen")]

use verum_common::Shared;
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
    let parsed = Parser::new(&source).parse_module().expect("source grammar");
    let original = VbcCodegen::with_config(CodegenConfig::new("signed_array_elements"))
        .compile_module(&parsed).expect("source lowering");
    let encoded = verum_vbc::serialize::serialize_module(&original).expect("encode VBC");
    let decoded = verum_vbc::deserialize::deserialize_module(&encoded).expect("decode VBC");
    for (route, module) in [("source", original), ("wire", decoded)] {
        let probe = module.functions.iter()
            .find(|function| module.get_string(function.name) == Some("signed_array_elements.probe"))
            .expect("exact probe declaration").id;
        let value = Interpreter::new(Shared::new(module).into_arc())
            .execute_function(probe)
            .unwrap_or_else(|error| panic!("{element} {boundary:?} {route}: {error}"));
        assert_eq!(value.as_i64(), 1, "{element} {boundary:?} {route}");
    }
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
