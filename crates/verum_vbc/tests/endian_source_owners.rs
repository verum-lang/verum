//! T1698: unrelated source imports must not alter primitive return descriptors.
#![cfg(feature = "codegen")]
use verum_fast_parser::Parser;
use verum_vbc::{codegen::{CodegenConfig, VbcCodegen}, types::{TypeId, TypeRef}};

// The independent same-leaf declaration failure is retained under T1703 in
// docs/architecture/evidence/endian-carrier-baseline/fixtures/.
#[test]
fn another_sources_alias_mount_cannot_retype_primitive_array_returns() {
    let foreign = Parser::new("module foreign; public type Word is { marker: Int };").parse_module().unwrap();
    let mounted = Parser::new("module unrelated; mount foreign.Word as Byte; fn holder(value: [Byte; 8]) -> [Byte; 8] { value }").parse_module().unwrap();
    let primitive = Parser::new("module core.base.primitives; implement UInt64 { public fn to_be_bytes(self) -> [Byte; 8] { to_be_bytes_8(self) } }").parse_module().unwrap();
    for files in [[&foreign, &mounted, &primitive], [&primitive, &mounted, &foreign]] {
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new("core.base"));
        codegen.collect_unit_declarations(&files).expect("whole source unit declarations");
        let functions = codegen.export_functions();
        let result = &functions.get("UInt64.to_be_bytes").expect("declared primitive method").return_type;
        assert_eq!(*result, Some(TypeRef::Array { element: Box::new(TypeRef::Concrete(TypeId::U8)), length: 8 }));
    }
}
