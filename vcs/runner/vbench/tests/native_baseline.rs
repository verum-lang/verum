#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Output};

fn run(reference: &str) -> Output {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    fs::write(path.join("input"), b"\x00\xff\n").unwrap();
    for (name, body) in [("candidate", "cat"), ("reference", reference)] {
        let script = path.join(name);
        fs::write(&script, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(script, fs::Permissions::from_mode(0o700)).unwrap();
    }
    Command::new(env!("CARGO_BIN_EXE_vbench"))
        .current_dir(path)
        .args([
            "baseline",
            "--language",
            "cpp",
            "--candidate",
            "candidate",
            "--reference",
            "reference",
            "--input",
            "input",
            "--expected-stdout",
            "input",
            "--samples",
            "2",
            "--timeout-secs",
            "1",
            "--format",
            "json",
        ])
        .output()
        .unwrap()
}

#[test]
fn reports_only_measured_programs_on_checked_input() {
    let output = run("cat");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let rows = report["results"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    for row in rows {
        assert_eq!(row["statistics"]["count"], 2);
        assert_eq!(
            row["metadata"]["executable_blake3"].as_str().unwrap().len(),
            64
        );
    }
}

#[test]
fn wrong_output_status_and_timeout_cannot_be_reported_as_comparisons() {
    for (script, reason) in [
        ("printf 'bad'", "stdout bytes"),
        ("cat; exit 1", "process status"),
        ("exec sleep 5", "exceeded"),
    ] {
        let output = run(script);
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(reason),
            "{output:?}"
        );
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn a_language_label_alone_cannot_request_a_comparison() {
    let output = Command::new(env!("CARGO_BIN_EXE_vbench"))
        .args(["run", "--compare", "c"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("vbench baseline"));
}
