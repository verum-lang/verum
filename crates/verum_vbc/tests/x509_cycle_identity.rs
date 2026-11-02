//! Exercise the path builder's cycle predicate with the X509Certificate wire layout.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_ast::ItemKind;
use verum_fast_parser::Parser;
use verum_vbc::codegen::VbcCodegen;
use verum_vbc::interpreter::Interpreter;

#[test]
fn complete_der_controls_cycle_membership() {
    let mut ast = Parser::new("type TbsCertificate is {}; type SignatureAlg is {};")
        .parse_module()
        .unwrap();
    let certificate = Parser::new(include_str!("../../../core/security/x509/certificate.vr"))
        .parse_module()
        .unwrap();
    ast.items
        .extend(certificate.items.into_iter().filter(|item| {
            matches!(&item.kind,
        ItemKind::Type(decl) if decl.name.name == "X509Certificate")
        }));
    let path_builder = Parser::new(include_str!("../../../core/security/x509/path_building.vr"))
        .parse_module()
        .unwrap();
    ast.items.extend(
        path_builder
            .items
            .into_iter()
            .filter(|item| match &item.kind {
                ItemKind::Function(decl) => matches!(
                    decl.name.name.as_str(),
                    "bytes_eq" | "path_contains_certificate"
                ),
                ItemKind::Const(_) => true,
                _ => false,
            }),
    );
    let fixture = Parser::new(
        r#"
        fn cert(der: List<Byte>) -> X509Certificate {
            X509Certificate { tbs: TbsCertificate {}, signature_alg: SignatureAlg {},
                signature_value: [], raw_der: der, tbs_der: [] }
        }
        fn check() -> Int {
            let ee = cert([0x30_u8, 0x01_u8]);
            let issuer = cert([0x30_u8, 0x02_u8]);
            let same = cert([0x30_u8, 0x02_u8]);
            let alternate = cert([0x30_u8, 0x03_u8]);
            let prefix = cert([0x30_u8]);
            let path: List<X509Certificate> = [issuer];
            if !path_contains_certificate(&ee, &path, &ee) { return 1; }
            if !path_contains_certificate(&ee, &path, &same) { return 2; }
            if path_contains_certificate(&ee, &path, &alternate) { return 3; }
            if path_contains_certificate(&ee, &path, &prefix) { return 4; }
            0
        }
    "#,
    )
    .parse_module()
    .unwrap();
    ast.items.extend(fixture.items);
    let module = VbcCodegen::new().compile_module(&ast).unwrap();
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let module = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    let module = Arc::new(module);
    let mut vm = Interpreter::new(module.clone());
    for (name, expected) in [
        ("check", 0),
        ("MAX_PATH_LENGTH", 16),
        ("MAX_CANDIDATES_PER_LEVEL", 8),
        ("MAX_TOTAL_PATHS", 64),
    ] {
        let entry = module.find_function_by_name(name).unwrap();
        let value = vm.execute_function(entry).unwrap();
        assert_eq!(value.as_i64(), expected, "{name}");
    }
}
