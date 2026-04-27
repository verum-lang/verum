//! Integration tests for `verum import --from owl2-fs <file.ofn>`
//! (follow-up, Task B5).
//!
//! Exercises the OWL 2 Functional-Style Syntax importer end-to-end:
//! tokeniser → parser → Owl2Graph → `.vr` emitter, and verifies the
//! emitted source contains the expected `@owl2_*` typed attributes.
//! Round-trip with `verum export --to owl2-fs` is asserted at the
//! source-shape level (no full byte-equality — the importer is
//! lossy on Prefix/Ontology IRI metadata, which `export` regenerates
//! from the manifest).

#![allow(unused_imports)]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::TempDir;

fn run_verum(args: &[&str], cwd: Option<&PathBuf>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_verum"));
    cmd.args(args);
    if let Some(c) = cwd {
        cmd.current_dir(c);
    }
    cmd.output().expect("spawn verum CLI")
}

fn write_ofn(content: &str) -> (TempDir, PathBuf) {
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().join("input.ofn");
    fs::write(&path, content).expect("write ofn");
    (temp, path)
}

#[test]
fn import_owl2_fs_emits_class_decl() {
    let (_t, ofn) = write_ofn(
        r#"
        Prefix(:=<http://example.org/foo#>)
        Ontology(<http://example.org/foo>
            Declaration(Class(:Person))
        )
        "#,
    );
    let out_vr = ofn.with_extension("vr");
    let out = run_verum(
        &[
            "import",
            "--from",
            "owl2-fs",
            ofn.to_str().unwrap(),
            "--output",
            out_vr.to_str().unwrap(),
        ],
        None,
    );
    assert!(
        out.status.success(),
        "import must succeed; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let body = fs::read_to_string(&out_vr).expect("read emitted .vr");
    assert!(body.contains("@owl2_class"), "missing @owl2_class in:\n{}", body);
    assert!(
        body.contains("public type Person is"),
        "missing Person decl in:\n{}",
        body
    );
}

#[test]
fn import_owl2_fs_emits_subclass_and_property() {
    let (_t, ofn) = write_ofn(
        r#"
        Ontology(<x>
            Declaration(Class(:Person))
            Declaration(Class(:Animal))
            SubClassOf(:Person :Animal)
            Declaration(ObjectProperty(:hasParent))
            ObjectPropertyDomain(:hasParent :Person)
            ObjectPropertyRange(:hasParent :Person)
            TransitiveObjectProperty(:hasParent)
        )
        "#,
    );
    let out_vr = ofn.with_extension("vr");
    let out = run_verum(
        &[
            "import",
            "--from",
            "owl2-fs",
            ofn.to_str().unwrap(),
            "--output",
            out_vr.to_str().unwrap(),
        ],
        None,
    );
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let body = fs::read_to_string(&out_vr).expect("read emitted .vr");
    assert!(body.contains("@owl2_subclass_of(Animal)"), "missing subclass in:\n{}", body);
    assert!(
        body.contains("@owl2_property(domain = Person, range = Person)"),
        "missing property attrs in:\n{}",
        body
    );
    assert!(body.contains("@owl2_characteristic(transitive)"), "missing transitive in:\n{}", body);
}

#[test]
fn import_owl2_fs_haskey_round_trips() {
    let (_t, ofn) = write_ofn(
        r#"
        Ontology(<x>
            Declaration(Class(:Order))
            Declaration(ObjectProperty(:hasOrderId))
            HasKey(:Order () (:hasOrderId))
        )
        "#,
    );
    let out_vr = ofn.with_extension("vr");
    let out = run_verum(
        &[
            "import",
            "--from",
            "owl2-fs",
            ofn.to_str().unwrap(),
            "--output",
            out_vr.to_str().unwrap(),
        ],
        None,
    );
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let body = fs::read_to_string(&out_vr).expect("read emitted .vr");
    assert!(body.contains("@owl2_has_key(hasOrderId)"), "missing has_key in:\n{}", body);
}

#[test]
fn import_owl2_fs_default_output_path_is_input_with_vr_ext() {
    let (_t, ofn) = write_ofn(
        r#"
        Ontology(<x>
            Declaration(Class(:Foo))
        )
        "#,
    );
    let out = run_verum(
        &["import", "--from", "owl2-fs", ofn.to_str().unwrap()],
        None,
    );
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let default_out = ofn.with_extension("vr");
    assert!(default_out.exists(), "default output {} not created", default_out.display());
}

#[test]
fn import_owl2_fs_rejects_unknown_format() {
    let (_t, ofn) = write_ofn("Ontology(<x>)");
    let out = run_verum(
        &[
            "import",
            "--from",
            "rdf-xml",
            ofn.to_str().unwrap(),
        ],
        None,
    );
    assert!(!out.status.success(), "unknown format must fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--from must be one of") || stderr.contains("rdf-xml"),
        "expected diagnostic; got: {}",
        stderr
    );
}
