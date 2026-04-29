//! Integration tests for `verum audit --kernel-recheck` (#122/#123).
//!
//! Walks every `.vr` file in the project, runs the kernel re-check
//! (K-Refine-omega / K-Universe-Ascent / K-Eps-Mu / K-Round-Trip)
//! against every theorem-shaped + axiom + function declaration,
//! and reports per-name admitted / rejected outcomes.

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
fn empty_project_admits_clean() {
    // No theorem-shaped items → empty result list, exit 0.
    let (_temp, dir) = create_project(
        "krch_empty",
        r#"public fn main() {}
"#,
    );
    let out = run_verum(&["audit", "--kernel-recheck"], &dir);
    assert!(
        out.status.success(),
        "empty project should exit clean: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Kernel-recheck report"),
        "expected report header; got `{}`",
        stdout
    );
}

#[test]
fn well_formed_theorem_admits_plain() {
    // A theorem with no refinement-type leakage → admitted.
    let (_temp, dir) = create_project(
        "krch_thm",
        r#"public fn main() {}

public theorem trivial_truth()
    ensures true
    proof by auto;
"#,
    );
    let out = run_verum(&["audit", "--kernel-recheck"], &dir);
    assert!(
        out.status.success(),
        "well-formed theorem should admit: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn json_format_well_formed() {
    let (_temp, dir) = create_project(
        "krch_json",
        r#"public fn main() {}

public theorem trivial_truth()
    ensures true
    proof by auto;
"#,
    );
    let out = run_verum(
        &["audit", "--kernel-recheck", "--format", "json"],
        &dir,
    );
    assert!(
        out.status.success(),
        "JSON format should succeed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
            panic!("JSON parse failed for `{}`: {}", stdout, e)
        });
    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(parsed["command"], "audit-kernel-recheck");
    assert!(parsed["total_files"].is_number());
    assert!(parsed["total_admitted"].is_number());
    assert!(parsed["total_rejected"].is_number());
    assert!(parsed["reports"].is_array());
}

#[test]
fn invalid_format_flag_rejected() {
    let (_temp, dir) = create_project("krch_bad_fmt", r#""#);
    let out = run_verum(
        &["audit", "--kernel-recheck", "--format", "garbage"],
        &dir,
    );
    assert!(!out.status.success(), "invalid format should reject");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--format must be") || stderr.contains("plain"),
        "expected format-error diagnostic; got `{}`",
        stderr
    );
}

#[test]
fn project_with_no_vr_files_runs_clean() {
    // Manifest only — no source files at all.
    let temp = TempDir::new().expect("tempdir");
    let dir = temp.path().join("krch_nosrc");
    fs::create_dir_all(&dir).expect("mkdir");
    fs::write(
        dir.join("Verum.toml"),
        r#"[cog]
name = "krch_nosrc"
version = "0.1.0"

[language]
profile = "application"

[dependencies]
"#,
    )
    .expect("manifest");
    let out = run_verum(&["audit", "--kernel-recheck"], &dir);
    assert!(
        out.status.success(),
        "no-source project should exit clean: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn json_report_no_rejections_for_trivially_true_theorems() {
    // Trivially-true theorems with no refinement-type obligations
    // produce empty result lists from `recheck_module` (the
    // walker emits per-name entries only when there's something
    // to attest — params with refined types, requires/ensures
    // clauses with predicates, body with let-binding refinements).
    // What we pin here is the contract that NO rejections surface
    // for well-formed theorems.
    let (_temp, dir) = create_project(
        "krch_count",
        r#"public fn main() {}

public theorem t1() ensures true proof by auto;
public theorem t2() ensures true proof by auto;
"#,
    );
    let out = run_verum(
        &["audit", "--kernel-recheck", "--format", "json"],
        &dir,
    );
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    let rejected = parsed["total_rejected"].as_u64().unwrap();
    assert_eq!(
        rejected, 0,
        "trivially-true theorems must not produce rejections"
    );
}
