//! T2279: actual CLI option routing must reach producer validation outside a workspace.
//! These controls do not compile a reduced stdlib or claim archive qualification.
use std::process::Command;
use tempfile::TempDir;

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
