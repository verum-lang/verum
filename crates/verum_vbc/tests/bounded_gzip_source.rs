//! T1740: execute the production gzip decoder with its source CRC implementation.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{ItemFailurePolicy, VbcCodegen};
use verum_vbc::interpreter::Interpreter;

fn run(caller: &str) {
    run_with_dispatch(caller, false);
}

fn run_with_dispatch(caller: &str, dispatch: bool) {
    let gzip = Parser::new(include_str!("../../../core/compress/mod_gzip.vr"))
        .parse_module()
        .expect("gzip source grammar");
    let crc = Parser::new(include_str!("../../../core/hash/checksum/crc32.vr"))
        .parse_module()
        .expect("CRC source grammar");
    let compress = Parser::new(include_str!("../../../core/compress/mod.vr"))
        .parse_module()
        .expect("compression dispatch source grammar");
    let maybe = Parser::new(include_str!("../../../core/base/maybe.vr"))
        .parse_module()
        .expect("Maybe source grammar");
    let result = Parser::new(include_str!("../../../core/base/result.vr"))
        .parse_module()
        .expect("Result source grammar");
    let caller = Parser::new(caller).parse_module().expect("fixture grammar");
    let mut codegen = VbcCodegen::new();
    codegen
        .compile_module_with_mounts(
            &gzip,
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../core/compress/mod_gzip.vr"
            ),
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../core"),
        )
        .expect("complete production gzip source");
    let mut modules = vec![&maybe, &result, &crc];
    if dispatch {
        modules.push(&compress);
    }
    modules.push(&caller);
    codegen
        .collect_unit_declarations(&modules)
        .expect("declared runtime dependencies");
    codegen
        .compile_unit_items(&modules, ItemFailurePolicy::Strict)
        .expect("production CRC and controls");
    let module = codegen.finalize_module().expect("source VBC");
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).expect("serialize source VBC"),
    )
    .expect("decode source VBC");
    for (route, module) in [("source", module), ("wire", wire)] {
        let entry = module.find_function_by_name("main").expect("fixture main");
        let mut interpreter = Interpreter::new(Arc::new(module));
        interpreter
            .execute_function(entry)
            .unwrap_or_else(|error| panic!("{route}: {error:?}"));
    }
}

#[test]
fn raw_deflate_valid_malformed_and_output_bounds() {
    run(include_str!(
        "../../../vcs/specs/L2-standard/compress/raw_decode.vr"
    ));
}
#[test]
fn gzip_framing_crc_and_member_bounds() {
    run(include_str!(
        "../../../vcs/specs/L2-standard/compress/gzip_decode.vr"
    ));
}
#[test]
fn single_gzip_requires_complete_input() {
    run(include_str!(
        "../../../vcs/specs/L2-standard/compress/single_decode.vr"
    ));
}

#[test]
fn public_algorithm_dispatch_uses_bounded_decoder() {
    run_with_dispatch(
        include_str!("../../../vcs/specs/L2-standard/compress/dispatch_decode.vr"),
        true,
    );
}

#[test]
fn real_cli_source_cog_decodes_to_exact_tar_bytes() {
    // Captured from the actual source-cog publisher. The independently decoded
    // expected tar includes its GNU long-name extension and binary files.
    let compressed =
        include_bytes!("../../../core-tests/archive/tar/fixtures/cli-source-cog/archive.tar.gz");
    let expected =
        include_bytes!("../../../core-tests/archive/tar/fixtures/cli-source-cog/archive.tar");
    let bytes = |data: &[u8]| {
        data.iter()
            .map(|byte| format!("{byte}_u8"))
            .collect::<Vec<_>>()
            .join(",")
    };
    run(&format!(
        r#"
mount core.compress.mod_gzip.Gzip;
fn main() {{
    let compressed: List<Byte> = [{}];
    let expected: List<Byte> = [{}];
    let mut decoded: List<Byte> = List.new();
    let count = Gzip.decode_single(&compressed, expected.len(), &mut decoded).unwrap();
    assert(count == expected.len());
    assert(decoded.len() == expected.len());
    let mut i = 0;
    while i < expected.len() {{ assert(decoded[i] == expected[i]); i = i + 1; }}
}}
"#,
        bytes(compressed),
        bytes(expected),
    ));
}
