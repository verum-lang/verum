#![cfg(feature = "codegen")]

use verum_ast::{Attribute, Module};
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, CodegenErrorKind, VbcCodegen},
    types::TypeRef,
    VbcModule,
};

const ROOT: &str = "
    module core;
    public module provider;
    public module prelude { public mount core.provider.Carrier; }
";
const PROVIDER: &str = "
    module core.provider;
    public type Carrier is { public id: Int };
";
const CONSUMER: &str = "
    module core.consumer;
    public type Holder is { public value: Carrier };
";

fn parse(source: &str) -> Module {
    Parser::new(source).parse_module().unwrap()
}

fn opt_out(module: &mut Module) {
    // Test the canonical module policy directly. Parsing its documented
    // @![no_implicit_prelude] spelling is tracked separately by T2127.
    module
        .attributes
        .push(Attribute::simple("no_implicit_prelude".into(), module.span));
}

fn assert_carrier_identity(module: &VbcModule) {
    for module in [
        module.clone(),
        verum_vbc::deserialize::deserialize_module(
            &verum_vbc::serialize::serialize_module(module).unwrap(),
        )
        .unwrap(),
    ] {
        let descriptor = |owner: &str, name: &str| {
            module
                .types
                .iter()
                .find(|ty| {
                    module.get_string(ty.name) == Some(name)
                        && ty.origin_module.and_then(|id| module.get_string(id)) == Some(owner)
                })
                .unwrap_or_else(|| panic!("missing declaration {owner}.{name}"))
        };
        let carrier = descriptor("core.provider", "Carrier");
        let holder = descriptor("core.consumer", "Holder");
        assert_eq!(
            holder.fields[0].declaration_type,
            Some(TypeRef::Concrete(carrier.id))
        );
    }
}

fn assert_unresolved_carrier(sources: &[(&str, &Module)]) {
    let error = VbcCodegen::with_config(CodegenConfig::new("interface"))
        .collect_owned_declaration_interface(sources)
        .err()
        .expect("Carrier must not enter this declaration scope");
    assert!(
        matches!(&error.kind, CodegenErrorKind::InvalidSourceSignature(message)
            if message.contains("unresolved nominal type 'Carrier'")),
        "{error:?}"
    );
}

#[test]
fn ordinary_mounted_compilation_applies_the_implicit_prelude() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("core");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("mod.vr"), ROOT).unwrap();
    std::fs::write(root.join("provider.vr"), PROVIDER).unwrap();
    let path = root.join("consumer.vr");
    std::fs::write(&path, CONSUMER).unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new("core.consumer").with_validation())
        .compile_module_with_mounts(
            &parse(CONSUMER),
            path.to_str().unwrap(),
            root.to_str().unwrap(),
        )
        .unwrap();
    assert_carrier_identity(&module);
}

#[test]
fn declaration_interface_applies_the_same_prelude_in_either_file_order() {
    let root = parse(ROOT);
    let provider = parse(PROVIDER);
    let consumer = parse(CONSUMER);
    for sources in [
        vec![("core", &root), ("core", &provider), ("core", &consumer)],
        vec![("core", &consumer), ("core", &provider), ("core", &root)],
    ] {
        let module = VbcCodegen::with_config(CodegenConfig::new("interface"))
            .collect_owned_declaration_interface(&sources)
            .unwrap();
        assert_carrier_identity(&module);
    }
}

#[test]
fn headerless_declaration_uses_its_parent_owned_prelude_scope() {
    let root = parse(ROOT);
    let provider = parse(PROVIDER);
    let consumer = parse("public type Holder is { public value: Carrier };");
    let module = VbcCodegen::with_config(CodegenConfig::new("interface"))
        .collect_owned_declaration_interface(&[
            ("core.consumer", &consumer),
            ("core", &provider),
            ("core", &root),
        ])
        .unwrap();
    assert_carrier_identity(&module);
}

#[test]
fn declaration_interface_preserves_ast_module_opt_out() {
    let root = parse(ROOT);
    let provider = parse(PROVIDER);
    let mut consumer = parse(CONSUMER);
    opt_out(&mut consumer);
    assert_unresolved_carrier(&[("core", &root), ("core", &provider), ("core", &consumer)]);
}

#[test]
fn explicit_mount_remains_available_under_ast_module_opt_out() {
    let root = parse(ROOT);
    let provider = parse(PROVIDER);
    let mut consumer = parse(&format!("{CONSUMER} mount core.prelude.*;"));
    opt_out(&mut consumer);
    let module = VbcCodegen::with_config(CodegenConfig::new("interface"))
        .collect_owned_declaration_interface(&[
            ("core", &root),
            ("core", &provider),
            ("core", &consumer),
        ])
        .unwrap();
    assert_carrier_identity(&module);
}

#[test]
fn implicit_prelude_does_not_become_a_public_reexport() {
    let root = parse(ROOT);
    let provider = parse(PROVIDER);
    let facade = parse("module core.facade; public type Local is { public id: Int };");
    let mut consumer = parse(&format!("{CONSUMER} mount core.facade.*;"));
    opt_out(&mut consumer);
    assert_unresolved_carrier(&[
        ("core", &root),
        ("core", &provider),
        ("core", &facade),
        ("core", &consumer),
    ]);
}
