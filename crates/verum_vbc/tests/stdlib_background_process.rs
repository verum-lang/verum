//! Poll the shell future against live, piped processes on Darwin.
#![cfg(all(feature = "codegen", feature = "ffi", target_os = "macos"))]
#[path = "fixtures/background_process.rs"]
mod fixture;

#[test]
fn a_background_process_starts_on_poll_and_reaps_its_child_on_cancel() {
    // Set the inherited environment on a dedicated process rather than racing
    // other test threads by modifying the current process environment.
    const MARKER: &str = "VERUM_TEST_BACKGROUND_ENV";
    if std::env::var(MARKER).as_deref() != Ok("inherit-73") {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "a_background_process_starts_on_poll_and_reaps_its_child_on_cancel"])
            .env(MARKER, "inherit-73")
            .status()
            .unwrap();
        assert!(status.success(), "background process checks failed: {status}");
        return;
    }
    let module = fixture::module();
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    let loaded = verum_vbc::deserialize::deserialize_module_validated(&wire).unwrap();
    for module in [module, loaded] {
        let entry = module
            .find_function_by_name("core.shell.exec.main")
            .unwrap();
        let mut vm = verum_vbc::interpreter::Interpreter::try_new_validated(module.into()).unwrap();
        vm.run_global_ctors().unwrap();
        assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
    }
}
