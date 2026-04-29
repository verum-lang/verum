//! End-to-end integration tests for `verum benchmark`.

use std::path::PathBuf;
use std::process::{Command, Output};

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
// run — single-system suite execution
// ─────────────────────────────────────────────────────────────────────

#[test]
fn run_single_system_smoke() {
    let out = run(&[
        "benchmark", "run",
        "--system", "verum",
        "--suite-name", "test-suite",
        "--theorem", "addnC",
        "--theorem", "addn0",
    ]);
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Benchmark transcript"));
}

#[test]
fn run_json_well_formed() {
    let out = run(&[
        "benchmark", "run",
        "--system", "coq",
        "--suite-name", "test",
        "--theorem", "thm1",
        "--format", "json",
    ]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let arr = parsed.as_array().unwrap();
    assert!(!arr.is_empty());
    for r in arr {
        assert_eq!(r["system"], "coq");
        assert_eq!(r["suite"], "test");
    }
}

#[test]
fn run_csv_format() {
    let out = run(&[
        "benchmark", "run",
        "--system", "verum",
        "--suite-name", "s",
        "--theorem", "t",
        "--format", "csv",
    ]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("system,theorem,metric,value,timestamp"));
}

#[test]
fn run_markdown_format() {
    let out = run(&[
        "benchmark", "run",
        "--system", "verum",
        "--suite-name", "s",
        "--theorem", "t",
        "--format", "markdown",
    ]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("# Benchmark transcript"));
    assert!(stdout.contains("| System "));
}

// ─────────────────────────────────────────────────────────────────────
// compare — head-to-head matrix
// ─────────────────────────────────────────────────────────────────────

#[test]
fn compare_default_runs_all_systems() {
    let out = run(&[
        "benchmark", "compare",
        "--suite-name", "s",
        "--theorem", "t",
        "--format", "json",
    ]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(parsed["schema_version"], 1);
    let leaders = parsed["leaders"].as_array().unwrap();
    // At least one leader per metric category that the canned data
    // populates.
    assert!(!leaders.is_empty());
}

#[test]
fn compare_explicit_two_systems_includes_both() {
    let out = run(&[
        "benchmark", "compare",
        "--system", "verum",
        "--system", "coq",
        "--suite-name", "s",
        "--theorem", "t",
        "--format", "json",
    ]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let matrix = parsed["matrix"].as_array().unwrap();
    let systems: std::collections::BTreeSet<&str> = matrix
        .iter()
        .map(|e| e["system"].as_str().unwrap())
        .collect();
    assert!(systems.contains("verum"));
    assert!(systems.contains("coq"));
    assert!(!systems.contains("lean4")); // not requested
}

#[test]
fn compare_markdown_emits_leader_marker() {
    let out = run(&[
        "benchmark", "compare",
        "--system", "verum",
        "--system", "coq",
        "--suite-name", "s",
        "--theorem", "t",
        "--format", "markdown",
    ]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("# Benchmark comparison"));
    // Verum leads kernel_loc → leader marker should appear.
    assert!(stdout.contains("⭐"));
}

#[test]
fn compare_csv_lists_every_metric_x_system_pair() {
    let out = run(&[
        "benchmark", "compare",
        "--system", "verum",
        "--system", "coq",
        "--suite-name", "s",
        "--theorem", "t",
        "--format", "csv",
    ]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("metric,system,value,is_leader\n"));
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(lines.len() > 1);
}

#[test]
fn compare_verum_leads_kernel_loc() {
    let out = run(&[
        "benchmark", "compare",
        "--system", "verum",
        "--system", "coq",
        "--system", "lean4",
        "--suite-name", "s",
        "--format", "json",
    ]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let leaders = parsed["leaders"].as_array().unwrap();
    let kernel_leader = leaders
        .iter()
        .find(|e| e["metric"] == "kernel_loc")
        .unwrap();
    assert_eq!(kernel_leader["leader"], "verum");
}

#[test]
fn compare_only_verum_leads_llm_acceptance() {
    let out = run(&[
        "benchmark", "compare",
        "--suite-name", "s",
        "--format", "json",
    ]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let leaders = parsed["leaders"].as_array().unwrap();
    let llm_leader = leaders
        .iter()
        .find(|e| e["metric"] == "llm_acceptance_percent")
        .unwrap();
    assert_eq!(llm_leader["leader"], "verum");
}

// ─────────────────────────────────────────────────────────────────────
// metrics
// ─────────────────────────────────────────────────────────────────────

#[test]
fn metrics_lists_nine_canonical_metrics() {
    let out = run(&["benchmark", "metrics", "--format", "json"]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(parsed["count"], 9);
    let ms = parsed["metrics"].as_array().unwrap();
    assert_eq!(ms.len(), 9);
}

#[test]
fn metrics_markdown_table() {
    let out = run(&["benchmark", "metrics", "--format", "markdown"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("# Benchmark metrics"));
    assert!(stdout.contains("kernel_loc"));
    assert!(stdout.contains("higher is better"));
    assert!(stdout.contains("lower is better"));
}

#[test]
fn metrics_csv() {
    let out = run(&["benchmark", "metrics", "--format", "csv"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("metric,higher_is_better\n"));
}

// ─────────────────────────────────────────────────────────────────────
// validation
// ─────────────────────────────────────────────────────────────────────

#[test]
fn run_rejects_unknown_system() {
    let out = run(&[
        "benchmark", "run",
        "--system", "garbage",
        "--suite-name", "s",
    ]);
    assert!(!out.status.success());
}

#[test]
fn run_rejects_empty_suite_name() {
    let out = run(&[
        "benchmark", "run",
        "--system", "verum",
        "--suite-name", "",
    ]);
    assert!(!out.status.success());
}

#[test]
fn run_rejects_unknown_format() {
    let out = run(&[
        "benchmark", "run",
        "--system", "verum",
        "--suite-name", "s",
        "--format", "yaml",
    ]);
    assert!(!out.status.success());
}

#[test]
fn compare_rejects_unknown_system() {
    let out = run(&[
        "benchmark", "compare",
        "--system", "garbage",
        "--suite-name", "s",
    ]);
    assert!(!out.status.success());
}

// ─────────────────────────────────────────────────────────────────────
// Acceptance pin
// ─────────────────────────────────────────────────────────────────────

#[test]
fn task_83_seven_canonical_categories_all_reachable() {
    // §1-§7 of #83 each map to a metric.  Verify every metric is
    // reachable through the CLI.
    let out = run(&["benchmark", "metrics", "--format", "json"]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let names: std::collections::BTreeSet<&str> = parsed["metrics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    for required in [
        "kernel_loc",                  // §1
        "lines_per_second",            // §2
        "peak_rss_bytes",              // §3
        "cross_format_exports",        // §4
        "tactic_coverage_percent",     // §5
        "trust_diversification_count", // §6
        "llm_acceptance_percent",      // §7
    ] {
        assert!(
            names.contains(required),
            "metric {} missing from `verum benchmark metrics` output",
            required
        );
    }
}
