//! `[verify.solver.*]` survives a real `Verum.toml` — T1233.
//!
//! The last link of the chain, and the only one the `verum_smt` tests
//! cannot reach: they deserialize a `SolverConfig` from a TOML string,
//! which proves serde can do it but says nothing about whether the
//! MANIFEST carries the block to that struct. Before this change it
//! did not: `VerifyConfig` had no `solver` field, so serde dropped
//! `[verify.solver]` and every table under it without a warning.
//!
//! `Manifest::from_file` also INSTALLS the block process-wide, which is
//! why this file writes a real file rather than calling `toml::from_str`
//! — the install is part of what a manifest load does, and a test that
//! bypassed the loader would not exercise it.

use std::io::Write;

fn manifest_with(body: &str) -> verum_cli::config::Manifest {
    let dir = std::env::temp_dir().join(format!(
        "verum-t1233-{}-{}",
        std::process::id(),
        body.len()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("Verum.toml");
    let mut f = std::fs::File::create(&path).expect("write manifest");
    write!(
        f,
        "[cog]\nname = \"t1233\"\nversion = \"0.1.0\"\n\n{body}"
    )
    .expect("write manifest body");
    verum_cli::config::Manifest::from_file(&path).expect("manifest must parse")
}

/// The value written in a file arrives in the struct the solver runs on.
#[test]
fn a_value_set_in_verum_toml_reaches_the_solver_config() {
    let m = manifest_with(
        "[verify.solver.sep_logic]\n\
         max_unfolding_depth = 1\n\
         entailment_timeout_ms = 250\n\n\
         [verify.solver.qe]\n\
         simplify_level = 0\n",
    );

    assert_eq!(m.verify.solver.sep_logic.max_unfolding_depth, 1);
    assert_eq!(m.verify.solver.sep_logic.entailment_timeout_ms, 250);
    assert_eq!(m.verify.solver.qe.simplify_level, 0);

    // Untouched keys keep their defaults — a manifest may set one key
    // of forty-seven.
    let d = verum_smt::config::SolverConfig::default();
    assert_eq!(
        m.verify.solver.sep_logic.enable_frame_inference,
        d.sep_logic.enable_frame_inference
    );
    assert_eq!(m.verify.solver.unsat_core.minimize, d.unsat_core.minimize);

    // And the rest of `[verify]` is unharmed by the new field.
    assert_eq!(m.verify.default_strategy.as_str(), "formal");
}

/// A manifest with no `[verify.solver]` block is the common case and
/// must parse to the defaults, not to an error.
#[test]
fn a_manifest_without_the_block_is_the_defaults() {
    let m = manifest_with("[build]\ntarget = \"native\"\n");
    assert!(
        m.verify.solver.is_default(),
        "absent block must equal Default"
    );
}

/// A misspelled sub-table fails the manifest parse and names the
/// offender. Silence was the defect; a fix that keeps quiet about
/// typos moves it one level down instead of closing it.
#[test]
fn a_misspelled_sub_table_fails_the_manifest() {
    let dir = std::env::temp_dir().join(format!("verum-t1233-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("Verum.toml");
    std::fs::write(
        &path,
        "[cog]\nname = \"t1233\"\nversion = \"0.1.0\"\n\n\
         [verify.solver.sep-logic]\nmax_unfolding_depth = 1\n",
    )
    .expect("write");

    let err = verum_cli::config::Manifest::from_file(&path)
        .err()
        .expect("a misspelled sub-table must be refused, not ignored");
    let msg = err.to_string();
    assert!(
        msg.contains("sep-logic"),
        "the error must name the offending table, got: {msg}"
    );
}
