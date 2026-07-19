//! MODULE-IDENTITY-1 / E602 self-mount pins (T0236).
//!
//! Class: false `E602: ambiguous name` on relative (`super.`) mounts
//! when a file is type-checked standalone.
//!
//! Root cause: `phase_type_check` computed the file's real module path
//! (e.g. `sub.leaf`, or `core.math.simple` for stdlib files) but only
//! threaded it as a LOCAL parameter into its Pass-0 `process_import`
//! calls — the checker's `current_module_path` FIELD stayed at the
//! default `cog`.  The main check loop later re-processed the same
//! `ItemKind::Mount` items through `check_import`, which resolves
//! relative mount paths against the FIELD.  The same mount therefore
//! minted TWO import-source spellings for the SAME target module
//! (`cog.sub.helper` from Pass 0 vs `cog.helper` from the check loop),
//! and the import-ambiguity check (`sources.len() > 1`) reported a
//! false E602 on every relatively-mounted name.  This made 27
//! theorem-bearing `core/` files unverifiable standalone (all of
//! `core/verify/kernel_v0/` plus 12 `core/math/` files), blocking the
//! T0230 proof-ratchet gate from claiming 66 theorems.
//!
//! Fix: the orchestrators install the computed module path via
//! `set_current_module_path` (ONE identity authority — the checker
//! field), so both passes resolve relative mounts identically and the
//! exact-string source dedup (`record_import_source`,
//! IMPORT-SOURCE-FUNNEL-1) collapses them.
//!
//! Two pins:
//!  * `super_mount_from_nested_file_is_not_ambiguous` — the
//!    false-positive shape must stay dead.
//!  * `distinct_modules_same_name_still_fire_e602` — GENUINE
//!    ambiguity (two different modules exporting the same name, both
//!    mounted) must stay loud.

use std::path::PathBuf;
use tempfile::TempDir;
use verum_compiler::{CompilationPipeline, CompilerOptions, OutputFormat, Session, VerifyMode};

/// Build the shared project shape:
///
/// ```text
/// <tmp>/Verum.toml            (marks the project root)
/// <tmp>/pkg/sub/<files>       (nested module dir => multi-segment
///                              module path `sub.<stem>`, which is
///                              what makes the `super.`-resolution
///                              divergence observable)
/// ```
///
/// Returns the temp dir guard and the absolute path of `leaf.vr`.
fn write_project(files: &[(&str, &str)]) -> (TempDir, PathBuf) {
    let tmp = TempDir::new().expect("tempdir");
    std::fs::write(tmp.path().join("Verum.toml"), "[package]\nname = \"pin_e602\"\n")
        .expect("write Verum.toml");
    let sub = tmp.path().join("pkg").join("sub");
    std::fs::create_dir_all(&sub).expect("mkdir pkg/sub");
    let mut leaf = PathBuf::new();
    for (name, content) in files {
        let p = sub.join(name);
        std::fs::write(&p, content).expect("write source file");
        if *name == "leaf.vr" {
            leaf = p;
        }
    }
    assert!(leaf.as_os_str().len() > 0, "fixture must include leaf.vr");
    (tmp, leaf)
}

/// Run the single-file check pipeline (`run_check_only`, the same flow
/// `verum check` / `verum verify` use) and return (result_ok, joined
/// diagnostics text).
fn check_file(input: PathBuf) -> (bool, String) {
    let options = CompilerOptions {
        input,
        verify_mode: VerifyMode::Runtime,
        output_format: OutputFormat::Human,
        check_only: true,
        ..Default::default()
    };
    let mut session = Session::new(options);
    let ok = {
        let mut pipeline = CompilationPipeline::new(&mut session);
        pipeline.run_check_only().is_ok()
    };
    let mut text = session.format_diagnostics();
    for diag in session.diagnostics().iter() {
        text.push_str(&format!("{:?}\n", diag));
    }
    (ok, text)
}

/// FALSE-POSITIVE PIN: a nested file that `super.`-mounts a sibling
/// must NOT report E602 — the mount names ONE module, even though the
/// import machinery visits the mount more than once.
///
/// Pre-fix this failed with:
///   `E602: ambiguous name: 'Widget' is imported from multiple
///    modules: cog.sub.helper, cog.helper`
/// (two spellings of the same file, minted by the two passes running
/// under different module identities).
#[test]
fn super_mount_from_nested_file_is_not_ambiguous() {
    let (_tmp, leaf) = write_project(&[
        ("helper.vr", "public type Widget is { id: Int };\n"),
        (
            "leaf.vr",
            "mount super.helper.{Widget};\n\
             \n\
             public fn make() -> Widget {\n\
                 Widget { id: 7 }\n\
             }\n",
        ),
    ]);

    let (ok, diagnostics) = check_file(leaf);
    assert!(
        !diagnostics.contains("E602"),
        "self-mount must not be reported as ambiguous; diagnostics:\n{}",
        diagnostics
    );
    assert!(
        ok,
        "clean nested file with a super-mount must type-check; diagnostics:\n{}",
        diagnostics
    );
}

/// GENUINE-AMBIGUITY PIN: two DISTINCT sibling modules exporting the
/// same name, both mounted — E602 must still fire, and must name both
/// real sources.
#[test]
fn distinct_modules_same_name_still_fire_e602() {
    let (_tmp, leaf) = write_project(&[
        ("alpha.vr", "public type Widget is { id: Int };\n"),
        ("beta.vr", "public type Widget is { tag: Text };\n"),
        (
            "leaf.vr",
            "mount super.alpha.{Widget};\n\
             mount super.beta.{Widget};\n\
             \n\
             public fn make() -> Widget {\n\
                 Widget { id: 7 }\n\
             }\n",
        ),
    ]);

    let (ok, diagnostics) = check_file(leaf);
    assert!(
        !ok,
        "two distinct modules exporting the same mounted name must fail the check"
    );
    assert!(
        diagnostics.contains("E602") && diagnostics.contains("ambiguous"),
        "genuine ambiguity must surface as E602; diagnostics:\n{}",
        diagnostics
    );
    assert!(
        diagnostics.contains("alpha") && diagnostics.contains("beta"),
        "E602 must name both distinct source modules; diagnostics:\n{}",
        diagnostics
    );
}
