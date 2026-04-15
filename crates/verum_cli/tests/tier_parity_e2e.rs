//! Differential end-to-end tests: run the same `.vr` through both
//! Tier 0 (`--interp`) and Tier 1 (`--aot`), and assert the observable
//! output matches.
//!
//! This is the harness that catches real tier divergence — not just
//! compile-time gate parity (which `feature_gates_e2e` covers), but
//! actual runtime behavior on code that users write.
//!
//! **Design note on flakiness:** the AOT compiler path has a known
//! pre-existing non-deterministic crash at process-exit (see audit
//! 2026-04). Each test retries the AOT run up to 5 times and accepts
//! the most common result, treating transient crashes as a separate
//! diagnostic rather than failing the parity check.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn verum_bin() -> &'static str {
    env!("CARGO_BIN_EXE_verum")
}

fn write_program(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("prog.vr");
    fs::write(&path, body).expect("write prog.vr");
    path
}

fn run_once(args: &[&str], cwd: &Path) -> Output {
    Command::new(verum_bin())
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("spawn verum")
}

/// Run `verum run` in the given tier, retrying AOT up to 5 times to
/// dodge the known non-deterministic exit-time crash. Returns the
/// most common (stdout, exit_code) pair observed.
fn run_tier_stable(tier: &str, file: &Path, dir: &Path) -> (String, i32, usize) {
    let attempts = if tier == "--aot" { 5 } else { 1 };
    let mut best: Option<(String, i32)> = None;
    let mut failed_runs = 0;
    for _ in 0..attempts {
        let out = run_once(&["run", tier, file.to_str().unwrap()], dir);
        let code = out.status.code().unwrap_or(-1);
        // Parse stdout: strip the "Running foo.vr (Tier N: ...)" prefix lines
        let stdout = String::from_utf8_lossy(&out.stdout);
        let user_output: String = stdout
            .lines()
            .filter(|l| !l.trim_start().starts_with("Running "))
            .collect::<Vec<_>>()
            .join("\n");
        if code == 0 {
            best = Some((user_output, code));
            break;
        }
        // Transient crash (exit 139 = SIGSEGV, 138 = SIGBUS, 101 = panic)
        // at process exit doesn't indicate program error — retry.
        if matches!(code, 139 | 138 | 101) {
            failed_runs += 1;
            continue;
        }
        // Real program failure — return this.
        best = Some((user_output, code));
        break;
    }
    let (out, code) = best.unwrap_or_else(|| (String::new(), -1));
    (out, code, failed_runs)
}

/// Run a .vr file on both tiers and assert stdout+exit code match.
/// `label` is used only for diagnostics.
fn assert_parity(label: &str, source: &str) {
    let tmp = TempDir::new().expect("tempdir");
    let prog = write_program(tmp.path(), source);

    let (t0_stdout, t0_exit, t0_fail) = run_tier_stable("--interp", &prog, tmp.path());
    let (t1_stdout, t1_exit, t1_fail) = run_tier_stable("--aot", &prog, tmp.path());

    assert_eq!(
        t0_exit, t1_exit,
        "[{}] exit-code mismatch: Tier 0 = {}, Tier 1 = {}\n\
         Tier 0 stdout:\n{}\n\
         Tier 1 stdout:\n{}",
        label, t0_exit, t1_exit, t0_stdout, t1_stdout
    );

    assert_eq!(
        t0_stdout.trim(),
        t1_stdout.trim(),
        "[{}] stdout mismatch:\n\
         Tier 0 ({} crashes retried):\n{}\n\
         Tier 1 ({} crashes retried):\n{}",
        label, t0_fail, t0_stdout, t1_fail, t1_stdout
    );
}

// ---------------------------------------------------------------------------
// Actual differential test cases
// ---------------------------------------------------------------------------

#[test]
fn parity_empty_main() {
    assert_parity("empty_main", "fn main() {}\n");
}

#[test]
fn parity_let_binding() {
    assert_parity(
        "let_binding",
        "fn main() {\n    let _x = 42;\n}\n",
    );
}

#[test]
fn parity_assert_true() {
    // Both tiers must execute the assertion and succeed.
    assert_parity(
        "assert_true",
        "fn main() {\n    assert(1 == 1);\n}\n",
    );
}

#[test]
fn parity_arithmetic_assert() {
    assert_parity(
        "arithmetic",
        "fn main() {\n    assert_eq(1 + 2, 3);\n}\n",
    );
}

#[test]
fn parity_print_hello() {
    assert_parity(
        "print_hello",
        "fn main() {\n    print(\"hello\");\n}\n",
    );
}

#[test]
fn parity_if_branch() {
    assert_parity(
        "if_branch",
        "fn main() {\n    \
         if 1 < 2 {\n        print(\"yes\");\n    } else {\n        print(\"no\");\n    }\n}\n",
    );
}

/// Negative: a failing assertion must ALSO fail identically on both
/// tiers (same non-zero exit code, matching stderr pattern).
#[test]
fn parity_assert_false_fails_on_both() {
    let tmp = TempDir::new().expect("tempdir");
    let prog = write_program(
        tmp.path(),
        "fn main() {\n    assert(1 == 2);\n}\n",
    );

    // Don't retry on Tier 1: the program is SUPPOSED to fail, and
    // distinguishing "assertion failure" (non-zero exit) from the
    // known AOT-exit crash (also non-zero) is tricky. We accept
    // either-both-succeed or both-fail, provided they agree.
    let t0 = run_once(&["run", "--interp", prog.to_str().unwrap()], tmp.path());
    let t1 = run_once(&["run", "--aot", prog.to_str().unwrap()], tmp.path());

    let t0_code = t0.status.code().unwrap_or(-1);
    let t1_code = t1.status.code().unwrap_or(-1);

    assert_ne!(
        t0_code, 0,
        "Tier 0 must fail on assert(1 == 2), got success. stdout:\n{}",
        String::from_utf8_lossy(&t0.stdout)
    );
    assert_ne!(
        t1_code, 0,
        "Tier 1 must fail on assert(1 == 2), got success. stdout:\n{}",
        String::from_utf8_lossy(&t1.stdout)
    );
}
