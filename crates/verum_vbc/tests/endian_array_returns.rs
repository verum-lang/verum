//! T1698: fixed endian arrays retain byte layout at declaration and runtime.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::{Interpreter, ObjectHeader},
    module::VbcModule,
    serialize::serialize_module,
    types::{TypeId, TypeRef},
};

fn compile(body: &str, declarations: &str) -> VbcModule {
    compile_source(&format!("module endian_arrays; {declarations} fn probe() -> [Byte; 8] {{ {body} }}"))
}

fn compile_source(source: &str) -> VbcModule {
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
            let name = format!("{owner}.{method}");
            let info = functions.get(&name).unwrap_or_else(|| panic!("exact declaration {name}"));
            assert_eq!(info.return_type, Some(TypeRef::Array { element: Box::new(TypeRef::Concrete(TypeId::U8)), length }), "{name}");
        }
    }
}

fn assert_integer(body: &str, declarations: &str, expected: i64) {
    let module = compile_source(&format!("module endian_arrays; {declarations} fn probe() -> Int {{ {body} }}"));
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for (route, module) in [("source", module), ("serialized", wire)] {
        let entry = module.functions.iter().find(|f|
            module.get_string(f.name) == Some("endian_arrays.probe")
        ).unwrap().id;
        let value = Interpreter::new(Arc::new(module)).execute_function(entry)
            .unwrap_or_else(|error| panic!("{route}, {body}: {error}"));
        assert_eq!(value.as_i64(), expected, "{route}, {body}");
    }
}

const DECLARED_U64: &str = "implement UInt64 { fn to_be_bytes(self) -> [Byte; 8] { to_be_bytes_8(self) } }";

#[test]
fn inferred_declared_endian_return_indexes_every_byte() {
    assert_integer("let number: UInt64 = 0x0102030405060708; let bytes = number.to_be_bytes(); let mut i = 0; let mut errors = 0; while i < 8 { if bytes[i] as Int != i + 1 { errors = errors + 1; } i = i + 1; } errors", DECLARED_U64, 0);
}

#[test]
fn returned_conversion_preserves_inferred_array_layout() {
    assert_integer("let bytes = encoded(); bytes[7] as Int", "fn encoded() -> [Byte; 8] { let number: UInt64 = 0x0102030405060708; number.to_be_bytes() }", 8);
}

#[test]
fn a_borrowed_conversion_keeps_its_byte_stride() {
    assert_integer("let number: UInt64 = 0x0102030405060708; let bytes = number.to_be_bytes(); last(&bytes)", &format!("{DECLARED_U64} fn last(bytes: &[Byte; 8]) -> Int {{ bytes[7] as Int }}"), 8);
}

#[test]
fn a_record_field_preserves_conversion_bytes() {
    assert_integer("let number: UInt64 = 0x0102030405060708; let value = Encoded { bytes: number.to_be_bytes() }; value.bytes[7] as Int", "type Encoded is { bytes: [Byte; 8] };", 8);
}

#[test]
fn ordinary_lists_remain_mutable_lists() {
    assert_integer("let mut bytes = byte_list(); bytes.push(9 as Byte); bytes[8] as Int", "fn byte_list() -> List<Byte> { [1 as Byte,2 as Byte,3 as Byte,4 as Byte,5 as Byte,6 as Byte,7 as Byte,8 as Byte] }", 9);
}

#[test]
fn dynamic_fixed_widths_and_float_bits_have_exact_byte_carriers() {
    for (ty, value, expected) in [
        ("Int32", "0x01020304", &[1,2,3,4][..]),
        ("Int32", "-2", &[255,255,255,254][..]),
        ("Int", "0x0102030405060708", &[1,2,3,4,5,6,7,8][..]),
        ("UInt64", "0xFEDCBA9876543210", &[254,220,186,152,118,84,50,16][..]),
        ("Float", "1.5", &[63,248,0,0,0,0,0,0][..]),
        ("Float", "-0.0", &[128,0,0,0,0,0,0,0][..]),
    ] {
        let big = format!("module endian_arrays; fn probe() -> [Byte; {}] {{ let number: {ty} = {value}; number.to_be_bytes() }}", expected.len());
        assert_packed_bytes(compile_source(&big), expected);
        let little = big.replace("to_be_bytes", "to_le_bytes");
        let reversed: verum_common::List<u8> = expected.iter().rev().copied().collect();
        assert_packed_bytes(compile_source(&little), &reversed);
    }
}

#[test]
fn fixed_endian_array_bounds_use_declared_byte_length() {
    let source = format!("module endian_arrays; {DECLARED_U64} fn probe() -> Int {{ let number: UInt64 = 0x0102030405060708; let bytes = number.to_be_bytes(); bytes[8] as Int }}");
    let module = compile_source(&source);
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for module in [module, wire] {
        let entry = module.functions.iter().find(|f| module.get_string(f.name) == Some("endian_arrays.probe")).unwrap().id;
        assert!(matches!(Interpreter::new(Arc::new(module)).execute_function(entry),
            Err(verum_vbc::interpreter::InterpreterError::IndexOutOfBounds { index: 8, length: 8 })));
    }
}

// These widths use real stdlib bodies instead of primitive dynamic handlers.
// Keep the selected method declarations/bodies unchanged; this component does
// not claim that every method in the complete primitives unit is executable.
fn compile_with_endian_methods(source: &str, owner: &str) -> VbcModule {
    use verum_ast::{ItemKind, decl::{ImplItemKind, ImplKind}, ty::TypeKind};
    let mut ast = Parser::new(source).parse_module().expect("probe grammar");
    let mut primitives = Parser::new(include_str!("../../../core/base/primitives.vr"))
        .parse_module().expect("actual primitive grammar");
    primitives.items.retain_mut(|item| {
        let ItemKind::Impl(declaration) = &mut item.kind else { return false; };
        if !matches!(&declaration.kind, ImplKind::Inherent(ty)
            if matches!(&ty.kind, TypeKind::Path(path) if path.last_segment_name() == owner)) {
            return false;
        }
        declaration.items.retain(|item| matches!(&item.kind, ImplItemKind::Function(function)
            if ["to_le_bytes", "to_be_bytes", "from_le_bytes", "from_be_bytes"].contains(&function.name.name.as_str())));
        !declaration.items.is_empty()
    });
    assert_eq!(primitives.items.len(), 1, "exact source owner {owner}");
    ast.items.extend(primitives.items);
    VbcCodegen::with_config(CodegenConfig::new("endian_arrays"))
        .compile_module(&ast).expect("actual method codegen")
}

fn actual_endian_bytes(owner: &str, value: &str, expected: &[u8]) {
    for endian in ["be", "le"] {
        let source = format!("module endian_arrays; fn probe() -> [Byte; {}] {{ let number: {owner} = {value}; number.to_{endian}_bytes() }}", expected.len());
        let mut bytes: verum_common::List<u8> = expected.iter().copied().collect();
        if endian == "le" { bytes.reverse(); }
        assert_packed_bytes(compile_with_endian_methods(&source, owner), &bytes);
    }
}

#[test]
fn actual_uint16_methods_produce_fixed_bytes() { actual_endian_bytes("UInt16", "0x0102", &[1,2]); }
#[test]
fn actual_uint32_methods_produce_fixed_bytes() { actual_endian_bytes("UInt32", "0x01020304", &[1,2,3,4]); }
#[test]
fn actual_int16_methods_preserve_negative_bits() { actual_endian_bytes("Int16", "-2", &[255,254]); }
#[test]
fn actual_float32_methods_preserve_negative_zero_bytes() { actual_endian_bytes("Float32", "-0.0", &[128,0,0,0]); }

#[test]
fn actual_float32_roundtrip_preserves_negative_zero_bits() {
    for endian in ["be", "le"] {
        let source = format!("module endian_arrays; fn probe() -> Int {{ let number: Float32 = -0.0; let bytes = number.to_{endian}_bytes(); let decoded = Float32.from_{endian}_bytes(bytes); f32_to_bits(decoded) as Int }}");
        let module = compile_with_endian_methods(&source, "Float32");
        let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
        for module in [module, wire] {
            let entry = module.functions.iter().find(|function| module.get_string(function.name) == Some("endian_arrays.probe")).unwrap().id;
            assert_eq!(Interpreter::new(Arc::new(module)).execute_function(entry).expect("Float32 roundtrip").as_i64(), 0x80000000);
        }
    }
}
