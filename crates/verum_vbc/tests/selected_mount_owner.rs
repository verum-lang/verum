#![cfg(feature = "codegen")]
//! A selected import owner must survive same-named descendants and wire transport.
use std::{fs, sync::Arc};
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, CodegenError, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
    types::TypeRef,
};

fn compile(
    provider: &str,
    import: &str,
    facade: &str,
    visible: bool,
    value: &str,
) -> Result<VbcModule, CodegenError> {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("core");
    fs::create_dir_all(root.join("parent")).unwrap();
    fs::create_dir_all(root.join("prelude")).unwrap();
    fs::write(root.join("prelude/mod.vr"), "module core.prelude;").unwrap();
    fs::write(root.join("mod.vr"), "module core; public module parent;").unwrap();
    fs::write(root.join("parent/mod.vr"), format!("module core.parent; public module {provider}; public module consumer; public module facade;")).unwrap();
    fs::write(
        root.join(format!("parent/{provider}.vr")),
        format!(
            "module core.parent.{provider}; {} type Token is {{ value: Int }};",
            if visible { "public" } else { "" }
        ),
    )
    .unwrap();
    fs::write(
        root.join("parent/facade.vr"),
        format!("module core.parent.facade; {facade}"),
    )
    .unwrap();
    let source = format!(
        "module core.parent.consumer; {import} type Holder is {{ token: {value} }}; fn main() -> Int {{ let holder = Holder {{ token: {value} {{ value: 41 }} }}; holder.token.value }}"
    );
    let path = root.join("parent/consumer.vr");
    fs::write(&path, &source).unwrap();
    VbcCodegen::with_config(CodegenConfig::new("core.parent.consumer").with_validation())
        .compile_module_with_mounts(
            &Parser::new(&source).parse_module().unwrap(),
            path.to_str().unwrap(),
            root.to_str().unwrap(),
        )
}
fn check(module: VbcModule, provider: &str) {
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for module in [module, wire] {
        let token = module
            .types
            .iter()
            .find(|t| {
                module.get_string(t.name) == Some("Token")
                    && t.origin_module.and_then(|id| module.get_string(id)) == Some(provider)
            })
            .unwrap();
        let holder = module
            .types
            .iter()
            .find(|t| module.get_string(t.name) == Some("Holder"))
            .unwrap();
        assert_eq!(
            holder.fields[0].declaration_type,
            Some(TypeRef::Concrete(token.id))
        );
        let main = module
            .find_function_by_name("core.parent.consumer.main")
            .unwrap();
        assert_eq!(
            Interpreter::new(Arc::new(module))
                .execute_function(main)
                .unwrap()
                .try_as_i64(),
            Some(41)
        );
    }
}
#[test]
fn explicit_import_owner_is_not_reanchored() {
    for provider in ["codec", "core"] {
        for written in [
            format!("core.parent.{provider}.Token"),
            format!("super.{provider}.Token"),
            format!("cog.parent.{provider}.Token"),
        ] {
            check(
                compile(provider, &format!("mount {written};"), "", true, "Token")
                    .unwrap_or_else(|e| panic!("{written}: {e}")),
                &format!("core.parent.{provider}"),
            );
        }
    }
}
#[test]
fn selected_namespace_alias_is_not_reanchored() {
    check(
        compile(
            "core",
            "mount super.core as selected;",
            "",
            true,
            "selected.Token",
        )
        .unwrap(),
        "core.parent.core",
    );
}
#[test]
fn selected_glob_owner_is_not_reanchored() {
    check(
        compile("core", "mount super.core.*;", "", true, "Token").unwrap(),
        "core.parent.core",
    );
}
#[test]
fn selected_reexport_owner_is_not_reanchored() {
    for facade in [
        "public mount super.core.Token;",
        "public mount super.core.*;",
    ] {
        check(
            compile("core", "mount super.facade.Token;", facade, true, "Token").unwrap(),
            "core.parent.core",
        );
    }
}
#[test]
fn private_selected_owner_still_refuses() {
    for import in [
        "mount super.core.Token;",
        "mount cog.parent.core.Token;",
        "mount super.core.*;",
    ] {
        assert!(
            compile("core", import, "", false, "Token").is_err(),
            "{import}"
        );
    }
}
#[test]
fn missing_selected_owner_still_refuses() {
    assert!(
        compile(
            "core",
            "mount super.core.Token; mount super.absent.Token;",
            "",
            true,
            "Token"
        )
        .is_err()
    );
}

#[test]
fn nearest_relative_namespace_is_selected_before_its_member() {
    for reversed in [true, false] {
        for has_member in [true, false] {
            let local = if has_member {
                "public type Token is { local: Bool };"
            } else {
                ""
            };
            let namespace = format!("module provider {{ {local} }}");
            let declaration = "mount provider.Token; type Holder is { token: Token };";
            let consumer = if reversed {
                format!("{declaration} {namespace}")
            } else {
                format!("{namespace} {declaration}")
            };
            let source = format!(
                "module app {{ module provider {{ public type Token is {{ outer: Int }}; }} module consumer {{ {consumer} }} }}"
            );
            let result =
                VbcCodegen::new().compile_module(&Parser::new(&source).parse_module().unwrap());
            if !has_member {
                assert!(
                    result.is_err(),
                    "an empty local namespace must block an outer member"
                );
                continue;
            }
            let module = result.unwrap();
            let token = module
                .types
                .iter()
                .find(|ty| {
                    module.get_string(ty.name) == Some("Token")
                        && ty.origin_module.and_then(|id| module.get_string(id))
                            == Some("app.consumer.provider")
                })
                .unwrap();
            let holder = module
                .types
                .iter()
                .find(|ty| module.get_string(ty.name) == Some("Holder"))
                .unwrap();
            assert_eq!(
                holder.fields[0].declaration_type,
                Some(TypeRef::Concrete(token.id))
            );
        }
    }
}

#[test]
fn single_head_glob_selects_the_lexical_namespace() {
    let source = r#"
        module app {
            module provider { public type Token is { value: Int }; }
            module consumer {
                mount provider.*;
                type Holder is { token: Token };
            }
        }
    "#;
    let module = VbcCodegen::new()
        .compile_module(&Parser::new(source).parse_module().unwrap())
        .unwrap();
    let holder = module
        .types
        .iter()
        .find(|ty| module.get_string(ty.name) == Some("Holder"))
        .unwrap();
    let token = module
        .types
        .iter()
        .find(|ty| {
            module.get_string(ty.name) == Some("Token")
                && ty.origin_module.and_then(|id| module.get_string(id)) == Some("app.provider")
        })
        .unwrap();
    assert_eq!(
        holder.fields[0].declaration_type,
        Some(TypeRef::Concrete(token.id))
    );
}
