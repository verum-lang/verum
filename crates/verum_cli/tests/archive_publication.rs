//! Execute source-publication archive controls through the shipped CLI and
//! its freshly embedded standard library, including project type checking.
use std::process::Command;

fn run_fixture(source: &str, expected: &str) {
    let project = tempfile::tempdir().expect("test-owned project");
    std::fs::create_dir(project.path().join("src")).unwrap();
    std::fs::write(project.path().join("verum.toml"),
        "[cog]\nname = \"archive-controls\"\nversion = \"0.1.0\"\nedition = \"2026\"\n").unwrap();
    std::fs::write(project.path().join("src/main.vr"), source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_verum"))
        .args(["run", "--tier", "interpret"])
        .current_dir(project.path())
        .env("NO_COLOR", "1")
        .env("VERUM_NO_OBJECT_CACHE", "1")
        .output().expect("ordinary CLI invocation");
    assert!(output.status.success(), "{}\n{}",
        String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), expected);
}

#[test]
fn complete_strict_reader_controls() {
    run_fixture(include_str!("../../../core-tests/archive/tar/strict_reader_test.vr"),
        "tar-strict-reader: 13 passed");
}

#[test]
fn real_cli_archive_with_long_name_and_binary_data() {
    run_fixture(include_str!("../../../core-tests/archive/tar/cli_source_cog_test.vr"),
        "tar-cli-producer: 1 passed");
}

#[test]
fn borrowed_entry_api() {
    run_fixture(include_str!("../../../core-tests/archive/tar/entry_api_test.vr"),
        "tar-entry-api: 3 passed");
}

#[test]
fn generic_archive_conversion() {
    run_fixture(include_str!("../../../core-tests/archive/tar/adapter_api_test.vr"),
        "tar-adapter-api: 3 passed");
}
