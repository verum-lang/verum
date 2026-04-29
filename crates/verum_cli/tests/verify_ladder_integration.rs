//! End-to-end integration tests for `verum verify --ladder`.
//!
//! Spawns the actual `verum` binary as a child process and validates
//! the entire wiring chain:
//!
//!   `verum verify --ladder` (CLI clap) →
//!     `commands::verify_ladder::run_verify_ladder` →
//!       `verum_verification::ladder_dispatch::DefaultLadderDispatcher` →
//!         per-theorem `LadderVerdict` (Closed / Open / DispatchPending / Timeout)
//!
//! Together with the 11 in-handler unit tests in
//! `commands::verify_ladder::tests`, this proves the dispatcher is
//! actually consumable from a shell, not just from Rust unit tests.
//! That was the critical gap #86 was opened to close.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::TempDir;

/// Locate the freshly-built `verum` binary via Cargo's canonical env var.
fn verum_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_verum"))
}

/// Create a tempdir-rooted Verum project with the given `main.vr` body
/// and a minimal `Verum.toml`. Returns `(TempDir, project_dir)`; keep
/// `TempDir` alive for the test's lifetime.
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
    Command::new(verum_bin())
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("spawn verum CLI")
}

// ─────────────────────────────────────────────────────────────────────
// Plain output (default format)
// ─────────────────────────────────────────────────────────────────────

#[test]
fn ladder_runtime_strategy_dispatches_to_closed_verdict() {
    // `Runtime` is one of the two V0 implemented strategies in
    // DefaultLadderDispatcher → must produce a `closed` verdict.
    let (_temp, dir) = create_project(
        "ladder_runtime",
        r#"@verify(runtime)
theorem t_runtime()
    ensures true
    proof by auto;

public fn main() {}
"#,
    );
    let out = run_verum(&["verify", "--ladder"], &dir);
    assert!(
        out.status.success(),
        "verify --ladder should succeed on a `runtime` theorem; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("closed"),
        "expected `closed` verdict in output: {stdout}"
    );
    assert!(
        stdout.contains("t_runtime"),
        "expected theorem name `t_runtime` in output: {stdout}"
    );
}

#[test]
fn ladder_static_strategy_dispatches_to_closed_verdict() {
    // `Static` is the second V0 implemented strategy.
    let (_temp, dir) = create_project(
        "ladder_static",
        r#"@verify(static)
theorem t_static()
    ensures true
    proof by auto;

public fn main() {}
"#,
    );
    let out = run_verum(&["verify", "--ladder"], &dir);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("closed"),
        "expected `closed` verdict in output: {stdout}"
    );
}

#[test]
fn ladder_formal_strategy_dispatches_to_pending_not_failure() {
    // `Formal` (ω) is V0-pending in the dispatcher → emits
    // `dispatch_pending`, NOT a hard failure.  Pending is advisory
    // because backends ship in V1+; it must NOT cause non-zero exit.
    let (_temp, dir) = create_project(
        "ladder_formal",
        r#"@verify(formal)
theorem t_formal()
    ensures true
    proof by auto;

public fn main() {}
"#,
    );
    let out = run_verum(&["verify", "--ladder"], &dir);
    assert!(
        out.status.success(),
        "DispatchPending must NOT be a hard failure (exit 0); stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("dispatch_pending"),
        "expected `dispatch_pending` verdict: {stdout}"
    );
}

#[test]
fn ladder_summary_includes_totals_block() {
    let (_temp, dir) = create_project(
        "ladder_totals",
        r#"@verify(runtime)
theorem t_a()
    ensures true
    proof by auto;

@verify(static)
theorem t_b()
    ensures true
    proof by auto;

@verify(formal)
theorem t_c()
    ensures true
    proof by auto;

public fn main() {}
"#,
    );
    let out = run_verum(&["verify", "--ladder"], &dir);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Verdict totals:"), "missing totals block: {stdout}");
    assert!(stdout.contains("closed"), "missing closed counter: {stdout}");
    assert!(stdout.contains("dispatch_pending"), "missing pending counter: {stdout}");
    assert!(stdout.contains("total"), "missing total counter: {stdout}");
}

#[test]
fn ladder_mixed_project_runs_clean() {
    // Multiple theorems with mixed strategies — none is a hard
    // failure, so exit 0.  Pins the success contract for the
    // realistic stdlib shape (mostly `formal`-pending today).
    let (_temp, dir) = create_project(
        "ladder_mixed",
        r#"@verify(runtime)
theorem t_runtime_a()
    ensures true
    proof by auto;

@verify(runtime)
theorem t_runtime_b()
    ensures true
    proof by auto;

@verify(static)
theorem t_static_a()
    ensures true
    proof by auto;

@verify(formal)
theorem t_formal_a()
    ensures true
    proof by auto;

@verify(thorough)
theorem t_thorough_a()
    ensures true
    proof by auto;

@verify(certified)
theorem t_certified_a()
    ensures true
    proof by auto;

public fn main() {}
"#,
    );
    let out = run_verum(&["verify", "--ladder"], &dir);
    assert!(
        out.status.success(),
        "mixed-strategy project should exit 0; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// ─────────────────────────────────────────────────────────────────────
// JSON output
// ─────────────────────────────────────────────────────────────────────

#[test]
fn ladder_json_output_is_well_formed() {
    let (_temp, dir) = create_project(
        "ladder_json",
        r#"@verify(runtime)
theorem t_a()
    ensures true
    proof by auto;

@verify(formal)
theorem t_b()
    ensures true
    proof by auto;

public fn main() {}
"#,
    );
    let out = run_verum(
        &["verify", "--ladder", "--ladder-format", "json"],
        &dir,
    );
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);

    // Round-trip through serde_json — proves output is valid JSON.
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("output must be valid JSON: {e}\nstdout={stdout}"));

    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(parsed["theorem_count"], 2);

    let totals = &parsed["totals"];
    assert_eq!(totals["closed"], 1, "one runtime theorem → 1 closed");
    assert_eq!(totals["dispatch_pending"], 1, "one formal theorem → 1 pending");
    assert_eq!(totals["open"], 0);
    assert_eq!(totals["timeout"], 0);

    let theorems = parsed["theorems"]
        .as_array()
        .expect("theorems must be an array");
    assert_eq!(theorems.len(), 2);

    // Each entry must carry the canonical fields.
    for t in theorems {
        assert!(t["kind"].is_string());
        assert!(t["name"].is_string());
        assert!(t["file"].is_string());
        assert!(t["strategy"].is_string());
        assert!(t["verdict"].is_string());
        assert!(t["detail"].is_string());
    }
}

#[test]
fn ladder_json_records_each_strategy_and_verdict_pair() {
    let (_temp, dir) = create_project(
        "ladder_json_pairs",
        r#"@verify(runtime)
theorem t_r()
    ensures true
    proof by auto;

@verify(static)
theorem t_s()
    ensures true
    proof by auto;

@verify(certified)
theorem t_c()
    ensures true
    proof by auto;

public fn main() {}
"#,
    );
    let out = run_verum(
        &["verify", "--ladder", "--ladder-format", "json"],
        &dir,
    );
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let theorems = parsed["theorems"].as_array().unwrap();

    // Map (strategy → verdict) for assertion.
    let mut got = std::collections::HashMap::<String, String>::new();
    for t in theorems {
        got.insert(
            t["strategy"].as_str().unwrap().to_string(),
            t["verdict"].as_str().unwrap().to_string(),
        );
    }
    assert_eq!(got.get("runtime").map(String::as_str), Some("closed"));
    assert_eq!(got.get("static").map(String::as_str), Some("closed"));
    assert_eq!(
        got.get("certified").map(String::as_str),
        Some("dispatch_pending"),
        "V0 dispatcher MUST surface `certified` as pending (not silently fall through to coarser `formal`)"
    );
}

// ─────────────────────────────────────────────────────────────────────
// Format validation + exit-code contract
// ─────────────────────────────────────────────────────────────────────

#[test]
fn ladder_rejects_unknown_format() {
    let (_temp, dir) = create_project(
        "ladder_bad_format",
        r#"public fn main() {}
"#,
    );
    let out = run_verum(
        &["verify", "--ladder", "--ladder-format", "yaml"],
        &dir,
    );
    assert!(
        !out.status.success(),
        "unknown --ladder-format must produce non-zero exit"
    );
}

#[test]
fn ladder_no_theorems_exits_zero() {
    // Empty project (no @verify annotations) → 0 records, 0 hard
    // failures → exit 0. Documents the "no work" path.
    let (_temp, dir) = create_project(
        "ladder_empty",
        r#"public fn main() {}
"#,
    );
    let out = run_verum(&["verify", "--ladder"], &dir);
    assert!(
        out.status.success(),
        "empty project must exit 0; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Either plain or JSON output, both ship a totals block — the
    // plain-format default emits "Verdict totals:" or "total".
    assert!(
        stdout.contains("Verdict totals") || stdout.contains("total"),
        "summary block missing on empty project: {stdout}"
    );
}

// ─────────────────────────────────────────────────────────────────────
// Walks lemma + corollary kinds, not just theorem
// ─────────────────────────────────────────────────────────────────────

#[test]
fn ladder_walks_lemma_kind() {
    let (_temp, dir) = create_project(
        "ladder_lemma",
        r#"@verify(runtime)
lemma l_a()
    ensures true
    proof by auto;

public fn main() {}
"#,
    );
    let out = run_verum(
        &["verify", "--ladder", "--ladder-format", "json"],
        &dir,
    );
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(parsed["theorem_count"], 1);
    let theorems = parsed["theorems"].as_array().unwrap();
    assert_eq!(theorems[0]["kind"], "lemma");
    assert_eq!(theorems[0]["name"], "l_a");
}
