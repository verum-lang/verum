//! T1698: fixed endian arrays retain byte layout at declaration and runtime.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::{Interpreter, heap::ObjectHeader},
    module::VbcModule,
    serialize::serialize_module,
    types::{TypeId, TypeRef},
};

fn compile(body: &str, declarations: &str) -> VbcModule {
    let source = format!("module endian_arrays; {declarations} fn probe() -> [Byte; 8] {{ {body} }}");
    VbcCodegen::with_config(CodegenConfig::new("endian_arrays"))
        .compile_module(&Parser::new(&source).parse_module().expect("source"))
        .expect("codegen")
}

fn assert_packed_bytes(module: VbcModule, expected: &[u8]) {
    let wire = deserialize_module(&serialize_module(&module).expect("serialize")).expect("deserialize");
    for (route, module) in [("source", module), ("serialized", wire)] {
        let entry = module.functions.iter().find(|f|
            module.get_string(f.name) == Some("endian_arrays.probe")
        ).expect("exact entry").id;
        let mut interpreter = Interpreter::new(Arc::new(module));
        let value = interpreter.execute_function(entry).expect("execute");
        assert!(value.is_ptr(), "{route}: array result");
        // The result is live for this interpreter's lifetime. Only inspect its
        // runtime header before choosing the declared byte payload layout.
        let header = unsafe { &*value.as_ptr::<ObjectHeader>() };
        assert_eq!(header.type_id, TypeId::U8, "{route}: fixed bytes must use the byte-array carrier");
        assert_eq!(header.size as usize, expected.len(), "{route}: fixed length");
        let data = unsafe { std::slice::from_raw_parts(value.as_ptr::<u8>().add(std::mem::size_of::<ObjectHeader>()), expected.len()) };
        assert_eq!(data, expected, "{route}: endian bytes");
    }
}

#[test]
fn literal_fixed_bytes_are_the_positive_layout_control() {
    assert_packed_bytes(compile("let bytes: [Byte; 8] = [1,2,3,4,5,6,7,8]; bytes", ""), &[1,2,3,4,5,6,7,8]);
}

#[test]
fn dynamic_big_endian_has_the_declared_fixed_byte_carrier() {
    assert_packed_bytes(compile("let number: UInt64 = 0x0102030405060708; number.to_be_bytes()", ""), &[1,2,3,4,5,6,7,8]);
}

#[test]
fn dynamic_little_endian_has_the_declared_fixed_byte_carrier() {
    assert_packed_bytes(compile("let number: UInt64 = 0x0807060504030201; number.to_le_bytes()", ""), &[1,2,3,4,5,6,7,8]);
}

#[test]
fn inline_big_endian_has_the_declared_fixed_byte_carrier() {
    assert_packed_bytes(compile("to_be_bytes_8(0x0102030405060708)", ""), &[1,2,3,4,5,6,7,8]);
}

#[test]
fn inline_little_endian_has_the_declared_fixed_byte_carrier() {
    assert_packed_bytes(compile("to_le_bytes_8(0x0807060504030201)", ""), &[1,2,3,4,5,6,7,8]);
}

#[test]
fn full_primitive_source_declarations_preserve_byte_array_elements() {
    let ast = Parser::new(include_str!("../../../core/base/primitives.vr")).parse_module().expect("complete primitive source");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("core.base.primitives"));
    codegen.collect_unit_declarations(&[&ast]).expect("all source declarations");
    let functions = codegen.export_functions();
    for (owner, length) in [("Int", 8), ("UInt64", 8), ("UInt32", 4), ("UInt16", 2)] {
        for method in ["to_be_bytes", "to_le_bytes"] {
            let name = format!("core.base.primitives.{owner}.{method}");
            let info = functions.get(&name).unwrap_or_else(|| panic!("exact declaration {name}"));
            assert_eq!(info.return_type, Some(TypeRef::Array { element: Box::new(TypeRef::Concrete(TypeId::U8)), length }), "{name}");
        }
    }
}
