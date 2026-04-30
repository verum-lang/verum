//! Integration tests for `verum audit --bridge-discharge` (#134 / MSFS-L4.1).
//!
//! Pin coverage:
//!   - Empty project (no `.vr` proof bodies citing kernel bridges) →
//!     report shows 0 callsites + 0 bridges + exit 0.
//!   - Project with a literal-arg discharge that the dispatcher
//!     accepts (`apply kernel_grothendieck_construction_strict(1)`) →
//!     report shows 1 callsite, `holds: true`, exit 0.
//!   - Project with a literal-arg discharge that the dispatcher
//!     REJECTS (e.g. `apply kernel_grothendieck_construction_strict(0)`
//!     — fails the `StrictPos` precondition at the dispatcher) →
//!     report shows 1 callsite with `holds: false`, exits non-zero.
//!   - Project that cites a `kernel_*` bridge with no dispatcher
//!     entry → audit reports it under `unknown_bridges` and exits
//!     non-zero.
//!   - JSON output carries the schema_version=1 envelope and the
//!     per-callsite `holds` field is rendered correctly.

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
fn empty_project_reports_zero_callsites() {
    let (_temp, dir) = create_project("bd_empty", "public fn main() {}");
    let out = run_verum(&["audit", "--bridge-discharge"], &dir);
    assert!(
        out.status.success(),
        "audit must exit 0 on a corpus with no bridge callsites.\n\
         stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("0 total callsite"));
    assert!(stdout.contains("bridge-free"));
}

#[test]
fn literal_arg_passing_discharge_succeeds() {
    // `kernel_grothendieck_construction_strict` requires `StrictPos`
    // (i.e., n > 0).  A literal `1` passes the dispatcher gate.
    let body = r#"
public theorem example_theorem()
    ensures true
    proof {
        apply kernel_grothendieck_construction_strict(1);
    };
"#;
    let (_temp, dir) = create_project("bd_pass", body);
    let out = run_verum(&["audit", "--bridge-discharge"], &dir);
    assert!(
        out.status.success(),
        "literal-arg discharge that passes the dispatcher must exit 0.\n\
         stdout: {}\n stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("kernel_grothendieck_construction"),
        "report must list the bridge name; got:\n{}",
        stdout,
    );
    assert!(stdout.contains("1 total"));
    assert!(stdout.contains("0 false"));
}

#[test]
fn literal_arg_failing_discharge_exits_nonzero() {
    // `kernel_grothendieck_construction_strict(0)` — `0` violates the
    // `StrictPos` precondition; dispatcher returns `holds: false`.
    let body = r#"
public theorem example_theorem()
    ensures true
    proof {
        apply kernel_grothendieck_construction_strict(0);
    };
"#;
    let (_temp, dir) = create_project("bd_fail", body);
    let out = run_verum(&["audit", "--bridge-discharge"], &dir);
    assert!(
        !out.status.success(),
        "literal-arg discharge that FAILS the dispatcher must exit non-zero.\n\
         stdout: {}\n stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("false-discharge") || combined.contains("1 false"),
        "stderr/stdout must mention the false-discharge count; got:\n{}",
        combined,
    );
}

#[test]
fn json_output_has_schema_v1_envelope() {
    let body = r#"
public theorem example_theorem()
    ensures true
    proof {
        apply kernel_grothendieck_construction_strict(1);
    };
"#;
    let (_temp, dir) = create_project("bd_json", body);
    let out = run_verum(&["audit", "--bridge-discharge", "--format", "json"], &dir);
    assert!(out.status.success(), "JSON-format audit must exit 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let payload: serde_json::Value =
        serde_json::from_str(&stdout).expect("audit JSON must be parseable");

    assert_eq!(payload["schema_version"], 1);
    assert_eq!(payload["command"], "audit-bridge-discharge");
    let bridges = payload["bridges"].as_array().expect("bridges array");
    assert_eq!(bridges.len(), 1);
    let b = &bridges[0];
    assert_eq!(b["bridge_name"], "kernel_grothendieck_construction");
    assert_eq!(b["callsites_total"], 1);
    assert_eq!(b["callsites_literal_args"], 1);
    assert_eq!(b["false_discharges"], 0);
    let callsites = b["callsites"].as_array().expect("callsites array");
    assert_eq!(callsites.len(), 1);
    let c = &callsites[0];
    assert_eq!(c["item_name"], "example_theorem");
    assert_eq!(c["all_literal_args"], true);
    assert_eq!(c["holds"], true);
}

#[test]
fn unknown_bridge_surfaces_in_unknown_bridges() {
    // `kernel_made_up_bridge` has no dispatcher entry — should be
    // reported under `unknown_bridges` and exit non-zero.
    let body = r#"
public theorem example_theorem()
    ensures true
    proof {
        apply kernel_made_up_bridge(1);
    };
"#;
    let (_temp, dir) = create_project("bd_unknown", body);
    let out = run_verum(
        &["audit", "--bridge-discharge", "--format", "json"],
        &dir,
    );
    // The JSON output is on stdout; the gate exits non-zero on unknown
    // bridges (verified by checking the stderr path's error chain).
    let stdout = String::from_utf8_lossy(&out.stdout);
    if let Ok(payload) = serde_json::from_str::<serde_json::Value>(&stdout) {
        let unknown = payload["unknown_bridges"]
            .as_array()
            .expect("unknown_bridges array");
        assert!(
            unknown.iter().any(|v| v == "kernel_made_up_bridge"),
            "unknown_bridges must list kernel_made_up_bridge; got: {:?}",
            unknown,
        );
    }
    assert!(
        !out.status.success(),
        "unknown bridge must trigger non-zero exit",
    );
}

#[test]
fn proof_body_with_let_bindings_walks_into_apply_steps() {
    // Mixing `let` (which the walker skips) with `apply` (which it
    // visits) — pin that the walker doesn't choke on `let` and still
    // captures the apply.
    let body = r#"
public theorem example_theorem()
    ensures true
    proof {
        let x = 42;
        apply kernel_grothendieck_construction_strict(1);
    };
"#;
    let (_temp, dir) = create_project("bd_let", body);
    let out = run_verum(&["audit", "--bridge-discharge"], &dir);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("kernel_grothendieck_construction"));
    assert!(stdout.contains("1 total"));
}
