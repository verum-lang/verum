//! T1762: real CRC32 source preserves checksum values within linear work bounds.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{ItemFailurePolicy, VbcCodegen};
use verum_vbc::interpreter::{Interpreter, InterpreterConfig};

#[test]
fn crc32_source_vectors_streaming_and_bounded_work() {
    let crc = Parser::new(include_str!("../../../core/hash/checksum/crc32.vr"))
        .parse_module()
        .expect("CRC source grammar");
    let source = format!(
        "{}\n{}",
        include_str!("../../../vcs/specs/L2-standard/database/sqlite/l2_record/crc32_vectors.vr"),
        r#"
fn large_input() {
 let mut input: List<Byte> = List.new();
 let mut i = 0;
 while i < 4096 { input.push(0_u8); i = i + 1; }
 assert_eq(crc32(&input), 0xC71C0011_u32);
}
"#
    );
    let caller = Parser::new(&source)
        .parse_module()
        .expect("CRC control grammar");
    let mut codegen = VbcCodegen::new();
    codegen
        .compile_module_with_mounts(
            &crc,
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../core/hash/checksum/crc32.vr"
            ),
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../core"),
        )
        .expect("complete production CRC source");
    codegen
        .collect_unit_declarations(&[&caller])
        .expect("control declarations");
    codegen
        .compile_unit_items(&[&caller], ItemFailurePolicy::Strict)
        .expect("complete controls");
    let module = codegen.finalize_module().expect("source VBC");
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).expect("serialize source VBC"),
    )
    .expect("decode source VBC");
    for (route, module) in [("source", module), ("wire", wire)] {
        let module = Arc::new(module);
        for name in ["main", "large_input"] {
            let entry = module.find_function_by_name(name).expect("control entry");
            let config = InterpreterConfig {
                max_instructions: 5_000_000,
                ..Default::default()
            };
            let mut interpreter =
                Interpreter::try_new_with_config(Arc::clone(&module), config).expect("interpreter");
            interpreter
                .execute_function(entry)
                .unwrap_or_else(|error| panic!("{route} {name}: {error:?}"));
        }
    }
}
