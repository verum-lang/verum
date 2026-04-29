//! End-to-end integration tests for `verum foreign-import`.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::TempDir;

fn verum_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_verum"))
}

fn run(args: &[&str]) -> Output {
    Command::new(verum_bin())
        .args(args)
        .output()
        .expect("CLI invocation must succeed")
}

fn write_temp_file(content: &str, name: &str) -> (TempDir, PathBuf) {
    let t = TempDir::new().unwrap();
    let p = t.path().join(name);
    fs::write(&p, content).unwrap();
    (t, p)
}

// ─────────────────────────────────────────────────────────────────────
// Coq import
// ─────────────────────────────────────────────────────────────────────

#[test]
fn import_coq_emits_skeleton_with_framework_attribution() {
    let coq_src = "Theorem add_comm : forall a b : nat, a + b = b + a.\nProof. admit. Qed.\n";
    let (_t, src) = write_temp_file(coq_src, "src.v");
    let out = run(&[
        "foreign-import",
        "--from",
        "coq",
        src.to_str().unwrap(),
        "--format",
        "skeleton",
    ]);
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("@framework(coq"));
    assert!(stdout.contains("public theorem add_comm"));
    assert!(stdout.contains("proof by axiom"));
    assert!(
        stdout.contains("forall a b"),
        "original statement preserved as comment: {stdout}"
    );
}

#[test]
fn import_coq_extracts_multiple_kinds() {
    let src_text = "Theorem t1 : True.\nLemma l1 : 0 = 0.\nAxiom ax1 : forall x, x = x.\n";
    let (_t, src) = write_temp_file(src_text, "src.v");
    let out = run(&[
        "foreign-import",
        "--from",
        "coq",
        src.to_str().unwrap(),
        "--format",
        "json",
    ]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(parsed["count"], 3);
    let theorems = parsed["theorems"].as_array().unwrap();
    let kinds: Vec<&str> = theorems
        .iter()
        .map(|t| t.get("kind").unwrap().as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"theorem"));
    assert!(kinds.contains(&"lemma"));
    assert!(kinds.contains(&"axiom"));
}

// ─────────────────────────────────────────────────────────────────────
// Lean4 import
// ─────────────────────────────────────────────────────────────────────

#[test]
fn import_lean_emits_skeleton() {
    let lean_src = "theorem add_comm : ∀ a b : Nat, a + b = b + a := by simp\n";
    let (_t, src) = write_temp_file(lean_src, "Algebra.lean");
    let out = run(&[
        "foreign-import",
        "--from",
        "lean4",
        src.to_str().unwrap(),
    ]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("@framework(lean_mathlib4"));
    assert!(stdout.contains("public theorem add_comm"));
    assert!(stdout.contains("proof by axiom"));
    // Statement is included as comment, but `:=` separator stripped.
    assert!(!stdout.lines().any(|l| l.contains(":= by simp")));
}

#[test]
fn import_lean_alias_mathlib_works() {
    let (_t, src) = write_temp_file("theorem t : True := trivial\n", "src.lean");
    let out = run(&[
        "foreign-import",
        "--from",
        "mathlib",
        src.to_str().unwrap(),
    ]);
    assert!(out.status.success());
}

// ─────────────────────────────────────────────────────────────────────
// JSON output
// ─────────────────────────────────────────────────────────────────────

#[test]
fn import_json_well_formed() {
    let coq_src = "Theorem foo : True.\nLemma bar : 1 = 1.\n";
    let (_t, src) = write_temp_file(coq_src, "src.v");
    let out = run(&[
        "foreign-import",
        "--from",
        "coq",
        src.to_str().unwrap(),
        "--format",
        "json",
    ]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(parsed["system"], "coq");
    assert_eq!(parsed["count"], 2);
    let theorems = parsed["theorems"].as_array().unwrap();
    for t in theorems {
        assert!(t.get("name").and_then(|v| v.as_str()).is_some());
        assert!(t.get("kind").and_then(|v| v.as_str()).is_some());
        assert!(t.get("line").and_then(|v| v.as_u64()).is_some());
        assert!(t.get("framework_citation").and_then(|v| v.as_str()).is_some());
    }
}

// ─────────────────────────────────────────────────────────────────────
// summary output
// ─────────────────────────────────────────────────────────────────────

#[test]
fn import_summary_lists_each_theorem() {
    let coq_src = "Theorem t1 : True.\nTheorem t2 : True.\nTheorem t3 : True.\n";
    let (_t, src) = write_temp_file(coq_src, "src.v");
    let out = run(&[
        "foreign-import",
        "--from",
        "coq",
        src.to_str().unwrap(),
        "--format",
        "summary",
    ]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Imported 3 declaration"));
    assert!(stdout.contains("t1"));
    assert!(stdout.contains("t2"));
    assert!(stdout.contains("t3"));
}

// ─────────────────────────────────────────────────────────────────────
// `--out` writes to disk
// ─────────────────────────────────────────────────────────────────────

#[test]
fn import_writes_to_out_path() {
    let (_t, src) = write_temp_file("Theorem foo : True.\n", "src.v");
    let out_dir = TempDir::new().unwrap();
    let dest = out_dir.path().join("imported.vr");
    let out = run(&[
        "foreign-import",
        "--from",
        "coq",
        src.to_str().unwrap(),
        "--out",
        dest.to_str().unwrap(),
    ]);
    assert!(out.status.success());
    assert!(dest.exists());
    let body = fs::read_to_string(&dest).unwrap();
    assert!(body.contains("@framework(coq"));
    assert!(body.contains("public theorem foo"));
}

// ─────────────────────────────────────────────────────────────────────
// Validation errors
// ─────────────────────────────────────────────────────────────────────

#[test]
fn import_rejects_unknown_system() {
    let (_t, src) = write_temp_file("anything\n", "src.txt");
    let out = run(&[
        "foreign-import",
        "--from",
        "garbage",
        src.to_str().unwrap(),
    ]);
    assert!(
        !out.status.success(),
        "unknown --from must produce non-zero exit"
    );
}

#[test]
fn import_rejects_unknown_format() {
    let (_t, src) = write_temp_file("Theorem foo : True.\n", "src.v");
    let out = run(&[
        "foreign-import",
        "--from",
        "coq",
        src.to_str().unwrap(),
        "--format",
        "yaml",
    ]);
    assert!(!out.status.success());
}

#[test]
fn import_rejects_missing_file() {
    let out = run(&[
        "foreign-import",
        "--from",
        "coq",
        "/nonexistent/path/file.v",
    ]);
    assert!(!out.status.success());
}

// ─────────────────────────────────────────────────────────────────────
// All four systems reachable
// ─────────────────────────────────────────────────────────────────────

#[test]
fn import_every_supported_system() {
    let cases: &[(&str, &str, &str)] = &[
        ("coq", "Theorem foo : True.\n", "src.v"),
        ("lean4", "theorem foo : True := trivial\n", "src.lean"),
        ("mizar", "theorem Th1: 0 = 0;\n", "src.miz"),
        ("isabelle", "theorem foo: \"True\" by auto\n", "src.thy"),
    ];
    for (system, content, fname) in cases {
        let (_t, src) = write_temp_file(content, fname);
        let out = run(&[
            "foreign-import",
            "--from",
            system,
            src.to_str().unwrap(),
            "--format",
            "json",
        ]);
        assert!(
            out.status.success(),
            "system {} failed: stderr={}",
            system,
            String::from_utf8_lossy(&out.stderr)
        );
        let parsed: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
        assert_eq!(parsed["system"], *system);
        assert_eq!(parsed["count"], 1);
    }
}
