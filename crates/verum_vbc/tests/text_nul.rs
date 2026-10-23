//! Short Text values must retain bytes which cannot fit the inline encoding.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{ItemFailurePolicy, VbcCodegen};
use verum_vbc::interpreter::Interpreter;

#[test]
fn nul_bytes_survive_literals_construction_and_text_operations() {
    let text = Parser::new(include_str!("../../../core/text/text.vr"))
        .parse_module().expect("Text grammar");
    let maybe = Parser::new(include_str!("../../../core/base/maybe.vr"))
        .parse_module().expect("Maybe grammar");
    let result = Parser::new(include_str!("../../../core/base/result.vr"))
        .parse_module().expect("Result grammar");
    let caller = Parser::new(r#"
module text_nul;
fn controls() -> Int {
    assert_eq("\0".len(), 1);
    assert_eq("\0\0".len(), 2);
    assert_eq("a\0".len(), 2);
    assert_eq("\0a".len(), 2);
    assert_eq("a\0b".len(), 3);
    assert_eq("abcde\0".len(), 6);
    assert_eq("abcdef\0".len(), 7);
    assert("\0" != "");
    assert("a\0" != "a");
    let joined = "ab" + "\0";
    assert_eq(joined.len(), 3);
    assert_eq(joined.as_bytes()[2], 0_u8);
    let sliced = "abcdef\0".slice(5, 7);
    assert_eq(sliced.len(), 2);
    assert_eq(sliced.as_bytes()[1], 0_u8);
    let bytes: List<Byte> = [0_u8, 65_u8, 0_u8];
    let decoded = Text.from_utf8(bytes.as_slice()).unwrap();
    assert_eq(decoded.len(), 3);
    assert_eq(decoded.as_bytes()[0], 0_u8);
    assert_eq(decoded.as_bytes()[1], 65_u8);
    assert_eq(decoded.as_bytes()[2], 0_u8);
    let mut dynamic = Text.new();
    dynamic.push_str(&"a"); dynamic.push_str(&"\0");
    assert_eq(dynamic.len(), 2);
    assert_eq(dynamic, "a\0");
    let copied = dynamic.clone();
    assert_eq(copied.len(), 2);
    assert_eq(copied.as_bytes()[1], 0_u8);
    22
}
"#).parse_module().expect("NUL controls grammar");
    let mut codegen = VbcCodegen::new();
    codegen.compile_module_with_mounts(&text,
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/text/text.vr"),
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../core"))
        .expect("production Text module");
    let modules = [&maybe, &result, &caller];
    codegen.collect_unit_declarations(&modules).expect("declarations");
    codegen.compile_unit_items(&modules, ItemFailurePolicy::Strict).expect("bodies");
    let mut module = codegen.finalize_module().expect("bytecode");
    module.resolve_protocol_dispatch();
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).expect("serialize"))
        .expect("deserialize");
    for (route, module) in [("source", module), ("wire", wire)] {
        let entry = module.find_function_by_name("text_nul.controls").unwrap();
        let actual = Interpreter::new(Arc::new(module)).execute_function(entry)
            .unwrap_or_else(|error| panic!("{route}: {error:?}"));
        assert_eq!(actual.as_i64(), 22, "{route}");
    }
}
