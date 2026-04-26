//! Suppression baseline contract tests.
//!
//! Each test exercises one knob in the workflow:
//! 1. `--write-baseline` snapshots current issues; subsequent run
//!    finds nothing to fail on.
//! 2. New issue introduced after the baseline → fails.
//! 3. Fixed issue dropped on next `--write-baseline`.
//! 4. Line drift within ±5 still suppressed; outside → fires.
//! 5. `--no-baseline` overrides default lookup.

use std::path::PathBuf;
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_verum")
}

fn make_fixture(name: &str, body: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("verum_lint_baseline_{}_{}", name, std::process::id()));
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

fn run(dir: &PathBuf, args: &[&str]) -> std::process::Output {
    Command::new(binary())
        .arg("lint")
        .arg("--no-cache")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("verum lint spawn")
}

#[test]
fn write_baseline_creates_default_path() {
    let dir = make_fixture("write", "fn main() {\n    // TODO: a\n}\n");
    let out = run(&dir, &["--write-baseline"]);
    assert!(
        out.status.success(),
        "write-baseline should succeed even with issues. stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let path = dir.join(".verum").join("lint-baseline.json");
    assert!(path.exists(), "default baseline path should be created");
    let content = std::fs::read_to_string(&path).expect("read");
    assert!(content.contains("todo-in-code"), "baseline should snapshot the issue");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn baseline_suppresses_known_issues() {
    let dir = make_fixture("suppress", "fn main() {\n    // TODO: a\n}\n");
    // Seed the baseline at default severity (warnings included).
    let out_seed = run(&dir, &["--write-baseline"]);
    assert!(
        out_seed.status.success(),
        "write-baseline should succeed. stderr: {}",
        String::from_utf8_lossy(&out_seed.stderr)
    );

    // Re-run with deny-warnings — without the baseline this would
    // fail. With the baseline the warning is suppressed.
    let out = run(&dir, &["--deny-warnings"]);
    assert!(
        out.status.success(),
        "baseline should suppress the warning. stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn new_issue_after_baseline_still_fires() {
    let dir = make_fixture("new_after", "fn main() {\n    // TODO: a\n}\n");
    let _ = run(&dir, &["--write-baseline"]);
    // Introduce a NEW issue past the baseline. Use FIXME so its
    // message body (`FIXME comment in code`) differs from the
    // baselined `TODO comment in code` — they hash differently and
    // the drift heuristic can't conflate them.
    std::fs::write(
        dir.join("src").join("main.vr"),
        "fn main() {\n    // TODO: a\n    // FIXME: brand new\n}\n",
    )
    .expect("rewrite");

    let out = run(&dir, &["--deny-warnings"]);
    assert!(
        !out.status.success(),
        "new issue should still fail despite the baseline"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fixed_issue_drops_off_baseline_on_rewrite() {
    let dir = make_fixture(
        "drop_off",
        "fn main() {\n    // TODO: a\n    // TODO: b\n}\n",
    );
    let _ = run(&dir, &["--write-baseline"]);

    // Fix one of the TODOs by adding the issue ref.
    std::fs::write(
        dir.join("src").join("main.vr"),
        "fn main() {\n    // TODO(#1): a\n    // TODO: b\n}\n",
    )
    .expect("rewrite");

    // Re-snapshot.
    let _ = run(&dir, &["--write-baseline"]);
    let path = dir.join(".verum").join("lint-baseline.json");
    let content = std::fs::read_to_string(&path).expect("read");
    // The baseline now snapshots the *current* issue set — only
    // one TODO remains.
    let count = content.matches("todo-in-code").count();
    assert_eq!(
        count, 1,
        "fixed issue should drop off; baseline should have 1 entry, got {count}\n{content}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn no_baseline_flag_disables_default_lookup() {
    let dir = make_fixture("no_baseline", "fn main() {\n    // TODO: a\n}\n");
    let _ = run(&dir, &["--write-baseline"]);

    // With --no-baseline, the default baseline file is ignored;
    // --deny-warnings should still fail.
    let out = run(&dir, &["--no-baseline", "--deny-warnings"]);
    assert!(
        !out.status.success(),
        "--no-baseline must bypass default lookup"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
