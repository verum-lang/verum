//! Per-rule fires/silent contract tests for `verum lint`.
//!
//! Every built-in rule has a matched pair: a fixture that *should*
//! fire the rule, and a fixture that explicitly should not. Together
//! they pin both sides of the rule's behaviour — silencing it
//! without breaking detection requires editing the test, which makes
//! the change visible at PR time.
//!
//! Tests run against `lint_source(...)` (no disk I/O) so this file
//! costs ~milliseconds to run and adds zero filesystem flakes.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use verum_cli::commands::lint::{has_issue, lint_source, LintConfig, LintIssue};

fn run(src: &str) -> Vec<LintIssue> {
    let path = Path::new("test_fixture.vr");
    lint_source(path, src, None).into_iter().collect()
}

fn run_with_config(src: &str, config: &LintConfig) -> Vec<LintIssue> {
    let path = Path::new("test_fixture.vr");
    lint_source(path, src, Some(config)).into_iter().collect()
}

fn fires(rule: &str, src: &str) {
    let issues = run(src);
    assert!(
        has_issue(&issues, rule),
        "expected rule `{rule}` to fire on:\n{src}\nactual: {:?}",
        issues.iter().map(|i| i.rule).collect::<Vec<_>>()
    );
}

fn silent(rule: &str, src: &str) {
    let issues = run(src);
    assert!(
        !has_issue(&issues, rule),
        "expected rule `{rule}` to be silent on:\n{src}\nactual rule firings: {:?}",
        issues
            .iter()
            .filter(|i| i.rule == rule)
            .map(|i| (i.line, i.column, &i.message))
            .collect::<Vec<_>>()
    );
}

fn fires_with(rule: &str, src: &str, cfg: &LintConfig) {
    let issues = run_with_config(src, cfg);
    assert!(
        has_issue(&issues, rule),
        "expected rule `{rule}` to fire on:\n{src}\nactual: {:?}",
        issues.iter().map(|i| i.rule).collect::<Vec<_>>()
    );
}

fn silent_with(rule: &str, src: &str, cfg: &LintConfig) {
    let issues = run_with_config(src, cfg);
    assert!(
        !has_issue(&issues, rule),
        "expected rule `{rule}` to be silent on:\n{src}\nactual: {:?}",
        issues
            .iter()
            .filter(|i| i.rule == rule)
            .collect::<Vec<_>>()
    );
}

fn make_config(rule_configs: &[(&str, toml::Value)]) -> LintConfig {
    let mut rules = HashMap::new();
    for (key, val) in rule_configs {
        rules.insert((*key).to_string(), val.clone());
    }
    LintConfig {
        extends: None,
        disabled: HashSet::new(),
        denied: HashSet::new(),
        allowed: HashSet::new(),
        warned: HashSet::new(),
        severity_map: HashMap::new(),
        rules,
        per_file_overrides: Vec::new(),
        profiles: HashMap::new(),
        custom_rules: Vec::new(),
    }
}

fn style_policy(
    max_line_length: u32,
    max_fn_lines: u32,
    max_fn_params: u32,
    max_match_arms: u32,
) -> LintConfig {
    let mut tbl = toml::value::Table::new();
    tbl.insert(
        "max_line_length".into(),
        toml::Value::Integer(max_line_length as i64),
    );
    tbl.insert(
        "max_fn_lines".into(),
        toml::Value::Integer(max_fn_lines as i64),
    );
    tbl.insert(
        "max_fn_params".into(),
        toml::Value::Integer(max_fn_params as i64),
    );
    tbl.insert(
        "max_match_arms".into(),
        toml::Value::Integer(max_match_arms as i64),
    );
    make_config(&[("style-policy", toml::Value::Table(tbl))])
}

fn doc_policy(public_must_have_doc: bool) -> LintConfig {
    let mut tbl = toml::value::Table::new();
    tbl.insert(
        "public_must_have_doc".into(),
        toml::Value::Boolean(public_must_have_doc),
    );
    make_config(&[("documentation-policy", toml::Value::Table(tbl))])
}

// ============================================================
// Verification rules
// ============================================================

#[test]
fn redundant_refinement_fires() {
    fires("redundant-refinement", "type Always is Int{ true };\n");
}

#[test]
fn redundant_refinement_silent_on_meaningful_predicate() {
    silent("redundant-refinement", "type Pos is Int{ it > 0 };\n");
}

#[test]
fn empty_refinement_bound_fires() {
    fires(
        "empty-refinement-bound",
        "type Empty is Int{ it > 100 && it < 50 };\n",
    );
}

#[test]
fn empty_refinement_bound_silent_on_satisfiable() {
    silent("empty-refinement-bound", "type Range is Int{ it > 0 && it < 100 };\n");
}

#[test]
fn unchecked_refinement_fires() {
    fires(
        "unchecked-refinement",
        "public fn divide(a: Int, b: Int{ it != 0 }) -> Int { a / b }\n",
    );
}

#[test]
fn unchecked_refinement_silent_with_verify() {
    silent(
        "unchecked-refinement",
        "@verify(formal)\npublic fn divide(a: Int, b: Int{ it != 0 }) -> Int { a / b }\n",
    );
}

// ============================================================
// Style rules — text-scan
// ============================================================

#[test]
fn deprecated_syntax_fires_on_box_new() {
    fires("deprecated-syntax", "fn make() { let x = Box::new(5); }\n");
}

#[test]
fn deprecated_syntax_silent_on_heap() {
    silent("deprecated-syntax", "fn make() { let x = Heap(5); }\n");
}

#[test]
fn deprecated_syntax_fires_on_struct_keyword() {
    fires(
        "deprecated-syntax",
        "struct Point { x: Int, y: Int }\n",
    );
}

#[test]
fn deprecated_syntax_fires_on_impl_keyword() {
    fires("deprecated-syntax", "impl Point {\n    fn zero() -> Point {}\n}\n");
}

#[test]
fn todo_in_code_fires_on_bare_todo() {
    fires("todo-in-code", "fn work() {\n    // TODO: handle this\n}\n");
}

#[test]
fn todo_in_code_silent_on_referenced_todo() {
    silent(
        "todo-in-code",
        "fn work() {\n    // TODO(#1234): handle this\n}\n",
    );
}

#[test]
fn todo_in_code_fires_on_trailing_comment() {
    fires(
        "todo-in-code",
        "fn work() { call_it(); // FIXME: brittle\n}\n",
    );
}

#[test]
fn unused_import_fires() {
    fires(
        "unused-import",
        "mount stdlib.collections.list;\n\nfn main() { print(\"hi\"); }\n",
    );
}

#[test]
fn unused_import_silent_when_used() {
    silent(
        "unused-import",
        "mount stdlib.collections.list;\n\nfn main() { let _: List<Int> = list::empty(); }\n",
    );
}

// ============================================================
// Safety rules
// ============================================================

#[test]
fn missing_timeout_fires_on_unbounded_recv() {
    fires(
        "missing-timeout",
        "fn main() {\n    let msg = ch.recv();\n}\n",
    );
}

#[test]
fn missing_timeout_silent_with_timeout() {
    silent(
        "missing-timeout",
        "fn main() {\n    let msg = ch.recv_timeout(Duration.from_secs(5));\n}\n",
    );
}

#[test]
fn unbounded_channel_fires() {
    fires("unbounded-channel", "fn main() { let ch = Channel.new(); }\n");
}

#[test]
fn unbounded_channel_silent_with_capacity() {
    silent(
        "unbounded-channel",
        "fn main() { let ch = Channel.bounded(64); }\n",
    );
}

#[test]
fn unbounded_channel_silent_when_capacity_passed() {
    silent(
        "unbounded-channel",
        "fn main() { let ch = Channel.new(64); }\n",
    );
}

// ============================================================
// Performance rules
// ============================================================

#[test]
fn unnecessary_heap_fires() {
    fires("unnecessary-heap", "fn main() { let x = Heap(42); }\n");
}

// ============================================================
// AST style ceiling rules — require [lint.style] config
// ============================================================

#[test]
fn max_line_length_fires_when_over_budget() {
    let cfg = style_policy(80, 80, 5, 12);
    let long_line: String = "//".chars().chain(std::iter::repeat('x').take(200)).collect();
    let src = format!("fn main() {{}}\n{long_line}\n");
    fires_with("max-line-length", &src, &cfg);
}

#[test]
fn max_line_length_silent_under_budget() {
    let cfg = style_policy(80, 80, 5, 12);
    silent_with("max-line-length", "fn main() {\n    let x = 1;\n}\n", &cfg);
}

#[test]
fn max_line_length_off_when_zero() {
    let cfg = style_policy(0, 80, 5, 12);
    let long_line: String = "//".chars().chain(std::iter::repeat('x').take(200)).collect();
    let src = format!("fn main() {{}}\n{long_line}\n");
    silent_with("max-line-length", &src, &cfg);
}

#[test]
fn max_fn_lines_fires_on_long_fn() {
    let cfg = style_policy(100, 80, 5, 12);
    let mut src = String::from("fn long() -> Int {\n");
    for i in 0..85 {
        src.push_str(&format!("    let x{i} = {i};\n"));
    }
    src.push_str("    0\n}\n");
    fires_with("max-fn-lines", &src, &cfg);
}

#[test]
fn max_fn_lines_silent_on_short_fn() {
    let cfg = style_policy(100, 80, 5, 12);
    silent_with(
        "max-fn-lines",
        "fn short() -> Int {\n    let x = 1;\n    x + 2\n}\n",
        &cfg,
    );
}

#[test]
fn max_fn_params_fires() {
    let cfg = style_policy(100, 80, 5, 12);
    fires_with(
        "max-fn-params",
        "fn many(a: Int, b: Int, c: Int, d: Int, e: Int, f: Int, g: Int) -> Int { 0 }\n",
        &cfg,
    );
}

#[test]
fn max_fn_params_silent_under_threshold() {
    let cfg = style_policy(100, 80, 5, 12);
    silent_with(
        "max-fn-params",
        "fn pair(a: Int, b: Int) -> Int { a + b }\n",
        &cfg,
    );
}

#[test]
fn max_match_arms_fires() {
    let cfg = style_policy(100, 80, 5, 12);
    let mut src = String::from("fn classify(x: Int) -> Int {\n    match x {\n");
    for i in 0..15 {
        src.push_str(&format!("        {i} => {i},\n"));
    }
    src.push_str("        _ => -1,\n    }\n}\n");
    fires_with("max-match-arms", &src, &cfg);
}

#[test]
fn max_match_arms_silent_on_few_arms() {
    let cfg = style_policy(100, 80, 5, 12);
    silent_with(
        "max-match-arms",
        "fn classify(x: Int) -> Int {\n    match x {\n        0 => 0,\n        _ => 1,\n    }\n}\n",
        &cfg,
    );
}

// ============================================================
// Public-must-have-doc — opt-in
// ============================================================

#[test]
fn public_must_have_doc_fires_when_enabled() {
    let cfg = doc_policy(true);
    fires_with("public-must-have-doc", "public fn no_doc() -> Int { 0 }\n", &cfg);
}

#[test]
fn public_must_have_doc_silent_when_disabled() {
    let cfg = doc_policy(false);
    silent_with("public-must-have-doc", "public fn no_doc() -> Int { 0 }\n", &cfg);
}

#[test]
fn public_must_have_doc_silent_with_doc() {
    let cfg = doc_policy(true);
    silent_with(
        "public-must-have-doc",
        "/// Returns zero.\npublic fn yes_doc() -> Int { 0 }\n",
        &cfg,
    );
}

#[test]
fn public_must_have_doc_silent_on_private() {
    let cfg = doc_policy(true);
    silent_with("public-must-have-doc", "fn private_no_doc() -> Int { 0 }\n", &cfg);
}

// ============================================================
// Custom rules — regex + AST match
// ============================================================

#[test]
fn custom_regex_rule_fires() {
    let mut cfg = make_config(&[]);
    cfg.custom_rules.push(verum_cli::commands::lint::CustomLintRule {
        name: "no-todo-issue-required".to_string(),
        pattern: "TODO".to_string(),
        message: "TODO without issue link".to_string(),
        level: verum_cli::commands::lint::LintLevel::Warning,
        paths: Vec::new(),
        exclude: Vec::new(),
        suggestion: None,
        ast_match: None,
    });
    fires_with(
        "no-todo-issue-required",
        "fn x() {\n    // TODO: ship me\n}\n",
        &cfg,
    );
}

#[test]
fn custom_ast_method_call_rule_fires() {
    let mut cfg = make_config(&[]);
    cfg.custom_rules.push(verum_cli::commands::lint::CustomLintRule {
        name: "no-unwrap".to_string(),
        pattern: String::new(),
        message: "use ? instead of unwrap".to_string(),
        level: verum_cli::commands::lint::LintLevel::Error,
        paths: Vec::new(),
        exclude: Vec::new(),
        suggestion: None,
        ast_match: Some(verum_cli::commands::lint::AstMatchSpec {
            kind: "method_call".to_string(),
            method: Some("unwrap".to_string()),
            path: None,
            name: None,
        }),
    });
    fires_with(
        "no-unwrap",
        "fn main() { let x: Maybe<Int> = Some(5); let y = x.unwrap(); }\n",
        &cfg,
    );
}

#[test]
fn custom_ast_unsafe_block_rule_fires() {
    let mut cfg = make_config(&[]);
    cfg.custom_rules.push(verum_cli::commands::lint::CustomLintRule {
        name: "no-unsafe".to_string(),
        pattern: String::new(),
        message: "no unsafe in this fixture".to_string(),
        level: verum_cli::commands::lint::LintLevel::Error,
        paths: Vec::new(),
        exclude: Vec::new(),
        suggestion: None,
        ast_match: Some(verum_cli::commands::lint::AstMatchSpec {
            kind: "unsafe_block".to_string(),
            method: None,
            path: None,
            name: None,
        }),
    });
    fires_with(
        "no-unsafe",
        "fn main() { let x = unsafe { 42 }; print(x); }\n",
        &cfg,
    );
}

// ============================================================
// span_to_line_col contract — exposed via an issue's line/col
// ============================================================

#[test]
fn issue_line_and_column_are_one_indexed() {
    let src = "type Empty is Int{ it > 100 && it < 50 };\n";
    let issues = run(src);
    let issue = issues
        .iter()
        .find(|i| i.rule == "empty-refinement-bound")
        .expect("expected empty-refinement-bound to fire");
    assert!(issue.line >= 1, "line should be 1-indexed");
    assert!(issue.column >= 1, "column should be 1-indexed");
}
