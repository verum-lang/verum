//! T1579: lookup aliases must not choose a shadowed declaration's metadata owner.

use verum_common::List;
use verum_compiler::archive_metadata::archive_to_core_metadata;
use verum_fast_parser::Parser;
use verum_types::core_metadata::TypeDescriptorKind;
use verum_vbc::archive::ArchiveBuilder;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};

fn check(aliases: &[&str]) {
    let foreign = Parser::new("module core.archive; public type ArchiveError is Format(Int);")
        .parse_module()
        .unwrap();
    let imported = VbcCodegen::with_config(CodegenConfig::new("core.archive"))
        .compile_module(&foreign)
        .unwrap();
    let descriptor = imported
        .types
        .iter()
        .find(|ty| imported.strings.get(ty.name) == Some("ArchiveError"))
        .unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("core.cog"));
    for alias in aliases {
        codegen.import_archive_type_with_protocol_remap_qualified(
            descriptor,
            &imported.strings,
            &Default::default(),
            Some(alias),
        );
    }
    let local =
        Parser::new("module core.cog.archive; public type ArchiveError is InvalidMagic(Text);")
            .parse_module()
            .unwrap();
    codegen.collect_unit_declarations(&[&local]).unwrap();
    let module = codegen.compile_function_bodies(&local).unwrap();
    let module = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    let mut builder = ArchiveBuilder::stdlib();
    builder.add_module("core.archive", &imported, &[]).unwrap();
    builder.add_module("core.cog", &module, &[]).unwrap();
    let metadata = archive_to_core_metadata(&builder.finish());
    for (key, expected_case) in [
        ("core.archive.ArchiveError", "Format"),
        ("core.cog.archive.ArchiveError", "InvalidMagic"),
    ] {
        let descriptor = metadata.types.get(&key.into()).unwrap();
        let TypeDescriptorKind::Variant { cases } = &descriptor.kind else {
            panic!("{key} lost its sum declaration");
        };
        let actual: List<_> = cases.iter().map(|case| case.name.as_str()).collect();
        assert_eq!(actual.as_slice(), [expected_case], "{key}");
        let owner = descriptor
            .origin_module_path
            .as_ref()
            .unwrap_or(&descriptor.module_path);
        assert_eq!(format!("{owner}.ArchiveError"), key);
    }
}

#[test]
fn source_shadow_metadata_keeps_exact_sibling_owners_for_both_alias_orders() {
    for aliases in [["archive", "core.archive"], ["core.archive", "archive"]] {
        for _ in 0..16 {
            check(&aliases);
        }
    }
}
