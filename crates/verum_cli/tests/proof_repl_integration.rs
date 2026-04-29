//! End-to-end integration tests for `verum proof-repl`.

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

// ─────────────────────────────────────────────────────────────────────
// batch — happy paths
// ─────────────────────────────────────────────────────────────────────

#[test]
fn batch_inline_commands_succeed() {
    let out = run(&[
        "proof-repl", "batch",
        "--theorem", "thm",
        "--goal", "P",
        "--cmd", "intro",
        "--cmd", "auto",
    ]);
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("REPL transcript"));
    assert!(stdout.contains("apply  intro"));
    assert!(stdout.contains("apply  auto"));
}

#[test]
fn batch_with_lemma_apply_succeeds() {
    let out = run(&[
        "proof-repl", "batch",
        "--theorem", "thm",
        "--goal", "P",
        "--lemma", "foo_lemma:::P",
        "--cmd", "apply foo_lemma",
    ]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("apply foo_lemma"));
}

#[test]
fn batch_undo_redo_navigation() {
    let out = run(&[
        "proof-repl", "batch",
        "--theorem", "thm",
        "--goal", "P",
        "--cmd", "intro",
        "--cmd", "auto",
        "--cmd", "undo",
        "--cmd", "redo",
        "--cmd", "status",
    ]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("undo"));
    assert!(stdout.contains("redo"));
    assert!(stdout.contains("status"));
}

#[test]
fn batch_hint_returns_suggestions() {
    let out = run(&[
        "proof-repl", "batch",
        "--theorem", "thm",
        "--goal", "P",
        "--cmd", "hint",
    ]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("hint"));
    assert!(stdout.contains("suggestion"));
}

#[test]
fn batch_visualise_emits_dot() {
    let out = run(&[
        "proof-repl", "batch",
        "--theorem", "thm",
        "--goal", "P",
        "--cmd", "intro",
        "--cmd", "visualise",
    ]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("digraph proof_tree"));
}

#[test]
fn batch_reads_commands_from_file() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("script.repl");
    fs::write(
        &path,
        "# header comment\nintro\n\nauto\nstatus\n",
    )
    .unwrap();
    let out = run(&[
        "proof-repl", "batch",
        "--theorem", "thm",
        "--goal", "P",
        "--commands", path.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("apply  intro"));
    assert!(stdout.contains("apply  auto"));
}

#[test]
fn batch_combines_file_and_inline() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("script.repl");
    fs::write(&path, "intro\n").unwrap();
    let out = run(&[
        "proof-repl", "batch",
        "--theorem", "thm",
        "--goal", "P",
        "--commands", path.to_str().unwrap(),
        "--cmd", "auto",
    ]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("apply  intro"));
    assert!(stdout.contains("apply  auto"));
}

// ─────────────────────────────────────────────────────────────────────
// batch — rejection produces non-zero exit
// ─────────────────────────────────────────────────────────────────────

#[test]
fn batch_rejection_produces_non_zero_exit() {
    let out = run(&[
        "proof-repl", "batch",
        "--theorem", "thm",
        "--goal", "P",
        "--cmd", "xyz_garbage_step",
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("rejected") || stderr.contains("REPL"),
        "stderr should mention rejection: {stderr}"
    );
}

// ─────────────────────────────────────────────────────────────────────
// batch — JSON output
// ─────────────────────────────────────────────────────────────────────

#[test]
fn batch_json_well_formed() {
    let out = run(&[
        "proof-repl", "batch",
        "--theorem", "thm",
        "--goal", "P",
        "--cmd", "intro",
        "--cmd", "status",
        "--format", "json",
    ]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(parsed["count"], 2);
    let responses = parsed["responses"].as_array().unwrap();
    assert_eq!(responses.len(), 2);
    let final_state = &parsed["final_state"];
    assert!(final_state["history_depth"].is_number());
}

#[test]
fn batch_json_summary_groups_by_kind() {
    let out = run(&[
        "proof-repl", "batch",
        "--theorem", "thm",
        "--goal", "P",
        "--cmd", "intro",
        "--cmd", "status",
        "--cmd", "hint",
        "--format", "json",
    ]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let summary = &parsed["summary"];
    assert_eq!(summary["Accepted"], 1);
    assert_eq!(summary["Status"], 1);
    assert_eq!(summary["Hints"], 1);
}

// ─────────────────────────────────────────────────────────────────────
// tree
// ─────────────────────────────────────────────────────────────────────

#[test]
fn tree_emits_dot_after_apply_sequence() {
    let out = run(&[
        "proof-repl", "tree",
        "--theorem", "thm",
        "--goal", "P",
        "--apply", "intro",
        "--apply", "auto",
    ]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("digraph proof_tree"));
    assert!(stdout.contains("step_1"));
    assert!(stdout.contains("step_2"));
    assert!(stdout.contains("intro"));
    assert!(stdout.contains("auto"));
}

#[test]
fn tree_rejection_produces_non_zero_exit() {
    let out = run(&[
        "proof-repl", "tree",
        "--theorem", "thm",
        "--goal", "P",
        "--apply", "xyz_garbage",
    ]);
    assert!(!out.status.success());
}

#[test]
fn tree_with_no_steps_emits_root_only() {
    let out = run(&[
        "proof-repl", "tree",
        "--theorem", "thm",
        "--goal", "P",
    ]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("digraph proof_tree"));
    assert!(stdout.contains("goal_root"));
    // No step_N nodes when no tactics applied.
    assert!(!stdout.contains("step_1"));
}

// ─────────────────────────────────────────────────────────────────────
// validation
// ─────────────────────────────────────────────────────────────────────

#[test]
fn batch_rejects_empty_theorem() {
    let out = run(&[
        "proof-repl", "batch", "--theorem", "", "--goal", "P",
    ]);
    assert!(!out.status.success());
}

#[test]
fn batch_rejects_empty_goal() {
    let out = run(&[
        "proof-repl", "batch", "--theorem", "t", "--goal", "",
    ]);
    assert!(!out.status.success());
}

#[test]
fn batch_rejects_unknown_format() {
    let out = run(&[
        "proof-repl", "batch",
        "--theorem", "t", "--goal", "P",
        "--format", "yaml",
    ]);
    assert!(!out.status.success());
}

#[test]
fn batch_rejects_malformed_lemma() {
    let out = run(&[
        "proof-repl", "batch",
        "--theorem", "t", "--goal", "P",
        "--lemma", "bare-no-separator",
    ]);
    assert!(!out.status.success());
}

#[test]
fn batch_command_script_error_carries_line_number() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("script.repl");
    fs::write(&path, "intro\nhint not-a-number\nauto\n").unwrap();
    let out = run(&[
        "proof-repl", "batch",
        "--theorem", "t", "--goal", "P",
        "--commands", path.to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("line 2"),
        "stderr should cite line number: {stderr}"
    );
}

// ─────────────────────────────────────────────────────────────────────
// Acceptance pin
// ─────────────────────────────────────────────────────────────────────

#[test]
fn task_75_full_session_navigation_and_visualisation() {
    // Apply two steps, undo one, redo, visualise — every command
    // must run cleanly and the final visualisation must reflect
    // the accepted history.
    let out = run(&[
        "proof-repl", "batch",
        "--theorem", "thm",
        "--goal", "P",
        "--cmd", "intro",
        "--cmd", "auto",
        "--cmd", "undo",
        "--cmd", "redo",
        "--cmd", "visualise",
    ]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("digraph proof_tree"));
    // Both apply steps should be in the final tree.
    assert!(stdout.contains("intro"));
    assert!(stdout.contains("auto"));
}
