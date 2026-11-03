#![cfg(feature = "codegen")]
//! Parent facades retain public declaration identity for all mount spellings.
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, CodegenError, CodegenErrorKind, VbcCodegen},
};

fn compile(
    mount: &str,
    nested: bool,
    public: bool,
    wrong_case: bool,
) -> Result<VbcModule, CodegenError> {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    let parent = core.join("parent");
    let directory = if nested {
        parent.join("child")
    } else {
        parent.clone()
    };
    std::fs::create_dir_all(&directory).unwrap();
    // Supply the ordinary prelude namespace without adding fixture exports.
    std::fs::create_dir_all(core.join("prelude")).unwrap();
    std::fs::write(core.join("prelude/mod.vr"), "module core.prelude;").unwrap();
    std::fs::write(
        core.join("mod.vr"),
        "module core; public mount .parent.provider.Context;",
    )
    .unwrap();
    std::fs::write(
        parent.join("mod.vr"),
        format!(
            "module core.parent; {}mount .provider.Context;",
            if public { "public " } else { "" },
        ),
    )
    .unwrap();
    std::fs::write(
        parent.join(if wrong_case {
            "Provider.vr"
        } else {
            "provider.vr"
        }),
        "module core.parent.provider; public type Context is { value: Int };",
    )
    .unwrap();
    // On case-insensitive hosts, this file must not impersonate a source item.
    std::fs::write(
        parent.join("context.vr"),
        "module core.parent.context; public type Context is { wrong: Bool };",
    )
    .unwrap();
    let owner = if nested {
        "core.parent.child.consumer"
    } else {
        "core.parent.consumer"
    };
    let source = format!(
        r#"
        module {owner}; {mount}
        type Holder is {{ item: Context }};
        fn read(value: Context) -> Int {{ value.value }}
        fn main() -> Int {{ read(Context {{ value: 41 }}) }}
    "#
    );
    let entry = directory.join("consumer.vr");
    std::fs::write(&entry, &source).unwrap();
    VbcCodegen::with_config(CodegenConfig::new(owner).with_validation()).compile_module_with_mounts(
        &Parser::new(&source).parse_module().unwrap(),
        entry.to_str().unwrap(),
        core.to_str().unwrap(),
    )
}

fn assert_identity(mount: &str, nested: bool) {
    assert_module_identity(compile(mount, nested, true, false).unwrap());
}

fn assert_module_identity(module: VbcModule) {
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    for module in [module, wire] {
        let target = module
            .types
            .iter()
            .find(|ty| {
                module.get_string(ty.name) == Some("Context")
                    && ty.origin_module.and_then(|id| module.get_string(id))
                        == Some("core.parent.provider")
            })
            .unwrap();
        let holder = module
            .types
            .iter()
            .find(|ty| module.get_string(ty.name) == Some("Holder"))
            .unwrap();
        assert_eq!(
            holder.fields[0]
                .declaration_type
                .as_ref()
                .and_then(|ty| ty.base_type_id()),
            Some(target.id)
        );
        let entry = module
            .functions
            .iter()
            .find(|f| {
                module
                    .get_string(f.name)
                    .is_some_and(|name| name.ends_with(".consumer.main"))
            })
            .unwrap()
            .id;
        assert_eq!(
            verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
                .execute_function(entry)
                .unwrap()
                .try_as_i64(),
            Some(41)
        );
    }
}

#[test]
fn direct_declaring_owner_control() {
    assert_identity("mount super.provider.Context;", false);
}

#[test]
fn named_parent_facade_resolves_public_reexport() {
    assert_identity("mount super.Context;", false);
}

#[test]
fn grouped_parent_facade_resolves_public_reexport() {
    assert_identity("mount super.{Context};", false);
}

#[test]
fn multiple_ancestors_share_parent_facade_candidates() {
    assert_identity("mount super.super.Context;", true);
    assert_identity("mount super.super.{Context};", true);
}

#[test]
fn absolute_root_facade_resolves_an_exported_item() {
    assert_identity("mount core.Context;", false);
}

fn assert_refused(result: Result<VbcModule, CodegenError>) {
    let error = result.expect_err("no authorized Context declaration");
    assert!(
        matches!(error.kind, CodegenErrorKind::InvalidSourceSignature(_)),
        "{error}"
    );
    assert!(error.to_string().contains("Context"), "{error}");
}

#[test]
fn private_parent_import_cannot_be_promoted_to_a_public_export() {
    assert_refused(compile("mount super.Context;", false, false, false));
}

#[test]
fn facade_discovery_cannot_bypass_exact_source_path_case() {
    // Keep the provider outside the consumer's directory-module glob. A file
    // reached through that glob legitimately contributes its explicit module
    // header independently of the spelling used to discover a different file.
    assert_refused(compile("mount super.super.Context;", true, true, true));
}

#[test]
fn sibling_glob_preserves_an_explicit_declaration_owner() {
    // Directory discovery supplies Provider.vr; its header declares provider.
    assert_module_identity(compile("mount super.Context;", false, true, true).unwrap());
}

#[test]
fn discovery_does_not_turn_current_module_mount_into_parent_export() {
    // Dot is the declaring namespace, while super names its parent. Loading a
    // candidate facade must never authorize a different written namespace.
    assert_refused(compile("mount .Context;", false, true, false));
}
