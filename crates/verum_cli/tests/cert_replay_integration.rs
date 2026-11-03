//! End-to-end integration tests for `verum cert-replay`.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::TempDir;

fn verum_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_verum"))
}

fn run(args: &[&str]) -> Output {
    Command::new(verum_bin())
        .args(args)
        .output()
        .expect("CLI invocation must succeed")
}

// Well-formed ALETHE body — decomposes to one `assume` + one
// `la_generic` step, both of which resolve in the canonical
// kernel-rule registry.
const STRUCTURAL_BODY: &str =
    "(assume h1 (>= x 0))\n(step t1 (cl (>= (+ x 1) 1)) :rule la_generic :premises (h1))";
const VALID_THEORY: &str = "QF_LIA";
const CLAIMED_CONCLUSION: &str = "(>= x 0)";

// ─────────────────────────────────────────────────────────────────────
// replay
// ─────────────────────────────────────────────────────────────────────

#[test]
fn replay_kernel_only_inline_reports_structure_without_acceptance() {
    let out = run(&[
        "cert-replay",
        "replay",
        "--backend",
        "kernel_only",
        "--format",
        "z3_proof",
        "--theory",
        VALID_THEORY,
        "--conclusion",
        CLAIMED_CONCLUSION,
        "--body",
        STRUCTURAL_BODY,
    ]);
    assert!(
        !out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("✓ accepted"));
    assert!(stdout.contains("structural inspection"));
    assert!(stdout.contains("Kernel-only check"));
}

#[test]
fn replay_external_backend_reports_unimplemented_adapter() {
    let out = run(&[
        "cert-replay",
        "replay",
        "--backend",
        "z3",
        "--format",
        "z3_proof",
        "--theory",
        VALID_THEORY,
        "--conclusion",
        CLAIMED_CONCLUSION,
        "--body",
        STRUCTURAL_BODY,
    ]);
    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Backend `z3`"));
    assert!(stdout.contains("inference replay adapter for z3 is not implemented"));
}

#[test]
fn replay_unknown_theory_fails() {
    let out = run(&[
        "cert-replay",
        "replay",
        "--backend",
        "kernel_only",
        "--format",
        "z3_proof",
        "--theory",
        "BOGUS",
        "--conclusion",
        CLAIMED_CONCLUSION,
        "--body",
        STRUCTURAL_BODY,
    ]);
    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("rejected"));
    assert!(stdout.contains("unknown theory"));
}

#[test]
fn replay_rejects_unknown_backend() {
    let out = run(&[
        "cert-replay",
        "replay",
        "--backend",
        "garbage",
        "--format",
        "z3_proof",
        "--theory",
        VALID_THEORY,
        "--conclusion",
        CLAIMED_CONCLUSION,
        "--body",
        STRUCTURAL_BODY,
    ]);
    assert!(!out.status.success());
}

#[test]
fn replay_json_well_formed() {
    let out = run(&[
        "cert-replay",
        "replay",
        "--backend",
        "kernel_only",
        "--format",
        "z3_proof",
        "--theory",
        VALID_THEORY,
        "--conclusion",
        CLAIMED_CONCLUSION,
        "--body",
        STRUCTURAL_BODY,
        "--output",
        "json",
    ]);
    assert!(!out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(parsed["schema_version"], 1);
    let cert = &parsed["certificate"];
    assert_eq!(cert["theory"], VALID_THEORY);
    assert_eq!(cert["conclusion"], CLAIMED_CONCLUSION);
    let kernel = &parsed["kernel_verdict"];
    assert_eq!(kernel["kind"], "StructuralOnly");
}

#[test]
fn replay_markdown_format() {
    let out = run(&[
        "cert-replay",
        "replay",
        "--backend",
        "kernel_only",
        "--format",
        "z3_proof",
        "--theory",
        VALID_THEORY,
        "--conclusion",
        CLAIMED_CONCLUSION,
        "--body",
        STRUCTURAL_BODY,
        "--output",
        "markdown",
    ]);
    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("# Certificate replay"));
    assert!(stdout.contains("## Kernel-only check"));
    assert!(!stdout.contains("✓ accepted"));
    assert!(stdout.contains("structural inspection"));
}

// ─────────────────────────────────────────────────────────────────────
// replay --cert FILE — round-trip
// ─────────────────────────────────────────────────────────────────────

#[test]
fn replay_loads_cert_from_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("cert.json");
    // Build the cert envelope via serde_json::Value so the multi-line
    // ALETHE body's newlines are properly JSON-escaped.
    let cert = serde_json::json!({
        "format": "cvc5_alethe",
        "theory": VALID_THEORY,
        "conclusion": CLAIMED_CONCLUSION,
        "body": STRUCTURAL_BODY,
        "body_hash": hex_blake3(STRUCTURAL_BODY),
        "source_solver": serde_json::Value::Null,
    });
    fs::write(&path, serde_json::to_string_pretty(&cert).unwrap()).unwrap();
    let out = run(&[
        "cert-replay",
        "replay",
        "--backend",
        "kernel_only",
        "--cert",
        path.to_str().unwrap(),
    ]);
    assert!(
        !out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn hex_blake3(s: &str) -> String {
    let h = blake3::hash(s.as_bytes());
    let mut out = String::new();
    for b in h.as_bytes() {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

#[test]
fn replay_rejects_tampered_cert_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("cert.json");
    // Write a cert whose body_hash doesn't match its body.
    let body = format!(
        r#"{{
            "format": "z3_proof",
            "theory": "{}",
            "conclusion": "{}",
            "body": "tampered body",
            "body_hash": "{}",
            "source_solver": null
        }}"#,
        VALID_THEORY,
        CLAIMED_CONCLUSION,
        hex_blake3("ORIGINAL UNMODIFIED")
    );
    fs::write(&path, body).unwrap();
    let out = run(&[
        "cert-replay",
        "replay",
        "--backend",
        "kernel_only",
        "--cert",
        path.to_str().unwrap(),
    ]);
    assert!(
        !out.status.success(),
        "tampered cert must produce non-zero exit"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("body_hash mismatch"));
}

#[test]
fn replay_rejects_invalid_cert_file_json() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("cert.json");
    fs::write(&path, "not valid json").unwrap();
    let out = run(&[
        "cert-replay",
        "replay",
        "--backend",
        "kernel_only",
        "--cert",
        path.to_str().unwrap(),
    ]);
    assert!(!out.status.success());
}

// ─────────────────────────────────────────────────────────────────────
// cross-check
// ─────────────────────────────────────────────────────────────────────

#[test]
fn false_conclusion_never_gets_mock_acceptance_or_success_exit() {
    for backend in ["kernel_only", "z3", "cvc5", "verit", "open_smt", "mathsat"] {
        for command in ["replay", "cross-check"] {
            let out = run(&[
                "cert-replay",
                command,
                "--backend",
                backend,
                "--format",
                "cvc5_alethe",
                "--theory",
                VALID_THEORY,
                "--conclusion",
                "false",
                "--body",
                "(assume h1 (>= x 0))",
                "--output",
                "json",
            ]);
            assert!(
                !out.status.success(),
                "{command}/{backend} must not authorize success"
            );
            let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
            let verdicts = if command == "replay" {
                vec![&parsed["kernel_verdict"], &parsed["backend_verdict"]]
            } else {
                assert_eq!(parsed["accept_count"], 0);
                assert_eq!(parsed["all_available_accept"], false);
                parsed["verdict"]["per_backend"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .collect()
            };
            assert!(verdicts.iter().all(|v| v["kind"] != "Accepted"));
            assert!(verdicts.iter().any(|v| v["kind"] == "StructuralOnly"));
            if backend != "kernel_only" {
                assert!(verdicts.iter().any(|v| v["kind"] == "Error"));
            }
            assert!(!String::from_utf8_lossy(&out.stdout).contains("mock"));
        }
    }
}

#[test]
fn cross_check_default_runs_kernel_plus_external_backends() {
    let out = run(&[
        "cert-replay",
        "cross-check",
        "--format",
        "z3_proof",
        "--theory",
        VALID_THEORY,
        "--conclusion",
        CLAIMED_CONCLUSION,
        "--body",
        STRUCTURAL_BODY,
        "--output",
        "json",
    ]);
    assert!(!out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let verdict = &parsed["verdict"];
    let per = verdict["per_backend"].as_array().unwrap();
    // Kernel-only + 5 external backends = 6 entries.
    assert_eq!(per.len(), 6);
}

#[test]
fn cross_check_explicit_backends() {
    let out = run(&[
        "cert-replay",
        "cross-check",
        "--backend",
        "z3",
        "--backend",
        "cvc5",
        "--format",
        "z3_proof",
        "--theory",
        VALID_THEORY,
        "--conclusion",
        CLAIMED_CONCLUSION,
        "--body",
        STRUCTURAL_BODY,
        "--output",
        "json",
    ]);
    assert!(!out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let verdict = &parsed["verdict"];
    let per = verdict["per_backend"].as_array().unwrap();
    // Structural inspection and two absent adapters grant no proof authority.
    assert_eq!(per.len(), 3);
    assert_eq!(parsed["all_available_accept"], false);
    assert_eq!(per[0]["kind"], "StructuralOnly");
    assert!(per[1..].iter().all(|v| v["kind"] == "Error"));
}

#[test]
fn cross_check_kernel_rejection_blocks_consensus() {
    // Tampered body → kernel-only rejects → consensus broken even
    // with require-consensus off.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("cert.json");
    let body = format!(
        r#"{{
            "format": "z3_proof",
            "theory": "{}",
            "conclusion": "{}",
            "body": "tampered",
            "body_hash": "{}",
            "source_solver": null
        }}"#,
        VALID_THEORY,
        CLAIMED_CONCLUSION,
        hex_blake3("original")
    );
    fs::write(&path, body).unwrap();
    let out = run(&[
        "cert-replay",
        "cross-check",
        "--cert",
        path.to_str().unwrap(),
        "--output",
        "json",
    ]);
    // Even without --require-consensus, an invalid certificate cannot succeed.
    assert!(!out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(parsed["all_available_accept"], false);
}

#[test]
fn cross_check_require_consensus_fails_on_kernel_rejection() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("cert.json");
    let body = format!(
        r#"{{
            "format": "z3_proof",
            "theory": "{}",
            "conclusion": "{}",
            "body": "tampered",
            "body_hash": "{}",
            "source_solver": null
        }}"#,
        VALID_THEORY,
        CLAIMED_CONCLUSION,
        hex_blake3("original")
    );
    fs::write(&path, body).unwrap();
    let out = run(&[
        "cert-replay",
        "cross-check",
        "--cert",
        path.to_str().unwrap(),
        "--require-consensus",
    ]);
    assert!(
        !out.status.success(),
        "--require-consensus must fail on kernel rejection"
    );
}

#[test]
fn cross_check_markdown_format() {
    let out = run(&[
        "cert-replay",
        "cross-check",
        "--backend",
        "z3",
        "--format",
        "z3_proof",
        "--theory",
        VALID_THEORY,
        "--conclusion",
        CLAIMED_CONCLUSION,
        "--body",
        STRUCTURAL_BODY,
        "--output",
        "markdown",
    ]);
    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("# Cross-backend cert verdict"));
    assert!(stdout.contains("**Consensus:**"));
}

// ─────────────────────────────────────────────────────────────────────
// formats / backends
// ─────────────────────────────────────────────────────────────────────

#[test]
fn formats_lists_six_canonical_formats() {
    let out = run(&["cert-replay", "formats", "--output", "json"]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(parsed["count"], 6);
}

#[test]
fn backends_lists_six_canonical_backends() {
    let out = run(&["cert-replay", "backends", "--output", "json"]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(parsed["count"], 6);
    let backends = parsed["backends"].as_array().unwrap();
    let kernel_only = backends
        .iter()
        .find(|b| b["name"] == "kernel_only")
        .unwrap();
    assert_eq!(kernel_only["is_intrinsic"], true);
}

// ─────────────────────────────────────────────────────────────────────
// validation
// ─────────────────────────────────────────────────────────────────────

#[test]
fn replay_rejects_empty_body() {
    let out = run(&[
        "cert-replay",
        "replay",
        "--backend",
        "kernel_only",
        "--format",
        "z3_proof",
        "--theory",
        VALID_THEORY,
        "--conclusion",
        CLAIMED_CONCLUSION,
        "--body",
        "",
    ]);
    assert!(!out.status.success());
}

#[test]
fn replay_rejects_empty_theory() {
    let out = run(&[
        "cert-replay",
        "replay",
        "--backend",
        "kernel_only",
        "--format",
        "z3_proof",
        "--theory",
        "",
        "--conclusion",
        CLAIMED_CONCLUSION,
        "--body",
        STRUCTURAL_BODY,
    ]);
    assert!(!out.status.success());
}

#[test]
fn replay_rejects_unknown_format() {
    let out = run(&[
        "cert-replay",
        "replay",
        "--backend",
        "kernel_only",
        "--format",
        "garbage",
        "--theory",
        VALID_THEORY,
        "--conclusion",
        CLAIMED_CONCLUSION,
        "--body",
        STRUCTURAL_BODY,
    ]);
    assert!(!out.status.success());
}

#[test]
fn replay_rejects_unknown_output() {
    let out = run(&[
        "cert-replay",
        "replay",
        "--backend",
        "kernel_only",
        "--format",
        "z3_proof",
        "--theory",
        VALID_THEORY,
        "--conclusion",
        CLAIMED_CONCLUSION,
        "--body",
        STRUCTURAL_BODY,
        "--output",
        "yaml",
    ]);
    assert!(!out.status.success());
}

// ─────────────────────────────────────────────────────────────────────
// Acceptance pin
// ─────────────────────────────────────────────────────────────────────

#[test]
fn task_81_kernel_check_is_unbypassable_trust_anchor() {
    // Pin the contract: the kernel-only check rejects a tampered
    // cert even when the user requested an unimplemented external adapter.
    // Integrity rejection is necessary, but alone cannot remove solvers from the TCB.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("cert.json");
    let body = format!(
        r#"{{
            "format": "z3_proof",
            "theory": "{}",
            "conclusion": "{}",
            "body": "malicious body the solver claims to have proven",
            "body_hash": "{}",
            "source_solver": "z3-evil"
        }}"#,
        VALID_THEORY,
        CLAIMED_CONCLUSION,
        hex_blake3("original-body")
    );
    fs::write(&path, body).unwrap();
    let out = run(&[
        "cert-replay",
        "replay",
        "--backend",
        "z3",
        "--cert",
        path.to_str().unwrap(),
    ]);
    assert!(
        !out.status.success(),
        "kernel-only check must reject tampered cert regardless of --backend choice"
    );
}
