use std::process::Command;

#[test]
fn bootstrap_uses_the_requested_worker_budget_and_refuses_invalid_counts() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    std::fs::create_dir(&core).unwrap();
    std::fs::write(core.join("mod.vr"), "module core; public module value; public module consumer;").unwrap();
    std::fs::write(core.join("value.vr"), "module core.value; public fn answer() -> Int { 73 }").unwrap();
    std::fs::write(core.join("consumer.vr"), "module core.consumer; mount core.value.answer; public fn result() -> Int { answer() }").unwrap();
    for count in ["1", "3", "0", "many", ""] {
        let output_dir = root.path().join(format!("out-{count}"));
        let output = Command::new(env!("CARGO_BIN_EXE_verum_stdlib_precompiler"))
            .arg(root.path()).env("VERUM_PRECOMPILE_THREADS", count)
            .env("VERUM_PRECOMPILE_OUT_DIR", &output_dir)
            .env_remove("RUST_MIN_STACK").output().unwrap();
        let error = verum_common::Text::from_utf8_lossy(&output.stderr);
        let valid = matches!(count, "1" | "3");
        assert_eq!(output.status.code(), Some(if valid { 0 } else { 1 }), "{error}");
        assert_eq!(output_dir.join("runtime.vbca").is_file(), valid);
        assert_eq!(output_dir.join("runtime.vbca.receipt").is_file(), valid);
        if !valid {
            assert!(error.contains("VERUM_PRECOMPILE_THREADS must be a positive integer"), "{error}");
            assert!(!output_dir.exists());
        }
    }
}
