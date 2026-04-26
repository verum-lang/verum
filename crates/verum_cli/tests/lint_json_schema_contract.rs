//! Schema contract for `verum lint --format json`.
//!
//! External consumers (CI gate scripts, dashboards, custom report
//! converters) parse this stream as NDJSON. The rule that protects
//! them: every line is one well-formed JSON object carrying a
//! `schema_version` field, and every documented field is present
//! and well-typed.
//!
//! When the schema_version bumps, this test file changes in lockstep
//! and consumers see the version field flip — that's the
//! deprecation signal. Adding fields without bumping the version is
//! safe (additive change); renaming or removing fields requires a
//! version bump and a deprecation period.

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;
use tempfile::TempDir;

const EXPECTED_SCHEMA_VERSION: u64 = 1;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_verum")
}

/// Per-test isolated fixture. Returns the TempDir to keep its
/// lifetime tied to the caller — dropping it deletes the tree, so
/// parallel tests can't trip over each other's files.
fn make_fixture() -> TempDir {
    let dir = tempfile::Builder::new()
        .prefix("verum_lint_json_schema_")
        .tempdir()
        .expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).expect("create src");
    std::fs::write(
        root.join("verum.toml"),
        "[package]\nname = \"json_schema\"\nversion = \"0.1.0\"\n",
    )
    .expect("manifest");
    std::fs::write(
        root.join("src").join("main.vr"),
        "fn main() {\n    let x = Box::new(5);\n    // TODO: tighten\n}\n",
    )
    .expect("fixture");
    dir
}

fn lint(dir: &PathBuf) -> String {
    let out = Command::new(binary())
        .args(["lint", "--no-cache", "--format", "json"])
        .current_dir(dir)
        .output()
        .expect("lint spawn");
    String::from_utf8(out.stdout).expect("UTF-8")
}

fn parsed_lines(stdout: &str) -> Vec<Value> {
    stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<Value>(l).expect("each line is valid JSON"))
        .collect()
}

#[test]
fn every_line_is_an_object() {
    let dir = make_fixture();
    let out = lint(&dir.path().to_path_buf());
    for v in parsed_lines(&out) {
        assert!(v.is_object(), "each line must be an object: {v}");
    }
}

#[test]
fn every_line_carries_schema_version_one() {
    let dir = make_fixture();
    let out = lint(&dir.path().to_path_buf());
    let lines = parsed_lines(&out);
    assert!(!lines.is_empty(), "fixture must produce at least one issue");
    for v in &lines {
        let sv = v
            .get("schema_version")
            .expect("schema_version field present");
        assert_eq!(sv.as_u64(), Some(EXPECTED_SCHEMA_VERSION));
    }
}

#[test]
fn every_line_has_event_lint() {
    let dir = make_fixture();
    let out = lint(&dir.path().to_path_buf());
    for v in parsed_lines(&out) {
        assert_eq!(v["event"], Value::String("lint".into()));
    }
}

#[test]
fn every_required_field_is_present_and_well_typed() {
    let dir = make_fixture();
    let out = lint(&dir.path().to_path_buf());
    for v in parsed_lines(&out) {
        // Strings.
        for f in &["rule", "level", "file", "message"] {
            assert!(
                v[*f].is_string(),
                "field `{f}` must be a string in: {v}"
            );
        }
        // Integers.
        for f in &["line", "column"] {
            assert!(
                v[*f].is_u64(),
                "field `{f}` must be a non-negative integer in: {v}"
            );
        }
        // Boolean.
        assert!(
            v["fixable"].is_boolean(),
            "field `fixable` must be a bool in: {v}"
        );
    }
}

#[test]
fn level_value_is_in_known_set() {
    let dir = make_fixture();
    let out = lint(&dir.path().to_path_buf());
    for v in parsed_lines(&out) {
        let lvl = v["level"].as_str().expect("level is string");
        assert!(
            matches!(lvl, "error" | "warning" | "info" | "hint" | "off"),
            "unexpected level value: {lvl}"
        );
    }
}

#[test]
fn suggestion_present_iff_fixable() {
    let dir = make_fixture();
    let out = lint(&dir.path().to_path_buf());
    for v in parsed_lines(&out) {
        let fixable = v["fixable"].as_bool().expect("fixable is bool");
        let has_suggestion = v.get("suggestion").is_some();
        if fixable {
            assert!(
                has_suggestion,
                "fixable issue must include suggestion: {v}"
            );
        }
        // The reverse direction (`suggestion implies fixable`) is
        // not strictly required — a suggestion can be a hint that
        // doesn't have an autofix yet.
    }
}

// ============================================================
// Streaming-path contract — `--format json` emits each file's
// diagnostics as they finish. The schema is the same; the only
// observable difference is the order isn't pre-sorted by file
// (consumers sort post-hoc). We verify the contract by running
// on a multi-file fixture and confirming every line still parses
// as a schema-conforming object and no issue is emitted twice.
// ============================================================

fn make_multifile_fixture() -> TempDir {
    let dir = tempfile::Builder::new()
        .prefix("verum_lint_stream_")
        .tempdir()
        .expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).expect("create src");
    std::fs::write(
        root.join("verum.toml"),
        "[package]\nname = \"stream\"\nversion = \"0.1.0\"\n",
    )
    .expect("manifest");
    for i in 0..6 {
        std::fs::write(
            root.join("src").join(format!("a{i}.vr")),
            format!(
                "fn entry_{i}() {{ let _x = Box::new({i}); }}\n// TODO: clean\n"
            ),
        )
        .expect("write fixture");
    }
    dir
}

#[test]
fn streaming_json_emits_each_issue_once_and_well_formed() {
    let dir = make_multifile_fixture();
    let out = lint(&dir.path().to_path_buf());
    let lines = parsed_lines(&out);
    assert!(
        lines.len() >= 6,
        "expected at least one diagnostic per fixture file, got {} lines:\n{out}",
        lines.len()
    );
    for v in &lines {
        assert!(v.is_object(), "line is not an object: {v}");
        assert_eq!(
            v.get("schema_version").and_then(|x| x.as_u64()),
            Some(EXPECTED_SCHEMA_VERSION)
        );
    }
    // Stable issue identity: (rule, file, line, column).
    let mut seen: std::collections::HashSet<(String, String, u64, u64)> =
        std::collections::HashSet::new();
    for v in &lines {
        let key = (
            v["rule"].as_str().unwrap_or("").to_string(),
            v["file"].as_str().unwrap_or("").to_string(),
            v["line"].as_u64().unwrap_or(0),
            v["column"].as_u64().unwrap_or(0),
        );
        assert!(
            seen.insert(key.clone()),
            "issue emitted twice: {:?}\nfull stream:\n{out}",
            key
        );
    }
}
