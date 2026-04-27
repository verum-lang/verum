//! Integration tests for `verum audit --hygiene-strict` (F3, V8.1
//! follow-up #196).
//!
//! Per V2: walk every top-level free function body for
//! raw `self` occurrences. A *free function* is one declared at
//! module scope whose first parameter is NOT a self-receiver.
//! Methods (functions inside `implement` / `protocol` blocks, or
//! free functions with `&self` / `self` params) are skipped.
//! Violations surface as `E_HYGIENE_UNFACTORED_SELF` and the CLI
//! exits non-zero.

#![allow(unused_imports)]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::TempDir;

fn create_project(name: &str, main_vr_body: &str) -> (TempDir, PathBuf) {
    let temp = TempDir::new().expect("create tempdir");
    let dir = temp.path().join(name);
    fs::create_dir_all(&dir).expect("create project dir");
    let manifest = format!(
        r#"[cog]
name = "{name}"
version = "0.1.0"

[language]
profile = "application"

[dependencies]
"#
    );
    fs::write(dir.join("Verum.toml"), manifest).expect("write Verum.toml");
    let src = dir.join("src");
    fs::create_dir_all(&src).expect("create src/");
    fs::write(src.join("main.vr"), main_vr_body).expect("write main.vr");
    (temp, dir)
}

fn run_verum(args: &[&str], cwd: &PathBuf) -> Output {
    Command::new(env!("CARGO_BIN_EXE_verum"))
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("spawn verum CLI")
}

#[test]
fn hygiene_strict_clean_project_succeeds() {
    let (_temp, dir) = create_project(
        "hygiene_strict_clean",
        r#"public fn main() -> Int { 0 }

public fn pure_helper(x: Int, y: Int) -> Int {
    x + y
}
"#,
    );
    let out = run_verum(&["audit", "--hygiene-strict"], &dir);
    assert!(
        out.status.success(),
        "clean project must pass; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn hygiene_strict_flags_raw_self_in_free_function() {
    // Free function `bad` uses `self.x` — illegal: `self` is not
    // bound in a free-function scope. Must surface as
    // E_HYGIENE_UNFACTORED_SELF and exit non-zero.
    let (_temp, dir) = create_project(
        "hygiene_strict_violation",
        r#"public fn bad(x: Int) -> Int {
    self.x + x
}

public fn main() -> Int { 0 }
"#,
    );
    let out = run_verum(&["audit", "--hygiene-strict"], &dir);
    assert!(
        !out.status.success(),
        "violation must fail; stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        combined.contains("E_HYGIENE_UNFACTORED_SELF"),
        "expected error code in output: {}",
        combined
    );
    assert!(combined.contains("bad"), "expected `bad` function in output: {}", combined);
}

#[test]
fn hygiene_strict_skips_methods_with_self_receiver() {
    // A free function with `&self` param IS a method (not a free
    // function in the §13.3 sense), so its `self.x` body is OK.
    // The walker skips methods entirely — `is_method()` is true.
    let (_temp, dir) = create_project(
        "hygiene_strict_method",
        r#"public type Counter is { count: Int };

implement Counter {
    public fn get(&self) -> Int {
        self.count
    }
}

public fn main() -> Int { 0 }
"#,
    );
    let out = run_verum(&["audit", "--hygiene-strict"], &dir);
    assert!(
        out.status.success(),
        "method body using self must not flag; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn hygiene_strict_json_format_emits_schema_v1() {
    let (_temp, dir) = create_project(
        "hygiene_strict_json",
        r#"public fn bad(x: Int) -> Int { self.x + x }
public fn main() -> Int { 0 }
"#,
    );
    let out = run_verum(
        &["audit", "--hygiene-strict", "--format", "json"],
        &dir,
    );
    assert!(!out.status.success(), "violation must fail");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("\"schema_version\": 1"),
        "JSON missing schema_version: {}",
        stdout
    );
    assert!(
        stdout.contains("\"error_code\": \"E_HYGIENE_UNFACTORED_SELF\""),
        "JSON missing error_code: {}",
        stdout
    );
    assert!(
        stdout.contains("\"violation_count\":"),
        "JSON missing violation_count: {}",
        stdout
    );
}

#[test]
fn hygiene_strict_no_files_succeeds_quietly() {
    let temp = TempDir::new().expect("create tempdir");
    let dir = temp.path().join("hygiene_strict_empty");
    fs::create_dir_all(&dir).expect("create project dir");
    let manifest = r#"[cog]
name = "hygiene_strict_empty"
version = "0.1.0"

[language]
profile = "application"

[dependencies]
"#;
    fs::write(dir.join("Verum.toml"), manifest).expect("write Verum.toml");
    let src = dir.join("src");
    fs::create_dir_all(&src).expect("create src/");
    let out = run_verum(&["audit", "--hygiene-strict"], &dir);
    // No .vr files → graceful success (V1 hygiene reporter does the same).
    assert!(out.status.success());
}
