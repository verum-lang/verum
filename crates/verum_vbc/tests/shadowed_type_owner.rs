#![cfg(feature = "codegen")]

use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::module::VbcModule;

fn source(owner: &str, variant: &str) -> VbcModule {
    let ast = Parser::new(&format!(
        "module {owner}; public type ArchiveError is {variant}(Int);"
    ))
    .parse_module()
    .unwrap();
    VbcCodegen::with_config(CodegenConfig::new(owner))
        .compile_module(&ast)
        .unwrap()
}

fn shadow(imported: &VbcModule, aliases: &[&str], owner: &str) -> VbcModule {
    let descriptor = imported
        .types
        .iter()
        .find(|ty| {
            imported
                .strings
                .get(ty.name)
                .is_some_and(|name| name.ends_with("ArchiveError"))
        })
        .unwrap();
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("core.cog"));
    for alias in aliases {
        cg.import_archive_type_with_protocol_remap_qualified(
            descriptor,
            &imported.strings,
            &Default::default(),
            Some(alias),
        );
    }
    let local = Parser::new(&format!(
        "module {owner}; public type ArchiveError is InvalidMagic(Text);"
    ))
    .parse_module()
    .unwrap();
    // Repeated declaration collection must not rebind either identity.
    cg.collect_unit_declarations(&[&local]).unwrap();
    cg.collect_unit_declarations(&[&local]).unwrap();
    let module = cg.compile_function_bodies(&local).unwrap();
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap()
}

fn variant_names(module: &VbcModule, owner: &str) -> List<(Text, Text)> {
    module
        .types
        .iter()
        .filter(|ty| ty.origin_module.and_then(|id| module.strings.get(id)) == Some(owner))
        .flat_map(|ty| {
            ty.variants.iter().map(move |variant| {
                (
                    Text::from(module.strings.get(ty.name).unwrap()),
                    Text::from(module.strings.get(variant.name).unwrap()),
                )
            })
        })
        .collect()
}

#[test]
fn source_shadow_uses_declaration_owner_for_every_alias_order_and_registry_seed() {
    for (owner, local) in [
        ("core.archive", "core.cog.archive"),
        ("core.cog.archive", "core.archive"),
    ] {
        let imported = source(owner, "Format");
        let short = owner.strip_prefix("core.").unwrap();
        for aliases in [[short, owner], [owner, short]] {
            // Each codegen creates independent RandomState registries. Before
            // the fix their iteration order selected two different names.
            for _ in 0..16 {
                let module = shadow(&imported, &aliases, local);
                assert_eq!(
                    variant_names(&module, owner).as_slice(),
                    [(
                        Text::from(format!("{owner}.ArchiveError")),
                        Text::from("Format")
                    )],
                );
                assert_eq!(
                    variant_names(&module, local).as_slice(),
                    [(Text::from("ArchiveError"), Text::from("InvalidMagic"))],
                );
            }
        }
    }
}

#[test]
fn already_qualified_imported_descriptor_keeps_one_declaring_prefix() {
    for name in ["archive.ArchiveError", "core.archive.ArchiveError"] {
        let mut imported = source("core.archive", "Format");
        let qualified = imported.strings.intern(name);
        imported
            .types
            .iter_mut()
            .find(|ty| {
                imported
                    .strings
                    .get(ty.name)
                    .is_some_and(|name| name.ends_with("ArchiveError"))
            })
            .unwrap()
            .name = qualified;
        let module = shadow(&imported, &["archive", "core.archive"], "core.cog.archive");
        assert_eq!(
            variant_names(&module, "core.archive").as_slice(),
            [(
                Text::from("core.archive.ArchiveError"),
                Text::from("Format")
            )],
        );
    }
}

#[test]
fn missing_origin_cannot_promote_an_unrelated_lookup_alias_to_declaration_owner() {
    let mut imported = source("core.archive", "Format");
    imported
        .types
        .iter_mut()
        .find(|ty| {
            imported
                .strings
                .get(ty.name)
                .is_some_and(|name| name.ends_with("ArchiveError"))
        })
        .unwrap()
        .origin_module = None;
    let module = shadow(&imported, &["unrelated.archive"], "core.cog.archive");
    let foreign = module
        .types
        .iter()
        .find(|ty| {
            ty.variants
                .iter()
                .any(|v| module.strings.get(v.name) == Some("Format"))
        })
        .unwrap();
    assert_eq!(foreign.origin_module, None);
    assert_eq!(
        module.strings.get(foreign.name),
        Some(format!("shadowed$ArchiveError${}", foreign.id.0).as_str()),
    );
}
