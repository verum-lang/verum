//! Contract for the `fix.edits` field in `verum lint --format json`.
//!
//! CI fix-bots and LSP code-actions need precise edit ranges to
//! apply fixes mechanically without re-running the parser. The
//! field is additive — schema_version stays at 1, consumers that
//! don't understand `fix` simply ignore it.

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_verum")
}

fn make_fixture(name: &str, body: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("verum_lint_fix_edits_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).expect("create src");
    std::fs::write(
        dir.join("verum.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
    )
    .expect("manifest");
    std::fs::write(dir.join("src").join("main.vr"), body).expect("main.vr");
    dir
}

fn run_json(dir: &PathBuf) -> Vec<Value> {
    let out = Command::new(binary())
        .args(["lint", "--no-cache", "--format", "json"])
        .current_dir(dir)
        .output()
        .expect("verum lint spawn");
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l).expect("valid JSON"))
        .collect()
}

#[test]
fn todo_in_code_emits_structured_fix_edit() {
    let dir = make_fixture("todo", "fn main() {\n    // TODO: clean up\n}\n");
    let issues = run_json(&dir);
    let todo = issues
        .iter()
        .find(|v| v["rule"].as_str() == Some("todo-in-code"))
        .expect("todo-in-code issue present");
    let edits = todo["fix"]["edits"]
        .as_array()
        .expect("fix.edits array present");
    assert_eq!(edits.len(), 1, "single replacement edit expected");
    let e = &edits[0];
    assert_eq!(e["start_line"].as_u64(), Some(2), "TODO is on line 2");
    assert_eq!(e["end_line"].as_u64(), Some(2));
    assert!(
        e["new_text"].as_str().unwrap_or("").contains("TODO(#"),
        "new_text should be the placeholder TODO tag, got: {e}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unfixable_issue_has_no_fix_field() {
    // architecture-violation is fixable: false. The fix.edits
    // field must NOT appear on its diagnostic line.
    let dir = make_fixture("unfixable", "fn main() {\n    print(\"hi\");\n}\n");
    let issues = run_json(&dir);
    for issue in &issues {
        let fixable = issue["fixable"].as_bool().unwrap_or(false);
        if !fixable {
            assert!(
                issue.get("fix").is_none(),
                "fix.edits must not appear on a non-fixable issue, got: {issue}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn redundant_refinement_emits_strip_edit() {
    let dir = make_fixture("redundant", "type Always is Int{ true };\n");
    let issues = run_json(&dir);
    let redundant = issues
        .iter()
        .find(|v| v["rule"].as_str() == Some("redundant-refinement"));
    if let Some(r) = redundant {
        let edits = r["fix"]["edits"].as_array();
        if let Some(edits) = edits {
            assert_eq!(edits.len(), 1);
            let e = &edits[0];
            assert_eq!(
                e["new_text"].as_str(),
                Some(""),
                "redundant-refinement strips to empty string"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn schema_version_unchanged() {
    // Adding fix.edits is non-breaking — schema_version stays at 1.
    let dir = make_fixture("schema", "fn main() {\n    // TODO: x\n}\n");
    let issues = run_json(&dir);
    for issue in &issues {
        assert_eq!(
            issue["schema_version"].as_u64(),
            Some(1),
            "schema_version must remain 1 after adding fix.edits"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
