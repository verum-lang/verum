//! Severity precedence integration tests.
//!
//! Locks down the precedence stack documented in
//! `internal/website/docs/reference/lint-configuration.md`:
//!
//!   1. per-file overrides (most specific glob wins)
//!   2. severity_map (`[lint.severity].<rule>`)
//!   3. disabled / allowed / denied / warned lists
//!   4. default level
//!
//! In-source `@allow` / `@deny` / `@warn` attributes win over all of
//! the above, scoped to the enclosing item.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use verum_cli::commands::lint::{
    lint_source, FileOverride, LintConfig, LintIssue, LintLevel,
};

fn empty_config() -> LintConfig {
    LintConfig {
        extends: None,
        disabled: HashSet::new(),
        denied: HashSet::new(),
        allowed: HashSet::new(),
        warned: HashSet::new(),
        severity_map: HashMap::new(),
        rules: HashMap::new(),
        per_file_overrides: Vec::new(),
        profiles: HashMap::new(),
        custom_rules: Vec::new(),
    }
}

fn lint(src: &str, cfg: &LintConfig) -> Vec<LintIssue> {
    let path = Path::new("src/main.vr");
    lint_source(path, src, Some(cfg)).into_iter().collect()
}

fn level_of(issues: &[LintIssue], rule: &str) -> Option<LintLevel> {
    issues.iter().find(|i| i.rule == rule).map(|i| i.level)
}

// ============================================================
// severity_map vs default
// ============================================================

#[test]
fn severity_map_promotes_warning_to_error() {
    let mut cfg = empty_config();
    cfg.severity_map
        .insert("todo-in-code".to_string(), LintLevel::Error);
    let issues = lint("// TODO: fix\nfn main() {}\n", &cfg);
    let level = cfg.effective_level("todo-in-code", LintLevel::Warning);
    assert_eq!(level, Some(LintLevel::Error));
    // The raw-emit path emits at default level; the runner applies
    // effective_level downstream. We test both layers.
    assert!(
        level_of(&issues, "todo-in-code").is_some(),
        "rule should still fire at raw level"
    );
}

#[test]
fn severity_map_off_disables_rule() {
    let mut cfg = empty_config();
    cfg.severity_map
        .insert("todo-in-code".to_string(), LintLevel::Off);
    assert_eq!(cfg.effective_level("todo-in-code", LintLevel::Warning), None);
}

// ============================================================
// disabled / allowed / denied / warned lists
// ============================================================

#[test]
fn disabled_list_suppresses_rule() {
    let mut cfg = empty_config();
    cfg.disabled.insert("todo-in-code".to_string());
    assert_eq!(cfg.effective_level("todo-in-code", LintLevel::Warning), None);
}

#[test]
fn denied_list_promotes_to_error() {
    let mut cfg = empty_config();
    cfg.denied.insert("todo-in-code".to_string());
    assert_eq!(
        cfg.effective_level("todo-in-code", LintLevel::Warning),
        Some(LintLevel::Error)
    );
}

#[test]
fn warned_list_demotes_to_warn() {
    let mut cfg = empty_config();
    cfg.warned.insert("deprecated-syntax".to_string());
    assert_eq!(
        cfg.effective_level("deprecated-syntax", LintLevel::Error),
        Some(LintLevel::Warning)
    );
}

#[test]
fn severity_map_beats_denied_list() {
    let mut cfg = empty_config();
    cfg.denied.insert("todo-in-code".to_string());
    cfg.severity_map
        .insert("todo-in-code".to_string(), LintLevel::Hint);
    assert_eq!(
        cfg.effective_level("todo-in-code", LintLevel::Warning),
        Some(LintLevel::Hint),
        "severity_map should override list-based promotions"
    );
}

// ============================================================
// per-file overrides
// ============================================================

#[test]
fn per_file_allow_suppresses_rule() {
    let mut cfg = empty_config();
    cfg.per_file_overrides.push((
        "src/legacy/**".to_string(),
        FileOverride {
            allow: vec!["todo-in-code".to_string()],
            deny: Vec::new(),
            warn: Vec::new(),
            disable: Vec::new(),
        },
    ));
    let path = Path::new("src/legacy/old.vr");
    assert_eq!(
        cfg.effective_level_for_file("todo-in-code", path, LintLevel::Warning),
        None,
        "per-file allow should suppress"
    );
}

#[test]
fn per_file_deny_promotes_to_error() {
    let mut cfg = empty_config();
    cfg.per_file_overrides.push((
        "src/critical/**".to_string(),
        FileOverride {
            allow: Vec::new(),
            deny: vec!["todo-in-code".to_string()],
            warn: Vec::new(),
            disable: Vec::new(),
        },
    ));
    let path = Path::new("src/critical/bank.vr");
    assert_eq!(
        cfg.effective_level_for_file("todo-in-code", path, LintLevel::Warning),
        Some(LintLevel::Error)
    );
}

#[test]
fn per_file_most_specific_match_wins() {
    let mut cfg = empty_config();
    cfg.per_file_overrides.push((
        "src/**".to_string(),
        FileOverride {
            allow: Vec::new(),
            deny: vec!["todo-in-code".to_string()],
            warn: Vec::new(),
            disable: Vec::new(),
        },
    ));
    cfg.per_file_overrides.push((
        "src/legacy/**".to_string(),
        FileOverride {
            allow: vec!["todo-in-code".to_string()],
            deny: Vec::new(),
            warn: Vec::new(),
            disable: Vec::new(),
        },
    ));
    let legacy = Path::new("src/legacy/old.vr");
    let normal = Path::new("src/new/code.vr");

    assert_eq!(
        cfg.effective_level_for_file("todo-in-code", legacy, LintLevel::Warning),
        None,
        "longer pattern src/legacy/** wins"
    );
    assert_eq!(
        cfg.effective_level_for_file("todo-in-code", normal, LintLevel::Warning),
        Some(LintLevel::Error),
        "outside src/legacy/**, src/** denies"
    );
}

// ============================================================
// In-source @allow attribute
// ============================================================

#[test]
fn at_allow_suppresses_rule_on_item() {
    let cfg = empty_config();
    let src = "@allow(\"todo-in-code\", reason = \"legacy\")\nfn legacy() {\n    // TODO: ship me\n}\n";
    let issues = lint(src, &cfg);
    // The suppression layer should remove the issue on the
    // suppressed item.
    let todo_issues: Vec<_> = issues
        .iter()
        .filter(|i| i.rule == "todo-in-code")
        .collect();
    assert!(
        todo_issues.is_empty(),
        "@allow should suppress todo-in-code, got: {:?}",
        todo_issues
    );
}

#[test]
fn at_allow_does_not_suppress_unrelated_items() {
    let cfg = empty_config();
    let src = "@allow(\"todo-in-code\", reason = \"legacy\")\nfn legacy() {\n}\n\nfn other() {\n    // TODO: real one\n}\n";
    let issues = lint(src, &cfg);
    // todo-in-code on the *other* fn must still fire — @allow is
    // scoped to its own item.
    assert!(
        issues.iter().any(|i| i.rule == "todo-in-code" && i.line >= 5),
        "the unsuppressed TODO on `other` should still fire, got: {:?}",
        issues
            .iter()
            .filter(|i| i.rule == "todo-in-code")
            .map(|i| (i.line, &i.message))
            .collect::<Vec<_>>()
    );
}
