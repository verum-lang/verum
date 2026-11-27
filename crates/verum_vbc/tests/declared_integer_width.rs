//! Fixed-width arithmetic through source declarations and their inherent methods.
#![cfg(feature = "codegen")]

use verum_ast::decl::{ImplItemKind, ImplKind};
use verum_ast::{ItemKind, ty::TypeKind};
use verum_fast_parser::Parser;
use verum_vbc::codegen::{ItemFailurePolicy, VbcCodegen};
use verum_vbc::deserialize::deserialize_module;
use verum_vbc::interpreter::Interpreter;
use verum_vbc::serialize::serialize_module;

#[test]
fn primitive_method_owners_preserve_arithmetic_width_after_wire_roundtrip() {
    let mut primitives = Parser::new(include_str!("../../../core/base/primitives.vr"))
        .parse_module()
        .unwrap();
    primitives.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Module(_) => true,
        ItemKind::Impl(implementation) => {
            let ImplKind::Inherent(ty) = &implementation.kind else {
                return false;
            };
            let owner = match &ty.kind {
                TypeKind::Path(path) => path.last_segment_name(),
                kind => kind.primitive_name().unwrap_or(""),
            };
            if !matches!(owner, "UInt32" | "Int32" | "UInt64") {
                return false;
            }
            implementation.items.retain(
                |item| matches!(&item.kind, ImplItemKind::Function(f) if f.name.as_str() == "min"),
            );
            !implementation.items.is_empty()
        }
        _ => false,
    });
    assert_eq!(
        primitives
            .items
            .iter()
            .filter(|i| matches!(i.kind, ItemKind::Impl(_)))
            .count(),
        3
    );
    let callers = Parser::new(
        r#"
        module width_client;
        fn shift(x: UInt32, count: Int) -> UInt32 { x << count }
        fn probe() -> Int {
            let values: [UInt32; 1] = [0x510e527f_u32];
            let word = values[0];
            if shift(word, 25) as UInt64 != 4261412864_u64 { return 1; }
            let rotated = (word >> 7) | (word << 25);
            if rotated as UInt64 != 4272037028_u64 { return 2; }
            let max = 4294967295_u32;
            if (max + 1_u32 + 1_u32) as UInt64 != 1_u64 { return 3; }
            let signed = 2147483647_i32;
            if (signed + 1_i32) as Int != -2147483648 { return 4; }
            let wide = 4294967296_u64;
            if wide + 1_u64 != 4294967297_u64 { return 5; }
            0
        }
    "#,
    )
    .parse_module()
    .unwrap();
    let units = [&primitives, &callers];
    let mut generator = VbcCodegen::new();
    generator.initialize();
    generator.collect_unit_declarations(&units).unwrap();
    generator.resolve_pending_imports();
    generator
        .compile_unit_items(&units, ItemFailurePolicy::Strict)
        .unwrap();
    let module = generator.finalize_module().unwrap();
    let decoded = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for (route, module) in [("source", module), ("serialized", decoded)] {
        let entry = module
            .functions
            .iter()
            .find(|f| module.get_string(f.name) == Some("width_client.probe"))
            .unwrap()
            .id;
        let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
        assert_eq!(vm.execute_function(entry).unwrap().as_i64(), 0, "{route}");
    }
}
