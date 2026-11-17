//! These test process/input binding, not Isabelle proof checking.
#![cfg(unix)]
#[path = "../src/commands/audit_schema_replay.rs"]
mod audit_schema_replay;

use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

fn executable(body: &str) -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("verifier-fixture");
    fs::write(&path, format!("#!/bin/sh\nset -eu\n{body}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    (directory, path)
}

#[test]
fn replay_stages_the_theory_and_session_and_preserves_process_failure() {
    for code in [0, 7] {
        let (_directory, path) = executable(&format!(
            "test -s ROOT\ntest -s KernelSoundness.thy\nexit {code}"
        ));
        let result = audit_schema_replay::isabelle(&path, b"test input, not a proof").unwrap();
        assert_eq!(result.status.code(), Some(code));
    }
}

#[test]
fn successful_exit_cannot_hide_changed_or_missing_replay_inputs() {
    for mutation in [
        ": > KernelSoundness.thy",
        "rm KernelSoundness.thy",
        ": > ROOT",
        "rm ROOT",
    ] {
        let (_directory, path) = executable(&format!("{mutation}\nexit 0"));
        assert!(
            audit_schema_replay::isabelle(&path, b"original input").is_err(),
            "{mutation}"
        );
    }
}
