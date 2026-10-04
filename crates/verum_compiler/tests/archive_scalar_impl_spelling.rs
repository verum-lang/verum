//! T1547: source-declared scalar impl targets survive the public archive metadata API.

use verum_compiler::archive_metadata::archive_to_core_metadata;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};

fn check(first: &str, second: &str) {
    let source = format!(
        r#"
        module scalar_alias_probe;
        type FirstOnly is protocol {{}};
        type SecondOnly is protocol {{}};
        implement FirstOnly for {first} {{}}
        implement SecondOnly for {second} {{}}
    "#
    );
    let ast = Parser::new(&source).parse_module().unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new("scalar_alias_probe"))
        .compile_module(&ast)
        .unwrap();
    let module = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    let mut archive = verum_vbc::archive::ArchiveBuilder::stdlib();
    archive
        .add_module("scalar_alias_probe", &module, &[])
        .unwrap();
    let metadata = archive_to_core_metadata(&archive.finish());
    let mut actual: Vec<_> = metadata
        .implementations
        .iter()
        .map(|implementation| {
            (
                implementation.target_type.as_str().to_owned(),
                implementation.protocol.as_str().to_owned(),
            )
        })
        .collect();
    actual.sort();
    let mut expected = vec![
        (first.to_owned(), "FirstOnly".into()),
        (second.to_owned(), "SecondOnly".into()),
    ];
    expected.sort();
    assert_eq!(actual, expected);
}

#[test]
fn source_scalar_alias_associations_survive_archive_metadata_in_both_orders() {
    for (first, second) in [
        ("Byte", "UInt8"),
        ("UInt8", "Byte"),
        ("USize", "ISize"),
        ("ISize", "USize"),
    ] {
        check(first, second);
    }
}

#[test]
fn source_distinct_scalar_associations_remain_independent() {
    check("Bool", "Int");
    check("Int", "Bool");
}
