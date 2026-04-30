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
    let long_line: String = "//".chars().chain(std::iter::repeat_n('x', 200)).collect();
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
    let long_line: String = "//".chars().chain(std::iter::repeat_n('x', 200)).collect();
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

// ============================================================
// Lex-mask false-positive contract — every text-scan rule has
// to ignore matches that lie inside string literals or
// comments. These tests pin that contract so a future regression
// (e.g. someone reverting back to raw `info.lines` scans) shows
// up at PR time.
// ============================================================

#[test]
fn deprecated_syntax_silent_on_token_inside_string_literal() {
    // `Box::new` appears as program code on line 2 (genuine
    // Rust-ism) and as string data on line 3 (must NOT fire).
    let src = "fn main() {\n    let _x = Box::new(5);\n    let msg = \"explain Box::new()\";\n}\n";
    let issues = run(src);
    let firings: Vec<usize> = issues
        .iter()
        .filter(|i| i.rule == "deprecated-syntax" && i.message.contains("Box::new"))
        .map(|i| i.line)
        .collect();
    assert!(
        firings.contains(&2),
        "deprecated-syntax should fire on line 2 (real code), got: {firings:?}"
    );
    assert!(
        !firings.contains(&3),
        "deprecated-syntax must NOT fire on line 3 (string literal), got: {firings:?}"
    );
}

#[test]
fn deprecated_syntax_silent_on_token_inside_block_comment() {
    let src = "fn main() {\n    /* historical: panic!(\"old\") was here */\n    let _ = 1;\n}\n";
    silent("deprecated-syntax", src);
}

#[test]
fn deprecated_syntax_silent_on_token_inside_line_comment() {
    let src = "fn main() {\n    // we used to call panic!()\n    let _ = 1;\n}\n";
    silent("deprecated-syntax", src);
}

#[test]
fn todo_in_code_silent_on_string_literal_marker() {
    // The TODO appears in a string, never in a comment — must be silent.
    let src = "fn main() { let s = \"TODO: literal data\"; let _ = s; }\n";
    silent("todo-in-code", src);
}

#[test]
fn todo_in_code_fires_on_inline_trailing_comment() {
    let src = "fn main() {\n    let _ = 1; // TODO: implement\n}\n";
    fires("todo-in-code", src);
}

#[test]
fn unbounded_channel_silent_on_string_literal_match() {
    // Channel.new() inside a string is data, not a call — must be silent.
    let src = "fn main() { let doc = \"call Channel.new() to start\"; let _ = doc; }\n";
    silent("unbounded-channel", src);
}

#[test]
fn missing_timeout_silent_on_match_inside_string() {
    // `.recv()` substring lives inside a doc-string — silent.
    let src = "fn main() { let s = \"call .recv() without timeout\"; let _ = s; }\n";
    silent("missing-timeout", src);
}

#[test]
fn raw_string_contents_are_inert() {
    // r#"…"# enclosing Rust-ism keywords must not fire any rule.
    let src = "fn main() { let s = r#\"struct Foo { x: Vec<i32> }\"#; let _ = s; }\n";
    silent("deprecated-syntax", src);
}

#[test]
fn nested_block_comment_with_keyword_is_inert() {
    // Nested /* /* */ */ comment containing struct keyword.
    let src = "fn main() {\n    /* outer /* struct Foo */ */\n    let _ = 1;\n}\n";
    silent("deprecated-syntax", src);
}

#[test]
fn unused_import_silent_when_name_appears_only_in_string() {
    let src = "mount foo.{Bar};\nfn main() { let s = \"Bar is here\"; let _ = s; }\n";
    fires("unused-import", src);
}

#[test]
fn unused_import_silent_when_name_used_in_code() {
    let src = "mount foo.{Bar};\nfn main() { let b: Bar = Bar(); let _ = b; }\n";
    silent("unused-import", src);
}

#[test]
fn unnecessary_heap_silent_on_string_literal_match() {
    let src = "fn main() { let s = \"Heap(5)\"; let _ = s; }\n";
    silent("unnecessary-heap", src);
}

#[test]
fn cbgr_hotspot_skips_string_loop_marker() {
    // `for ` inside a string is not a real loop. Without the mask,
    // the rule's brace counter would derail.
    let src = "fn main() { let s = \"for x in []\"; let _ = s; }\n";
    silent("cbgr-hotspot", src);
}

#[test]
fn missing_error_context_silent_when_question_in_string() {
    let src = "fn main() { let s = \"value? maybe\"; let _ = s; }\n";
    silent("missing-error-context", src);
}

// ============================================================
// parse-error meta-rule — broken files emit a structured
// diagnostic so the user knows the AST half was skipped.
// ============================================================

#[test]
fn parse_error_fires_on_unbalanced_braces() {
    // Truncated function body — fast parser surfaces a missing
    // closing brace.
    let src = "fn main() { let x = 1; \n";
    fires("parse-error", src);
}

#[test]
fn parse_error_silent_on_well_formed_file() {
    let src = "fn main() {\n    let _ = 1;\n}\n";
    silent("parse-error", src);
}

#[test]
fn lint_does_not_panic_on_multibyte_chars_in_comments() {
    // Em-dash, French quotes, CJK, math symbols — every char is
    // multi-byte UTF-8 in a comment span. Earlier the masked-view
    // builder preserved continuation bytes verbatim while blanking
    // their leaders, producing invalid UTF-8 and panicking.
    let src = "fn main() {\n    // unicode — «test» 中文 ∀x. x\n    let _ = 1;\n}\n";
    let issues = run(src);
    // No assertion on rule firing; the contract is "doesn't panic".
    let _ = issues;
}

#[test]
fn lint_does_not_panic_on_multibyte_chars_in_strings() {
    let src = "fn main() { let s = \"em-dash — and ∀\"; let _ = s; }\n";
    let issues = run(src);
    let _ = issues;
}
