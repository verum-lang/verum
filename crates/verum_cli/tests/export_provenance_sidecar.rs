//! Integration tests for `verum export --with-provenance` (
//! follow-up, §8.5 V2-foundation).
//!
//! Statement-level export (Admitted / sorry / `?` placeholder) is
//! unchanged when the flag is absent. With the flag, a per-decl
//! provenance JSON sidecar lands at `<output>.provenance.json`
//! carrying name / kind / source_file / framework_name /
//! framework_citation / discharge_strategy / obligation_hash /
//! proof_term. The sidecar is the V2 wire-format that V2.1+ SMT
//! replay will populate (`obligation_hash` + `proof_term` are
//! `null` today).

#![allow(unused_imports)]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::TempDir;

fn create_project(name: &str, main_vr: &str) -> (TempDir, PathBuf) {
    let temp = TempDir::new().expect("tempdir");
    let dir = temp.path().join(name);
    fs::create_dir_all(&dir).expect("project dir");
    let manifest = format!(
        r#"[cog]
name = "{name}"
version = "0.1.0"

[language]
profile = "application"

[dependencies]
"#
    );
    fs::write(dir.join("Verum.toml"), manifest).expect("Verum.toml");
    let src = dir.join("src");
    fs::create_dir_all(&src).expect("src/");
    fs::write(src.join("main.vr"), main_vr).expect("main.vr");
    (temp, dir)
}

fn run_verum(args: &[&str], cwd: &PathBuf) -> Output {
    Command::new(env!("CARGO_BIN_EXE_verum"))
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("verum CLI")
}

#[test]
fn export_without_flag_does_not_emit_sidecar() {
    let (_t, dir) = create_project(
        "export_no_sidecar",
        r#"@framework(lurie_htt, "HTT 6.2.2.7")
public theorem yoneda() -> Bool { true }

public fn main() -> Int { 0 }
"#,
    );
    let out_path = dir.join("out.lean");
    let out = run_verum(
        &[
            "export",
            "--to",
            "lean",
            "--output",
            out_path.to_str().unwrap(),
        ],
        &dir,
    );
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(out_path.exists(), "main certificate must be written");
    let sidecar = dir.join("out.lean.provenance.json");
    assert!(
        !sidecar.exists(),
        "sidecar must NOT be written without --with-provenance"
    );
}

#[test]
fn export_with_flag_emits_sidecar_with_schema_v1() {
    let (_t, dir) = create_project(
        "export_sidecar_basic",
        r#"@framework(lurie_htt, "HTT 6.2.2.7")
public theorem yoneda() -> Bool { true }

public axiom ax_a() -> Bool;

public fn main() -> Int { 0 }
"#,
    );
    let out_path = dir.join("out.lean");
    let out = run_verum(
        &[
            "export",
            "--to",
            "lean",
            "--output",
            out_path.to_str().unwrap(),
            "--with-provenance",
        ],
        &dir,
    );
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let sidecar_path = dir.join("out.lean.provenance.json");
    assert!(sidecar_path.exists(), "sidecar must exist");
    let body = fs::read_to_string(&sidecar_path).expect("read sidecar");
    assert!(body.contains("\"schema_version\": 1"), "schema_version: {}", body);
    assert!(body.contains("\"target_format\": \"lean\""), "target_format: {}", body);
    assert!(body.contains("\"discharge_strategy\": \"statement_only\""), "discharge: {}", body);
    assert!(body.contains("\"obligation_hash\": null"), "obligation_hash V2.1 slot must be null: {}", body);
    assert!(body.contains("\"proof_term\": null"), "proof_term V2.1 slot must be null: {}", body);
    assert!(body.contains("\"name\": \"yoneda\""), "yoneda missing: {}", body);
    assert!(
        body.contains("\"framework_name\": \"lurie_htt\""),
        "framework attr missing: {}",
        body
    );
    assert!(
        body.contains("\"framework_citation\": \"HTT 6.2.2.7\""),
        "citation missing: {}",
        body
    );
}

#[test]
fn export_sidecar_handles_no_framework_attribution() {
    let (_t, dir) = create_project(
        "export_sidecar_unattributed",
        r#"public theorem unattested() -> Bool { true }
public fn main() -> Int { 0 }
"#,
    );
    let out_path = dir.join("out.coq");
    let out = run_verum(
        &[
            "export",
            "--to",
            "coq",
            "--output",
            out_path.to_str().unwrap(),
            "--with-provenance",
        ],
        &dir,
    );
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let sidecar = fs::read_to_string(dir.join("out.coq.provenance.json")).expect("read");
    assert!(
        sidecar.contains("\"framework_name\": null"),
        "no-framework decl must serialise framework_name=null: {}",
        sidecar
    );
}

#[test]
fn export_sidecar_export_proofs_alias_works() {
    // export-proofs alias must accept --with-provenance too.
    let (_t, dir) = create_project(
        "export_sidecar_alias",
        r#"public theorem t1() -> Bool { true }
public fn main() -> Int { 0 }
"#,
    );
    let out_path = dir.join("out.dk");
    let out = run_verum(
        &[
            "export-proofs",
            "--to",
            "dedukti",
            "--output",
            out_path.to_str().unwrap(),
            "--with-provenance",
        ],
        &dir,
    );
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let sidecar = dir.join("out.dk.provenance.json");
    assert!(sidecar.exists(), "alias must emit sidecar");
    let body = fs::read_to_string(&sidecar).unwrap();
    assert!(body.contains("\"target_format\": \"dedukti\""));
}
