//! Contract for `verum lint --max-warnings N`.
//!
//! Exit-code expectations:
//! - `--max-warnings 0`: any warning fails. Equivalent to today's
//!   `--deny-warnings`.
//! - `--max-warnings N (>0)`: passes when warnings ≤ N, fails when
//!   warnings > N. The error names the budget.
//! - Errors always fail the run regardless of N.

use std::path::PathBuf;
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_verum")
}

fn make_fixture(name: &str, body: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("verum_lint_maxwarn_{}_{}", name, std::process::id()));
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

// ============================================================
// Pass paths
// ============================================================

#[test]
fn budget_high_passes_warnings() {
    // 3 TODO comments → 3 todo-in-code warnings. Budget 100 passes.
    let dir = make_fixture(
        "high",
        "fn main() {\n    // TODO: a\n    // TODO: b\n    // TODO: c\n}\n",
    );
    let out = run(&dir, &["--max-warnings", "100"]);
    assert!(
        out.status.success(),
        "100-budget should pass on 3 warnings. stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn budget_exact_passes() {
    let dir = make_fixture("exact", "fn main() {\n    // TODO: a\n}\n");
    let out = run(&dir, &["--max-warnings", "1"]);
    assert!(
        out.status.success(),
        "budget=1 with exactly 1 warning should pass. stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ============================================================
// Fail paths
// ============================================================

#[test]
fn budget_zero_fails_on_any_warning() {
    let dir = make_fixture("zero", "fn main() {\n    // TODO: a\n}\n");
    let out = run(&dir, &["--max-warnings", "0"]);
    assert!(
        !out.status.success(),
        "budget=0 with 1 warning should fail"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("budget") || stderr.contains("max-warnings"),
        "error should name the budget, got: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn budget_exceeded_reports_count_and_budget() {
    let dir = make_fixture(
        "exceed",
        "fn main() {\n    // TODO: a\n    // TODO: b\n    // TODO: c\n}\n",
    );
    let out = run(&dir, &["--max-warnings", "1"]);
    assert!(!out.status.success(), "3 warnings > 1 budget should fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("budget of 1") || (stderr.contains("3") && stderr.contains("budget")),
        "error should name count + budget, got: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn errors_always_fail_regardless_of_budget() {
    // Box::new(...) is a deprecated-syntax ERROR. No budget should
    // let it through.
    let dir = make_fixture("error", "fn main() {\n    let x = Box::new(5);\n}\n");
    let out = run(&dir, &["--max-warnings", "100"]);
    assert!(
        !out.status.success(),
        "error must fail even with high warnings budget"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn no_max_warnings_falls_back_to_deny_warnings_semantics() {
    // Without --max-warnings, --deny-warnings still fails.
    let dir = make_fixture("deny", "fn main() {\n    // TODO: a\n}\n");
    let out = run(&dir, &["--deny-warnings"]);
    assert!(
        !out.status.success(),
        "--deny-warnings without --max-warnings should still fail on warnings"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
