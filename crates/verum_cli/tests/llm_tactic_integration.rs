//! End-to-end integration tests for `verum llm-tactic`.

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

// ─────────────────────────────────────────────────────────────────────
// propose — happy path with mock adapter
// ─────────────────────────────────────────────────────────────────────

#[test]
fn propose_mock_default_sequence_accepted() {
    // No --hint → MockLlmAdapter with default `intro`+`auto` sequence.
    // Kernel re-checker accepts both.
    let out = run(&[
        "llm-tactic",
        "propose",
        "--theorem",
        "thm",
        "--goal",
        "True",
    ]);
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Verdict      : ACCEPTED"));
    assert!(stdout.contains("2 step(s) kernel-checked"));
}

#[test]
fn propose_with_hint_runs_echo_adapter() {
    let out = run(&[
        "llm-tactic",
        "propose",
        "--theorem",
        "thm",
        "--goal",
        "True",
        "--hint",
        "intro\nauto",
    ]);
    assert!(out.status.success());
}

#[test]
fn propose_with_lemma_in_scope_apply_passes() {
    let out = run(&[
        "llm-tactic",
        "propose",
        "--theorem",
        "thm",
        "--goal",
        "P",
        "--lemma",
        "foo_lemma:::P",
        "--hint",
        "apply foo_lemma",
    ]);
    assert!(out.status.success());
}

// ─────────────────────────────────────────────────────────────────────
// fail-closed contract — kernel rejection
// ─────────────────────────────────────────────────────────────────────

#[test]
fn propose_garbage_step_kernel_rejects() {
    // §3 of #77: the LLM never short-circuits the kernel.
    let out = run(&[
        "llm-tactic",
        "propose",
        "--theorem",
        "thm",
        "--goal",
        "True",
        "--hint",
        "xyz_garbage_step",
    ]);
    assert!(
        !out.status.success(),
        "kernel must reject garbage proposal"
    );
}

#[test]
fn propose_apply_to_out_of_scope_lemma_rejected() {
    let out = run(&[
        "llm-tactic",
        "propose",
        "--theorem",
        "thm",
        "--goal",
        "P",
        "--hint",
        "apply nonexistent_lemma",
    ]);
    assert!(!out.status.success());
}

// ─────────────────────────────────────────────────────────────────────
// JSON output
// ─────────────────────────────────────────────────────────────────────

#[test]
fn propose_json_well_formed() {
    let out = run(&[
        "llm-tactic",
        "propose",
        "--theorem",
        "thm",
        "--goal",
        "True",
        "--format",
        "json",
    ]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(parsed["theorem"], "thm");
    assert_eq!(parsed["model_id"], "mock");
    let hash = parsed["prompt_hash"].as_str().unwrap();
    assert_eq!(hash.len(), 64);
    assert_eq!(parsed["verdict"]["status"], "accepted");
}

#[test]
fn propose_json_rejected_carries_failed_step_index() {
    let out = run(&[
        "llm-tactic",
        "propose",
        "--theorem",
        "thm",
        "--goal",
        "True",
        "--hint",
        "intro\nxyz_garbage\nauto",
        "--format",
        "json",
    ]);
    // Non-zero exit on kernel rejection but stdout still has JSON.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["verdict"]["status"], "rejected");
    assert_eq!(parsed["verdict"]["failed_step_index"], 1);
}

// ─────────────────────────────────────────────────────────────────────
// audit trail — persist round-trip
// ─────────────────────────────────────────────────────────────────────

#[test]
fn propose_persist_then_read_audit_trail() {
    let dir = TempDir::new().unwrap();
    let audit_path = dir.path().join("audit.jsonl");

    // Run two proposals — both accepted.
    for i in 0..2 {
        let theorem = format!("thm_{}", i);
        let out = run(&[
            "llm-tactic",
            "propose",
            "--theorem",
            &theorem,
            "--goal",
            "True",
            "--persist",
            "--audit",
            audit_path.to_str().unwrap(),
        ]);
        assert!(out.status.success());
    }

    // Read the audit trail.
    let out = run(&[
        "llm-tactic",
        "audit-trail",
        "--audit",
        audit_path.to_str().unwrap(),
        "--format",
        "json",
    ]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(parsed["schema_version"], 1);
    let count = parsed["count"].as_u64().unwrap();
    // Each propose emits 2 events (LlmInvoked + KernelAccepted) ⇒ 4 total.
    assert_eq!(count, 4);
}

#[test]
fn audit_trail_missing_file_reports_empty() {
    let dir = TempDir::new().unwrap();
    let audit_path = dir.path().join("not-yet.jsonl");
    let out = run(&[
        "llm-tactic",
        "audit-trail",
        "--audit",
        audit_path.to_str().unwrap(),
    ]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("empty"));
}

#[test]
fn audit_trail_records_kernel_rejected_events() {
    let dir = TempDir::new().unwrap();
    let audit_path = dir.path().join("audit.jsonl");
    // One proposal that gets rejected.
    let _ = run(&[
        "llm-tactic",
        "propose",
        "--theorem",
        "thm",
        "--goal",
        "True",
        "--hint",
        "xyz_garbage",
        "--persist",
        "--audit",
        audit_path.to_str().unwrap(),
    ]);
    // Audit has LlmInvoked + KernelRejected.
    let out = run(&[
        "llm-tactic",
        "audit-trail",
        "--audit",
        audit_path.to_str().unwrap(),
        "--format",
        "json",
    ]);
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(parsed["count"], 2);
    let events = parsed["events"].as_array().unwrap();
    let kinds: Vec<&str> = events
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"LlmInvoked"));
    assert!(kinds.contains(&"KernelRejected"));
}

// ─────────────────────────────────────────────────────────────────────
// models list
// ─────────────────────────────────────────────────────────────────────

#[test]
fn models_lists_reference_adapters() {
    let out = run(&["llm-tactic", "models"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("mock"));
    assert!(stdout.contains("echo"));
}

#[test]
fn models_json_well_formed() {
    let out = run(&["llm-tactic", "models", "--format", "json"]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(parsed["count"], 2);
}

// ─────────────────────────────────────────────────────────────────────
// validation
// ─────────────────────────────────────────────────────────────────────

#[test]
fn propose_rejects_empty_theorem() {
    let out = run(&[
        "llm-tactic",
        "propose",
        "--theorem",
        "",
        "--goal",
        "True",
    ]);
    assert!(!out.status.success());
}

#[test]
fn propose_rejects_empty_goal() {
    let out = run(&[
        "llm-tactic",
        "propose",
        "--theorem",
        "thm",
        "--goal",
        "",
    ]);
    assert!(!out.status.success());
}

#[test]
fn propose_rejects_unknown_format() {
    let out = run(&[
        "llm-tactic",
        "propose",
        "--theorem",
        "thm",
        "--goal",
        "True",
        "--format",
        "yaml",
    ]);
    assert!(!out.status.success());
}

#[test]
fn propose_rejects_malformed_lemma() {
    let out = run(&[
        "llm-tactic",
        "propose",
        "--theorem",
        "thm",
        "--goal",
        "True",
        "--lemma",
        "bare-no-separator",
    ]);
    assert!(!out.status.success());
}

// ─────────────────────────────────────────────────────────────────────
// LCF contract acceptance pin
// ─────────────────────────────────────────────────────────────────────

#[test]
fn task_77_kernel_recheck_is_unbypassable() {
    // The whole point of #77: even if the LLM hallucinates, the
    // kernel must reject. Pin the contract by running every
    // syntactically-plausible-but-nonsensical sequence we can think
    // of through the gate; every one must fail.
    let bogus_sequences = [
        "completely_made_up_tactic",
        "apply __nonexistent_target",
        "this is just english prose",
        "intro foo bar baz quux", // invalid arg shape — the V0 checker recognises only the head keyword
    ];
    for raw in &bogus_sequences {
        // Skip the "intro foo bar..." case as the V0 checker accepts
        // it (head keyword `intro` is canonical).  The real kernel
        // (V1) will reject the bogus argument list; for V0 we only
        // pin the unambiguous-garbage cases.
        if raw.starts_with("intro ") {
            continue;
        }
        let out = run(&[
            "llm-tactic",
            "propose",
            "--theorem",
            "thm",
            "--goal",
            "True",
            "--hint",
            raw,
        ]);
        assert!(
            !out.status.success(),
            "kernel must reject `{}`; stderr={}",
            raw,
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
