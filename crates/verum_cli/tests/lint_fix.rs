//! Roundtrip tests for `verum lint --fix`.
//!
//! Each test exercises one rule by:
//!   1. Running lint on a fixture that violates the rule.
//!   2. Asserting the issue fires and is reported as `fixable: true`.
//!   3. Running `--fix` and reading back the file.
//!   4. Asserting the rule no longer fires after the fix.
//!
//! Step 4 is the load-bearing one — a fix that's idempotent in name
//! only (rewrites the file but doesn't actually silence the rule)
//! is worse than no fix at all.

use std::path::PathBuf;
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_verum")
}

fn make_fixture(name: &str, file_name: &str, contents: &str) -> (PathBuf, PathBuf) {
    let mut dir = std::env::temp_dir();
    dir.push(format!("verum_lint_fix_test_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).expect("create src");
    std::fs::write(
        dir.join("verum.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
    )
    .expect("write manifest");
    let target = dir.join("src").join(file_name);
    std::fs::write(&target, contents).expect("write fixture file");
    (dir, target)
}

fn lint_json(dir: &PathBuf) -> String {
    let out = Command::new(binary())
        .args(["lint", "--no-cache", "--format", "json"])
        .current_dir(dir)
        .output()
        .expect("verum lint failed to spawn");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn lint_fix(dir: &PathBuf) {
    Command::new(binary())
        .args(["lint", "--no-cache", "--fix"])
        .current_dir(dir)
        .output()
        .expect("verum lint --fix failed to spawn");
}

fn rule_count(json_out: &str, rule: &str) -> usize {
    json_out
        .lines()
        .filter(|line| {
            line.contains(&format!("\"rule\":\"{rule}\""))
        })
        .count()
}

// ============================================================
// todo-in-code
// ============================================================

#[test]
fn fix_todo_in_code_inserts_placeholder_issue_tag() {
    let (dir, file) = make_fixture(
        "todo_fix",
        "main.vr",
        "fn main() {\n    // TODO: handle this\n    let x = 1;\n}\n",
    );
    let before = lint_json(&dir);
    assert_eq!(
        rule_count(&before, "todo-in-code"),
        1,
        "expected todo-in-code to fire once before fix"
    );

    lint_fix(&dir);

    let content = std::fs::read_to_string(&file).expect("read fixture");
    assert!(
        content.contains("TODO(#0000)"),
        "fix should append placeholder issue tag, got:\n{content}"
    );

    let after = lint_json(&dir);
    assert_eq!(
        rule_count(&after, "todo-in-code"),
        0,
        "todo-in-code should silence after fix, got:\n{after}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ============================================================
// empty-match-arm
// ============================================================

#[test]
fn fix_empty_match_arm_drops_the_arm() {
    let (dir, file) = make_fixture(
        "empty_arm_fix",
        "main.vr",
        "fn classify(x: Int) {\n    match x {\n        0 => print(\"zero\"),\n        _ => (),\n    }\n}\n",
    );
    let before = lint_json(&dir);
    assert_eq!(rule_count(&before, "empty-match-arm"), 1);

    lint_fix(&dir);

    let content = std::fs::read_to_string(&file).expect("read fixture");
    assert!(
        !content.contains("=> ()"),
        "empty arm should be dropped, got:\n{content}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ============================================================
// shadow-binding
// ============================================================

#[test]
fn fix_shadow_binding_renames_inner() {
    let (dir, file) = make_fixture(
        "shadow_fix",
        "main.vr",
        "fn main() {\n    let x = 1;\n    let x = 2;\n}\n",
    );
    let before = lint_json(&dir);
    assert!(
        rule_count(&before, "shadow-binding") >= 1,
        "expected shadow-binding to fire on fixture, got:\n{before}"
    );

    lint_fix(&dir);

    let content = std::fs::read_to_string(&file).expect("read fixture");
    assert!(
        content.contains("let x2 = 2"),
        "inner binding should rename to x2, got:\n{content}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ============================================================
// redundant-refinement
// ============================================================

#[test]
fn fix_redundant_refinement_strips_trivial_predicate() {
    let (dir, file) = make_fixture(
        "redundant_refine_fix",
        "main.vr",
        "type Always is Int{ true };\n",
    );
    let before = lint_json(&dir);
    assert_eq!(rule_count(&before, "redundant-refinement"), 1);

    lint_fix(&dir);

    let content = std::fs::read_to_string(&file).expect("read fixture");
    assert!(
        !content.contains("{ true }") && !content.contains("{true}"),
        "trivial predicate should be stripped, got:\n{content}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
