//! Archive names are containers, never the authority for source ownership.
use std::{fs, path::Path, sync::Arc};
use verum_common::Text;
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
use verum_vbc::{VbcModule, archive::read_archive_from_file};
fn write(root: &Path, path: &str, source: &str) {
    let file = root.join(path);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, source).unwrap();
}
fn produce(files: &[(&str, &str)], entry_module: &str) -> Result<VbcModule, Text> {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("core");
    fs::create_dir(&root).unwrap();
    for (file, source) in files {
        write(&root, file, source);
    }
    let out = temp.path().join("runtime.vbca");
    let mut session = Session::new(CompilerOptions {
        input: root.join("mod.vr"),
        ..Default::default()
    });
    let result = CompilationPipeline::new_core(&mut session, CoreConfig::new(&root).with_output(&out))
        .compile_core()
        .map_err(|error| {
            assert!(!out.exists(), "invalid source published an archive");
            Text::from(format!("{error}\n{}", session.format_diagnostics()))
        })?;
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    Ok(read_archive_from_file(&out).unwrap().load_module(entry_module).unwrap())
}
fn execute(module: VbcModule, name: &str, expected: i64) {
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    let decoded = verum_vbc::deserialize::deserialize_module(&wire).unwrap();
    for module in [module, decoded] {
        let entry = module.find_function_by_name(name).unwrap();
        let value = verum_vbc::interpreter::Interpreter::new(Arc::new(module))
            .execute_function(entry)
            .unwrap();
        assert_eq!(value.try_as_i64(), Some(expected));
    }
}
#[test]
fn archive_with_root_name_still_supplies_external_nominal_dependencies() {
    let module = produce(
        &[
            (
                "mod.vr",
                "module core; public module model; public module consumer;",
            ),
            (
                "model/mod.vr",
                "public type Packet is { public value:Int };",
            ),
            (
                "consumer.vr",
                "mount core.model.Packet; public type Report is { public packet:Packet }; public fn answer()->Int { let item=Report { packet:Packet { value:42 } }; item.packet.value }",
            ),
        ],
        "core",
    ).unwrap();
    let packet = module
        .types
        .iter()
        .find(|t| {
            module.get_string(t.name) == Some("Packet")
                && t.origin_module.and_then(|id| module.get_string(id)) == Some("core.model")
        })
        .unwrap();
    let report = module
        .types
        .iter()
        .find(|t| {
            module.get_string(t.name) == Some("Report")
                && t.origin_module.and_then(|id| module.get_string(id)) == Some("core.consumer")
        })
        .unwrap();
    assert_eq!(report.fields[0].type_ref.base_type_id(), Some(packet.id));
    execute(module, "core.consumer.answer", 42);
}
#[test]
fn compiling_root_does_not_discard_forward_interfaces_for_later_units() {
    let files = [
        (
            "mod.vr",
            "module core; public module alpha; public module omega; public fn root_value()->Int { 3 }",
        ),
        (
            "alpha/mod.vr",
            "mount core.omega.Peer; public type Local is { public value:Int }; public fn read(peer:Peer)->Int { peer.value } public fn answer()->Int { read(Peer { value:42 }) }",
        ),
        (
            "omega/mod.vr",
            "mount core.alpha.Local; public type Peer is { public value:Int }; public fn read(local:Local)->Int { local.value } public fn answer()->Int { read(Local { value:43 }) }",
        ),
    ];
    execute(produce(&files, "core.alpha").unwrap(), "core.alpha.answer", 42);
    execute(produce(&files, "core.omega").unwrap(), "core.omega.answer", 43);
}

#[test]
fn mounted_generic_return_keeps_its_private_type_alias_through_execution() {
    let compile = |result: &str, method: &str| {
        let consumer = format!("module core.consumer; mount core.other.Issued; mount core.factory.{{issue as deliver}}; public fn answer() -> {result} {{ deliver(73).{method}() }}");
        let files = [
            ("mod.vr", "module core; public module model; public module factory; public module other; public module consumer;"),
            ("model.vr", "module core.model; public type Receipt<T> is { public value: T }; implement<T> Receipt<T> { public fn read(&self) -> T { self.value } private fn hidden(&self) -> T { self.value } }"),
            ("factory.vr", "module core.factory; mount core.model.{Receipt as Issued}; public fn issue<T>(value: T) -> Issued<T> { Issued { value } } public fn deliver(other: Int) -> Bool { false }"),
            ("other.vr", "module core.other; public type Issued<T> is { public value: Bool }; implement<T> Issued<T> { public fn read(&self) -> Bool { self.value } }"),
            ("consumer.vr", consumer.as_str()),
        ];
        produce(&files, "core")
    };
    execute(compile("Int", "read").unwrap(), "core.consumer.answer", 73);
    for (result, method, expected) in [
        ("Bool", "read", "type mismatch"),
        ("Int", "hidden", "hidden"),
        ("Int", "missing", "missing"),
    ] {
        let error = compile(result, method).expect_err("invalid receiver call was admitted");
        assert!(error.to_lowercase().contains(expected), "{error}");
        assert!(error.contains("consumer.vr"), "{error}");
    }
}
