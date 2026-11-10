//! Real directory discovery retains implicit file owners through archive execution.
use std::{fs, path::Path, sync::Arc};
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
use verum_vbc::{VbcModule, archive::read_archive_from_file};

fn write(root: &Path, file: &str, text: &str) {
    let path = root.join(file);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}
fn produce(files: &[(&str, &str)]) -> Result<VbcModule, String> {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    fs::create_dir(&core).unwrap();
    for (file, text) in files {
        write(&core, file, text);
    }
    let output = root.path().join("runtime.vbca");
    let mut session = Session::new(CompilerOptions {
        input: core.join("mod.vr"),
        ..Default::default()
    });
    let result =
        CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
            .compile_core();
    match result {
        Ok(result) => {
            assert!(result.errors.is_empty(), "{:?}", result.errors);
            Ok(read_archive_from_file(&output)
                .unwrap()
                .load_module("core")
                .unwrap())
        }
        Err(error) => {
            assert!(!output.exists());
            Err(format!("{error:#}"))
        }
    }
}
fn execute(module: VbcModule, name: &str, expected: i64) {
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    for module in [module, wire] {
        let entry = module.find_function_by_name(name).unwrap();
        assert!(!verum_vbc::stub_ranges::is_stub_id(entry.0));
        assert_eq!(
            verum_vbc::interpreter::Interpreter::new(Arc::new(module))
                .execute_function(entry)
                .unwrap()
                .try_as_i64(),
            Some(expected)
        );
    }
}
const PACKET: &str = "public type Packet is { public value: Int }; implement Packet { public fn new(value: Int) -> Packet { Packet { value } } } public fn read(x: Packet) -> Int { x.value }";
#[test]
fn headerless_files_compile_and_execute_through_public_reexport() {
    let module = produce(&[
        (
            "mod.vr",
            "module core; public module packet; public module api; public module client;",
        ),
        ("packet.vr", PACKET),
        ("api.vr", "public mount core.packet.{Packet, read};"),
        (
            "client.vr",
            "mount core.api.{Packet, read}; public fn run() -> Int { read(Packet.new(42)) }",
        ),
    ])
    .unwrap();
    let packet = module
        .types
        .iter()
        .find(|t| {
            module.get_string(t.name) == Some("Packet")
                && t.origin_module.and_then(|id| module.get_string(id)) == Some("core.packet")
        })
        .unwrap();
    let reader = module
        .get_function(module.find_function_by_name("core.packet.read").unwrap())
        .unwrap();
    assert_eq!(reader.params[0].type_ref.base_type_id(), Some(packet.id));
    execute(module, "core.client.run", 42);
}
#[test]
fn explicit_header_overrides_filename_in_registry_and_archive() {
    let module = produce(&[
        ("mod.vr", "public module provider; public module client;"),
        ("provider.vr", "module core.declared; public type Packet is { public value: Int }; public fn read(x: Packet) -> Int { x.value }"),
        ("client.vr", "mount core.declared.{Packet, read}; public fn run() -> Int { read(Packet { value: 42 }) }"),
    ]).unwrap();
    assert!(module.find_function_by_name("core.declared.read").is_some());
    assert!(module.find_function_by_name("core.provider.read").is_none());
    execute(module, "core.client.run", 42);
}
#[test]
fn bootstrap_reports_the_file_and_line_that_owns_a_type_error() {
    let error = produce(&[
        ("mod.vr", "module core; public module left; public module right;"),
        ("left.vr", "public fn works() -> Int { 1 }"),
        ("right.vr", "\n\npublic fn broken() -> Int { true }"),
    ]).unwrap_err();
    assert!(error.contains("right.vr:3:") && error.contains("E400"), "{error}");
}

#[test]
fn wrong_sibling_mount_cannot_publish_an_archive() {
    let error = produce(&[
        ("mod.vr", "module core; public module wrong; public module client;"),
        ("wrong.vr", PACKET),
        (
            "client.vr",
            "mount core.packet.Packet; public fn keep(x: Packet) -> Packet { x }",
        ),
    ])
    .unwrap_err();
    assert!(
        error.contains("module `core.packet` not found"),
        "{error}"
    );
}

#[test]
fn a_public_file_header_cannot_also_claim_a_siblings_owner() {
    // The first bodyless module declaration names this file, irrespective of
    // visibility. It must not silently overwrite packet.vr's declarations.
    let error = produce(&[
        ("mod.vr", "public module packet; public module client;"),
        ("packet.vr", PACKET),
        ("client.vr", "public fn run() -> Int { 42 }"),
    ])
    .unwrap_err();
    assert!(error.contains("conflicting source parts for owner core.packet"), "{error}");
}
#[test]
fn export_index_agrees_with_explicit_and_implicit_file_owners() {
    use verum_compiler::core_compiler::build_export_index;
    let parse = |s| verum_fast_parser::Parser::new(s).parse_module().unwrap();
    let source = vec![(
        "fixture".into(),
        vec![
            ("mod.vr".into(), parse("public type Root is Int;")),
            ("packet.vr".into(), parse("public type Packet is Int;")),
            (
                "wrong.vr".into(),
                parse("module fixture.actual; public type Actual is Int;"),
            ),
        ],
    )];
    let index = build_export_index(&source);
    assert!(index["fixture"].contains("Root"));
    assert!(index["fixture.packet"].contains("Packet"));
    assert!(index["fixture.actual"].contains("Actual"));
    assert!(!index.contains_key("fixture.wrong"));
}

#[test]
fn resolved_file_mount_alias_is_already_a_complete_owner() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    fs::create_dir(&core).unwrap();
    write(&core, "mod.vr", "mount ./_support/helper.vr as Util;");
    write(
        &core,
        "_support/helper.vr",
        "public type Token is { value: Int }; public fn answer() -> Int { 42 }",
    );
    let output = root.path().join("runtime.vbca");
    let mut session = Session::new(CompilerOptions {
        input: core.join("mod.vr"),
        ..Default::default()
    });
    let result =
        CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
            .compile_core()
            .unwrap();
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let module = read_archive_from_file(&output)
        .unwrap()
        .load_module("Util")
        .unwrap();
    assert!(
        module
            .types
            .iter()
            .any(|t| module.get_string(t.name) == Some("Token")
                && t.origin_module.and_then(|id| module.get_string(id)) == Some("Util"))
    );
    assert!(module.find_function_by_name("Util.helper.answer").is_none());
    execute(module, "Util.answer", 42);
}

#[test]
fn file_mount_cannot_escape_the_declared_cog_root() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    fs::create_dir(&core).unwrap();
    write(&core, "mod.vr", "mount ../helper.vr as Util;");
    write(root.path(), "helper.vr", "public fn answer() -> Int { 42 }");
    let output = root.path().join("runtime.vbca");
    let mut session = Session::new(CompilerOptions {
        input: core.join("mod.vr"),
        ..Default::default()
    });
    let error =
        CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
            .compile_core()
            .unwrap_err();
    assert!(
        format!("{error:#}").contains("resolves outside the cog root"),
        "{error:#}"
    );
    assert!(!output.exists());
}
