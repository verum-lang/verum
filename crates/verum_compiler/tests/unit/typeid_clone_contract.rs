//! The actual TypeId provider satisfies collection bounds and preserves all bits.
use super::*;
use verum_ast::{ItemKind, decl::ImplKind};

fn typeid_provider(module: &Module) -> Module {
    let mut provider = module.clone();
    provider.items.retain(|item| match &item.kind {
        ItemKind::Module(_) => true,
        ItemKind::Impl(decl) => matches!(&decl.kind, ImplKind::Protocol { protocol, for_type, .. }
            if protocol.to_string() == "Clone" && verum_ast::pretty::format_type(for_type) == "TypeId"),
        _ => item.kind.type_or_protocol_name().is_some_and(|name| matches!(name.as_str(), "Clone" | "TypeId")),
    });
    assert_eq!(provider.items.len(), 4, "owner, Clone, TypeId and its complete Clone implementation");
    provider
}

#[test]
fn actual_provider_and_collection_consumers_satisfy_clone_in_the_complete_graph() {
    let session = crate::Session::new(crate::options::CompilerOptions::default());
    let (registry, phase) = release_stdlib_graph(&session);
    let owner = "core.base.protocols";
    let module = registry.read().get_by_path(owner).unwrap().ast.clone();
    check_source_types(&mut phase.configured_checker(), owner, &module, &registry,
        |error| error.to_diagnostic_with_span(Some(|span| session.convert_span(span))))
        .unwrap_or_else(|errors| panic!("{owner}: {errors:#?}"));
    let consumer = Parser::new(r#"
        module clone_client;
        mount core.base.protocols.{TypeId, Clone};
        mount core.collections.list.List;
        fn duplicate<T: Clone>(value: &T)->T { value.clone() }
        fn one(value: &TypeId)->TypeId { duplicate(value) }
        fn many(value: &List<TypeId>)->List<TypeId> { value.clone() }
    "#).parse_module().unwrap();
    check_source_module(&mut phase.configured_checker(), "clone_client", &consumer, &registry)
        .unwrap_or_else(|errors| panic!("TypeId collection consumer: {errors:#?}"));
}

#[test]
fn actual_typeid_clone_body_preserves_all_identifier_bits_through_archive() {
    use verum_vbc::{codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen}, interpreter::Interpreter};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../core/base/protocols.vr");
    let provider = typeid_provider(&Parser::new(&std::fs::read_to_string(path).unwrap()).parse_module().unwrap());
    for (bits, high, low) in [
        (0u64, 0u64, 0u64), (73, 0, 73),
        (0x8000_0000_0000_0001, 0x8000_0000, 1),
        (u64::MAX, 0xffff_ffff, 0xffff_ffff),
    ] {
        let consumer = Parser::new(&format!(r#"
            module clone_probe;
            mount core.base.protocols.TypeId;
            fn probe()->Int {{
                let raw: UInt64 = {bits};
                let original = TypeId(raw);
                let copied = original.clone();
                if (copied.0 >> 32) == {high} && (copied.0 & 4294967295) == {low} {{73}} else {{0}}
            }}
        "#)).parse_module().unwrap();
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new("clone_probe"));
        let units: List<_> = [&provider, &consumer].into_iter().collect();
        codegen.collect_unit_declarations(&units).unwrap();
        codegen.resolve_pending_imports();
        codegen.compile_unit_items(&units, ItemFailurePolicy::Strict).unwrap();
        let module = codegen.finalize_module_from_state().unwrap();
        let mut archive = verum_vbc::archive::ArchiveBuilder::new();
        archive.add_module("clone_probe", &module, &[]).unwrap();
        let mut bytes = List::new();
        verum_vbc::archive::write_archive(&archive.finish(), &mut bytes).unwrap();
        let archive = verum_vbc::archive::read_archive(std::io::Cursor::new(bytes.as_slice())).unwrap();
        let module = archive.load_module("clone_probe").unwrap();
        let entry = module.find_function_by_name("clone_probe.probe").unwrap();
        let result = Interpreter::try_new_validated(module.into()).unwrap().execute_function(entry).unwrap();
        assert_eq!(result.try_as_i64(), Some(73), "identifier {bits}");
    }
}
