//! The export index carries EVERY public declaration form.
//!
//! Measured (MSFS cross-audit, 2026-08-21): nine of ten E401s in the
//! math corpus shared one cause — a `public type X is protocol { ... }`
//! declaration whose module's export entry listed the file's axioms
//! but not the protocol type, so every `mount m.{X}` of it died with
//! E401 while the declaration sat in plain sight. This test drives
//! the REAL pair (VerumParser -> build_export_index) over the real
//! witness file and over a synthetic module of every public form.

use std::path::PathBuf;
use verum_compiler::core_compiler::build_export_index;
use verum_fast_parser::VerumParser;
use verum_lexer::Lexer;

fn parse(source: &str) -> verum_ast::Module {
    let file_id = verum_ast::FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    VerumParser::new()
        .parse_module(lexer, file_id)
        .expect("witness source must parse")
}

fn exports_of(source: &str, file: &str) -> Vec<String> {
    let module = parse(source);
    let all = vec![(
        "core".to_string(),
        vec![(PathBuf::from(file), module)],
    )];
    let index = build_export_index(&all);
    let mut names: Vec<String> = index
        .values()
        .flat_map(|s| s.iter().cloned())
        .collect();
    names.sort();
    names
}

#[test]
fn public_type_is_protocol_reaches_the_export_index() {
    let names = exports_of(
        r#"
module core.math.witness;

@framework(msfs, "witness")
public axiom witness_axiom(x: Bool) -> Bool
    requires true
    ensures x == x;

/// The form nine E401s traced to.
public type FormallySDefinable is protocol {
    fn has_witness(self) -> Bool;
};
"#,
        "core/math/witness.vr",
    );
    assert!(
        names.iter().any(|n| n == "witness_axiom"),
        "the axiom half of the witness pair must export, got {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "FormallySDefinable"),
        "a `public type X is protocol` declaration must be importable — \
         the axioms of the same file exporting while the protocol type \
         does not is exactly the measured E401 signature; got {names:?}"
    );
}

#[test]
fn public_theorem_reaches_the_export_index() {
    // The tenth measured E401: `public theorem
    // msfs_lemma_3_4_outputs_in_s_s_global` was mounted by two corpus
    // files and reported as a phantom, while sitting in plain sight —
    // Theorem/Lemma/Corollary had no arm in the export match and the
    // silent `_ => {}` swallowed the form.
    let source = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("workspace root")
            .join("core/math/s_definable/lemma_3_4.vr"),
    )
    .expect("witness file readable");
    let names = exports_of(&source, "core/math/s_definable/lemma_3_4.vr");
    assert!(
        names
            .iter()
            .any(|n| n == "msfs_lemma_3_4_outputs_in_s_s_global"),
        "a public theorem is importable like an axiom; got {names:?}"
    );
}

#[test]
fn the_real_witness_file_exports_its_protocol_type() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf();
    let path = root.join("core/math/s_definable/lemma_3_4.vr");
    let source = std::fs::read_to_string(&path).expect("witness file readable");
    let names = exports_of(&source, "core/math/s_definable/lemma_3_4.vr");
    assert!(
        names.iter().any(|n| n == "FormallySDefinable"),
        "core/math/s_definable/lemma_3_4.vr declares `public type \
         FormallySDefinable is protocol` at its top level; the export \
         index must carry it (measured E401 witness: five_axis.vr:34, \
         ac_oc_duality.vr:49, afnt_alpha.vr:38, afnt_beta.vr:53); got {names:?}"
    );
}
