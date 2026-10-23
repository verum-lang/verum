//! Tar entry views, generic conversion and the CLI archive through source/wire VBC.
//! Complete strict-reader controls run in the ordinary CLI archive_publication gate;
//! its canonical stdlib assembly supplies the production Map runtime carrier.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_common::List;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{ItemFailurePolicy, VbcCodegen};
use verum_vbc::interpreter::Interpreter;

fn run(caller: &str) {
    run_with_adapter(caller, false);
}

fn run_with_adapter(caller: &str, adapter: bool) {
    let tar = Parser::new(include_str!("../../../core/archive/tar.vr"))
        .parse_module()
        .expect("tar source grammar");
    let maybe = Parser::new(include_str!("../../../core/base/maybe.vr"))
        .parse_module()
        .expect("Maybe source grammar");
    let result = Parser::new(include_str!("../../../core/base/result.vr"))
        .parse_module()
        .expect("Result source grammar");
    let text = Parser::new(include_str!("../../../core/text/text.vr"))
        .parse_module()
        .expect("Text source grammar");
    let map = Parser::new(include_str!("../../../core/collections/map.vr"))
        .parse_module()
        .expect("Map source grammar");
    let list = Parser::new(include_str!("../../../core/collections/list.vr"))
        .parse_module()
        .expect("List source grammar");
    let memory = Parser::new(include_str!("../../../core/base/memory.vr"))
        .parse_module()
        .expect("allocator source grammar");
    let primitives = Parser::new(include_str!("../../../core/base/primitives.vr"))
        .parse_module()
        .expect("primitive source grammar");
    let archive = Parser::new(include_str!("../../../core/archive/mod.vr"))
        .parse_module()
        .expect("archive adapter source grammar");
    let caller = Parser::new(caller).parse_module().expect("fixture grammar");
    let controls: List<_> = caller
        .items
        .iter()
        .filter_map(|item| {
            let verum_ast::ItemKind::Function(function) = &item.kind else {
                return None;
            };
            function
                .attributes
                .iter()
                .any(|attribute| attribute.is_named("test"))
                .then(|| function.name.name.to_string())
        })
        .collect();
    assert!(
        !controls.is_empty(),
        "fixture must have executable controls"
    );
    let mut codegen = VbcCodegen::new();
    codegen
        .compile_module_with_mounts(
            &tar,
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/archive/tar.vr"),
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../core"),
        )
        .expect("complete production tar source");
    let mut modules = vec![&maybe, &result, &text, &map, &list, &memory, &primitives];
    if adapter {
        modules.push(&archive);
    }
    modules.push(&caller);
    codegen
        .collect_unit_declarations(&modules)
        .expect("declared controls");
    codegen
        .compile_unit_items(&modules, ItemFailurePolicy::Strict)
        .expect("complete controls");
    let module = codegen.finalize_module().expect("source VBC");
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).expect("serialize source VBC"),
    )
    .expect("decode source VBC");
    for (route, module) in [("source", module), ("wire", wire)] {
        let module = Arc::new(module);
        for control in &controls {
            eprintln!("{route}: {control}");
            let entry = module.find_function_by_name(control).expect("control");
            let mut interpreter = Interpreter::new(Arc::clone(&module));
            interpreter
                .execute_function(entry)
                .unwrap_or_else(|error| panic!("{route} {control}: {error:?}"));
        }
    }
}
#[test]
fn real_cli_gnu_long_name_binary_archive() {
    run(include_str!(
        "../../../core-tests/archive/tar/cli_source_cog_test.vr"
    ));
}
#[test]
fn borrowed_entry_metadata_and_binary_contents() {
    run(include_str!(
        "../../../core-tests/archive/tar/entry_api_test.vr"
    ));
}

#[test]
fn generic_adapter_preserves_metadata_and_refuses_unsupported_kinds() {
    run_with_adapter(
        include_str!("../../../core-tests/archive/tar/adapter_api_test.vr"),
        true,
    );
}
