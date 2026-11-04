#![cfg(feature = "codegen")]
use verum_ast::cfg::TargetConfig;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
fn codegen(target: &str) -> VbcCodegen {
    VbcCodegen::with_config(
        CodegenConfig::new("fixture").with_target(TargetConfig::for_target(target)),
    )
}
const MAC: &str = "aarch64-apple-darwin";
const WIN: &str = "x86_64-pc-windows-msvc";
const CONDITIONAL: &str = "@cfg(target_os = \"windows\") module fixture.foreign; public fn read()->UnavailablePacket { panic(\"not called\") }";
#[test]
fn inactive_file_header_prevents_signature_and_body_collection() {
    let ast = Parser::new(CONDITIONAL).parse_module().unwrap();
    for target in [MAC, "x86_64-unknown-linux-gnu"] {
        let module = codegen(target).compile_module(&ast).unwrap();
        assert!(!module.functions.iter().any(|f| {
            f.origin_module.and_then(|id| module.get_string(id)) == Some("fixture.foreign")
        }));
    }
}
#[test]
fn owned_interface_obeys_file_header_selection() {
    let ast = Parser::new(CONDITIONAL).parse_module().unwrap();
    codegen(MAC)
        .collect_owned_declaration_interface(&[("fixture", &ast)])
        .unwrap();
    let error = codegen(WIN)
        .collect_owned_declaration_interface(&[("fixture", &ast)])
        .unwrap_err();
    assert!(error.to_string().contains("UnavailablePacket"), "{error}");
}
#[test]
fn inactive_file_cannot_hide_malformed_cfg() {
    let ast = Parser::new(
        "@cfg(target_os = \"windows\") @cfg(42) module fixture.foreign; fn ok()->Int{1}",
    )
    .parse_module()
    .unwrap();
    let error = codegen(MAC).compile_module(&ast).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("invalid @cfg on module declaration"),
        "{error}"
    );
}
#[test]
fn conditional_import_is_not_a_file_header() {
    let provider =
        Parser::new("module garden.devices.common; public type PacketFixture is { n:Int };")
            .parse_module()
            .unwrap();
    let consumer=Parser::new("module garden.devices.socket; @cfg(target_os = \"windows\") mount devices.common.PacketFixture; fn read(x:PacketFixture)->PacketFixture{x}").parse_module().unwrap();
    let error = codegen(MAC)
        .collect_owned_declaration_interface(&[
            ("garden.devices", &provider),
            ("garden.devices", &consumer),
        ])
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unresolved nominal type 'PacketFixture'"),
        "{error}"
    );
    codegen(WIN)
        .collect_owned_declaration_interface(&[
            ("garden.devices", &provider),
            ("garden.devices", &consumer),
        ])
        .unwrap();
}
#[test]
fn inactive_entry_does_not_load_its_dependencies() {
    let root = std::env::temp_dir().join(format!("verum-source-cfg-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("mod.vr"), "module fixture;").unwrap();
    std::fs::write(root.join("poisoned.vr"), "this is not valid Verum").unwrap();
    let source = "@cfg(target_os = \"windows\") module fixture.entry; mount poisoned.Bad; public fn run()->Int{7}";
    let entry = root.join("entry.vr");
    std::fs::write(&entry, source).unwrap();
    let ast = Parser::new(source).parse_module().unwrap();
    let result = codegen(MAC).compile_module_with_mounts(
        &ast,
        entry.to_str().unwrap(),
        root.to_str().unwrap(),
    );
    std::fs::remove_dir_all(root).unwrap();
    result.unwrap();
}
#[test]
fn actual_windows_file_is_inactive_on_macos() {
    let core = std::env::var_os("VERUM_TEST_CFG_CORE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../core"));
    let path = core.join("sys/windows/winsock2.vr");
    let ast = Parser::new(&std::fs::read_to_string(&path).unwrap())
        .parse_module()
        .unwrap();
    let module = codegen(MAC)
        .compile_module_with_mounts(&ast, path.to_str().unwrap(), core.to_str().unwrap())
        .unwrap();
    assert!(
        !module
            .functions
            .iter()
            .any(|f| f.origin_module.and_then(|id| module.get_string(id))
                == Some("core.sys.windows.winsock2"))
    );
}

#[test]
fn actual_windows_oserror_signature_retains_its_declaring_owner() {
    use verum_ast::ItemKind;
    let core = std::env::var_os("VERUM_TEST_CFG_CORE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../core"));
    let mut provider = Parser::new(&std::fs::read_to_string(core.join("sys/common.vr")).unwrap())
        .parse_module()
        .unwrap();
    provider.items.retain(|item|matches!(&item.kind,ItemKind::Type(d) if ["OSError","FileDesc"].contains(&d.name.name.as_str())) || matches!(item.kind,ItemKind::Module(_)));
    let mut consumer =
        Parser::new(&std::fs::read_to_string(core.join("sys/windows/winsock2.vr")).unwrap())
            .parse_module()
            .unwrap();
    consumer.items.retain(|item| {
        matches!(&item.kind,ItemKind::Function(f) if f.name.name.as_str()=="wsa_error")
            || matches!(item.kind, ItemKind::Module(_) | ItemKind::Mount(_))
    });
    let module = codegen(WIN)
        .collect_owned_declaration_interface(&[
            ("core.sys", &provider),
            ("core.sys.windows", &consumer),
        ])
        .unwrap();
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    for module in [module, wire] {
        let error = module
            .types
            .iter()
            .find(|ty| {
                module.get_string(ty.name) == Some("OSError")
                    && ty.origin_module.and_then(|id| module.get_string(id))
                        == Some("core.sys.common")
            })
            .unwrap();
        let function = module
            .functions
            .iter()
            .find(|f| {
                module
                    .get_string(f.name)
                    .is_some_and(|name| name.ends_with("wsa_error"))
            })
            .unwrap();
        assert_eq!(
            function.return_type,
            verum_vbc::types::TypeRef::Concrete(error.id)
        );
    }
}
#[test]
fn malformed_module_metadata_is_refused() {
    let mut ast = Parser::new("@cfg(target_os = \"windows\") @cfg(42) module fixture.foreign;")
        .parse_module()
        .unwrap();
    let header = ast.file_header_index().unwrap();
    ast.attributes = ast.items[header].attributes.clone();
    ast.items[header].attributes.clear();
    let error = codegen(MAC)
        .compile_module(&ast)
        .err()
        .expect("malformed metadata cannot be skipped");
    assert!(
        error
            .to_string()
            .contains("invalid @cfg on module declaration"),
        "{error}"
    );
}
#[test]
fn incremental_body_entrypoints_share_file_selection() {
    let ast=Parser::new("@cfg(target_os = \"windows\") module fixture.foreign; public fn unavailable()->MissingCarrier { unavailable_callback() }").parse_module().unwrap();
    for lenient in [false, true] {
        let mut codegen = codegen(MAC);
        codegen.collect_all_declarations(&ast).unwrap();
        if lenient {
            codegen.compile_module_items_lenient(&ast).unwrap();
        } else {
            codegen.compile_module_items(&ast).unwrap();
        }
        let module = codegen.finalize_module().unwrap();
        assert!(!module.functions.iter().any(|f| {
            f.origin_module.and_then(|id| module.get_string(id)) == Some("fixture.foreign")
        }));
    }
}
#[test]
fn protocol_prepass_skips_inactive_files_and_retains_invalid_cfg_error() {
    let ast=Parser::new("@cfg(target_os = \"windows\") module fixture.foreign; public type ContextualProtocolFixture is protocol { fn value()->Int; };").parse_module().unwrap();
    let mut codegen = codegen(MAC);
    codegen.collect_protocol_definitions(&ast);
    assert!(codegen.export_protocols().is_empty());
    let invalid = Parser::new("@cfg(target_os = \"windows\") @cfg(42) module fixture.foreign;")
        .parse_module()
        .unwrap();
    codegen.collect_protocol_definitions(&invalid);
    let error = codegen
        .finalize_module()
        .err()
        .expect("void prepass must retain its failure");
    assert!(
        error
            .to_string()
            .contains("invalid @cfg on module declaration"),
        "{error}"
    );
}
