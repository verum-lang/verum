//! End-to-end integration tests for `verum export` with persistent
//! certificate loading.
//!
//! When an `SmtCertificate` is on-disk in `.verum/cache/certificates/`
//! for a theorem, `verum export` should load it via the
//! `FileSystemCertificateStore`, route it through the
//! `ProofReplayBackend` for the chosen target, and splice the
//! resulting tactic chain into the exported file instead of the
//! V1 `Admitted.` / `sorry` / `?` scaffold.

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

/// Drop a JSON SmtCertificate at `<dir>/.verum/cache/certificates/<name>.smt-cert.json`.
fn write_cert(dir: &PathBuf, decl_name: &str, backend: &str, trace: &str) {
    let cert_dir = dir.join(".verum").join("cache").join("certificates");
    fs::create_dir_all(&cert_dir).expect("create cert dir");
    let trace_bytes: Vec<u8> = trace.bytes().collect();
    let trace_json: String = trace_bytes
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let cert_json = format!(
        r#"{{
  "schema_version": 1,
  "backend": "{backend}",
  "backend_version": "test-1.0",
  "trace": [{trace_json}],
  "obligation_hash": "blake3:test",
  "verum_version": "0.1.0",
  "created_at": "",
  "metadata": []
}}"#
    );
    fs::write(cert_dir.join(format!("{decl_name}.smt-cert.json")), cert_json)
        .expect("write cert");
}

#[test]
fn export_without_cert_falls_back_to_admitted_for_coq() {
    let (_t, dir) = create_project(
        "no_cert_coq",
        r#"public theorem foo() -> Bool { true }
public fn main() -> Int { 0 }
"#,
    );
    let out_path = dir.join("out.v");
    let out = run_verum(
        &["export", "--to", "coq", "--output", out_path.to_str().unwrap()],
        &dir,
    );
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let body = fs::read_to_string(&out_path).unwrap();
    assert!(body.contains("Admitted"), "no-cert path must keep admitted scaffold");
}

#[test]
fn export_with_cert_splices_real_proof_for_coq() {
    let (_t, dir) = create_project(
        "with_cert_coq",
        r#"public theorem foo() -> Bool { true }
public fn main() -> Int { 0 }
"#,
    );
    // Drop a Z3-style trace that the CoqProofReplay recognises:
    // "(asserted ...)" → `intros.`, "(th-lemma ...)" → `lia.`
    write_cert(&dir, "foo", "z3", "(asserted (= a b)) (th-lemma arith)");

    let out_path = dir.join("out.v");
    let out = run_verum(
        &["export", "--to", "coq", "--output", out_path.to_str().unwrap()],
        &dir,
    );
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let body = fs::read_to_string(&out_path).unwrap();
    // Real lowered proof — `Proof. ... Qed.` envelope, NOT Admitted.
    assert!(body.contains("Qed."), "lowered proof must end with Qed.; got:\n{}", body);
    assert!(!body.contains("Admitted"), "lowered proof must NOT use Admitted; got:\n{}", body);
    // Tactic vocabulary recognised by CoqProofReplay:
    assert!(body.contains("intros."), "expected `intros.` from (asserted); got:\n{}", body);
    assert!(body.contains("lia."), "expected `lia.` from (th-lemma); got:\n{}", body);
}

#[test]
fn export_with_cert_splices_real_proof_for_lean() {
    let (_t, dir) = create_project(
        "with_cert_lean",
        r#"public theorem bar() -> Bool { true }
public fn main() -> Int { 0 }
"#,
    );
    write_cert(&dir, "bar", "z3", "(refl x) (true-axiom)");

    let out_path = dir.join("out.lean");
    let out = run_verum(
        &["export", "--to", "lean", "--output", out_path.to_str().unwrap()],
        &dir,
    );
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let body = fs::read_to_string(&out_path).unwrap();
    // Lean's `by ... ` tactic-block envelope.
    assert!(body.contains("by"), "expected lean `by` envelope; got:\n{}", body);
    // Tactic vocabulary recognised by LeanProofReplay:
    assert!(body.contains("rfl"), "expected `rfl` from (refl); got:\n{}", body);
    assert!(body.contains("trivial"), "expected `trivial` from (true-axiom); got:\n{}", body);
    // Lean baseline lemma close-out (tauto), present for theorem kind:
    assert!(body.contains("tauto"));
}

#[test]
fn export_summary_reports_replayed_vs_admitted_counts() {
    let (_t, dir) = create_project(
        "summary_counts",
        r#"public theorem with_cert() -> Bool { true }
public theorem without_cert() -> Bool { true }
public fn main() -> Int { 0 }
"#,
    );
    write_cert(&dir, "with_cert", "z3", "(true-axiom)");

    let out_path = dir.join("out.v");
    let out = run_verum(
        &["export", "--to", "coq", "--output", out_path.to_str().unwrap()],
        &dir,
    );
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let combined = format!("{}{}", stdout, String::from_utf8_lossy(&out.stderr));
    // Summary should mention "1 of 2 theorem proof(s) replayed".
    assert!(
        combined.contains("of 2 theorem"),
        "expected summary count `of 2 theorem`; got:\n{}",
        combined
    );
    assert!(
        combined.contains("admitted"),
        "expected summary to mention admitted; got:\n{}",
        combined
    );
}

#[test]
fn export_corrupt_cert_falls_back_to_admitted_silently() {
    let (_t, dir) = create_project(
        "corrupt_cert",
        r#"public theorem foo() -> Bool { true }
public fn main() -> Int { 0 }
"#,
    );
    // Write garbage instead of valid JSON.
    let cert_dir = dir.join(".verum").join("cache").join("certificates");
    fs::create_dir_all(&cert_dir).unwrap();
    fs::write(cert_dir.join("foo.smt-cert.json"), "not json {{{").unwrap();

    let out_path = dir.join("out.v");
    let out = run_verum(
        &["export", "--to", "coq", "--output", out_path.to_str().unwrap()],
        &dir,
    );
    // Export must NOT fail because of corrupt cert — it falls
    // through to admitted scaffold.
    assert!(out.status.success(), "corrupt cert must not fail export; stderr: {}", String::from_utf8_lossy(&out.stderr));
    let body = fs::read_to_string(&out_path).unwrap();
    assert!(body.contains("Admitted"), "corrupt cert must fall back to Admitted");
}

#[test]
fn export_with_cert_splices_real_proof_for_agda() {
    let (_t, dir) = create_project(
        "with_cert_agda",
        r#"public theorem refl_thm() -> Bool { true }
public fn main() -> Int { 0 }
"#,
    );
    write_cert(&dir, "refl_thm", "z3", "(refl x)");

    let out_path = dir.join("out.agda");
    let out = run_verum(
        &["export", "--to", "agda", "--output", out_path.to_str().unwrap()],
        &dir,
    );
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let body = fs::read_to_string(&out_path).unwrap();
    // Term-style Agda definition with `refl` witness.
    assert!(body.contains("refl_thm = refl"), "expected `refl_thm = refl`; got:\n{}", body);
}

#[test]
fn export_with_cert_splices_real_proof_for_dedukti() {
    let (_t, dir) = create_project(
        "with_cert_dedukti",
        r#"public theorem r_thm() -> Bool { true }
public fn main() -> Int { 0 }
"#,
    );
    write_cert(&dir, "r_thm", "z3", "(refl x)");

    let out_path = dir.join("out.dk");
    let out = run_verum(
        &["export", "--to", "dedukti", "--output", out_path.to_str().unwrap()],
        &dir,
    );
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let body = fs::read_to_string(&out_path).unwrap();
    // Lowered λΠ term is emitted as `def name : Prop := <term>.`
    assert!(body.contains("def r_thm : Prop :="), "expected `def r_thm := ...`; got:\n{}", body);
    assert!(body.contains("logic.refl"), "expected logic.refl term; got:\n{}", body);
}

#[test]
fn export_with_cert_splices_real_proof_for_metamath() {
    let (_t, dir) = create_project(
        "with_cert_mm",
        r#"public theorem mm_thm() -> Bool { true }
public fn main() -> Int { 0 }
"#,
    );
    write_cert(&dir, "mm_thm", "z3", "(refl x)");

    let out_path = dir.join("out.mm");
    let out = run_verum(
        &["export", "--to", "metamath", "--output", out_path.to_str().unwrap()],
        &dir,
    );
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let body = fs::read_to_string(&out_path).unwrap();
    // Real proof step uses `eqid` (set.mm reflexivity axiom).
    assert!(body.contains("th-mm_thm $p wff"), "expected th-... header; got:\n{}", body);
    assert!(body.contains("eqid"), "expected `eqid` from real lowering; got:\n{}", body);
    assert!(!body.contains("$= ? $."), "lowered proof must NOT contain `?` placeholder; got:\n{}", body);
}

#[test]
fn export_axiom_without_cert_keeps_axiom_form() {
    // Axioms are postulates — they don't carry proofs and the
    // cert-store doesn't apply to them. Verify they emit as
    // `Axiom ... : Prop.` regardless of cert-store presence.
    let (_t, dir) = create_project(
        "axiom_kind",
        r#"public axiom yoneda_post() -> Bool;
public fn main() -> Int { 0 }
"#,
    );
    let out_path = dir.join("out.v");
    let out = run_verum(
        &["export", "--to", "coq", "--output", out_path.to_str().unwrap()],
        &dir,
    );
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let body = fs::read_to_string(&out_path).unwrap();
    assert!(body.contains("Axiom yoneda_post : Prop"), "axiom shape preserved");
}
