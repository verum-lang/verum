#![cfg(feature = "codegen")]
//! Module identity follows declared, case-sensitive paths on every filesystem.
use std::{fs, sync::Arc};
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
};

#[test]
fn member_named_like_a_file_keeps_its_facade_export() {
    assert_facade_identity("Item", "Item");
}

#[test]
fn renamed_facade_export_retains_its_declaration_identity() {
    assert_facade_identity("Item as SelectedItem", "SelectedItem");
}

fn assert_facade_identity(mount: &str, name: &str) {
    let scratch = tempfile::tempdir().unwrap();
    let core = scratch.path().join("core");
    fs::create_dir_all(core.join("model")).unwrap();
    fs::write(core.join("mod.vr"), "module core; public module model;").unwrap();
    fs::write(
        core.join("model/mod.vr"),
        "module core.model; public mount .item.Item;",
    )
    .unwrap();
    fs::write(
        core.join("model/item.vr"),
        "module core.model.item; public type Item is { value: Int };",
    )
    .unwrap();
    let source = format!(
        "module core.consumer; mount core.model.{mount};
        type Holder is {{ inner: {name} }};
        fn main() -> Int {{ let h = Holder {{ inner: {name} {{ value: 37 }} }}; h.inner.value }}"
    );
    let path = core.join("consumer.vr");
    fs::write(&path, &source).unwrap();
    let ast = Parser::new(&source).parse_module().unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new("core.consumer").with_validation())
        .compile_module_with_mounts(&ast, path.to_str().unwrap(), core.to_str().unwrap())
        .unwrap();
    let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    let item = module
        .types
        .iter()
        .find(|ty| {
            module.get_string(ty.name) == Some("Item")
                && ty.origin_module.and_then(|id| module.get_string(id)) == Some("core.model.item")
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
        Some(item.id)
    );
    let entry = module.find_function_by_name("core.consumer.main").unwrap();
    let result = Interpreter::new(Arc::new(module))
        .execute_function(entry)
        .unwrap();
    assert!(result.is_int(), "{result:?}");
    assert_eq!(result.as_i64(), 37);
}

#[test]
fn recompilation_observes_changes_to_a_glob_mount() {
    let scratch = tempfile::tempdir().unwrap();
    let core = scratch.path().join("core");
    fs::create_dir_all(core.join("model")).unwrap();
    fs::write(core.join("mod.vr"), "module core; public module model;").unwrap();
    fs::write(core.join("model/mod.vr"), "module core.model;").unwrap();
    let source = "module core.consumer; mount core.model.*; fn main() -> Int { 37 }";
    let path = core.join("consumer.vr");
    fs::write(&path, source).unwrap();
    let compile = || {
        let ast = Parser::new(source).parse_module().unwrap();
        let module = VbcCodegen::with_config(CodegenConfig::new("core.consumer").with_validation())
            .compile_module_with_mounts(&ast, path.to_str().unwrap(), core.to_str().unwrap())
            .unwrap();
        deserialize_module(&serialize_module(&module).unwrap()).unwrap()
    };
    let declares_item = |module: &verum_vbc::module::VbcModule| {
        module.types.iter().any(|ty| {
            module.get_string(ty.name) == Some("Item")
                && ty.origin_module.and_then(|id| module.get_string(id)) == Some("core.model.item")
        })
    };
    assert!(!declares_item(&compile()));
    let item_file = core.join("model/item.vr");
    fs::write(
        &item_file,
        "module core.model.item; public type Item is { value: Int };",
    )
    .unwrap();
    assert!(declares_item(&compile()));
    fs::remove_file(item_file).unwrap();
    assert!(!declares_item(&compile()));
}
