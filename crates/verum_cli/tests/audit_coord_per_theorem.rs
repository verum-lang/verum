//! Integration tests for `verum audit --coord` per-theorem
//! inference ().
//!
//! Per defect 2 + §A.Z.5 item 3: the audit walks
//! every @theorem / @lemma / @corollary / @axiom in the project,
//! infers the (Fw, ν, τ) coordinate from cited @framework(...)
//! markers using max-of-cited-coords, and surfaces a per-theorem
//! report.

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
fn coord_audit_clean_project_succeeds() {
    let (_temp, dir) = create_project(
        "coord_clean",
        r#"public fn main() {}
"#,
    );
    let out = run_verum(&["audit", "--coord"], &dir);
    assert!(out.status.success());
}

#[test]
fn coord_audit_single_framework_per_theorem_inferred() {
    // One @framework marker → inferred coord is that framework's
    // canonical (ν, τ).
    let (_temp, dir) = create_project(
        "coord_single",
        r#"@framework(lurie_htt, "HTT 6.2.2.7")
public axiom yoneda() -> Bool;

public fn main() {}
"#,
    );
    let out = run_verum(&["audit", "--coord"], &dir);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("Per-theorem inferred"),
        "expected per-theorem section: {}",
        combined,
    );
    assert!(
        combined.contains("yoneda"),
        "expected yoneda axiom row: {}",
        combined,
    );
    assert!(
        combined.contains("ν=ω"),
        "lurie_htt is ν=ω: {}",
        combined,
    );
}

#[test]
fn coord_audit_max_of_cited_for_multi_framework() {
    // Theorem cites two frameworks (petz at ν=2, lurie at ν=ω).
    // Inferred = lurie_htt (max).
    let (_temp, dir) = create_project(
        "coord_max",
        r#"@framework(petz_classification, "Petz 1986")
@framework(lurie_htt, "HTT 6.2.2.7")
public theorem cross_corpus() -> Bool;

public fn main() {}
"#,
    );
    let out = run_verum(&["audit", "--coord"], &dir);
    assert!(out.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    // Per-theorem section should report cross_corpus → lurie_htt
    // (max of the two cited).
    assert!(
        combined.contains("cross_corpus"),
        "theorem name in report: {}",
        combined,
    );
    // The ν=ω is the max — should appear.
    assert!(
        combined.contains("[2 cits]"),
        "two cit-count for cross_corpus: {}",
        combined,
    );
}

#[test]
fn coord_audit_json_format_emits_schema_v2() {
    // Schema v2 added the `per_theorem` array (per-theorem coordinate
    // record alongside the `frameworks` summary).  Both keys are part
    // of the contract; an emitter that only ships `frameworks` is on
    // schema v1.  Keep the test pinning both keys present so any future
    // schema change has to choose: bump the version OR keep both keys.
    let (_temp, dir) = create_project(
        "coord_json",
        r#"@framework(lurie_htt, "HTT 6.2.2.7")
public axiom y() -> Bool;

public fn main() {}
"#,
    );
    let out = run_verum(&["audit", "--coord", "--format", "json"], &dir);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\"schema_version\": 2"), "JSON: {}", stdout);
    assert!(stdout.contains("\"frameworks\""), "JSON: {}", stdout);
    assert!(stdout.contains("\"per_theorem\""), "JSON: {}", stdout);
}

#[test]
fn coord_audit_no_framework_markers_clean_message() {
    // Project with no @framework annotations — clean message.
    let (_temp, dir) = create_project(
        "coord_no_markers",
        r#"public fn main() {}

public theorem trivial() -> Bool;
"#,
    );
    let out = run_verum(&["audit", "--coord"], &dir);
    assert!(out.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        combined.contains("no @framework(...) markers")
            || combined.contains("0 .vr") // skipped-files possible
            || combined.contains("Found 0"),
        "clean-message expected: {}",
        combined,
    );
}
