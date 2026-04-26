//! Integration tests for `verum audit --framework-conflicts` (V1, #205).
//!
//! End-to-end coverage of the CLI surface introduced as the V1
//! shipping-target for the framework-compat module. The plain
//! formatter and the JSON formatter are both exercised; clean
//! and conflicting projects are both exercised; the exit-code
//! contract (non-zero on conflict) is verified.

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

[profile.dev]
tier = "interpreter"
verification = "runtime"
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
fn framework_conflicts_clean_project_exits_zero() {
    // No @framework markers → no corpora → no conflicts.
    let (_temp, dir) = create_project(
        "fc_clean",
        r#"public fn main() {}
"#,
    );
    let out = run_verum(&["audit", "--framework-conflicts"], &dir);
    assert!(
        out.status.success(),
        "clean project must exit zero: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        combined.contains("No incompatible-pair conflicts")
            || combined.contains("Distinct corpora:    0"),
        "expected clean-report message, got:\n{}",
        combined,
    );
}

#[test]
fn framework_conflicts_uip_plus_univalence_detected_and_exits_nonzero() {
    // Two contradictory @framework markers — UIP + univalence
    // is the canonical entry in the V0 catalogue.
    let (_temp, dir) = create_project(
        "fc_conflict",
        r#"@framework(uip, "test")
public axiom uip_axiom() -> Bool;

@framework(univalence, "test")
public axiom univalence_axiom() -> Bool;

public fn main() {}
"#,
    );
    let out = run_verum(&["audit", "--framework-conflicts"], &dir);
    assert!(
        !out.status.success(),
        "conflict must produce non-zero exit"
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(combined.contains("uip"), "uip in report: {}", combined);
    assert!(
        combined.contains("univalence"),
        "univalence in report: {}",
        combined,
    );
    assert!(
        combined.contains("HoTT Book"),
        "literature citation surfaced: {}",
        combined,
    );
}

#[test]
fn framework_conflicts_json_format_matches_schema() {
    let (_temp, dir) = create_project(
        "fc_json",
        r#"@framework(uip, "test")
public axiom uip_axiom() -> Bool;

@framework(univalence, "test")
public axiom univalence_axiom() -> Bool;

public fn main() {}
"#,
    );
    let out = run_verum(
        &["audit", "--framework-conflicts", "--format", "json"],
        &dir,
    );
    // JSON exits non-zero on conflict (same contract as plain).
    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\"schema_version\":1"), "JSON: {}", stdout);
    assert!(stdout.contains("\"corpora\":["), "JSON: {}", stdout);
    assert!(stdout.contains("\"conflicts\":["), "JSON: {}", stdout);
    assert!(stdout.contains("\"rule\":\"R4\""), "JSON: {}", stdout);
    assert!(
        stdout.contains("\"severity\":\"error\""),
        "JSON: {}",
        stdout,
    );
}

#[test]
fn framework_conflicts_compatible_pair_passes() {
    // lurie_htt + schreiber_dcct is an explicitly compatible pair
    // (both standard Standard catalogue, no entry in matrix).
    let (_temp, dir) = create_project(
        "fc_compat",
        r#"@framework(lurie_htt, "Lurie HTT 2009")
public axiom yoneda_full() -> Bool;

@framework(schreiber_dcct, "Schreiber 2013")
public axiom shape_modality() -> Bool;

public fn main() {}
"#,
    );
    let out = run_verum(&["audit", "--framework-conflicts"], &dir);
    assert!(
        out.status.success(),
        "compatible pair must exit zero: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        combined.contains("No incompatible-pair conflicts"),
        "clean-message expected: {}",
        combined,
    );
}
