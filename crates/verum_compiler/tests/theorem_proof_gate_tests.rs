#![cfg(test)]

//! PROOF-TACTIC-ACCEPT-1 (T0105) regression gate.
//!

//! A `theorem` / `lemma` / `corollary` with a written proof body is a
//! hard claim: if the proof does not discharge, compilation MUST fail
//! (E0319), and if the proof is admitted (`admit` / `sorry` anywhere
//! in the body), compilation MUST warn (W0319) on every build. Before
//! this gate the pipeline logged a `tracing::warn` (invisible at
//! default log levels) and returned `Ok(())` — a false theorem with a
//! nonsense proof compiled silently.
//!

//! Related code:
//! - `crates/verum_compiler/src/pipeline/theorem_proofs.rs` — the
//!  diagnostic gate (E0319 / W0319).
//! - `crates/verum_ast/src/decl.rs` — `ProofBody::contains_unsafe`
//!  (recursive admit/sorry detection, the AST-level authority).
//! - `crates/verum_compiler/src/phases/proof_verification.rs` — the
//!  SMT-backed proof discharge engine.

use std::io::Write;
use std::path::PathBuf;
use tempfile::NamedTempFile;
use verum_compiler::{CompilationPipeline, CompilerOptions, Session, VerifyMode};

// =============================================================================
// Helpers
// =============================================================================

/// Create a temp `.vr` file containing the given source and return it.
fn create_temp_source(source: &str) -> NamedTempFile {
    let mut file = NamedTempFile::new().expect("temp file");
    write!(file, "{}", source).expect("write temp file");
    file
}

/// Options with SMT proof verification ON (the theorem-proof gate runs
/// only when `verify_mode.use_smt()`).
fn opts_for(temp: &NamedTempFile) -> CompilerOptions {
    CompilerOptions {
        input: temp.path().to_path_buf(),
        output: PathBuf::from("/tmp/theorem_proof_gate_test.out"),
        verify_mode: VerifyMode::Proof,
        ..Default::default()
    }
}

/// Outcome of a check-only run: (pipeline_ok, error codes, warning codes).
fn check(source: &str) -> (bool, Vec<String>, Vec<String>) {
    let temp = create_temp_source(source);
    let opts = opts_for(&temp);
    let mut session = Session::new(opts);
    let mut pipeline = CompilationPipeline::new(&mut session);
    let ok = pipeline.run_check_only().is_ok() && !session.has_errors();

    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for d in session.diagnostics().iter() {
        let code = d.code().unwrap_or("").to_string();
        match d.severity() {
            verum_diagnostics::Severity::Error => errors.push(code),
            verum_diagnostics::Severity::Warning => warnings.push(code),
            _ => {}
        }
    }
    (ok, errors, warnings)
}

// =============================================================================
// The gate
// =============================================================================

/// A FALSE proposition with a real (non-admitting) proof must fail
/// compilation with E0319 — this exact shape compiled silently before
/// the gate landed.
#[test]
fn false_theorem_with_real_tactic_fails_compilation() {
    let source = r#"
        theorem impossible()
            ensures 1 == 2
            proof by trivial;

        fn main() { }
    "#;
    let (ok, errors, _warnings) = check(source);
    assert!(
        !ok,
        "a theorem whose proof cannot discharge MUST fail compilation"
    );
    assert!(
        errors.iter().any(|c| c == "E0319"),
        "expected E0319 proof-verification error, got errors: {:?}",
        errors
    );
}

/// `proof by auto` on a false proposition must fail the same way —
/// the automated tactic must not fabricate a discharge.
#[test]
fn false_theorem_with_auto_fails_compilation() {
    let source = r#"
        theorem also_impossible(n: Int)
            requires n > 0
            ensures n < 0
            proof by auto;

        fn main() { }
    "#;
    let (ok, errors, _warnings) = check(source);
    assert!(!ok, "auto on an unsatisfiable goal MUST fail compilation");
    assert!(
        errors.iter().any(|c| c == "E0319"),
        "expected E0319, got errors: {:?}",
        errors
    );
}

/// An admitted proof compiles — that is the sanctioned development
/// escape hatch — but MUST carry the W0319 unverified-proposition
/// warning.
#[test]
fn admitted_theorem_compiles_with_unverified_warning() {
    let source = r#"
        theorem admitted_claim()
            ensures 1 == 2
            proof by admit;

        fn main() { }
    "#;
    let (ok, errors, warnings) = check(source);
    assert!(
        ok,
        "an admitted proof is the explicit escape hatch and must compile; errors: {:?}",
        errors
    );
    assert!(
        warnings.iter().any(|c| c == "W0319"),
        "expected W0319 admitted-proof warning, got warnings: {:?}",
        warnings
    );
}

/// `sorry` behaves exactly like `admit`.
#[test]
fn sorry_theorem_compiles_with_unverified_warning() {
    let source = r#"
        theorem sorried_claim()
            ensures 2 == 3
            proof by sorry;

        fn main() { }
    "#;
    let (ok, _errors, warnings) = check(source);
    assert!(ok, "a sorry proof must compile");
    assert!(
        warnings.iter().any(|c| c == "W0319"),
        "expected W0319 for sorry, got warnings: {:?}",
        warnings
    );
}

/// An admit nested inside a combinator is still an admitted proof —
/// the detection is a recursive AST walk, not a top-level match (the
/// pre-gate detector string-matched a Debug render and never fired).
#[test]
fn admit_nested_in_combinator_still_flags_unverified() {
    let source = r#"
        theorem nested_admit()
            ensures 1 == 2
            proof by first [ admit, trivial ];

        fn main() { }
    "#;
    let (ok, errors, warnings) = check(source);
    // Whether the alternative closes the goal or not, the admitted
    // escape hatch in the source must surface: either the proof is
    // accepted-with-W0319 or rejected outright with E0319. Silence is
    // the only forbidden outcome.
    assert!(
        warnings.iter().any(|c| c == "W0319") || (!ok && errors.iter().any(|c| c == "E0319")),
        "nested admit must not pass silently (ok={}, errors={:?}, warnings={:?})",
        ok,
        errors,
        warnings
    );
}

/// Control: a TRUE proposition with a working proof compiles with no
/// proof diagnostics at all.
#[test]
fn true_theorem_verifies_without_diagnostics() {
    let source = r#"
        theorem arithmetic_holds()
            ensures 1 + 1 == 2
            proof by auto;

        fn main() { }
    "#;
    let (ok, errors, warnings) = check(source);
    assert!(ok, "a true, proven theorem must compile; errors: {:?}", errors);
    assert!(
        !errors.iter().any(|c| c == "E0319"),
        "no E0319 expected: {:?}",
        errors
    );
    assert!(
        !warnings.iter().any(|c| c == "W0319"),
        "no W0319 expected: {:?}",
        warnings
    );
}

/// Control: an axiom (no proof body) is accepted without proof
/// diagnostics — assuming is not proving, and the axiom form is the
/// honest way to assume.
#[test]
fn axiom_accepted_without_proof_diagnostics() {
    let source = r#"
        axiom assumed_ordering(a: Int, b: Int)
            requires a < b
            ensures a <= b;

        fn main() { }
    "#;
    let (ok, errors, warnings) = check(source);
    assert!(ok, "axiom must compile; errors: {:?}", errors);
    assert!(
        !errors.iter().any(|c| c == "E0319") && !warnings.iter().any(|c| c == "W0319"),
        "axioms carry no proof-gate diagnostics (errors={:?}, warnings={:?})",
        errors,
        warnings
    );
}

/// A failed lemma is gated exactly like a failed theorem.
#[test]
fn false_lemma_fails_compilation() {
    let source = r#"
        lemma bogus_helper(x: Int)
            ensures x * 0 == 1
            proof by auto;

        fn main() { }
    "#;
    let (ok, errors, _warnings) = check(source);
    assert!(!ok, "a lemma whose proof cannot discharge MUST fail");
    assert!(
        errors.iter().any(|c| c == "E0319"),
        "expected E0319 for lemma, got: {:?}",
        errors
    );
}
