//! T1766: the production TOML bare-key classifier returns Bool for every byte.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_ast::ItemKind;
use verum_fast_parser::Parser;
use verum_vbc::Value;
use verum_vbc::codegen::{ItemFailurePolicy, VbcCodegen};
use verum_vbc::interpreter::Interpreter;

#[test]
fn every_byte_has_the_toml_bare_key_classification() {
    let mut source = Parser::new(include_str!("../../../core/configuration/toml.vr"))
        .parse_module()
        .expect("production TOML grammar");
    // This pure helper has no source dependencies. Keep its complete, unchanged
    // body and declaring module; full document parsing has a separate gate.
    source.items.retain(|item| match &item.kind {
        ItemKind::Module(_) => true,
        ItemKind::Function(function) => function.name.as_str() == "is_bare_key_char",
        _ => false,
    });
    assert_eq!(
        source
            .items
            .iter()
            .filter(|item| matches!(&item.kind, ItemKind::Function(_)))
            .count(),
        1,
    );
    let mut codegen = VbcCodegen::new();
    codegen
        .collect_unit_declarations(&[&source])
        .expect("source declarations");
    codegen
        .compile_unit_items(&[&source], ItemFailurePolicy::Strict)
        .expect("actual classifier body");
    let module = codegen.finalize_module().expect("source VBC");
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).expect("serialize"),
    )
    .expect("deserialize");
    for (route, module) in [("source", module), ("wire", wire)] {
        let entry = module
            .find_function_by_name("core.configuration.toml.is_bare_key_char")
            .expect("exact production function");
        let mut interpreter = Interpreter::new(Arc::new(module));
        for byte in u8::MIN..=u8::MAX {
            let actual = interpreter
                .execute_function_with_args(entry, &[Value::from_i64(i64::from(byte))])
                .expect("classify byte");
            assert!(actual.is_bool(), "{route}: byte {byte} returned {actual:?}");
            assert_eq!(
                actual.as_bool(),
                byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_',
                "{route}: byte {byte}",
            );
        }
    }
}
