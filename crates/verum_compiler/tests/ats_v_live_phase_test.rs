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

#[test]
fn theorem_without_cve_closure_errors_on_check() -> Result<()> {
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
        errors > 0 && text.contains("ATS-V-AP-010"),
        "AT-2 must be live on the user path: Lifecycle.Theorem without CVE closure \
         is AP-010 CveIncomplete — this firing NOWHERE is how 2311 boilerplate \
         Theorem claims accumulated in core/ (T0834); diags: {text}"
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
