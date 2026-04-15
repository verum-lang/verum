//! End-to-end tests for language-feature gates exercised through the
//! real CLI binary.
//!
//! These tests close the audit gap "no test that actually runs
//! `verum <cmd>` against a real verum.toml and checks the observable
//! behavior". We don't compile .vr source here (that's what
//! cli_integration_tests.rs covers); instead we focus on the
//! **config/feature-gate layer**: does the CLI actually honor the
//! verum.toml values and CLI overrides, and does it produce the
//! audit-specified error format for invalid input?

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Test harness — ported from cli_integration_tests.rs so this file is
// self-contained.
// ---------------------------------------------------------------------------

fn project(tag: &str) -> (TempDir, PathBuf) {
    let temp = TempDir::new().expect("mkdtemp");
    let dir = temp.path().join(tag);
    fs::create_dir_all(&dir).expect("mkdir");
    (temp, dir)
}

fn write_manifest(dir: &PathBuf, extra: &str) {
    let body = format!(
        "[cog]\nname = \"gate-test\"\nversion = \"0.1.0\"\n\n\
         [language]\nprofile = \"application\"\n\n\
         {}",
        extra
    );
    fs::write(dir.join("verum.toml"), body).expect("write verum.toml");
}

fn verum(args: &[&str], cwd: &PathBuf) -> Output {
    Command::new(env!("CARGO_BIN_EXE_verum"))
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run verum")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}
fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

// ---------------------------------------------------------------------------
// `verum config show` — human panel and JSON
// ---------------------------------------------------------------------------

#[test]
fn config_show_reflects_toml_values() {
    let (_tmp, dir) = project("cfg-toml");
    write_manifest(
        &dir,
        "[types]\ncubical = false\nuniverse_polymorphism = true\n\n\
         [safety]\nunsafe_allowed = false\n",
    );

    let out = verum(&["config", "show"], &dir);
    assert!(
        out.status.success(),
        "config show must succeed on valid manifest\nstderr: {}",
        stderr(&out)
    );
    let text = stdout(&out);
    // Check the resolved feature set reflects the TOML.
    assert!(
        text.contains("cubical") && text.contains("disabled"),
        "panel must show cubical=disabled\n{}",
        text
    );
    assert!(
        text.contains("universe_polymorphism") && text.contains("enabled"),
        "panel must show universe_polymorphism=enabled\n{}",
        text
    );
    assert!(
        text.contains("unsafe_allowed") && text.contains("disabled"),
        "panel must show unsafe_allowed=disabled\n{}",
        text
    );
}

#[test]
fn config_show_json_is_valid_and_carries_values() {
    let (_tmp, dir) = project("cfg-json");
    write_manifest(&dir, "[meta]\nderive = false\n");

    let out = verum(&["config", "show", "--json"], &dir);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let body = stdout(&out);
    let value: serde_json::Value =
        serde_json::from_str(&body).expect("config show --json must emit valid JSON");
    assert_eq!(value["features"]["meta"]["derive"], false);
    // The cog name also surfaces, so tools can identify the project.
    assert_eq!(value["cog"]["name"], "gate-test");
}

#[test]
fn cli_z_override_wins_over_toml() {
    let (_tmp, dir) = project("cfg-override");
    write_manifest(&dir, "[meta]\nderive = true\n");

    // TOML says derive=true; -Z flips it to false.
    let out = verum(
        &["config", "show", "--json", "-Z", "meta.derive=false"],
        &dir,
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(
        value["features"]["meta"]["derive"], false,
        "-Z meta.derive=false must win over verum.toml derive=true"
    );
}

#[test]
fn high_level_flag_and_z_compose() {
    let (_tmp, dir) = project("cfg-compose");
    write_manifest(&dir, "");

    let out = verum(
        &[
            "config",
            "show",
            "--json",
            "--no-cubical",
            "-Z",
            "safety.unsafe_allowed=false",
            "-Z",
            "runtime.async_worker_threads=8",
        ],
        &dir,
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(v["features"]["types"]["cubical"], false);
    assert_eq!(v["features"]["safety"]["unsafe_allowed"], false);
    assert_eq!(v["features"]["runtime"]["async_worker_threads"], 8);
}

// ---------------------------------------------------------------------------
// Validation errors — file, allowed values, "did you mean"
// ---------------------------------------------------------------------------

#[test]
fn enum_typo_suggests_correct_value() {
    let (_tmp, dir) = project("cfg-typo");
    // "mxed" is one edit away from "mixed" — must trigger suggestion.
    write_manifest(&dir, "[runtime]\ncbgr_mode = \"mxed\"\n");

    let out = verum(&["config", "show"], &dir);
    assert!(!out.status.success(), "invalid config must exit non-zero");
    let err = stderr(&out);
    assert!(err.contains("verum.toml"), "error must cite verum.toml:\n{}", err);
    assert!(
        err.contains("[runtime].cbgr_mode"),
        "error must cite the section.field path:\n{}",
        err
    );
    assert!(
        err.contains("allowed values:") && err.contains("mixed"),
        "error must list allowed values:\n{}",
        err
    );
    assert!(
        err.contains("did you mean") && err.contains("mixed"),
        "typo must produce did-you-mean suggestion:\n{}",
        err
    );
}

#[test]
fn far_value_has_no_suggestion_but_lists_allowed() {
    let (_tmp, dir) = project("cfg-far");
    // "quantum" is far from every allowed tier — no suggestion.
    write_manifest(&dir, "[codegen]\ntier = \"quantum\"\n");

    let out = verum(&["config", "show"], &dir);
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(
        err.contains("allowed values:") && err.contains("aot"),
        "must still list allowed values:\n{}",
        err
    );
    assert!(
        !err.contains("did you mean"),
        "no near-match should omit suggestion:\n{}",
        err
    );
}

#[test]
fn unknown_z_key_errors_with_discoverable_message() {
    let (_tmp, dir) = project("cfg-unknown-z");
    write_manifest(&dir, "");

    let out = verum(&["config", "show", "-Z", "nope.bogus=true"], &dir);
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(
        err.contains("unknown override key"),
        "unknown -Z must surface a clear error:\n{}",
        err
    );
    // Error mentions the supported prefixes so users know where to look.
    assert!(
        err.contains("types") && err.contains("runtime") && err.contains("codegen"),
        "error must list supported prefixes:\n{}",
        err
    );
}

#[test]
fn malformed_z_reports_format() {
    let (_tmp, dir) = project("cfg-malformed-z");
    write_manifest(&dir, "");

    let out = verum(&["config", "show", "-Z", "missing_equals"], &dir);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("KEY=VALUE"),
        "malformed -Z must mention expected format"
    );
}

// ---------------------------------------------------------------------------
// Help discoverability — `Language features` group appears on every
// compile-adjacent subcommand.
// ---------------------------------------------------------------------------

#[test]
fn language_features_group_appears_in_build_help() {
    let (_tmp, dir) = project("help");
    let out = verum(&["build", "--help"], &dir);
    let text = stdout(&out);
    assert!(
        text.contains("Language features"),
        "`verum build --help` must show the Language features group"
    );
    assert!(text.contains("--tier"), "build help must list --tier");
    assert!(
        text.contains("-Z") && text.contains("KEY=VAL"),
        "build help must document -Z KEY=VAL"
    );
}

#[test]
fn language_features_group_appears_in_lsp_help() {
    // After P0.2, LSP must also expose the same group.
    let (_tmp, dir) = project("help-lsp");
    let out = verum(&["lsp", "--help"], &dir);
    assert!(
        stdout(&out).contains("Language features"),
        "verum lsp --help must show Language features group (P0.2)"
    );
}

#[test]
fn language_features_group_appears_in_fmt_help() {
    let (_tmp, dir) = project("help-fmt");
    let out = verum(&["fmt", "--help"], &dir);
    assert!(
        stdout(&out).contains("Language features"),
        "verum fmt --help must show Language features group (P0.2)"
    );
}
