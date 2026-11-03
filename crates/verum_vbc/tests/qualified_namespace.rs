#![cfg(feature = "codegen")]
//! Qualified public exports retain declaration identity and visibility.
use std::{fs, sync::Arc};
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, CodegenError, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
};

fn compile(provider: &str, facade: &str, consumer: &str) -> Result<VbcModule, CodegenError> {
    let scratch = tempfile::tempdir().unwrap();
    let core = scratch.path().join("core");
    fs::create_dir_all(core.join("parent")).unwrap();
    fs::create_dir_all(core.join("prelude")).unwrap();
    fs::write(core.join("prelude/mod.vr"), "module core.prelude;").unwrap();
    fs::write(core.join("mod.vr"), "module core; public module parent;").unwrap();
    fs::write(
        core.join("parent/mod.vr"),
        format!("module core.parent; {facade}"),
    )
    .unwrap();
    fs::write(
        core.join("parent/provider.vr"),
        format!("module core.parent.provider; {provider}"),
    )
    .unwrap();
    fs::write(
        core.join("parent/other.vr"),
        "module core.parent.other; public type Context is { value: Int };",
    )
    .unwrap();
    let source = format!("module core.consumer; {consumer}");
    let path = core.join("consumer.vr");
    fs::write(&path, &source).unwrap();
    VbcCodegen::with_config(CodegenConfig::new("core.consumer").with_validation())
        .compile_module_with_mounts(
            &Parser::new(&source).parse_module().unwrap(),
            path.to_str().unwrap(),
            core.to_str().unwrap(),
        )
}

const PUBLIC: &str = "public type Context is { value: Int };";
const FACADE: &str = "public mount .provider.Context;";

fn assert_identity(mount: &str, name: &str, generic: bool, facade: &str) {
    let application = if generic {
        format!("{name}<Int>")
    } else {
        name.to_owned()
    };
    let consumer = format!("{mount}
        type Context is {{ wrong_owner: Bool }};
        type Holder is {{ inner: {application} }};
        fn main() -> Int {{ let holder = Holder {{ inner: {name} {{ value: 37 }} }}; holder.inner.value }}");
    let module = compile(
        if generic {
            "public type Context<T> is { value: T };"
        } else {
            PUBLIC
        },
        facade,
        &consumer,
    )
    .expect("qualified public declaration");
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for module in [module, wire] {
        let provider = module
            .types
            .iter()
            .find(|ty| {
                module.get_string(ty.name) == Some("Context")
                    && ty.origin_module.and_then(|id| module.get_string(id))
                        == Some("core.parent.provider")
            })
            .expect("actual declaring owner");
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
            Some(provider.id)
        );
        let entry = module.find_function_by_name("core.consumer.main").unwrap();
        let result = Interpreter::new(Arc::new(module))
            .execute_function(entry)
            .unwrap();
        assert_eq!(result.try_as_i64(), Some(37));
    }
}

#[test]
fn direct_owner_qualified_type_control() {
    assert_identity(
        "mount core.parent.provider;",
        "core.parent.provider.Context",
        false,
        FACADE,
    );
}

#[test]
fn namespace_mount_qualified_public_reexport() {
    assert_identity("mount core.parent;", "core.parent.Context", false, FACADE);
}

#[test]
fn namespace_mount_qualified_generic_public_reexport() {
    assert_identity("mount core.parent;", "core.parent.Context", true, FACADE);
}

#[test]
fn explicit_namespace_alias_retains_the_declaring_owner() {
    assert_identity("mount core.parent as api;", "api.Context", false, FACADE);
}

#[test]
fn mounted_relative_namespace_retains_the_declaring_owner() {
    assert_identity("mount core.parent;", "parent.Context", false, FACADE);
}

#[test]
fn convergent_public_reexports_are_not_ambiguous() {
    assert_identity(
        "mount core.parent;",
        "core.parent.Context",
        false,
        "public mount .provider.Context; public mount .provider.Context;",
    );
}

fn assert_refused(provider: &str, facade: &str, mount: &str, path: &str) {
    // An unrelated same-leaf record must not rescue an invalid export.
    let consumer = format!(
        "{mount} type Context is {{ value: Int }};
        type Holder is {{ inner: {path} }};"
    );
    let error = match compile(provider, facade, &consumer) {
        Err(error) => error,
        Ok(_) => panic!("invalid Context binding was accepted: {path}"),
    };
    assert!(error.to_string().contains("Context"), "{error}");
}

#[test]
fn private_facade_import_cannot_become_a_public_export() {
    assert_refused(
        PUBLIC,
        "mount .provider.Context;",
        "mount core.parent;",
        "core.parent.Context",
    );
}

#[test]
fn facade_cannot_publish_a_private_provider_declaration() {
    assert_refused(
        "type Context is { value: Int };",
        FACADE,
        "mount core.parent;",
        "core.parent.Context",
    );
}

#[test]
fn qualified_path_cannot_read_another_modules_private_type() {
    assert_refused(
        "type Context is { value: Int };",
        "",
        "mount core.parent.provider;",
        "core.parent.provider.Context",
    );
}

#[test]
fn ambiguous_public_reexports_do_not_select_one_layout() {
    assert_refused(
        PUBLIC,
        "public mount .provider.Context; public mount .other.Context;",
        "mount core.parent;",
        "core.parent.Context",
    );
}

#[test]
fn ambiguous_namespace_alias_does_not_select_one_owner() {
    assert_refused(
        PUBLIC,
        FACADE,
        "mount core.parent.provider as api; mount core.parent.other as api;",
        "api.Context",
    );
}

#[test]
fn named_type_import_does_not_introduce_an_implicit_relative_namespace() {
    assert_refused(
        PUBLIC,
        FACADE,
        "mount core.parent.provider.Context;",
        "parent.provider.Context",
    );
}

#[test]
fn missing_qualified_export_cannot_use_a_local_same_leaf_type() {
    assert_refused(PUBLIC, "", "mount core.parent;", "core.parent.Context");
}

#[test]
fn private_qualified_type_remains_visible_in_its_declaring_module() {
    compile(
        PUBLIC,
        FACADE,
        "type Hidden is { value: Int };
        type Holder is { inner: core.consumer.Hidden };
        fn read(holder: Holder) -> Int { holder.inner.value }",
    )
    .unwrap();
}

#[test]
fn private_qualified_type_remains_visible_in_an_inline_lexical_child() {
    compile(
        PUBLIC,
        FACADE,
        "type Hidden is { value: Int };
        module inner {
            type Holder is { inner: core.consumer.Hidden };
            fn read(holder: Holder) -> Int { holder.inner.value }
        }",
    )
    .unwrap();
}

#[test]
fn named_private_type_import_is_refused() {
    let result = compile(
        "type Context is { value: Int };",
        "",
        "mount core.parent.provider.Context; type Holder is { inner: Context };",
    );
    assert!(
        result.is_err(),
        "a named private type import bypassed visibility"
    );
}

#[test]
fn renamed_private_type_import_is_refused() {
    let result = compile(
        "type Context is { value: Int };",
        "",
        "mount core.parent.provider.Context as Selected; type Holder is { inner: Selected };",
    );
    assert!(
        result.is_err(),
        "renaming a private type import bypassed visibility"
    );
}

#[test]
fn headerless_inline_modules_do_not_expose_private_imports_in_either_order() {
    let provider = "module alpha { type Hidden is { value: Int }; }";
    let consumer = "module beta { mount alpha.Hidden; type Holder is { value: Hidden }; }";
    for source in [
        format!("{provider} {consumer}"),
        format!("{consumer} {provider}"),
    ] {
        let result =
            VbcCodegen::new().compile_module(&Parser::new(&source).parse_module().unwrap());
        assert!(
            result.is_err(),
            "a headerless module bypassed private declaration visibility"
        );
    }
}
