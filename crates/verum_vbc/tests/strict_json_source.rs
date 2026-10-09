//! T1662: compile the complete production JSON source and conformance controls.
//!
//! The mount loader supplies dependency declarations, not their runtime bodies.
//! This gate establishes source codegen and archive round-trip only. Execute the
//! accompanying VCS fixtures with a coherent baked stdlib for runtime acceptance.
#![cfg(feature = "codegen")]

use verum_common::List;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{ItemFailurePolicy, VbcCodegen};

#[test]
fn strict_json_source_and_controls_compile_with_declared_dependencies() {
    compile_source_control(
        include_str!("../../../vcs/specs/L2-standard/encoding/json_strict_publication.vr"),
        &[
            "core.encoding.json.parse_strict",
            "core.encoding.json.parse_document",
            "core.encoding.json.parse_object",
        ],
    );
}

#[test]
fn legacy_json_control_compiles_with_the_same_declared_dependencies() {
    compile_source_control(
        include_str!("../../../vcs/specs/L2-standard/encoding/json_legacy_source_control.vr"),
        &[
            "core.encoding.json.parse",
            "core.encoding.json.parse_object",
        ],
    );
}

fn compile_source_control(caller: &str, selected_parsers: &[&str]) {
    let json = Parser::new(include_str!("../../../core/encoding/json.vr"))
        .parse_module()
        .expect("production JSON grammar");
    let caller = Parser::new(caller)
        .parse_module()
        .expect("runtime control grammar");

    // Keep modules and their declaration owners intact. No AST item filtering,
    // parser copy, source rewriting or native JSON replacement is involved.
    let mut codegen = VbcCodegen::new();
    codegen
        .compile_module_with_mounts(
            &json,
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/encoding/json.vr"),
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../core"),
        )
        .expect("complete production JSON source and mounted dependency declarations");
    codegen
        .collect_unit_declarations(&[&caller])
        .expect("complete caller declarations");
    codegen
        .compile_unit_items(&[&caller], ItemFailurePolicy::Strict)
        .expect("complete runtime control bodies");
    let mut module = codegen.finalize_module().expect("source VBC");
    module.resolve_protocol_dispatch();

    // Inspect exact selected source descriptors. The mount loader's one-byte
    // return placeholders cannot qualify as compiled parser implementations.
    for &expected in selected_parsers {
        let candidates: List<_> = module
            .functions
            .iter()
            .filter(|function| module.get_string(function.name) == Some(expected))
            .collect();
        assert_eq!(candidates.len(), 1, "exact source descriptor {expected}");
        let function = candidates[0];
        assert!(function.bytecode_length > 1, "source body {expected}");
        eprintln!("source parser: {expected}, id={:?}", function.id);
    }

    let bytes = verum_vbc::serialize::serialize_module(&module).expect("source wire");
    let module = verum_vbc::deserialize::deserialize_module(&bytes).expect("source wire reload");
    let entries: List<_> = module
        .functions
        .iter()
        .filter(|function| {
            module
                .get_string(function.name)
                .is_some_and(|name| name == "main" || name.ends_with(".main"))
        })
        .collect();
    assert_eq!(entries.len(), 1, "one fixture entry point");
    let entry = entries[0];
    assert!(entry.bytecode_length > 1, "fixture main has a body");
    eprintln!(
        "fixture entry: {:?}, id={:?}",
        module.get_string(entry.name),
        entry.id
    );
}
