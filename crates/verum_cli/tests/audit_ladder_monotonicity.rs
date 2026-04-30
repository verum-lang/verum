//! Integration tests for `verum audit --ladder-monotonicity`
//! (#139 / MSFS-L4.6).
//!
//! Pin coverage:
//!   - Empty project (no `@verify(...)` annotations) → 0 walks, 0
//!     violations, exit 0.
//!   - Theorem annotated `@verify(formal)` whose proof body trivially
//!     closes → walk visits every backbone strategy from Runtime to
//!     Formal, all close, no violations, exit 0.
//!   - JSON output carries the schema_version=1 envelope and per-walk
//!     metadata.
//!
//! NOTE: producing a real *violation* in an end-to-end test requires
//! a custom dispatcher impl (the `DefaultLadderDispatcher` in
//! `verum_verification` is monotone-by-construction).  Violation
//! detection is tested directly at the unit-test level in
//! `crates/verum_verification/src/ladder_dispatch.rs::tests`.

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
fn empty_project_reports_zero_walks() {
    // No @verify annotations → no walks, no violations, exit 0.
    let (_temp, dir) = create_project("lm_empty", "public fn main() {}");
    let out = run_verum(&["audit", "--ladder-monotonicity"], &dir);
    assert!(
        out.status.success(),
        "audit must exit 0 on a corpus with no annotated theorems.\n\
         stdout: {}\n stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("0 backbone walk"));
    assert!(stdout.contains("0 runtime monotonicity violation"));
}

#[test]
fn formal_annotated_theorem_walks_full_backbone_no_violations() {
    // A trivial-tautology theorem annotated @verify(formal) — the
    // DefaultLadderDispatcher closes it at every backbone slot from
    // Runtime to Formal, so the monotonicity walk produces no
    // violations.
    let body = r#"
@verify(formal)
public theorem trivial_thm()
    ensures true
    proof by trivial;
"#;
    let (_temp, dir) = create_project("lm_formal", body);
    let out = run_verum(&["audit", "--ladder-monotonicity"], &dir);
    assert!(
        out.status.success(),
        "audit must exit 0 when no monotonicity violations occur.\n\
         stdout: {}\n stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // At least one walk happened.
    assert!(
        stdout.contains("backbone walk(s)"),
        "report must mention the backbone walks; got:\n{}",
        stdout,
    );
    assert!(stdout.contains("0 runtime monotonicity violation"));
    assert!(stdout.contains("strict ν-monotonicity"));
}

#[test]
fn json_output_has_schema_v1_envelope() {
    let body = r#"
@verify(formal)
public theorem trivial_thm()
    ensures true
    proof by trivial;
"#;
    let (_temp, dir) = create_project("lm_json", body);
    let out = run_verum(
        &["audit", "--ladder-monotonicity", "--format", "json"],
        &dir,
    );
    assert!(out.status.success(), "JSON-format audit must exit 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let payload: serde_json::Value =
        serde_json::from_str(&stdout).expect("audit JSON must be parseable");
    assert_eq!(payload["schema_version"], 1);
    assert_eq!(payload["command"], "audit-ladder-monotonicity");
    assert!(payload["total_walks"].is_number());
    assert_eq!(payload["total_violations"], 0);
    let violations = payload["violations"]
        .as_array()
        .expect("violations array must be present");
    assert!(violations.is_empty());
}

#[test]
fn unannotated_theorem_is_not_walked() {
    // Theorem without @verify(...) → not eligible for ladder walk.
    let body = r#"
public theorem unannotated()
    ensures true
    proof by trivial;
"#;
    let (_temp, dir) = create_project("lm_unannotated", body);
    let out = run_verum(
        &["audit", "--ladder-monotonicity", "--format", "json"],
        &dir,
    );
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let payload: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        payload["total_walks"], 0,
        "unannotated theorem must not be walked",
    );
}

#[test]
fn multiple_annotated_theorems_each_walk_independently() {
    let body = r#"
@verify(runtime)
public theorem thm_a()
    ensures true
    proof by trivial;

@verify(static)
public theorem thm_b()
    ensures true
    proof by trivial;

@verify(formal)
public theorem thm_c()
    ensures true
    proof by trivial;
"#;
    let (_temp, dir) = create_project("lm_multi", body);
    let out = run_verum(
        &["audit", "--ladder-monotonicity", "--format", "json"],
        &dir,
    );
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let payload: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let walks = payload["total_walks"].as_u64().expect("total_walks number");
    assert!(walks >= 3, "expected ≥ 3 walks for 3 annotated theorems; got {}", walks);
    assert_eq!(payload["total_violations"], 0);
}
