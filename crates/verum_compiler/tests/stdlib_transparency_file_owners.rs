use std::{fs, path::Path, sync::Arc};
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
use verum_vbc::archive::read_archive_from_file;
fn write(root: &Path, file: &str, text: &str) {
    let p = root.join(file);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}
fn check(record_first: bool, tuple: bool) {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    fs::create_dir(&core).unwrap();
    write(
        &core,
        "mod.vr",
        "module core; public module cells; public module consumer;",
    );
    write(
        &core,
        "cells/mod.vr",
        "module core.cells; public module alpha; public module beta;",
    );
    let (opaque, transparent) = if record_first {
        ("alpha", "beta")
    } else {
        ("beta", "alpha")
    };
    let opaque_decl = if tuple {
        "public type Cell is (Int, Int);"
    } else {
        "public type Cell is { public value:Int, public spare:Int };"
    };
    write(&core, &format!("cells/{opaque}.vr"), opaque_decl);
    write(
        &core,
        &format!("cells/{transparent}.vr"),
        "public type Cell is (Int);",
    );
    let expr = if tuple {
        "let item=Cell(42, 5); item.0"
    } else {
        "let item=Cell { value:42, spare:5 }; item.value"
    };
    write(
        &core,
        "consumer/mod.vr",
        &format!("mount core.cells.{opaque}.Cell; public fn probe()->Int {{ {expr} }}"),
    );
    let output = root.path().join("runtime.vbca");
    let mut session = Session::new(CompilerOptions {
        input: core.join("mod.vr"),
        ..Default::default()
    });
    let config = CoreConfig::new(&core).with_output(&output);
    let result = CompilationPipeline::new_core(&mut session, config)
        .compile_core()
        .unwrap();
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let archive = read_archive_from_file(&output).unwrap();
    let module = archive.load_module("core.consumer").unwrap();
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    for (wired, module) in [(false, module), (true, wire)] {
        let entry = module.find_function_by_name("core.consumer.probe").unwrap();
        let descriptor = module.get_function(entry).unwrap();
        let body = &module.bytecode[descriptor.bytecode_offset as usize
            ..(descriptor.bytecode_offset + descriptor.bytecode_length) as usize];
        println!(
            "record_first={record_first}, tuple={tuple}, wire={wired}, bytecode={:?}",
            verum_vbc::bytecode::decode_instructions(body).unwrap()
        );
        let result =
            verum_vbc::interpreter::Interpreter::new(Arc::new(module)).execute_function(entry);
        assert_eq!(
            result.as_ref().ok().and_then(|v| v.try_as_i64()),
            Some(42),
            "record_first={record_first}, tuple={tuple}, wire={wired}: {result:?}"
        );
    }
}
#[test]
fn record_before_newtype() {
    check(true, false)
}
#[test]
fn newtype_before_record() {
    check(false, false)
}
#[test]
fn tuple_before_newtype() {
    check(true, true)
}
#[test]
fn newtype_before_tuple() {
    check(false, true)
}
