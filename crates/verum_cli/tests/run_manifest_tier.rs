//! T1561: exercise real CLI dispatch before either backend compiles source.
//!
//! Missing main identifies the interpreter entry path. A regular `target`
//! file blocks the native output directory after its Compiling banner.
//! Thus these decisions cannot accidentally execute a program or run LLVM.

use std::{
    fs,
    process::{Command, Output},
};
use verum_common::Text;

struct Observation(Output);

impl Observation {
    fn contains(&self, needle: &str) -> bool {
        String::from_utf8_lossy(&self.0.stdout).contains(needle)
            || String::from_utf8_lossy(&self.0.stderr).contains(needle)
    }

    fn interpreter(&self) {
        assert!(!self.0.status.success(), "{:?}", self.0);
        assert!(self.contains("Entry point not found"), "{:?}", self.0);
        assert!(!self.contains("Compiling"), "{:?}", self.0);
    }

    fn aot(&self) {
        assert!(!self.0.status.success(), "{:?}", self.0);
        assert!(self.contains("Compiling"), "{:?}", self.0);
        assert!(!self.contains("Entry point not found"), "{:?}", self.0);
    }
}

fn project(codegen_tier: Option<&str>, flags: &[&str], explicit_directory: bool) -> Observation {
    let dir = tempfile::tempdir().expect("project fixture");
    let mut manifest: Text = "[cog]\nname = \"tier_probe\"\nversion = \"0.1.0\"\n".into();
    if let Some(tier) = codegen_tier {
        manifest.push_str(&format!("[codegen]\ntier = {tier:?}\n"));
    }
    fs::write(dir.path().join("Verum.toml"), manifest).expect("manifest");
    fs::write(dir.path().join("target"), "not a directory").expect("native obstruction");
    let mut command = Command::new(env!("CARGO_BIN_EXE_verum"));
    command.arg("run").args(flags).current_dir(dir.path());
    if explicit_directory {
        command.arg(dir.path());
    }
    Observation(command.output().expect("CLI decision"))
}

#[test]
fn absent_selector_honors_each_manifest_execution_tier() {
    for explicit_directory in [false, true] {
        project(None, &[], explicit_directory).interpreter();
        project(Some("interpret"), &[], explicit_directory).interpreter();
        project(Some("aot"), &[], explicit_directory).aot();
    }
}

#[test]
fn release_does_not_override_an_interpreter_manifest() {
    project(Some("interpret"), &["--release"], false).interpreter();
    project(Some("aot"), &["--release"], false).aot();
}

#[test]
fn explicit_named_selectors_override_the_manifest() {
    project(Some("aot"), &["--tier", "interpret"], false).interpreter();
    project(Some("aot"), &["--tier", "interpreter"], false).interpreter();
    project(Some("interpret"), &["--tier", "aot"], false).aot();
}

#[test]
fn legacy_shortcuts_keep_precedence_over_named_selectors() {
    project(Some("aot"), &["--interp"], false).interpreter();
    project(Some("interpret"), &["--aot"], false).aot();
    project(Some("aot"), &["--interp", "--tier", "aot"], false).interpreter();
    project(Some("interpret"), &["--aot", "--tier", "interpret"], false).aot();
}

#[test]
fn conflicting_shortcuts_are_rejected_before_project_dispatch() {
    let observation = project(Some("aot"), &["--interp", "--aot"], false);
    assert_eq!(observation.0.status.code(), Some(2), "{:?}", observation.0);
    assert!(
        observation.contains("cannot be used with"),
        "{:?}",
        observation.0
    );
    assert!(!observation.contains("Compiling"));
    assert!(!observation.contains("Entry point not found"));
}

#[test]
fn check_only_is_rejected_from_manifest_and_explicit_selector() {
    let manifest = project(Some("check"), &[], false);
    assert!(!manifest.0.status.success());
    assert!(manifest.contains("[codegen].tier"), "{:?}", manifest.0);
    assert!(manifest.contains("not `verum run`"), "{:?}", manifest.0);
    let explicit = project(Some("aot"), &["--tier", "check"], false);
    assert!(!explicit.0.status.success());
    assert!(explicit.contains("--tier check"), "{:?}", explicit.0);
}

#[test]
fn explicit_execution_selector_overrides_check_only_manifest() {
    project(Some("check"), &["--interp"], false).interpreter();
    project(Some("check"), &["--tier", "aot"], false).aot();
}

#[test]
fn unknown_explicit_tier_is_rejected() {
    let observation = project(Some("interpret"), &["--tier", "jit"], false);
    assert!(!observation.0.status.success());
    assert!(observation.contains("unknown tier"), "{:?}", observation.0);
}
