//! T2279: actual CLI option routing must reach producer validation outside a workspace.
//! These controls do not compile a reduced stdlib or claim archive qualification.
use std::process::Command;
use tempfile::TempDir;

#[test]
fn worker_budget_preserves_production_success_and_refusal() {
    let root = TempDir::new().unwrap();
    let core = root.path().join("core");
    std::fs::create_dir(&core).unwrap();
    std::fs::write(core.join("mod.vr"), "module core; public module value; public module consumer;").unwrap();
    std::fs::write(core.join("value.vr"), "module core.value; public fn answer() -> Int { 73 }").unwrap();
    let consumer = core.join("consumer.vr");
    for valid in [true, false] {
        std::fs::write(&consumer, if valid {
            "module core.consumer; mount core.value.answer; public fn result() -> Int { answer() }"
        } else {
            "module core.consumer; mount core.value.answer; public fn result() -> Bool { answer() }"
        }).unwrap();
        for threads in ["1", "3"] {
            let archive = root.path().join(format!("{valid}-{threads}.vbca"));
            let output = Command::new(env!("CARGO_BIN_EXE_verum"))
                .env_remove("RUST_MIN_STACK")
                .args(["stdlib", "precompile", "--stdlib-path"]).arg(&core)
                .arg("--out").arg(&archive).args(["--threads", threads])
                .current_dir(root.path()).output().unwrap();
            let error = verum_common::Text::from_utf8_lossy(&output.stderr);
            assert_eq!(output.status.code(), Some(if valid { 0 } else { 1 }), "{error}");
            for artifact in [&archive, &archive.with_extension("core_metadata"),
                &archive.with_extension("symbol_graph"), &archive.with_added_extension("receipt")] {
                assert_eq!(artifact.exists(), valid, "{}: {error}", artifact.display());
            }
            if !valid { assert!(error.contains("core.consumer") && error.contains("Type mismatch"), "{error}"); }
        }
    }
}

#[test]
fn worker_budget_rejects_zero_and_non_integer_counts() {
    let root = TempDir::new().unwrap();
    for count in ["0", "many", "1.5"] {
        let output = Command::new(env!("CARGO_BIN_EXE_verum"))
            .args(["stdlib", "precompile", "--threads", count])
            .current_dir(root.path()).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        let error = verum_common::Text::from_utf8_lossy(&output.stderr);
        assert!(error.contains("--threads") && error.contains("invalid value"), "{error}");
        assert!(!root.path().join("target").exists());
    }
}

#[test]
fn explicit_absolute_paths_reach_source_validation_outside_workspace() {
    let cwd = TempDir::new().unwrap();
    let source = cwd.path().join("missing-core");
    let archive = cwd.path().join("explicit.vbca");
    let output = Command::new(env!("CARGO_BIN_EXE_verum"))
        .args(["stdlib", "precompile", "--stdlib-path"])
        .arg(&source)
        .arg("--out")
        .arg(&archive)
        .current_dir(cwd.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(
        error.contains("stdlib precompile failed: stdlib path is not a directory"),
        "{error}"
    );
    assert!(error.contains(&source.display().to_string()), "{error}");
    assert!(
        !error.contains("failed to resolve workspace root"),
        "{error}"
    );
    assert!(!archive.exists());
}

#[test]
fn explicit_source_without_output_reaches_root_validation() {
    let cwd = TempDir::new().unwrap();
    let source = cwd.path().join("empty-core");
    std::fs::create_dir(&source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_verum"))
        .args(["stdlib", "precompile", "--stdlib-path"])
        .arg(&source)
        .current_dir(cwd.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(
        error.contains("stdlib precompile failed: stdlib path missing mod.vr"),
        "{error}"
    );
    assert!(
        !error.contains("failed to resolve workspace root"),
        "{error}"
    );
    assert!(!cwd.path().join("target").exists());
}

#[test]
fn output_alone_does_not_replace_implicit_source_discovery() {
    let cwd = TempDir::new().unwrap();
    let archive = cwd.path().join("explicit.vbca");
    let output = Command::new(env!("CARGO_BIN_EXE_verum"))
        .args(["stdlib", "precompile", "--out"])
        .arg(&archive)
        .current_dir(cwd.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(
        error.contains("failed to resolve workspace root from cwd"),
        "{error}"
    );
    assert!(!archive.exists());
}
