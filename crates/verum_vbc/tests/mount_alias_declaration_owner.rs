//! T1536: archive mount aliases cannot resurrect a name reclaimed by a declaration.
#![cfg(feature = "codegen")]

use verum_common::List;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};

fn roundtrip(source: &str) -> verum_vbc::module::VbcModule {
    let ast = Parser::new(source).parse_module().expect("source syntax");
    let module = VbcCodegen::with_config(CodegenConfig::new("test"))
        .compile_module(&ast)
        .expect("source compiles");
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).expect("serialize"),
    )
    .expect("deserialize")
}

#[test]
fn a_local_definition_reclaims_a_mounted_spelling_in_the_archive() {
    for local in [
        "mount upstream.{choose}; public fn choose()->Int {37}",
        "public fn choose()->Int {37} mount upstream.{choose};",
    ] {
        let module = roundtrip(&format!(
            "module upstream {{ public fn choose()->Int {{7}} }} module local {{ {local} }}"
        ));
        assert!(
            module
                .functions
                .iter()
                .any(|function| { module.get_string(function.name) == Some("local.choose") })
        );
        assert!(
            module.mount_alias_target("local.choose").is_none(),
            "a stale alias must not override the source declaration after loading"
        );
        let names: List<_> = module
            .mount_aliases
            .iter()
            .filter_map(|(name, _, _)| module.get_string(*name))
            .collect();
        assert!(!names.contains(&"local.choose"));
    }
}

#[test]
fn a_real_reexport_retains_its_exact_declared_target() {
    let module = roundtrip(
        "module upstream { public fn choose()->Int {7} } module facade { public mount upstream.{choose}; }",
    );
    let (_, target) = module
        .mount_alias_target("facade.choose")
        .expect("real reexport");
    assert_eq!(target, "upstream.choose");
}

#[test]
fn another_leaf_does_not_remove_a_reexport() {
    let module = roundtrip(
        "module upstream { public fn choose()->Int {7} } module facade { public mount upstream.{choose}; public fn other()->Int {37} }",
    );
    let (_, target) = module
        .mount_alias_target("facade.choose")
        .expect("unshadowed reexport");
    assert_eq!(target, "upstream.choose");
}

#[test]
fn a_historical_stale_alias_cannot_override_an_exact_body_in_either_order() {
    for reverse in [false, true] {
        let mut module = roundtrip(
            "module upstream { public fn choose()->Int {7} } module local { public fn choose()->Int {37} }",
        );
        let target = module
            .functions
            .iter()
            .find(|function| module.get_string(function.name) == Some("upstream.choose"))
            .unwrap()
            .id;
        let alias = module.intern_string("local.choose");
        let canonical = module.intern_string("upstream.choose");
        // Restore the precise historical producer row; the declaration/body
        // and both nominal names were compiled from source.
        module.mount_aliases.push((alias, target, canonical));
        if reverse {
            module.functions.reverse();
        }
        assert!(module.mount_alias_shadowed_by_definition("local.choose"));
        assert!(module.mount_alias_target("local.choose").is_none());
        assert!(!module.mount_alias_shadowed_by_definition("foreign.choose"));
    }
}

#[test]
fn a_name_resolved_placeholder_does_not_block_a_genuine_reexport() {
    let mut module = roundtrip(
        "module upstream { public fn choose()->Int {7} } module facade { public mount upstream.{choose}; }",
    );
    let name = module.intern_string("facade.choose");
    module
        .functions
        .push(verum_vbc::module::FunctionDescriptor {
            id: verum_vbc::module::FunctionId(verum_vbc::stub_ranges::STAGE5_BASE),
            name,
            ..Default::default()
        });
    assert!(!module.mount_alias_shadowed_by_definition("facade.choose"));
    assert_eq!(
        module.mount_alias_target("facade.choose").unwrap().1,
        "upstream.choose"
    );
}

#[test]
fn declaration_ownership_does_not_survive_a_new_compilation() {
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("test"));
    let first = Parser::new("module facade { public fn choose()->Int {37} }")
        .parse_module()
        .unwrap();
    codegen.compile_module(&first).unwrap();
    let second = Parser::new("module upstream { public fn choose()->Int {7} } module facade { public mount upstream.{choose}; }")
        .parse_module().unwrap();
    let module = codegen.compile_module(&second).unwrap();
    assert_eq!(
        module.mount_alias_target("facade.choose").unwrap().1,
        "upstream.choose"
    );
}

#[test]
fn renamed_mount_is_reclaimed_in_its_own_declaration_scope() {
    for local in [
        "mount upstream.{choose as renamed}; public fn renamed()->Int {37}",
        "public fn renamed()->Int {37} mount upstream.{choose as renamed};",
    ] {
        let module = roundtrip(&format!(
            "module upstream {{ public fn choose()->Int {{7}} }} module local {{ {local} }}"
        ));
        assert!(module.mount_alias_target("local.renamed").is_none());
        assert!(module.mount_alias_target("renamed").is_none());
    }
}

#[test]
fn renamed_reexport_without_local_override_remains_live() {
    let module = roundtrip(
        "module upstream { public fn choose()->Int {7} } module facade { public mount upstream.{choose as renamed}; }",
    );
    assert_eq!(
        module.mount_alias_target("facade.renamed").unwrap().1,
        "upstream.choose"
    );
    assert_eq!(
        module.mount_alias_target("renamed").unwrap().1,
        "upstream.choose"
    );
}

#[test]
fn top_level_renamed_mount_is_reclaimed_with_configured_or_main_owner() {
    for owner in ["consumer", "main"] {
        for local in [
            "mount upstream.{choose as renamed}; public fn renamed()->Int {37}",
            "public fn renamed()->Int {37} mount upstream.{choose as renamed};",
        ] {
            let source = format!("module upstream {{ public fn choose()->Int {{7}} }} {local}");
            let ast = Parser::new(&source).parse_module().unwrap();
            let module = VbcCodegen::with_config(CodegenConfig::new(owner))
                .compile_module(&ast)
                .unwrap();
            assert!(
                module.mount_alias_target("renamed").is_none(),
                "owner {owner}"
            );
        }
    }
}
