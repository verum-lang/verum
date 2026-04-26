//! `verum lint --new-only-since GIT_REF` contract tests.
//!
//! Set up a temporary git repo, commit a state with a baseline of
//! issues, add a NEW issue on top, and verify only the new one is
//! reported.

use std::path::PathBuf;
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_verum")
}

fn run_git(dir: &PathBuf, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("git spawn");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn make_repo(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("verum_lint_new_only_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).expect("create src");
    std::fs::write(
        dir.join("verum.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
    )
    .expect("manifest");
    run_git(&dir, &["init", "-q"]);
    run_git(&dir, &["config", "commit.gpgsign", "false"]);
    dir
}

#[test]
fn new_only_since_reports_only_new_issues() {
    let dir = make_repo("new_only");
    // First commit: one TODO at line 2.
    std::fs::write(
        dir.join("src").join("main.vr"),
        "fn main() {\n    // TODO: a\n}\n",
    )
    .expect("v1");
    run_git(&dir, &["add", "."]);
    run_git(&dir, &["commit", "-q", "-m", "v1"]);
    run_git(&dir, &["tag", "v1"]);

    // Second commit: add a FIXME (different message body so it
    // doesn't look like drift of the existing TODO).
    std::fs::write(
        dir.join("src").join("main.vr"),
        "fn main() {\n    // TODO: a\n    // FIXME: brand new\n}\n",
    )
    .expect("v2");
    run_git(&dir, &["add", "."]);
    run_git(&dir, &["commit", "-q", "-m", "v2"]);

    let out = Command::new(binary())
        .args(["lint", "--no-cache", "--format", "json", "--new-only-since", "v1"])
        .current_dir(&dir)
        .output()
        .expect("lint --new-only-since spawn");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    // Exactly the FIXME line should be reported. The pre-existing
    // TODO from v1 must NOT appear.
    let fixme_count = lines
        .iter()
        .filter(|l| l.contains("FIXME") || l.contains("\"line\":3"))
        .count();
    let todo_count = lines
        .iter()
        .filter(|l| l.contains("\"line\":2"))
        .count();
    assert_eq!(
        fixme_count, 1,
        "expected FIXME issue in --new-only-since output, got {} lines:\n{stdout}",
        lines.len()
    );
    assert_eq!(
        todo_count, 0,
        "pre-existing TODO from v1 must NOT appear in --new-only-since output, got:\n{stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn new_only_since_silent_when_nothing_new() {
    let dir = make_repo("no_change");
    std::fs::write(
        dir.join("src").join("main.vr"),
        "fn main() {\n    // TODO: a\n}\n",
    )
    .expect("v1");
    run_git(&dir, &["add", "."]);
    run_git(&dir, &["commit", "-q", "-m", "v1"]);
    run_git(&dir, &["tag", "v1"]);
    // No second commit.

    let out = Command::new(binary())
        .args(["lint", "--no-cache", "--format", "json", "--new-only-since", "v1"])
        .current_dir(&dir)
        .output()
        .expect("lint spawn");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let count = stdout.lines().filter(|l| !l.is_empty()).count();
    assert_eq!(
        count, 0,
        "no new commits → no new issues, got:\n{stdout}"
    );
    assert!(
        out.status.success(),
        "exit 0 when there are no new issues"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
