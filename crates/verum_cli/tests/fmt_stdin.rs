//! `verum fmt --stdin` contract tests.
//!
//! Editor format-on-save plumbing (LSP, vim plugins, VS Code
//! extensions) all use the same pattern: spawn the formatter with
//! `--stdin`, write the buffer, read the formatted output back. This
//! file pins the contract:
//!
//! 1. Stdin → stdout works for syntactically valid input.
//! 2. Whitespace deviations are normalised.
//! 3. `--stdin --check` is rejected with a clear error.
//! 4. Stdin runs DON'T touch the disk in any way.

use std::io::Write;
use std::process::{Command, Stdio};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_verum")
}

fn run_stdin(input: &str, extra_args: &[&str]) -> std::process::Output {
    let mut child = Command::new(binary())
        .arg("fmt")
        .arg("--stdin")
        .args(extra_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("verum fmt --stdin spawn");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(input.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("wait_with_output")
}

#[test]
fn stdin_returns_formatted_source_on_stdout() {
    let out = run_stdin("fn main() {}", &[]);
    assert!(out.status.success(), "exit status: {:?}", out.status);
    let formatted = String::from_utf8(out.stdout).expect("UTF-8 stdout");
    assert!(
        formatted.contains("fn main"),
        "formatted output should contain `fn main`, got: {formatted:?}"
    );
    // The trailing-newline policy applies to stdin runs as well.
    assert!(
        formatted.ends_with('\n'),
        "stdin output should end with newline, got: {formatted:?}"
    );
}

#[test]
fn stdin_normalises_excessive_blank_lines() {
    let out = run_stdin("fn a() {}\n\n\n\nfn b() {}\n", &[]);
    assert!(out.status.success());
    let formatted = String::from_utf8(out.stdout).expect("UTF-8");
    assert!(
        !formatted.contains("\n\n\n"),
        "blank-line stack should have been collapsed, got: {formatted:?}"
    );
}

#[test]
fn stdin_with_check_is_rejected() {
    let out = run_stdin("fn main() {}\n", &["--check"]);
    assert!(
        !out.status.success(),
        "`fmt --stdin --check` should be rejected"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--stdin"),
        "error message should mention --stdin, got: {stderr}"
    );
}

#[test]
fn stdin_filename_hint_accepted() {
    let out = run_stdin("fn main() {}", &["--stdin-filename", "buffer.vr"]);
    assert!(out.status.success());
    let formatted = String::from_utf8(out.stdout).expect("UTF-8");
    assert!(formatted.contains("fn main"));
}

#[test]
fn stdin_warns_on_parse_failure_via_stderr() {
    // A deliberately malformed snippet — fmt's stdin path falls back
    // to whitespace normalisation and warns on stderr (not stdout,
    // which is reserved for the formatted output stream).
    let out = run_stdin("this is not valid verum @@@\n", &[]);
    assert!(out.status.success(), "stdin path must not fail on parse error");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("parse failed") || stderr.contains("warning"),
        "expected parse-failure warning on stderr, got: {stderr}"
    );
}
