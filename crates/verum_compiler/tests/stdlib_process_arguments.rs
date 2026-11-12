//! The OS argument table is raw process-lifetime storage, not a managed loan.
#![cfg(target_os = "macos")]

use std::{ffi::CString, fs, path::Path, sync::Arc};
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
use verum_fast_parser::Parser;
use verum_vbc::{archive::read_archive_from_file, interpreter::Interpreter, value::Value};

fn source(path: &str) -> verum_common::Text {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path),
    )
    .unwrap()
    .into()
}

fn between<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let first = source.find(start).unwrap();
    let last = source[first..].find(end).unwrap() + first;
    &source[first..last]
}

#[test]
fn os_initialization_and_env_facade_share_the_actual_argument_table() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    let common = source("core/sys/common.vr");
    let common = between(
        &common,
        "/// Global storage for command line arguments count.",
        "/// Get a command line argument by index.",
    );
    let env = source("core/base/env.vr");
    let env = between(
        &env,
        "/// Initialize process arguments. Called by V-LLSI _start",
        "// Command Line Arguments",
    );
    let darwin = source("core/sys/darwin/libsystem.vr");
    let binding = between(&darwin, "    fn _NSGetEnviron()", ";");
    let accessor = between(
        &darwin,
        "public fn process_environment()",
        "/// Get an environment variable,",
    );
    for (path, contents) in [
        (
            "mod.vr",
            "module core; public module sys; public module base;".to_owned(),
        ),
        (
            "sys/mod.vr",
            "module core.sys; public module common; public module darwin;".to_owned(),
        ),
        ("sys/common.vr", format!("module core.sys.common; {common}")),
        (
            "sys/darwin/mod.vr",
            "module core.sys.darwin; public module libsystem;".to_owned(),
        ),
        (
            "sys/darwin/libsystem.vr",
            format!(
                "module core.sys.darwin.libsystem; @ffi(\"libSystem.B.dylib\") extern {{ {binding}; }} {accessor}"
            ),
        ),
        (
            "base/mod.vr",
            "module core.base; public module env;".to_owned(),
        ),
        (
            "base/env.vr",
            format!(
                "module core.base.env; mount core.sys.common.{{get_argc, get_argv, get_environ, init_process_args as initialize_os_args}}; {env}"
            ),
        ),
    ] {
        let path = core.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
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
    let archive = read_archive_from_file(&output).unwrap();
    let app = Parser::new(
        r#"
        module app;
        mount core.base.env.init_process_args;
        mount core.sys.common.{get_argc, get_argv};
        fn entry(argc: Int, argv: *const *const Byte) -> Int {
            unsafe {
                init_process_args(argc, argv);
                let stored = get_argv();
                let argument = *stored.offset(1);
                get_argc() + (*argument as Int)
            }
        }
    "#,
    )
    .parse_module()
    .unwrap();
    let mut cg =
        verum_vbc::codegen::VbcCodegen::with_config(verum_vbc::codegen::CodegenConfig::new("app"));
    verum_compiler::archive_ctx_loader::ArchiveCtxCache::new(&archive)
        .apply_lazy_with_types(&mut cg, &app, &[])
        .unwrap();
    cg.collect_unit_declarations(&[&app]).unwrap();
    cg.resolve_pending_imports();
    cg.compile_unit_items(&[&app], verum_vbc::codegen::ItemFailurePolicy::Strict)
        .unwrap();
    let module = cg.finalize_module_from_state().unwrap();
    let entry = module.find_function_by_name("app.entry").unwrap();
    let name = CString::new("unit-program").unwrap();
    let argument = CString::new("alpha").unwrap();
    let table = [name.as_ptr(), argument.as_ptr(), std::ptr::null()];
    let mut vm = Interpreter::new(Arc::new(module));
    let result = vm
        .execute_function_with_args(
            entry,
            &[
                Value::from_i64(2),
                Value::from_ptr(table.as_ptr() as *mut u8),
            ],
        )
        .unwrap();
    assert_eq!(result.try_as_i64(), Some(99));
    // vm is dropped before the backing OS-table analogue and its strings.
}
