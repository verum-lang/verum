//! T2835: a glob selects a source module, not its fallback file's directory.
#![cfg(feature = "codegen")]

use std::{fs, path::Path, sync::Arc};
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, CodegenError, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
};

fn write(root: &Path, path: &str, source: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, source).unwrap();
}

fn compile(root: &Path, mount: &str, implicit: bool) -> Result<VbcModule, CodegenError> {
    let source = format!(r#"
        {} module core.entry;
        {mount}
        type Holder is {{ token:Token }};
        public fn probe()->Int {{ let value = Holder {{ token:Token {{ value:41 }} }}; value.token.value }}
    "#, if implicit { "" } else { "@no_implicit_prelude" });
    let path = root.join("entry.vr");
    fs::write(&path, &source).unwrap();
    VbcCodegen::with_config(CodegenConfig::new("core.entry").with_validation())
        .compile_module_with_mounts(&Parser::new(&source).parse_module().unwrap(),
            path.to_str().unwrap(), root.to_str().unwrap())
}

fn assert_source_and_wire(module: VbcModule, owner: &str) {
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for module in [module, wire] {
        let token = module.types.iter().find(|ty|
            module.get_string(ty.name) == Some("Token")
                && ty.origin_module.and_then(|id| module.get_string(id)) == Some(owner))
            .expect("selected canonical declaration");
        let holder = module.types.iter().find(|ty| module.get_string(ty.name) == Some("Holder")).unwrap();
        assert_eq!(holder.fields[0].declaration_type.as_ref().and_then(|ty| ty.base_type_id()), Some(token.id));
        let entry = module.find_function_by_name("core.entry.probe").unwrap();
        let result = Interpreter::try_new_validated(Arc::new(module)).unwrap().execute_function(entry).unwrap();
        assert_eq!(result.try_as_i64(), Some(41));
    }
}

#[test]
fn implicit_inline_prelude_does_not_read_unrelated_source() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    write(root, "mod.vr", "module core; public module prelude { public mount core.token.Token; }");
    write(root, "token.vr", "module core.token; public type Token is { public value:Int };");
    write(root, "unrelated/poison.vr", "module core.unrelated.poison; type Broken is {");
    assert_source_and_wire(compile(root, "", true).unwrap(), "core.token");
}

#[test]
fn nested_inline_globs_use_the_actual_source_owner_and_remaining_path() {
    for (path, source, mount) in [
        ("mod.vr", "module core; public module facade { public module deep { public module selected { public mount core.token.Token; } } }", "mount core.facade.deep.selected.*;"),
        ("facade.vr", "module core.facade; public module deep { public module selected { public mount core.token.Token; } }", "mount core.facade.deep.selected.*;"),
        ("facade.vr", "public module deep { public module selected { public mount core.token.Token; } }", "mount core.facade.deep.selected.*;"),
    ] {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        write(root, "mod.vr", "module core;");
        write(root, path, source);
        write(root, "token.vr", "module core.token; public type Token is { public value:Int };");
        write(root, "unrelated/poison.vr", "module core.unrelated.poison; type Broken is {");
        assert_source_and_wire(compile(root, mount, false)
            .unwrap_or_else(|error| panic!("{path}: {source}: {error}")), "core.token");
    }
}

#[test]
fn directory_glob_reads_the_selected_directory_and_reports_its_poison() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    write(root, "mod.vr", "module core; public module selected;");
    write(root, "selected/mod.vr", "module core.selected; public mount .token.Token;");
    write(root, "selected/token.vr", "module core.selected.token; public type Token is { public value:Int };");
    write(root, "unrelated/poison.vr", "module core.unrelated.poison; type Broken is {");
    assert_source_and_wire(compile(root, "mount core.selected.*;", false).unwrap(), "core.selected.token");
    write(root, "selected/poison.vr", "module core.selected.poison; type Broken is {");
    let error = compile(root, "mount core.selected.*;", false).expect_err("selected source was not inspected");
    assert!(error.to_string().contains("selected/poison.vr"), "{error}");
}

#[test]
fn inline_glob_cannot_bypass_private_missing_or_wrong_case_targets() {
    for (visibility, provider, mount) in [
        ("public", "type Token is { public value:Int };", "mount core.facade.selected.*;"),
        ("private", "public type Token is { public value:Int };", "mount core.facade.selected.*;"),
        ("public", "public type Token is { public value:Int };", "mount core.facade.Selected.*;"),
        ("public", "public type Token is { public value:Int };", "mount core.facade.absent.*;"),
    ] {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        write(root, "mod.vr", "module core;");
        write(root, "facade.vr", &format!("module core.facade; {visibility} module selected {{ public mount core.token.Token; }}"));
        write(root, "token.vr", &format!("module core.token; {provider}"));
        let error = compile(root, mount, false).expect_err("unavailable target acquired declaration authority");
        assert!(error.to_string().contains("Token") || error.to_string().contains("selected"), "{error}");
    }
}

#[test]
fn inline_glob_cycles_are_bounded_without_dropping_reachable_exports() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    write(root, "mod.vr", "module core; public module first { public mount core.second.*; } public module second { public mount core.first.*; public mount core.token.Token; }");
    write(root, "token.vr", "module core.token; public type Token is { public value:Int };");
    assert_source_and_wire(compile(root, "mount core.first.*;", false).unwrap(), "core.token");
}

#[test]
fn repeated_selected_source_does_not_fall_through_to_an_unrelated_parent() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    write(root, "mod.vr", "module core;");
    write(root, "tree/leaf.vr", "module core.tree.leaf; public type Token is { public value:Int };");
    write(root, "tree.vr", "module core.tree; type Broken is {");
    assert_source_and_wire(compile(root,
        "mount core.tree.leaf.{Token, Token as Other};", false).unwrap(), "core.tree.leaf");
    let error = compile(root, "mount core.tree.Token;", false).expect_err("selected parent was ignored");
    assert!(error.to_string().contains("tree.vr"), "{error}");
}

#[test]
fn relative_inline_export_uses_the_declaring_module_instead_of_the_file_directory() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    write(root, "mod.vr", "module core;");
    write(root, "facade.vr", "module core.facade; public module selected { public mount super.token.Token; }");
    write(root, "facade/token.vr", "module core.facade.token; public type Token is { public value:Int };");
    write(root, "token.vr", "module core.token; public type Token is { public value:Bool };");
    assert_source_and_wire(compile(root, "mount core.facade.selected.*;", false).unwrap(), "core.facade.token");
}
