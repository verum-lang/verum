#![cfg(feature = "codegen")]
//! One source-owner roster drives interface, dependency and executable phases.
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
};

fn ast(source: &str) -> verum_ast::Module {
    Parser::new(source).parse_module().unwrap()
}
const PROVIDER: &str =
    "public type Packet is { public value: Int }; public fn read(x: Packet) -> Int { x.value }";
const CLIENT: &str = "mount fixture.models.packet.{Packet, read}; public fn run() -> Int { read(Packet { value: 42 }) }";
fn built(files: &[(&str, &verum_ast::Module)]) -> VbcModule {
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("bundle"));
    codegen.collect_owned_unit_declarations(files).unwrap();
    codegen
        .compile_owned_unit_items(files, ItemFailurePolicy::Strict)
        .unwrap();
    codegen.finalize_module_from_state().unwrap()
}
fn assert_packet(module: &VbcModule) {
    let packet = module
        .types
        .iter()
        .find(|ty| {
            module.get_string(ty.name) == Some("Packet")
                && ty.origin_module.and_then(|id| module.get_string(id))
                    == Some("fixture.models.packet")
        })
        .unwrap();
    let read = module
        .functions
        .iter()
        .find(|f| {
            f.origin_module.and_then(|id| module.get_string(id)) == Some("fixture.models.packet")
                && module
                    .get_string(f.name)
                    .is_some_and(|n| n == "read" || n == "fixture.models.packet.read")
        })
        .unwrap();
    assert_eq!(read.params[0].type_ref.base_type_id(), Some(packet.id));
    assert_eq!(
        read.origin_module.and_then(|id| module.get_string(id)),
        Some("fixture.models.packet")
    );
}
fn assert_execution(module: VbcModule) {
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    for module in [module, wire] {
        assert_eq!(module.name, "bundle");
        assert_packet(&module);
        let id = module.find_function_by_name("fixture.client.run").unwrap();
        assert_eq!(
            verum_vbc::interpreter::Interpreter::new(Arc::new(module))
                .execute_function(id)
                .unwrap()
                .try_as_i64(),
            Some(42)
        );
    }
}
#[test]
fn headerless_files_use_one_owner_through_graph_bodies_wire_and_vm() {
    let provider = ast(PROVIDER);
    let client = ast(CLIENT);
    // Parser::new intentionally gives both files the same ID. Source ownership
    // is explicit input and must not depend on a globally unique parser ID.
    assert_eq!(provider.file_id, client.file_id);
    for reverse in [false, true] {
        let mut files = vec![
            ("fixture.models.packet", &provider),
            ("fixture.client", &client),
        ];
        if reverse {
            files.reverse();
        }
        let interface = VbcCodegen::with_config(CodegenConfig::new("bundle"))
            .collect_owned_declaration_interface(&files)
            .unwrap();
        assert_packet(&interface);
        assert_execution(built(&files));
    }
}
#[test]
fn explicit_header_overrides_file_fallback_in_every_phase() {
    let provider = ast(&format!("module fixture.models.packet; {PROVIDER}"));
    let client = ast(&format!("module fixture.client; {CLIENT}"));
    assert_execution(built(&[
        ("wrong.folder", &provider),
        ("other.file", &client),
    ]));
}
#[test]
fn public_file_reexport_preserves_declaring_identity() {
    let provider = ast(PROVIDER);
    let facade = ast("public mount fixture.models.packet.{Packet, read};");
    let client = ast(
        "mount fixture.api.{Packet, read}; public fn run() -> Int { read(Packet { value: 42 }) }",
    );
    assert_execution(built(&[
        ("fixture.models.packet", &provider),
        ("fixture.api", &facade),
        ("fixture.client", &client),
    ]));
}
#[test]
fn wrong_sibling_cannot_supply_requested_file_declaration() {
    let provider = ast(PROVIDER);
    let client =
        ast("mount fixture.models.packet.Packet; public fn keep(x: Packet) -> Packet { x }");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("bundle"));
    let error = codegen
        .collect_owned_unit_declarations(&[
            ("fixture.models.other", &provider),
            ("fixture.client", &client),
        ])
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unresolved nominal type 'Packet'"),
        "{error}"
    );
}
#[test]
fn dependency_import_uses_each_file_owner_for_relative_mounts() {
    let provider = ast(PROVIDER);
    let available = built(&[("fixture.models.packet", &provider)]);
    let client =
        ast("mount super.models.packet.{Packet, read}; public fn keep(x: Packet) -> Packet { x }");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("bundle"));
    let sources = [("fixture.client", &client)];
    codegen
        .import_owned_bootstrap_nominal_dependencies_with_resolver(
            &sources,
            &[&available],
            |name| Some(name.into()),
        )
        .unwrap();
    codegen.collect_owned_unit_declarations(&sources).unwrap();
    codegen
        .compile_owned_unit_items(&sources, ItemFailurePolicy::Strict)
        .unwrap();
    let module = codegen.finalize_module_from_state().unwrap();
    assert_eq!(module.name, "bundle");
    let packet = module
        .types
        .iter()
        .find(|t| {
            t.origin_module.and_then(|id| module.get_string(id)) == Some("fixture.models.packet")
                && module.get_string(t.name) == Some("Packet")
        })
        .unwrap();
    let keep = module
        .get_function(module.find_function_by_name("fixture.client.keep").unwrap())
        .unwrap();
    assert_eq!(keep.params[0].type_ref.base_type_id(), Some(packet.id));
}
#[test]
fn failed_owned_body_restores_the_callers_module_scope() {
    let bad = ast("public fn broken() -> Int { undefined_symbol }");
    let good = ast("public fn after() -> Int { 42 }");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("bundle"));
    codegen
        .collect_owned_unit_declarations(&[("fixture.broken", &bad)])
        .unwrap();
    codegen
        .compile_owned_unit_items(&[("fixture.broken", &bad)], ItemFailurePolicy::Strict)
        .unwrap_err();
    codegen.collect_unit_declarations(&[&good]).unwrap();
    codegen
        .compile_unit_items(&[&good], ItemFailurePolicy::Strict)
        .unwrap();
    let module = codegen.finalize_module_from_state().unwrap();
    assert_eq!(module.name, "bundle");
    assert!(module.find_function_by_name("bundle.after").is_some());
    assert!(
        module
            .find_function_by_name("fixture.broken.after")
            .is_none()
    );
}
