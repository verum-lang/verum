//! T1547: source impls belong to their declared scalar spelling.
#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::module::VbcModule;
use verum_vbc::types::{TypeId, TypeKind};

fn wire(module: &VbcModule) -> VbcModule {
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(module).unwrap(),
    )
    .unwrap()
}

fn compile(source: &str, imported: &[&VbcModule]) -> VbcModule {
    let ast = Parser::new(source).parse_module().unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    for module in imported {
        codegen.import_archive_module_types(module);
    }
    codegen.collect_unit_declarations(&[&ast]).unwrap();
    wire(&codegen.compile_function_bodies(&ast).unwrap())
}

fn associations(module: &VbcModule) -> Vec<(String, String)> {
    let mut result = Vec::new();
    for carrier in &module.types {
        for implementation in &carrier.protocols {
            let protocol = module.get_type(TypeId(implementation.protocol.0)).unwrap();
            result.push((
                module.strings.get(carrier.name).unwrap().to_owned(),
                module.strings.get(protocol.name).unwrap().to_owned(),
            ));
        }
    }
    result.sort();
    result
}

fn aliases(first: &str, second: &str) {
    let module = compile(
        &format!(
            r#"
        module scalar_alias_probe;
        type FirstOnly is protocol {{ fn first_marker(&self) -> Int; }};
        type SecondOnly is protocol {{ fn second_marker(&self) -> Int; }};
        implement FirstOnly for {first} {{ fn first_marker(&self) -> Int {{ 1 }} }}
        implement SecondOnly for {second} {{ fn second_marker(&self) -> Int {{ 2 }} }}
    "#
        ),
        &[],
    );
    let mut expected = vec![
        (first.to_owned(), "FirstOnly".into()),
        (second.to_owned(), "SecondOnly".into()),
    ];
    expected.sort();
    assert_eq!(associations(&module), expected);
    for name in [first, second] {
        let carrier = module
            .types
            .iter()
            .find(|ty| module.strings.get(ty.name) == Some(name))
            .unwrap();
        assert_eq!(carrier.kind, TypeKind::Primitive);
        assert_eq!(Some(carrier.id), TypeId::from_well_known_scalar_name(name));
    }
}

#[test]
fn byte_then_uint8() {
    aliases("Byte", "UInt8");
}
#[test]
fn uint8_then_byte() {
    aliases("UInt8", "Byte");
}
#[test]
fn usize_then_isize() {
    aliases("USize", "ISize");
}
#[test]
fn isize_then_usize() {
    aliases("ISize", "USize");
}
#[test]
fn distinct_bool_then_int() {
    aliases("Bool", "Int");
}
#[test]
fn distinct_int_then_bool() {
    aliases("Int", "Bool");
}

#[test]
fn nominal_bool_keeps_precedence_over_the_builtin_spelling() {
    let module = compile(
        "module local; type Bool is { tag: Int }; type LocalOnly is protocol {}; implement LocalOnly for Bool {}",
        &[],
    );
    assert_eq!(associations(&module), [("Bool".into(), "LocalOnly".into())]);
    let carrier = module
        .types
        .iter()
        .find(|ty| module.strings.get(ty.name) == Some("Bool"))
        .unwrap();
    assert_eq!(carrier.kind, TypeKind::Record);
    assert_ne!(carrier.id, TypeId::BOOL);
}

#[test]
fn imported_scalar_then_local_nominal_bool_keep_their_own_implementations() {
    let imported = compile(
        "module foreign; type ForeignOnly is protocol {}; implement ForeignOnly for Bool {}",
        &[],
    );
    let module = compile(
        "module local; type Bool is { tag: Int }; type LocalOnly is protocol {}; implement LocalOnly for Bool {}",
        &[&imported],
    );
    for (protocol, kind) in [
        ("ForeignOnly", TypeKind::Primitive),
        ("LocalOnly", TypeKind::Record),
    ] {
        let carrier = module
            .types
            .iter()
            .find(|ty| {
                ty.protocols.iter().any(|implementation| {
                    module
                        .get_type(TypeId(implementation.protocol.0))
                        .is_some_and(|p| module.strings.get(p.name) == Some(protocol))
                })
            })
            .unwrap();
        assert_eq!(carrier.kind, kind);
        assert_eq!(carrier.id == TypeId::BOOL, kind == TypeKind::Primitive);
    }
}

#[test]
fn imported_carrier_and_new_alias_keep_their_declared_targets_after_reimport() {
    for (first, second) in [
        ("Byte", "UInt8"),
        ("UInt8", "Byte"),
        ("USize", "ISize"),
        ("ISize", "USize"),
    ] {
        let imported = compile(
            &format!(
                "module foreign; type ForeignOnly is protocol {{}}; implement ForeignOnly for {first} {{}}"
            ),
            &[],
        );
        let module = compile(
            &format!(
                "module local; type LocalOnly is protocol {{}}; implement LocalOnly for {second} {{}}"
            ),
            &[&imported, &imported],
        );
        let mut expected = vec![
            (first.to_owned(), "ForeignOnly".into()),
            (second.to_owned(), "LocalOnly".into()),
        ];
        expected.sort();
        assert_eq!(associations(&module), expected);
    }
}

#[test]
fn adding_to_the_same_imported_spelling_preserves_existing_protocols() {
    let imported = compile(
        "module foreign; type ForeignOnly is protocol {}; implement ForeignOnly for Byte {}",
        &[],
    );
    let module = compile(
        "module local; type LocalOnly is protocol {}; implement LocalOnly for Byte {}",
        &[&imported, &imported],
    );
    assert_eq!(
        associations(&module),
        [
            ("Byte".into(), "ForeignOnly".into()),
            ("Byte".into(), "LocalOnly".into())
        ]
    );
}
