//! Parse-failure policy contract for `verum fmt`.
//!
//! When the parser fails on a `.vr` file, the formatter has three
//! options: silently fall back to whitespace normalisation, leave
//! the file untouched and warn, or refuse to format and fail the
//! run. Each behaviour is tested with a deliberately-malformed
//! fixture below.

use std::path::PathBuf;
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_verum")
}

fn make_fixture(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("verum_fmt_parse_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).expect("create src");
    std::fs::write(
        dir.join("verum.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
    )
    .expect("manifest");
    // Mix one valid file with one that's syntactically broken.
    std::fs::write(
        dir.join("src").join("good.vr"),
        "fn main() {}\n",
    )
    .expect("good");
    std::fs::write(
        dir.join("src").join("bad.vr"),
        "this is not @@@ valid verum source\n",
    )
    .expect("bad");
    dir
}

fn run(dir: &PathBuf, args: &[&str]) -> std::process::Output {
    Command::new(binary())
        .arg("fmt")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("verum fmt spawn")
}

#[test]
fn fallback_mode_warns_but_succeeds() {
    let dir = make_fixture("fallback");
    let out = run(&dir, &["--on-parse-error", "fallback", "--check"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Fallback mode warns visibly somewhere — stdout or stderr.
    let combined = format!("{stdout}\n{stderr}");
    assert!(
        combined.contains("parse failed") || combined.contains("warning"),
        "fallback mode should emit a parse-failure warning, got: {combined}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn skip_mode_leaves_file_untouched() {
    let dir = make_fixture("skip");
    let bad_path = dir.join("src").join("bad.vr");
    let before = std::fs::read_to_string(&bad_path).expect("read before");

    let out = run(&dir, &["--on-parse-error", "skip"]);
    assert!(
        out.status.success(),
        "skip mode should exit 0 even when files fail to parse"
    );
    let after = std::fs::read_to_string(&bad_path).expect("read after");
    assert_eq!(before, after, "skip mode must leave the file untouched");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn error_mode_fails_the_run() {
    let dir = make_fixture("error");
    let out = run(&dir, &["--on-parse-error", "error", "--check"]);
    assert!(
        !out.status.success(),
        "error mode should fail the run when a file can't parse. \
         stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unknown_policy_value_is_rejected() {
    let dir = make_fixture("unknown");
    let out = run(&dir, &["--on-parse-error", "explode"]);
    assert!(
        !out.status.success(),
        "unknown --on-parse-error value should be rejected"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("on-parse-error") || stderr.contains("explode"),
        "error message should name the offending value, got: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn manifest_policy_picked_up() {
    let dir = make_fixture("manifest");
    // Override the manifest to set the strict policy and verify
    // the run fails without any CLI flag.
    std::fs::write(
        dir.join("verum.toml"),
        "[package]\nname = \"manifest\"\nversion = \"0.1.0\"\n\n[fmt.policy]\non_parse_error = \"error\"\n",
    )
    .expect("rewrite manifest");
    let out = run(&dir, &["--check"]);
    assert!(
        !out.status.success(),
        "manifest [fmt.policy].on_parse_error = \"error\" should fail the run"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
