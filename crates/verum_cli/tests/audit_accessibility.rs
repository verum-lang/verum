//! Integration tests for `verum audit --accessibility` (V8 #231,
//! A.Z.5 item 4 V2).
//!
//! Per VVA §A.Z.5 item 4 + Diakrisis Axi-4: every `@enact(...)`
//! marker in the project must carry an `@accessibility(λ)`
//! annotation certifying the λ-accessibility bound the
//! framework author has audited. The CLI walker enforces this
//! as a CI gate (non-zero exit on any missing annotation).

#![allow(unused_imports)]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::TempDir;

fn create_project(name: &str, main_vr_body: &str) -> (TempDir, PathBuf) {
    let temp = TempDir::new().expect("create tempdir");
    let dir = temp.path().join(name);
    fs::create_dir_all(&dir).expect("create project dir");
    let manifest = format!(
        r#"[cog]
name = "{name}"
version = "0.1.0"

[language]
profile = "application"

[dependencies]
"#
    );
    fs::write(dir.join("Verum.toml"), manifest).expect("write Verum.toml");
    let src = dir.join("src");
    fs::create_dir_all(&src).expect("create src/");
    fs::write(src.join("main.vr"), main_vr_body).expect("write main.vr");
    (temp, dir)
}

fn run_verum(args: &[&str], cwd: &PathBuf) -> Output {
    Command::new(env!("CARGO_BIN_EXE_verum"))
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("spawn verum CLI")
}

#[test]
fn no_enact_markers_audit_passes_clean() {
    let (_temp, dir) = create_project(
        "acc_no_enact",
        r#"public fn main() {}

public theorem trivial() -> Bool;
"#,
    );
    let out = run_verum(&["audit", "--accessibility"], &dir);
    assert!(
        out.status.success(),
        "no @enact markers → clean: {} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        combined.contains("no @enact markers found"),
        "clean message: {}",
        combined,
    );
}

#[test]
fn enact_with_accessibility_audit_passes() {
    let (_temp, dir) = create_project(
        "acc_covered",
        r#"@enact(epsilon = "ε_math")
@accessibility(omega)
public axiom covered_enact() -> Bool;

public fn main() {}
"#,
    );
    let out = run_verum(&["audit", "--accessibility"], &dir);
    assert!(out.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        combined.contains("covered_enact") && combined.contains("omega"),
        "covered enact reported: {}",
        combined,
    );
}

#[test]
fn enact_without_accessibility_audit_fails() {
    let (_temp, dir) = create_project(
        "acc_missing",
        r#"@enact(epsilon = "ε_math")
public axiom missing_acc() -> Bool;

public fn main() {}
"#,
    );
    let out = run_verum(&["audit", "--accessibility"], &dir);
    assert!(!out.status.success(), "missing @accessibility must fail");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        combined.contains("missing_acc"),
        "report cites the offender: {}",
        combined,
    );
    assert!(
        combined.contains("Axi-4")
            || combined.contains("accessibility-certificate gap"),
        "diagnostic explains the Axi-4 gap: {}",
        combined,
    );
}

#[test]
fn mixed_audit_lists_both_covered_and_missing() {
    let (_temp, dir) = create_project(
        "acc_mixed",
        r#"@enact(epsilon = "ε_math")
@accessibility(omega)
public axiom good() -> Bool;

@enact(epsilon = "ε_compute")
public axiom bad() -> Bool;

public fn main() {}
"#,
    );
    let out = run_verum(&["audit", "--accessibility"], &dir);
    // One missing → non-zero.
    assert!(!out.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(combined.contains("good"));
    assert!(combined.contains("bad"));
    assert!(combined.contains("1 of 2") || combined.contains("1 @enact"));
}

#[test]
fn json_format_emits_schema_v1() {
    let (_temp, dir) = create_project(
        "acc_json",
        r#"@enact(epsilon = "ε_math")
@accessibility(omega_1)
public axiom a() -> Bool;

@enact(epsilon = "ε_compute")
public axiom b() -> Bool;

public fn main() {}
"#,
    );
    let out = run_verum(
        &["audit", "--accessibility", "--format", "json"],
        &dir,
    );
    assert!(!out.status.success(), "missing on `b` → non-zero");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\"schema_version\": 1"));
    assert!(stdout.contains("\"items\""));
    assert!(stdout.contains("\"total_enact_sites\": 2"));
    assert!(stdout.contains("\"missing_accessibility\": 1"));
    // `a` is covered with λ=omega_1; `b` is null.
    assert!(stdout.contains("\"accessibility\": \"omega_1\""));
    assert!(stdout.contains("\"accessibility\": null"));
}
