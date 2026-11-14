//! A synchronous callback either returns its value or executes the selected panic.
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
use verum_vbc::{archive::read_archive_from_file, interpreter::Interpreter};

#[test]
fn builtin_panic_in_a_callback_preserves_success_and_failure_after_archive_reload() {
    let directory = tempfile::tempdir().unwrap();
    let core = directory.path().join("core");
    std::fs::create_dir(&core).unwrap();
    std::fs::write(
        core.join("mod.vr"),
        r#"
        module core;
        fn invoke<F:fn()->Int>(callback:F)->Int {callback()}
        public fn success()->Int {
            let callback:fn()->Int=|| {if false {panic("unreachable failure");} 73};
            invoke(callback)
        }
        public fn failure()->Int {invoke(||panic("callback failure"))}
    "#,
    )
    .unwrap();
    let output = directory.path().join("callbacks.vbca");
    let mut session = Session::new(CompilerOptions {
        input: core.join("mod.vr"),
        ..Default::default()
    });
    let result =
        CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
            .compile_core()
            .expect("ordinary source compilation");
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let archive = read_archive_from_file(&output).unwrap();
    let module = archive.load_module("core").unwrap();
    let success = module.find_function_by_name("core.success").unwrap();
    let failure = module.find_function_by_name("core.failure").unwrap();
    let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
    vm.run_global_ctors().unwrap();
    assert_eq!(vm.execute_function(success).unwrap().try_as_i64(), Some(73));
    let error = vm
        .execute_function(failure)
        .expect_err("panic callback returned normally");
    assert!(error.to_string().contains("callback failure"), "{error:?}");
}
