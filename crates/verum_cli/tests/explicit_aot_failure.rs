//! T1535: native build failures cannot become interpreter successes.

use std::{fs, process::Command};

#[test]
fn explicit_aot_failure_does_not_execute_the_program() {
    let temp = tempfile::tempdir().expect("fixture directory");
    let source = temp.path().join("probe.vr");
    let marker = "INTERPRETER_FALLBACK_EXECUTED";
    fs::write(&source, format!("fn main() {{ print(\"{marker}\"); }}\n")).expect("source");
    // A regular file deterministically prevents native output-directory
    // creation, without platform permissions or an unavailable LLVM install.
    fs::write(temp.path().join("target"), "not a directory").expect("output obstruction");

    for flags in [&["--aot"][..], &["--tier", "aot"][..]] {
        let output = Command::new(env!("CARGO_BIN_EXE_verum"))
            .arg("run")
            .args(flags)
            .arg(&source)
            .current_dir(temp.path())
            .output()
            .expect("run CLI");
        let combined = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!output.status.success(), "{flags:?}: {combined}");
        assert!(
            combined.contains("Failed to create target directory"),
            "wrong failure: {combined}"
        );
        assert!(
            !combined.contains(marker),
            "program executed after failed native build: {combined}"
        );
        assert!(
            !combined.contains("Falling back to interpreter"),
            "{combined}"
        );
    }

    // Prove this is a valid program and that the obstruction affects the
    // requested native output rather than parsing or ordinary interpretation.
    let output = Command::new(env!("CARGO_BIN_EXE_verum"))
        .args(["run", "--interp"])
        .arg(&source)
        .current_dir(temp.path())
        .output()
        .expect("interpreter control");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains(marker));
}
