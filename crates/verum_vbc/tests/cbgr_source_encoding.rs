//! The actual standard-library codec must agree with runtime ThinRef bits.
#![cfg(feature = "codegen")]
use verum_ast::ItemKind;
use verum_common::layout;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::VbcCodegen, deserialize::deserialize_module, interpreter::Interpreter,
    serialize::serialize_module, value::Value,
};

#[test]
fn actual_stdlib_epoch_codec_agrees_with_runtime_reference_fields() {
    let mut source = Parser::new(include_str!("../../../core/mem/capability.vr"))
        .parse_module()
        .expect("actual capability grammar");
    source.items.retain(|item| match &item.kind {
        ItemKind::Module(declaration) => declaration.items.is_none(),
        ItemKind::Function(function) => matches!(
            function.name.name.as_str(),
            "pack_epoch_caps" | "unpack_epoch" | "unpack_caps"
        ),
        _ => false,
    });
    assert_eq!(
        source.items.len(),
        4,
        "header plus three original functions"
    );
    let original = VbcCodegen::new().compile_module(&source).unwrap();
    let wire = deserialize_module(&serialize_module(&original).unwrap()).unwrap();
    verum_vbc::validate::validate_module(&wire).unwrap();
    let pack = wire
        .find_function_by_name("core.mem.capability.pack_epoch_caps")
        .unwrap();
    let epoch_fn = wire
        .find_function_by_name("core.mem.capability.unpack_epoch")
        .unwrap();
    let caps_fn = wire
        .find_function_by_name("core.mem.capability.unpack_caps")
        .unwrap();
    let mut vm = Interpreter::new(wire.into());
    for epoch in [0u16, 1, 0x1234, u16::MAX] {
        for caps in [0u16, 1, 3, 0x00ff] {
            let expected = (u32::from(epoch) << layout::CAPS_BITS) | u32::from(caps);
            let packed = vm
                .execute_function_with_args(
                    pack,
                    &[Value::from_i64(epoch.into()), Value::from_i64(caps.into())],
                )
                .unwrap();
            assert_eq!(packed.try_as_i64(), Some(i64::from(expected)));
            for (function, expected) in [(epoch_fn, epoch), (caps_fn, caps)] {
                let result = vm.execute_function_with_args(function, &[packed]).unwrap();
                assert_eq!(result.try_as_i64(), Some(i64::from(expected)));
            }
        }
    }
}
