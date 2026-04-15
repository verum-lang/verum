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

// ---------------------------------------------------------------------------
// `[debug]` gates — dap_enabled + port
// ---------------------------------------------------------------------------

#[test]
fn dap_disabled_via_toml_refuses_to_start() {
    let (_tmp, dir) = project("dap-off");
    write_manifest(&dir, "[debug]\ndap_enabled = false\n");

    let out = verum(&["dap", "--transport", "stdio"], &dir);
    assert!(!out.status.success(), "disabled DAP must exit non-zero");
    let err = stderr(&out);
    assert!(
        err.contains("disabled") && err.contains("[debug]"),
        "error must cite the config key:\n{}",
        err
    );
    assert!(
        err.contains("dap_enabled = true") || err.contains("-Z debug.dap_enabled"),
        "error must suggest the fix:\n{}",
        err
    );
}

#[test]
fn dap_disabled_via_z_override_refuses_to_start() {
    let (_tmp, dir) = project("dap-off-z");
    write_manifest(&dir, "");

    let out = verum(
        &["dap", "--transport", "stdio", "-Z", "debug.dap_enabled=false"],
        &dir,
    );
    assert!(
        !out.status.success(),
        "-Z debug.dap_enabled=false must also disable DAP"
    );
}

#[test]
fn dap_socket_without_port_but_toml_has_it_succeeds_in_parse() {
    // Request socket transport; manifest provides [debug].port; CLI
    // --port omitted. Startup should get past the port-resolution
    // check. We can't actually bind the socket in a test without
    // flakiness, but we can verify the argument-parsing path no longer
    // errors with "--port required".
    let (_tmp, dir) = project("dap-port");
    write_manifest(&dir, "[debug]\nport = 7777\n");

    // Give it a short timeout so we don't actually hold a server.
    // The important assertion is that the "no port" error doesn't appear.
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_verum"));
    cmd.args(&["dap", "--transport", "socket"]);
    cmd.current_dir(&dir);
    cmd.stdin(std::process::Stdio::null());
    let child = cmd.spawn().expect("spawn");
    // Kill almost immediately — we only care that parsing succeeded.
    std::thread::sleep(std::time::Duration::from_millis(150));
    let _ = nix_kill(&child);
    let out = child.wait_with_output().expect("wait");
    let err = stderr(&out);
    assert!(
        !err.contains("--port required") && !err.contains("--port is required"),
        "[debug].port must be used as fallback when --port is absent:\n{}",
        err
    );
}

// Portable child-kill: kill by PID on unix, TerminateProcess via the
// `Command::id` on Windows. For our purposes, a simple kill works.
#[cfg(unix)]
fn nix_kill(child: &std::process::Child) -> std::io::Result<()> {
    use std::process::Command as Cmd;
    Cmd::new("kill")
        .arg(child.id().to_string())
        .status()
        .map(|_| ())
}
#[cfg(not(unix))]
fn nix_kill(child: &std::process::Child) -> std::io::Result<()> {
    // Fallback: spawn tool kill. Tests on non-unix skip the precise kill.
    let _ = child.id();
    Ok(())
}

// ---------------------------------------------------------------------------
// Both-paths integration — gates fire identically in Tier 0 (interpreter)
// AND Tier 1 (AOT). Answers the core directive: "all language mechanisms
// must be fully and correctly integrated into the pipeline for both
// interpreter and AOT modes."
// ---------------------------------------------------------------------------

fn write_unsafe_main(dir: &PathBuf) -> PathBuf {
    let path = dir.join("main.vr");
    fs::write(
        &path,
        "fn main() {\n    unsafe {\n        let _x = 0;\n    }\n}\n",
    )
    .expect("write main.vr");
    path
}

/// Tier 0 (interpreter) — running a .vr with `unsafe { ... }` under
/// `-Z safety.unsafe_allowed=false` must produce a feature-gate error.
#[test]
fn tier0_interp_rejects_unsafe_when_gate_off() {
    let (_tmp, dir) = project("tier0-unsafe");
    let main = write_unsafe_main(&dir);

    let out = verum(
        &[
            "run",
            "--interp",
            main.to_str().unwrap(),
            "-Z",
            "safety.unsafe_allowed=false",
        ],
        &dir,
    );
    assert!(
        !out.status.success(),
        "Tier 0 must reject unsafe with gate off. stdout:\n{}\nstderr:\n{}",
        stdout(&out),
        stderr(&out)
    );
    let combined = format!("{}\n{}", stdout(&out), stderr(&out));
    assert!(
        combined.contains("safety gate"),
        "Tier 0 error must name the safety gate:\n{}",
        combined
    );
    // The fallback to interpreter must NOT hide the gate error.
    assert!(
        !combined.contains("Falling back to interpreter"),
        "Tier 0 must not attempt to fall back on gate rejection:\n{}",
        combined
    );
}

/// Tier 1 (AOT) — same .vr file, same `-Z` override, same expected
/// behavior. This proves the gate is consumed BEFORE the codegen-tier
/// fork (in the shared type-check phase), not after.
///
/// Additionally proves the previous "AOT fails → silently fall back
/// to interpreter" bug is fixed: a feature-gate rejection must
/// propagate as a non-zero exit, not quietly run the program.
#[test]
fn tier1_aot_rejects_unsafe_when_gate_off() {
    let (_tmp, dir) = project("tier1-unsafe");
    let main = write_unsafe_main(&dir);

    let out = verum(
        &[
            "run",
            "--aot",
            main.to_str().unwrap(),
            "-Z",
            "safety.unsafe_allowed=false",
        ],
        &dir,
    );
    assert!(
        !out.status.success(),
        "Tier 1 must reject unsafe with gate off. stdout:\n{}\nstderr:\n{}",
        stdout(&out),
        stderr(&out)
    );
    let combined = format!("{}\n{}", stdout(&out), stderr(&out));
    assert!(
        combined.contains("safety gate"),
        "Tier 1 error must name the safety gate:\n{}",
        combined
    );
    assert!(
        !combined.contains("Falling back to interpreter"),
        "Tier 1 must not silently fall back on gate rejection — that \
         would let the unsafe code actually run. The fallback is reserved \
         for infrastructure errors (LLVM glitch, etc.), not feature gates.\n{}",
        combined
    );
}

/// `verum check` — uses the same shared type-check phase that Tier 0
/// and Tier 1 do. Proves the gate fires at check time regardless of
/// whether codegen ever runs.
#[test]
fn verum_check_rejects_unsafe_when_gate_off() {
    let (_tmp, dir) = project("check-unsafe");
    let main = write_unsafe_main(&dir);

    let out = verum(
        &[
            "check",
            main.to_str().unwrap(),
            "-Z",
            "safety.unsafe_allowed=false",
        ],
        &dir,
    );
    assert!(
        !out.status.success(),
        "verum check must reject unsafe with gate off. stderr:\n{}",
        stderr(&out)
    );
}
