//! T0834 — the ATS-V architectural phase runs on the paths users take.
//!
//! `phase_ats_v` existed since the layer landed, but its only caller
//! was the test harness (`validate_module` ← `run_for_test`): on
//! `verum check` / `build` / `run` the entire 32-anti-pattern layer
//! was inert.  A module declaring `stratum: MsfsStratum.LAbs` — the
//! one value the ontology forbids outright (AFN-T α) — passed
//! `verum check` in silence, and so did `MsfsStratum.NoSuchStratum`,
//! which the kernel's own parser rejects.
//!
//! These tests drive the REAL check pipeline (`check_project`, the
//! same entry `verum check` takes) and pin the T0834 acceptance:
//!
//!   1. `MsfsStratum.LAbs` is diagnosed as AP-011
//!      `AbsoluteBoundaryAttempt` — the specific code is contract,
//!      promised by the catalog, the in-language twin
//!      (`core/architecture/types.vr`) and the alignment roster alike.
//!   2. An unparseable `@arch_module` value is an ERROR, not silence.
//!   3. AT-2 is live: `Lifecycle.Theorem` without CVE closure errors.
//!   4. An honest declaration (`Lifecycle.Definition`) passes — the
//!      phase judges, it does not blanket-reject annotated modules.
//!
//! Gate: this file runs in the `nightly-aot.yml` measurement lane
//! (verum_compiler suites are not PR-gated); the kernel-side halves
//! of pins 1-2 are unit tests in `verum_kernel` next to the checks.

use anyhow::Result;
use std::path::PathBuf;
use tempfile::TempDir;
use verum_compiler::{CompilationPipeline, CompilerOptions, Session};

/// Run `check_project` over one in-memory source file and return
/// `(error_count, all_diagnostic_text)`.
fn check_source(source: &str) -> Result<(usize, String)> {
    let temp_dir = TempDir::new()?;
    let main_file = temp_dir.path().join("main.vr");
    std::fs::write(&main_file, source)?;

    let options = CompilerOptions {
        input: main_file,
        output: PathBuf::new(),
        verbose: 0,
        ..Default::default()
    };
    let mut session = Session::new(options);
    let errors = {
        let mut pipeline = CompilationPipeline::new_check(&mut session);
        // check_project returns Err when diagnostics aborted the run;
        // either way the diagnostics list carries the evidence.
        match pipeline.check_project() {
            Ok(result) => result.errors,
            Err(_) => 1,
        }
    };
    let text = session
        .diagnostics()
        .iter()
        .map(|d| format!("{:?}", d))
        .collect::<Vec<_>>()
        .join("\n");
    Ok((errors, text))
}

#[test]
fn labs_stratum_is_diagnosed_as_ap_011_on_check() -> Result<()> {
    let (errors, text) = check_source(
        r#"
@arch_module(
    foundation: Foundation.ZfcTwoInacc,
    stratum: MsfsStratum.LAbs,
    lifecycle: Lifecycle.Definition,
)
module probe.labs;
fn main() { print("ok"); }
"#,
    )?;
    assert!(
        errors > 0,
        "a module declaring MsfsStratum.LAbs must fail `verum check`; got 0 errors, diags: {text}"
    );
    assert!(
        text.contains("ATS-V-AP-011") || text.contains("AbsoluteBoundaryAttempt"),
        "LAbs must be diagnosed as AP-011 AbsoluteBoundaryAttempt (T0834 acceptance), diags: {text}"
    );
    Ok(())
}

#[test]
fn unparseable_arch_module_value_is_an_error_not_silence() -> Result<()> {
    let (errors, text) = check_source(
        r#"
@arch_module(
    stratum: MsfsStratum.NoSuchStratum,
)
module probe.unparseable;
fn main() { print("ok"); }
"#,
    )?;
    assert!(
        errors > 0,
        "an unparseable @arch_module value must be an error rather than silence \
         (T0834 acceptance); diags: {text}"
    );
    assert!(
        text.contains("NoSuchStratum"),
        "the diagnostic must name the unknown value so the author can fix it, diags: {text}"
    );
    Ok(())
}

/// An unmet OBLIGATION is judged by the audit, not by the compiler
/// (T0866). `Lifecycle.Theorem` without CVE closure owes work; it does
/// not state anything false, so the module still compiles — and the
/// verdict is still visible, carrying its stable code, so the debt
/// cannot hide (the failure mode behind 2311 boilerplate `Theorem`
/// claims in core/, T0834).
///
/// The teeth live in `verum arch check --strict` / `verum audit`,
/// which own CI's exit code; `arch_check_strict_refuses_unclosed_theorem`
/// below is the other polarity of this pair.
#[test]
fn theorem_without_cve_closure_warns_on_check_but_compiles() -> Result<()> {
    let (errors, text) = check_source(
        r#"
@arch_module(
    foundation: Foundation.ZfcTwoInacc,
    stratum: MsfsStratum.LFnd,
    lifecycle: Lifecycle.Theorem("v0.1"),
)
module probe.at2;
fn main() { print("ok"); }
"#,
    )?;
    assert!(
        text.contains("ATS-V-AP-010"),
        "the debt must still be SHOWN, with its stable code; diags: {text}"
    );
    assert_eq!(
        errors, 0,
        "owed work must not stop the build — one such module used to \
         cascade: its registration failed, dependents lost its names, and \
         a hundred spurious errors buried the real ones; diags: {text}"
    );
    Ok(())
}

/// A false CLAIM stays a compile error: `MsfsStratum.LAbs` names a
/// stratum a theorem proves empty, so the declaration is wrong, not
/// merely unfinished. (Pinned separately from the obligation case
/// above so a future change cannot quietly collapse the two.)
#[test]
fn false_claim_still_refuses_to_compile() -> Result<()> {
    let (errors, text) = check_source(
        r#"
@arch_module(
    foundation: Foundation.ZfcTwoInacc,
    stratum: MsfsStratum.LAbs,
    lifecycle: Lifecycle.Definition,
)
module probe.false_claim;
fn main() { print("ok"); }
"#,
    )?;
    assert!(
        errors > 0 && text.contains("ATS-V-AP-011"),
        "a declaration the system can show is FALSE is a defect of the \
         same order as a type error; diags: {text}"
    );
    Ok(())
}

#[test]
fn honest_declaration_passes_check() -> Result<()> {
    let (errors, text) = check_source(
        r#"
@arch_module(
    foundation: Foundation.ZfcTwoInacc,
    stratum: MsfsStratum.LFnd,
    lifecycle: Lifecycle.Definition,
)
module probe.honest;
fn main() { print("ok"); }
"#,
    )?;
    assert_eq!(
        errors, 0,
        "an honest @arch_module declaration (Definition lifecycle, admissible \
         stratum) must pass — the phase judges claims, it does not reject \
         annotation itself; diags: {text}"
    );
    Ok(())
}
