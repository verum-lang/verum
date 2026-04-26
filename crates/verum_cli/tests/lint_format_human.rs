//! End-to-end tests for `verum lint --format human`.
//!
//! Each test runs the binary against a fixture and asserts the
//! span-underlined output contains the expected structural pieces:
//! rule code, file location with `--> `, source line, caret
//! underline, help text. ANSI is stripped before comparison so the
//! tests work in colour-disabled CI environments too.

use std::path::PathBuf;
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_verum")
}

fn make_fixture(name: &str, body: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("verum_lint_human_{}_{}", name, std::process::id()));
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

fn strip_ansi(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1B && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'm' {
                i += 1;
            }
            if i < bytes.len() {
                i += 1;
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

fn run_human(dir: &PathBuf) -> String {
    let out = Command::new(binary())
        .args(["lint", "--no-cache", "--format", "human"])
        .env("NO_COLOR", "1")
        .current_dir(dir)
        .output()
        .expect("verum lint --format human spawn");
    strip_ansi(&String::from_utf8_lossy(&out.stdout))
}

#[test]
fn human_format_renders_rule_code_and_arrow() {
    let dir = make_fixture("arrow", "fn main() {\n    let x = Box::new(5);\n}\n");
    let out = run_human(&dir);
    assert!(out.contains("[deprecated-syntax]"), "rule code in brackets: {out}");
    assert!(out.contains("-->"), "arrow before file path: {out}");
    assert!(out.contains("src/main.vr:2:"), "file:line:col: {out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn human_format_includes_source_line_and_caret() {
    let dir = make_fixture("caret", "fn main() {\n    let x = Box::new(5);\n}\n");
    let out = run_human(&dir);
    // The source line itself must be reproduced in the output.
    assert!(
        out.contains("let x = Box::new(5);"),
        "source line should appear verbatim: {out}"
    );
    // And there must be a caret underline somewhere.
    assert!(out.contains("^"), "caret underline missing: {out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn human_format_includes_help_when_suggestion_present() {
    let dir = make_fixture("help", "fn main() {\n    let x = Box::new(5);\n}\n");
    let out = run_human(&dir);
    assert!(out.contains("help"), "help line missing: {out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn human_format_uses_level_keywords() {
    let dir = make_fixture("levels", "fn main() {\n    let x = Box::new(5);\n    // TODO: x\n}\n");
    let out = run_human(&dir);
    // Box::new is a deprecated-syntax error.
    assert!(out.contains("error"), "expected `error` keyword: {out}");
    // TODO is a todo-in-code warning.
    assert!(out.contains("warning"), "expected `warning` keyword: {out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn human_format_passes_check_on_clean_corpus() {
    let dir = make_fixture("clean", "fn main() {}\n");
    let out = run_human(&dir);
    // No issues → no rule-code bracket lines in the output.
    assert!(
        !out.contains("[deprecated-syntax]"),
        "clean corpus should not produce deprecated-syntax issues: {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn human_format_unknown_rejected() {
    let dir = make_fixture("unknown", "fn main() {}\n");
    let out = Command::new(binary())
        .args(["lint", "--format", "explode"])
        .current_dir(&dir)
        .output()
        .expect("spawn");
    assert!(!out.status.success(), "unknown format must fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unknown lint format") || stderr.contains("explode"),
        "error should name the offending format: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
