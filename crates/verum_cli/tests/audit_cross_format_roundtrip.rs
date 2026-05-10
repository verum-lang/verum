//! Integration tests for `verum audit --cross-format-roundtrip`
//! (#138 / MSFS-L4.5).
//!

//! Pin coverage:
//!  - Empty project (no `@theorem` / `@lemma` / `@corollary`) →
//!  0 theorems walked, exit 0, no files emitted.
//!  - Single-theorem project → emits Coq + Lean files into
//!  `target/audit-reports/cross-format-roundtrip/{coq,lean}/`.
//!  - Per-theorem files have the right shape (Theorem … Admitted /
//!  theorem … sorry) for proof-bearing decls; Axiom for proofless.
//!  - JSON output carries the schema_version=1 envelope and per-
//!  theorem rows.
//!  - Tool-missing host: gate exits 0 (observability without
//!  blocking) on a host without coqc / lean installed.
//!  - Theorem-name sanitisation: weird characters → safe Verum-
//!  prefixed identifiers.

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
fn empty_project_walks_zero_theorems() {
    let (_temp, dir) = create_project("cfr_empty", "public fn main() {}");
    let out = run_verum(&["audit", "--cross-format-roundtrip"], &dir);
    assert!(
        out.status.success(),
        "audit must exit 0 on a corpus with no theorems.\n\
         stdout: {}\n stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("walked 0 theorem"));
}

#[test]
fn single_theorem_emits_coq_and_lean_files() {
    let body = r#"
public theorem trivial_thm()
    ensures true
    proof by trivial;
"#;
    let (_temp, dir) = create_project("cfr_single", body);
    let out = run_verum(&["audit", "--cross-format-roundtrip"], &dir);
    assert!(
        out.status.success(),
        "audit must exit 0 (tools missing is observability, not failure).\n\
         stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    let report_dir = dir
        .join("target")
        .join("audit-reports")
        .join("cross-format-roundtrip");
    let coq_path = report_dir.join("coq").join("trivial_thm.v");
    let lean_path = report_dir.join("lean").join("trivial_thm.lean");
    assert!(coq_path.exists(), "Coq file missing: {:?}", coq_path);
    assert!(lean_path.exists(), "Lean file missing: {:?}", lean_path);

    let coq_text = fs::read_to_string(&coq_path).unwrap();
    // `ensures true` renders to Coq's `True` (the propositional
    // top type) and proves via `trivial`. Pre-fix the emitter used
    // `Prop` (the kind of propositions, not a proposition) and
    // `Admitted` — which actually didn't typecheck (`trivial`
    // can't prove `Prop`).  Verified passing against installed
    // Coq via the `coq` foreign-tool roundtrip check.
    assert!(
        coq_text.contains("Theorem trivial_thm : True."),
        "expected `Theorem trivial_thm : True.`; got:\n{}",
        coq_text,
    );
    assert!(coq_text.contains("trivial."));
    let lean_text = fs::read_to_string(&lean_path).unwrap();
    // Same correction for Lean — `True` propositional type, proved
    // by `trivial`, not `sorry`.  Verified against installed Lean
    // via the `lean` foreign-tool roundtrip check.
    assert!(
        lean_text.contains("theorem trivial_thm : True := by trivial"),
        "expected Lean term `theorem trivial_thm : True := by trivial`; got:\n{}",
        lean_text,
    );
}

#[test]
fn proofless_theorem_emits_axiom_form() {
    // Theorems without a `proof { … }` body emit `Axiom` / `axiom`,
    // not `Theorem … Admitted` / `theorem … sorry`. Pin this so a
    // future refactor doesn't accidentally promote axioms to
    // admitted theorems (which would change the foreign-tool
    // semantics from "postulate" to "incomplete proof").
    let body = r#"
public theorem unproven_thm()
    ensures true;
"#;
    let (_temp, dir) = create_project("cfr_axiom", body);
    let out = run_verum(&["audit", "--cross-format-roundtrip"], &dir);
    assert!(out.status.success());

    let report_dir = dir
        .join("target")
        .join("audit-reports")
        .join("cross-format-roundtrip");
    let coq_text = fs::read_to_string(report_dir.join("coq").join("unproven_thm.v")).unwrap();
    // `ensures true` renders to the propositional `True` type
    // (mirrors Lean's `True`); `Axiom unproven_thm : True.` is
    // the canonical proofless form — postulate-equivalent.
    assert!(
        coq_text.contains("Axiom unproven_thm : True."),
        "proofless theorem must emit Coq Axiom form; got:\n{}",
        coq_text,
    );
    assert!(!coq_text.contains("Admitted."));
    let lean_text = fs::read_to_string(report_dir.join("lean").join("unproven_thm.lean")).unwrap();
    assert!(
        lean_text.contains("axiom unproven_thm : True"),
        "proofless theorem must emit Lean axiom form; got:\n{}",
        lean_text,
    );
    assert!(!lean_text.contains("sorry"));
}

#[test]
fn json_output_has_schema_v1_envelope() {
    let body = r#"
public theorem trivial_thm()
    ensures true
    proof by trivial;
"#;
    let (_temp, dir) = create_project("cfr_json", body);
    let out = run_verum(
        &["audit", "--cross-format-roundtrip", "--format", "json"],
        &dir,
    );
    assert!(out.status.success(), "JSON-format audit must exit 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let payload: serde_json::Value =
        serde_json::from_str(&stdout).expect("audit JSON must be parseable");

    assert_eq!(payload["schema_version"], 1);
    assert_eq!(payload["command"], "audit-cross-format-roundtrip");
    assert_eq!(payload["theorems_walked"], 1);
    // 5 backends now wired: coq + lean + agda + isabelle + dedukti
    // (pre-#156 this was 2; the four-foundation cross-validation
    // matrix added Agda/Isabelle/Dedukti).
    assert_eq!(payload["backend_count"], 5);
    let roundtrips = payload["roundtrips"].as_array().expect("roundtrips array");
    // 1 theorem × 5 backends = 5 rows.
    assert_eq!(roundtrips.len(), 5);
    let backends: Vec<&str> = roundtrips
        .iter()
        .map(|r| r["backend"].as_str().unwrap())
        .collect();
    for required in ["coq", "lean", "agda", "isabelle", "dedukti"] {
        assert!(
            backends.contains(&required),
            "JSON output missing backend `{}`; got {:?}",
            required,
            backends,
        );
    }
    for r in roundtrips {
        assert_eq!(r["theorem"], "trivial_thm");
        // verdict is one of: passed / failed / tool_missing /
        // runner_error / no_checker. When the tool is missing,
        // verdict_kind is `tool_missing` and gate still exits 0.
        let verdict = r["verdict"].as_str().unwrap();
        assert!(
            [
                "passed",
                "failed",
                "tool_missing",
                "runner_error",
                "no_checker"
            ]
            .contains(&verdict),
            "unexpected verdict: {}",
            verdict,
        );
    }
}

#[test]
fn multiple_theorems_each_emit_their_own_files() {
    let body = r#"
public theorem thm_a()
    ensures true
    proof by trivial;

public theorem thm_b()
    ensures true
    proof by trivial;

public theorem thm_c()
    ensures true;
"#;
    let (_temp, dir) = create_project("cfr_multi", body);
    let out = run_verum(&["audit", "--cross-format-roundtrip"], &dir);
    assert!(out.status.success());

    let report_dir = dir
        .join("target")
        .join("audit-reports")
        .join("cross-format-roundtrip");
    for name in ["thm_a", "thm_b", "thm_c"] {
        assert!(
            report_dir.join("coq").join(format!("{}.v", name)).exists(),
            "missing Coq file for {}",
            name,
        );
        assert!(
            report_dir
                .join("lean")
                .join(format!("{}.lean", name))
                .exists(),
            "missing Lean file for {}",
            name,
        );
    }
}

#[test]
fn declared_strategy_is_preserved_in_emitted_files() {
    let body = r#"
@verify(formal)
public theorem annotated_thm()
    ensures true
    proof by trivial;
"#;
    let (_temp, dir) = create_project("cfr_annotated", body);
    let out = run_verum(&["audit", "--cross-format-roundtrip"], &dir);
    assert!(out.status.success());

    let coq_text = fs::read_to_string(
        dir.join("target/audit-reports/cross-format-roundtrip/coq/annotated_thm.v"),
    )
    .unwrap();
    assert!(coq_text.contains("@verify(formal)"));
    let lean_text = fs::read_to_string(
        dir.join("target/audit-reports/cross-format-roundtrip/lean/annotated_thm.lean"),
    )
    .unwrap();
    assert!(lean_text.contains("@verify(formal)"));
}
