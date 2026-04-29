//! Integration tests for `verum extract` (,
//! V2.1).
//!
//! Walks @extract / @extract_witness / @extract_contract markers
//! in `.vr` files, dispatches to the program-extraction pipeline
//! at the attribute's ExtractTarget, and emits per-target
//! scaffolds at <output>/<decl>.{vr,ml,lean,v}.

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
fn extract_emits_verum_scaffold_for_default_target() {
    let (_t, dir) = create_project(
        "extract_verum",
        r#"@extract
public fn plus_comm() -> Bool { true }

public fn main() -> Int { 0 }
"#,
    );
    let out = run_verum(&["extract"], &dir);
    assert!(
        out.status.success(),
        "extract must succeed; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out_file = dir.join("extracted").join("plus_comm.vr");
    assert!(out_file.exists(), "expected output file at {}", out_file.display());
    let body = fs::read_to_string(&out_file).expect("read");
    assert!(body.contains("@extracted"));
    assert!(body.contains("public fn plus_comm"));
    assert!(body.contains("Extracted by `verum extract`"));
}

#[test]
fn extract_emits_lean_scaffold_for_lean_target() {
    let (_t, dir) = create_project(
        "extract_lean",
        r#"@extract(lean)
public theorem yoneda() -> Bool { true }

public fn main() -> Int { 0 }
"#,
    );
    let out = run_verum(&["extract"], &dir);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let out_file = dir.join("extracted").join("yoneda.lean");
    assert!(out_file.exists());
    let body = fs::read_to_string(&out_file).expect("read");
    assert!(body.contains("def yoneda"));
    assert!(body.contains("--"));
}

#[test]
fn extract_emits_separate_files_per_target() {
    // One declaration with both @extract(verum) and @extract(coq)
    // produces two output files.
    let (_t, dir) = create_project(
        "extract_multi",
        r#"@extract(verum)
@extract(coq)
public theorem div_unique() -> Bool { true }

public fn main() -> Int { 0 }
"#,
    );
    let out = run_verum(&["extract"], &dir);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(dir.join("extracted").join("div_unique.vr").exists());
    assert!(dir.join("extracted").join("div_unique.v").exists());
}

#[test]
fn extract_witness_marker_emits_witness_kind_scaffold() {
    let (_t, dir) = create_project(
        "extract_witness",
        r#"@extract_witness(coq)
public theorem div_witness() -> Bool { true }

public fn main() -> Int { 0 }
"#,
    );
    let out = run_verum(&["extract"], &dir);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let body = fs::read_to_string(dir.join("extracted").join("div_witness.v"))
        .expect("read");
    assert!(body.contains("@extract_witness(coq)"));
}

#[test]
fn extract_no_markers_succeeds_quietly() {
    let (_t, dir) = create_project(
        "extract_clean",
        r#"public fn main() -> Int { 0 }
"#,
    );
    let out = run_verum(&["extract"], &dir);
    assert!(out.status.success());
    // No `extracted/` dir should be created when no markers found.
    assert!(!dir.join("extracted").exists());
}

#[test]
fn extract_explicit_input_path() {
    let (_t, dir) = create_project(
        "extract_explicit",
        r#"@extract
public fn solo() -> Bool { true }
"#,
    );
    let input_path = dir.join("src").join("main.vr");
    let out = run_verum(
        &["extract", input_path.to_str().unwrap()],
        &dir,
    );
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(dir.join("extracted").join("solo.vr").exists());
}

#[test]
fn extract_v12_1_splices_real_body_for_verum_target() {
    // V12.1 behaviour: when body source is captured via spans,
    // the extracted Verum file contains the function's actual
    // body verbatim (not a `/* V12.1 body */` placeholder).
    let (_t, dir) = create_project(
        "extract_v12_1_body",
        r#"@extract
public fn double(n: Int) -> Int { n + n }

public fn main() -> Int { 0 }
"#,
    );
    let out = run_verum(&["extract"], &dir);
    assert!(
        out.status.success(),
        "extract must succeed; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let body = fs::read_to_string(dir.join("extracted").join("double.vr"))
        .expect("read");
    // Re-checkable extraction marker in the header.
    assert!(
        body.contains("re-checkable extraction"),
        "header must indicate re-checkable extraction; got:\n{}",
        body
    );
    // Real body spliced verbatim.
    assert!(
        body.contains("public fn double(n: Int) -> Int"),
        "must preserve full signature; got:\n{}",
        body
    );
    assert!(
        body.contains("n + n"),
        "must preserve body expression; got:\n{}",
        body
    );
    // No body placeholder when the body was actually captured.
    assert!(
        !body.contains("/* body pending */"),
        "must NOT contain pending-body placeholder; got:\n{}",
        body
    );
    // @extracted marker carried.
    assert!(body.contains("@extracted"));
}

#[test]
fn extract_v12_2_ocaml_lowers_arithmetic_body() {
    // V12.2 OCaml lowerer: arithmetic expressions are lowered to
    // OCaml syntax (no longer V12.1 metadata-comment fallback).
    let (_t, dir) = create_project(
        "extract_v12_2_ocaml_arith",
        r#"@extract(ocaml)
public fn double(n: Int) -> Int { n + n }

public fn main() -> Int { 0 }
"#,
    );
    let out = run_verum(&["extract"], &dir);
    assert!(out.status.success());
    let body = fs::read_to_string(dir.join("extracted").join("double.ml"))
        .expect("read");
    // V12.2 marker present.
    assert!(
        body.contains("(lowered)"),
        "header must indicate V12.2 lowering; got:\n{}",
        body
    );
    // Real OCaml expression spliced (not "V12.2 body" placeholder).
    assert!(
        body.contains("let double () = (n + n)"),
        "must contain lowered OCaml; got:\n{}",
        body
    );
    assert!(
        !body.contains("(* body pending *)"),
        "body-pending placeholder must be gone when lowering succeeds; got:\n{}",
        body
    );
}

#[test]
fn extract_v12_2_ocaml_lowers_boolean_literal() {
    let (_t, dir) = create_project(
        "extract_v12_2_ocaml_bool",
        r#"@extract(ocaml)
public fn yes() -> Bool { true }

public fn main() -> Int { 0 }
"#,
    );
    let out = run_verum(&["extract"], &dir);
    assert!(out.status.success());
    let body = fs::read_to_string(dir.join("extracted").join("yes.ml"))
        .expect("read");
    assert!(
        body.contains("let yes () = true"),
        "must lower boolean literal; got:\n{}",
        body
    );
}

#[test]
fn extract_v12_2_ocaml_falls_back_for_unsupported_constructs() {
    // Unsupported construct (e.g. complex match) should fall
    // back to V12.1 metadata comment + stub.
    let (_t, dir) = create_project(
        "extract_v12_2_ocaml_fallback",
        r#"@extract(ocaml)
public fn power(n: Int) -> Int { n ** 2 }

public fn main() -> Int { 0 }
"#,
    );
    let out = run_verum(&["extract"], &dir);
    assert!(out.status.success());
    let body = fs::read_to_string(dir.join("extracted").join("power.ml"))
        .expect("read");
    // `**` (Pow) is not in the V12.2 OCaml lowerer's vocabulary,
    // so the fallback path fires.
    assert!(
        body.contains("lowering pending") || body.contains("body pending"),
        "must fall back to metadata comment for unsupported `**`; got:\n{}",
        body
    );
}

#[test]
fn extract_v12_2_lean_lowers_arithmetic_body() {
    let (_t, dir) = create_project(
        "extract_v12_2_lean_arith",
        r#"@extract(lean)
public fn double(n: Int) -> Int { n + n }

public fn main() -> Int { 0 }
"#,
    );
    let out = run_verum(&["extract"], &dir);
    assert!(out.status.success());
    let body = fs::read_to_string(dir.join("extracted").join("double.lean"))
        .expect("read");
    assert!(
        body.contains("(lowered)"),
        "Lean header must indicate V12.2; got:\n{}",
        body
    );
    assert!(
        body.contains("def double : Unit := (n + n)"),
        "Lean body must be lowered; got:\n{}",
        body
    );
}

#[test]
fn extract_v12_2_lean_lowers_eq_with_double_equals() {
    // Lean's `==` is the Decidable runtime equality (matching
    // Verum's `==`). Verify the lowerer emits `==` not `=` (which
    // is propositional equality in Lean).
    let (_t, dir) = create_project(
        "extract_v12_2_lean_eq",
        r#"@extract(lean)
public fn cmp(a: Int, b: Int) -> Bool { a == b }

public fn main() -> Int { 0 }
"#,
    );
    let out = run_verum(&["extract"], &dir);
    assert!(out.status.success());
    let body = fs::read_to_string(dir.join("extracted").join("cmp.lean"))
        .expect("read");
    assert!(
        body.contains("(a == b)"),
        "Lean must use double-equals for Decidable equality; got:\n{}",
        body
    );
}

#[test]
fn extract_v12_2_coq_lowers_arithmetic_body() {
    // V12.2 Coq lowerer: simple body lowered to gallina.
    let (_t, dir) = create_project(
        "extract_v12_2_coq_arith",
        r#"@extract(coq)
public fn double(n: Int) -> Int { n + n }

public fn main() -> Int { 0 }
"#,
    );
    let out = run_verum(&["extract"], &dir);
    assert!(out.status.success());
    let body = fs::read_to_string(dir.join("extracted").join("double.v"))
        .expect("read");
    assert!(
        body.contains("(lowered)"),
        "Coq header must indicate V12.2; got:\n{}",
        body
    );
    assert!(
        body.contains("Definition double := (n + n)."),
        "Coq body must be lowered; got:\n{}",
        body
    );
}

#[test]
fn extract_v12_2_coq_uses_andb_for_logical_and() {
    let (_t, dir) = create_project(
        "extract_v12_2_coq_andb",
        r#"@extract(coq)
public fn both(a: Bool, b: Bool) -> Bool { a && b }

public fn main() -> Int { 0 }
"#,
    );
    let out = run_verum(&["extract"], &dir);
    assert!(out.status.success());
    let body = fs::read_to_string(dir.join("extracted").join("both.v"))
        .expect("read");
    // Coq Bool conjunction uses `andb` function, not `&&` infix.
    assert!(
        body.contains("(andb a b)"),
        "Coq must use `andb` for Bool && ; got:\n{}",
        body
    );
}

#[test]
fn extract_v12_2_coq_falls_back_for_bitwise() {
    // Bitwise ops lack infix Coq syntax (require Z.land / Z.lor
    // prefix functions); V12.2 returns None, fallback fires.
    let (_t, dir) = create_project(
        "extract_v12_2_coq_bitwise",
        r#"@extract(coq)
public fn mask(a: Int, b: Int) -> Int { a & b }

public fn main() -> Int { 0 }
"#,
    );
    let out = run_verum(&["extract"], &dir);
    assert!(out.status.success());
    let body = fs::read_to_string(dir.join("extracted").join("mask.v"))
        .expect("read");
    // Extractor recognises that bitwise `&` has no Coq-side lowering
    // and emits a fallback `Definition ... := tt. (* body pending *)`
    // stub plus a `lowering pending` marker in the captured-body block.
    assert!(
        body.contains("lowering pending") || body.contains("body pending"),
        "must fall back for bitwise; got:\n{}",
        body
    );
}

#[test]
fn extract_custom_output_dir() {
    let (_t, dir) = create_project(
        "extract_custom_out",
        r#"@extract
public fn t() -> Bool { true }
"#,
    );
    let custom = dir.join("artefacts").join("v1");
    let out = run_verum(
        &[
            "extract",
            "--output",
            custom.to_str().unwrap(),
        ],
        &dir,
    );
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(custom.join("t.vr").exists(), "custom output path must be honoured");
}
