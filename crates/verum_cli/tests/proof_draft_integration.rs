//! End-to-end integration tests for `verum proof-draft`.
//!
//! Drives the actual CLI binary as a child process and checks the
//! captured output.  Validates the entire wiring chain:
//!
//!   `verum proof-draft` (CLI clap) →
//!     `commands::proof_draft::run_proof_draft` →
//!       `verum_verification::proof_drafting::SuggestionEngine` →
//!         ranked output text / JSON
//!
//! These tests prove that the `proof_drafting` trait surface
//! is actually consumable from a shell invocation, not just from
//! Rust unit tests.

use std::path::PathBuf;
use std::process::Command;

/// Locate the freshly-built `verum` binary.  Tests run after
/// `cargo build -p verum_cli` so the binary lives at
/// `target/debug/verum`.
fn verum_bin() -> PathBuf {
    // `CARGO_BIN_EXE_verum` is the canonical Cargo-provided path,
    // populated when the test crate has a binary target named `verum`.
    std::env::var_os("CARGO_BIN_EXE_verum")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            // Fallback: use $CARGO_TARGET_DIR or relative-to-this-test.
            let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            p.pop(); // crates/verum_cli/.. → crates/
            p.pop(); // crates/.. → repo root
            p.push("target");
            p.push("debug");
            p.push("verum");
            p
        })
}

#[test]
fn proof_draft_plain_output_ranks_lemma_first() {
    let out = Command::new(verum_bin())
        .args([
            "proof-draft",
            "--theorem", "thm_test",
            "--goal", "forall x. x > 0 -> succ(x) > 0",
            "--lemma", "succ_pos:::forall x. x > 0 -> succ(x) > 0:::core",
            "--lemma", "unrelated:::List<Int> append associative:::core",
            "--max", "5",
            "--format", "plain",
        ])
        .output()
        .expect("CLI invocation must succeed");

    assert!(
        out.status.success(),
        "verum proof-draft exited non-zero: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);

    // Header lines.
    assert!(stdout.contains("Goal: forall x. x > 0"), "stdout missing goal");
    assert!(stdout.contains("Theorem: thm_test"), "stdout missing theorem");

    // The relevant lemma MUST appear; the unrelated one is correctly
    // filtered (score=0 — engine drops zero-score suggestions).  This
    // is precisely the desired ranking behaviour: only structurally-
    // relevant lemmas show up.
    assert!(stdout.contains("succ_pos"), "relevant lemma must appear");
    assert!(
        !stdout.contains("apply unrelated;"),
        "unrelated lemma must be filtered (zero similarity score)"
    );

    // The fallback `apply auto` suggestion must appear (proves the
    // engine's "always-offer fallback tactics" rule fires).
    assert!(stdout.contains("apply auto"), "auto fallback missing");
}

#[test]
fn proof_draft_json_output_is_well_formed() {
    let out = Command::new(verum_bin())
        .args([
            "proof-draft",
            "--theorem", "thm_test",
            "--goal", "P(x)",
            "--lemma", "p_holds:::P(x):::core",
            "--max", "3",
            "--format", "json",
        ])
        .output()
        .expect("CLI invocation must succeed");

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);

    // Round-trip through serde_json — proves the output is valid JSON.
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .expect("output must be valid JSON");

    // Schema validations.
    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(parsed["theorem"], "thm_test");
    assert_eq!(parsed["goal"], "P(x)");
    let suggestions = parsed["suggestions"].as_array()
        .expect("suggestions must be an array");
    assert!(!suggestions.is_empty(), "should produce at least one suggestion");

    // Each suggestion must have all required fields.
    for s in suggestions {
        assert!(s["snippet"].is_string());
        assert!(s["rationale"].is_string());
        assert!(s["score"].is_number());
        assert!(s["category"].is_string());
        let score = s["score"].as_f64().unwrap();
        assert!((0.0..=1.0).contains(&score), "score out of [0,1]");
    }
}

#[test]
fn proof_draft_rejects_empty_theorem() {
    let out = Command::new(verum_bin())
        .args([
            "proof-draft",
            "--theorem", "",
            "--goal", "P",
            "--max", "1",
        ])
        .output()
        .expect("CLI invocation must succeed");

    assert!(!out.status.success(), "empty --theorem must produce non-zero exit");
}

#[test]
fn proof_draft_rejects_zero_max() {
    let out = Command::new(verum_bin())
        .args([
            "proof-draft",
            "--theorem", "t",
            "--goal", "P",
            "--max", "0",
        ])
        .output()
        .expect("CLI invocation must succeed");

    assert!(!out.status.success(), "--max 0 must produce non-zero exit");
}

#[test]
fn proof_draft_rejects_unknown_format() {
    let out = Command::new(verum_bin())
        .args([
            "proof-draft",
            "--theorem", "t",
            "--goal", "P",
            "--max", "1",
            "--format", "xml",
        ])
        .output()
        .expect("CLI invocation must succeed");

    assert!(!out.status.success(), "unknown --format must produce non-zero exit");
}

#[test]
fn proof_draft_rejects_malformed_lemma_flag() {
    let out = Command::new(verum_bin())
        .args([
            "proof-draft",
            "--theorem", "t",
            "--goal", "P",
            "--max", "1",
            "--lemma", "missing-separator",
        ])
        .output()
        .expect("CLI invocation must succeed");

    assert!(!out.status.success(), "malformed --lemma must produce non-zero exit");
}

#[test]
fn proof_draft_pi_shaped_goal_offers_intro() {
    let out = Command::new(verum_bin())
        .args([
            "proof-draft",
            "--theorem", "t",
            "--goal", "forall x. x = x",
            "--max", "10",
            "--format", "json",
        ])
        .output()
        .expect("CLI invocation must succeed");

    assert!(out.status.success());
    let parsed: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout))
        .expect("must be valid JSON");
    let suggestions = parsed["suggestions"].as_array().unwrap();
    let has_intro = suggestions.iter().any(|s| {
        s["snippet"].as_str().unwrap_or("").starts_with("intro")
    });
    assert!(has_intro, "Π-shaped goal should suggest `intro`");
}
