//! The X.509 and TLS error owners remain distinct across bytecode serialization.
#![cfg(feature = "codegen")]

use verum_ast::{FileId, ItemKind, Module, decl::ImplKind};
use verum_fast_parser::FastParser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

fn parse(source: &str, file: u32) -> Module {
    FastParser::new()
        .parse_module_str(source, FileId::new(file))
        .unwrap()
}

#[test]
fn transport_conversion_preserves_both_categories_and_their_payloads() {
    let mut protocols = parse(include_str!("../../../core/base/protocols.vr"), 1);
    protocols.items.retain(|item| {
        matches!(&item.kind, ItemKind::Module(decl) if decl.items.is_none())
            || item
                .kind
                .type_or_protocol_name()
                .is_some_and(|name| name.as_str() == "From")
    });
    let mut credential = parse(include_str!("../../../core/security/x509/credential.vr"), 2);
    credential.items.retain(|item| {
        matches!(&item.kind, ItemKind::Module(decl) if decl.items.is_none())
            || matches!(&item.kind, ItemKind::Type(decl) if decl.name.as_str() == "CredentialError")
    });
    let mut tls = parse(include_str!("../../../core/net/tls.vr"), 3);
    tls.items.retain(|item| match &item.kind {
        ItemKind::Module(decl) => decl.items.is_none(),
        ItemKind::Mount(_) => true,
        ItemKind::Type(decl) => matches!(decl.name.as_str(), "TlsVersion" | "LegacyTlsError"),
        ItemKind::Impl(decl) => matches!(&decl.kind, ImplKind::Protocol { protocol, for_type, .. }
            if protocol.to_string() == "From" && verum_ast::pretty::format_type(for_type) == "LegacyTlsError"),
        _ => false,
    });
    let consumer = parse(
        r#"
        module publication;
        mount core.security.x509.credential.CredentialError;
        mount core.net.tls.LegacyTlsError;
        fn check() -> Int {
            let cert = LegacyTlsError.from(CredentialError.InvalidCertificate("certificate detail"));
            let key = LegacyTlsError.from(CredentialError.InvalidKey("key detail"));
            let cert_ok = match cert {
                LegacyTlsError.InvalidCertificate(s) => s == "certificate detail",
                _ => false,
            };
            let key_ok = match key {
                LegacyTlsError.InvalidKey(s) => s == "key detail",
                _ => false,
            };
            if cert_ok && key_ok { 73 } else { 0 }
        }
    "#,
        4,
    );
    let units = [&protocols, &credential, &tls, &consumer];
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("core"));
    codegen.collect_unit_declarations(&units).unwrap();
    codegen.resolve_pending_imports();
    codegen
        .compile_unit_items(&units, ItemFailurePolicy::Strict)
        .unwrap();
    let module = codegen.finalize_module_from_state().unwrap();
    let encoded = verum_vbc::serialize::serialize_module(&module).unwrap();
    for module in [
        module,
        verum_vbc::deserialize::deserialize_module(&encoded).unwrap(),
    ] {
        verum_vbc::validate::validate_module(&module).unwrap();
        let entry = module.find_function_by_name("publication.check").unwrap();
        let result = Interpreter::new(module.into())
            .execute_function(entry)
            .unwrap();
        assert_eq!(result.try_as_i64(), Some(73));
    }
}
