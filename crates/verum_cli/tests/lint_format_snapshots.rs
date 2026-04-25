//! Format snapshot tests for `verum lint`.
//!
//! Each format renders a fixed list of issues and we assert that
//! every documented field shows up in the expected place. These
//! lock down the schema so a future formatting tweak that breaks a
//! consumer (a CI parser, an LSP client, GitHub Code Scanning) is
//! caught at PR time, not after deploy.
//!
//! The tests are *structural*, not byte-for-byte snapshot diffs —
//! we assert the things that actually matter (well-formedness,
//! every field present, schema version, level mapping) instead of
//! locking in trivia like whitespace.

use std::path::PathBuf;

use verum_cli::commands::lint::{
    format_issue_gha, format_issue_json, format_sarif, format_tap, LintIssue, LintLevel,
};
use verum_common::Text;

fn sample_issues() -> Vec<LintIssue> {
    vec![
        LintIssue {
            rule: "deprecated-syntax",
            level: LintLevel::Error,
            file: PathBuf::from("src/main.vr"),
            line: 4,
            column: 13,
            message: "use `Heap(x)` instead of `Box::new(x)`".to_string(),
            suggestion: Some(Text::from("Heap(x)".to_string())),
            fixable: true,
        },
        LintIssue {
            rule: "todo-in-code",
            level: LintLevel::Warning,
            file: PathBuf::from("src/auth.vr"),
            line: 17,
            column: 5,
            message: "TODO comment in code".to_string(),
            suggestion: None,
            fixable: false,
        },
        LintIssue {
            rule: "shadow-binding",
            level: LintLevel::Info,
            file: PathBuf::from("src/handler.vr"),
            line: 42,
            column: 9,
            message: "Variable `x` shadows previous binding".to_string(),
            suggestion: None,
            fixable: false,
        },
    ]
}

// ============================================================
// JSON (NDJSON) — schema_version: 1 contract
// ============================================================

#[test]
fn json_emits_schema_version() {
    let issue = &sample_issues()[0];
    let line = format_issue_json(issue);
    assert!(
        line.contains("\"schema_version\":1"),
        "missing schema_version in: {line}"
    );
}

#[test]
fn json_emits_event_lint() {
    let issue = &sample_issues()[0];
    let line = format_issue_json(issue);
    assert!(line.contains("\"event\":\"lint\""), "got: {line}");
}

#[test]
fn json_emits_all_required_fields() {
    let issue = &sample_issues()[0];
    let line = format_issue_json(issue);
    for field in &[
        "\"rule\":",
        "\"level\":",
        "\"file\":",
        "\"line\":",
        "\"column\":",
        "\"message\":",
        "\"fixable\":",
    ] {
        assert!(line.contains(field), "missing {field} in: {line}");
    }
}

#[test]
fn json_emits_suggestion_only_when_fixable() {
    let issues = sample_issues();
    let with_fix = format_issue_json(&issues[0]);
    let without = format_issue_json(&issues[1]);
    assert!(with_fix.contains("\"suggestion\":"), "got: {with_fix}");
    assert!(!without.contains("\"suggestion\":"), "got: {without}");
}

#[test]
fn json_levels_lowercase() {
    let issues = sample_issues();
    assert!(format_issue_json(&issues[0]).contains("\"level\":\"error\""));
    assert!(format_issue_json(&issues[1]).contains("\"level\":\"warning\""));
    assert!(format_issue_json(&issues[2]).contains("\"level\":\"info\""));
}

#[test]
fn json_is_a_single_line() {
    for issue in sample_issues() {
        let line = format_issue_json(&issue);
        assert!(
            !line.contains('\n'),
            "JSON output must be one line per issue: {line}"
        );
    }
}

#[test]
fn json_escapes_quotes_in_message() {
    let issue = LintIssue {
        rule: "todo-in-code",
        level: LintLevel::Warning,
        file: PathBuf::from("a.vr"),
        line: 1,
        column: 1,
        message: "message with \"quote\"".to_string(),
        suggestion: None,
        fixable: false,
    };
    let line = format_issue_json(&issue);
    assert!(line.contains("\\\"quote\\\""), "got: {line}");
}

// ============================================================
// GitHub Actions
// ============================================================

#[test]
fn gha_uses_correct_directive_per_level() {
    let issues = sample_issues();
    assert!(format_issue_gha(&issues[0]).starts_with("::error "));
    assert!(format_issue_gha(&issues[1]).starts_with("::warning "));
    // info → notice (GHA has only error/warning/notice)
    assert!(format_issue_gha(&issues[2]).starts_with("::notice "));
}

#[test]
fn gha_includes_file_line_col() {
    let line = format_issue_gha(&sample_issues()[0]);
    assert!(line.contains("file=src/main.vr"), "got: {line}");
    assert!(line.contains("line=4"), "got: {line}");
    assert!(line.contains("col=13"), "got: {line}");
}

#[test]
fn gha_off_level_returns_empty() {
    let issue = LintIssue {
        rule: "shadow-binding",
        level: LintLevel::Off,
        file: PathBuf::from("a.vr"),
        line: 1,
        column: 1,
        message: "msg".to_string(),
        suggestion: None,
        fixable: false,
    };
    assert_eq!(format_issue_gha(&issue), "");
}

#[test]
fn gha_encodes_newlines_as_percent_zero_a() {
    let issue = LintIssue {
        rule: "x",
        level: LintLevel::Error,
        file: PathBuf::from("a.vr"),
        line: 1,
        column: 1,
        message: "first\nsecond".to_string(),
        suggestion: None,
        fixable: false,
    };
    let line = format_issue_gha(&issue);
    assert!(line.contains("first%0Asecond"), "got: {line}");
    assert!(!line.contains("first\nsecond"), "raw newline survived: {line}");
}

// ============================================================
// SARIF 2.1.0
// ============================================================

#[test]
fn sarif_carries_version_and_schema_url() {
    let s = format_sarif(&sample_issues());
    assert!(s.contains("\"version\": \"2.1.0\""), "missing version");
    assert!(s.contains("sarif-schema-2.1.0.json"), "missing schema URL");
}

#[test]
fn sarif_emits_one_result_per_issue() {
    let s = format_sarif(&sample_issues());
    let result_count = s.matches("\"ruleId\":").count();
    assert_eq!(result_count, 3, "expected 3 results, got {result_count}\n{s}");
}

#[test]
fn sarif_maps_levels_to_sarif_taxonomy() {
    // SARIF level vocabulary: error | warning | note | none
    let s = format_sarif(&sample_issues());
    assert!(s.contains("\"level\": \"error\""));
    assert!(s.contains("\"level\": \"warning\""));
    // info AND hint both fold to "note" per SARIF
    assert!(s.contains("\"level\": \"note\""));
}

#[test]
fn sarif_is_well_formed_json() {
    let s = format_sarif(&sample_issues());
    serde_json::from_str::<serde_json::Value>(&s)
        .unwrap_or_else(|e| panic!("SARIF output is not valid JSON: {e}\n---\n{s}\n---"));
}

#[test]
fn sarif_includes_tool_metadata() {
    let s = format_sarif(&sample_issues());
    assert!(s.contains("\"name\": \"verum-lint\""));
    assert!(s.contains("\"informationUri\":"));
    // The rules array must be present (consumers cross-reference ruleId).
    assert!(s.contains("\"rules\":"));
}

// ============================================================
// TAP v13
// ============================================================

#[test]
fn tap_starts_with_version_and_plan() {
    let s = format_tap(&sample_issues());
    let mut lines = s.lines();
    assert_eq!(lines.next(), Some("TAP version 13"));
    assert_eq!(lines.next(), Some("1..3"));
}

#[test]
fn tap_emits_not_ok_for_errors_and_warnings() {
    let s = format_tap(&sample_issues());
    assert!(s.contains("not ok 1 - "), "first issue (error) should be not-ok\n{s}");
    assert!(s.contains("not ok 2 - "), "second issue (warning) should be not-ok\n{s}");
}

#[test]
fn tap_emits_ok_skip_for_info() {
    let s = format_tap(&sample_issues());
    assert!(s.contains("ok 3 - ") && s.contains("# SKIP info"), "got:\n{s}");
}

#[test]
fn tap_includes_yaml_diagnostic_block_on_failures() {
    let s = format_tap(&sample_issues());
    assert!(s.contains("  ---"), "missing YAML start");
    assert!(s.contains("  rule: deprecated-syntax"), "missing rule field\n{s}");
    assert!(s.contains("  ..."), "missing YAML end");
}
