//! T1662: execute the production JSON source, without a stale baked JSON body.
//!
//! This pins the parser algorithm through VBC codegen, archive round-trip and
//! interpretation. The normal typed/baked import gate is the accompanying VCS
//! fixture and remains a separate acceptance boundary.
#![cfg(feature = "codegen")]

use verum_common::{List, Shared};
use verum_fast_parser::Parser;
use verum_vbc::codegen::{ItemFailurePolicy, VbcCodegen};
use verum_vbc::interpreter::Interpreter;

#[test]
fn strict_json_runtime_controls_use_the_complete_production_module() {
    let json = Parser::new(include_str!("../../../core/encoding/json.vr"))
        .parse_module()
        .expect("production JSON grammar");
    let caller = Parser::new(include_str!(
        "../../../vcs/specs/L2-standard/encoding/json_strict_publication.vr"
    ))
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
        .expect("complete production JSON source and mounted dependencies");
    codegen
        .collect_unit_declarations(&[&caller])
        .expect("complete caller declarations");
    codegen
        .compile_unit_items(&[&caller], ItemFailurePolicy::Strict)
        .expect("complete runtime control bodies");
    let mut module = codegen.finalize_module().expect("source VBC");
    module.resolve_protocol_dispatch();

    // Inspect exact selected descriptors, including nonempty bodies, before
    // executing. A forward declaration from an older archive cannot qualify.
    for expected in [
        "core.encoding.json.parse_strict",
        "core.encoding.json.parse_document",
        "core.encoding.json.parse_object",
    ] {
        let candidates: List<_> = module
            .functions
            .iter()
            .filter(|function| module.get_string(function.name) == Some(expected))
            .collect();
        assert_eq!(candidates.len(), 1, "exact source descriptor {expected}");
        let function = candidates[0];
        assert!(function.bytecode_length > 0, "source body {expected}");
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
    assert!(entry.bytecode_length > 0, "fixture main has a body");
    eprintln!(
        "fixture entry: {:?}, id={:?}",
        module.get_string(entry.name),
        entry.id
    );
    let entry_id = entry.id;
    Interpreter::new(Shared::new(module).into_arc())
        .execute_function(entry_id)
        .expect("strict/compatibility assertions must execute successfully");
}
