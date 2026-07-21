//! T0528 pins — unresolved `mount m.{X}` items are LOUD (E401), with
//! the leniencies that must survive pinned as controls.
//!
//! Class: `mount m.{NonExistent}` was SILENTLY DROPPED — the program
//! compiled and ran (name-resolution.md §1 defect #3; measured: 52 of
//! 234 sqlite `_api` specs mount names that do not exist and report
//! green; `progress_smoke.vr` mounts a nonexistent `Config` and exits
//! 0).  Two drop legs, both fixed:
//!  * source-registry leg: `import_item_from_module_impl`'s pre-flight
//!    existence gate early-returned `Ok(())` for explicit mounts;
//!  * baked-archive leg: the module-not-found tail raised E402 (wrong
//!    diagnostic) and the Nested-arm leniency then swallowed it for any
//!    name that was ambient (`type_defs`/env membership).  The
//!    leniency is now keyed on `builtin_ambient_names` — the checker's
//!    OWN constructor-time registrations — never on stdlib ambience.
//!
//! Pins:
//!  * `unresolved_mount_item_is_loud_e401` — the false-green shape
//!    must stay dead.
//!  * `resolved_mount_item_stays_clean` — the fix must not over-fire.
//!  * `cfg_gated_item_still_mount_resolves` — an item behind
//!    `@cfg(target_os = ...)` for ANOTHER platform still resolves as a
//!    mount item: the export table is built from the full unfiltered
//!    AST (`extract_exports_from_module` checks only visibility), and
//!    that invariant is what keeps the stdlib bakeable from any single
//!    host.
//!  * `builtin_primitive_mount_stays_lenient` — `mount base.{Bool}`
//!    (a language builtin no stdlib module declares) must stay green;
//!    ~1700 such mounts exist across core/ and the spec suites.

use std::path::PathBuf;
use tempfile::TempDir;
use verum_compiler::{CompilationPipeline, CompilerOptions, OutputFormat, Session, VerifyMode};

/// Project shape: `<tmp>/Verum.toml`, `<tmp>/pkg/sub/<files>`.
/// Returns the temp-dir guard and the absolute path of `leaf.vr`.
fn write_project(files: &[(&str, &str)]) -> (TempDir, PathBuf) {
    let tmp = TempDir::new().expect("tempdir");
    std::fs::write(
        tmp.path().join("Verum.toml"),
        "[package]\nname = \"pin_e401\"\n",
    )
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

/// Run the single-file check pipeline (the same flow `verum check`
/// uses) and return (result_ok, joined diagnostics text).
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

/// FALSE-GREEN PIN: mounting an item the target module does not
/// publish must be a loud E401 naming the item and the module, not a
/// silent drop.
#[test]
fn unresolved_mount_item_is_loud_e401() {
    let (_tmp, leaf) = write_project(&[
        ("helper.vr", "public type Widget is { id: Int };\n"),
        (
            "leaf.vr",
            "mount super.helper.{Widgett};\n\
             \n\
             public fn make() -> Int { 7 }\n",
        ),
    ]);

    let (_ok, diagnostics) = check_file(leaf);
    assert!(
        diagnostics.contains("E401"),
        "unresolved mount item must be a loud E401; diagnostics:\n{}",
        diagnostics
    );
    assert!(
        diagnostics.contains("Widgett"),
        "the diagnostic must name the missing item; diagnostics:\n{}",
        diagnostics
    );
    // did-you-mean over the module's real surface
    assert!(
        diagnostics.contains("Widget"),
        "the diagnostic must suggest the module's real item; diagnostics:\n{}",
        diagnostics
    );
}

/// OVER-FIRE CONTROL: a mount whose every item exists must stay clean.
#[test]
fn resolved_mount_item_stays_clean() {
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
        !diagnostics.contains("E401"),
        "resolved mount must not fire E401; diagnostics:\n{}",
        diagnostics
    );
    assert!(
        ok,
        "clean file with a resolved mount must type-check; diagnostics:\n{}",
        diagnostics
    );
}

/// CFG CONTROL: an item declared behind `@cfg(target_os = ...)` for a
/// platform that is NOT the check host still resolves as a mount item.
/// The mount is only NAME resolution; using the item is a separate,
/// platform-gated concern.  Both foreign-platform spellings are
/// exercised so the pin holds on any CI host.
#[test]
fn cfg_gated_item_still_mount_resolves() {
    let (_tmp, leaf) = write_project(&[
        (
            "helper.vr",
            "@cfg(target_os = \"windows\")\n\
             public fn win_only() -> Int { 1 }\n\
             \n\
             @cfg(target_os = \"linux\")\n\
             public fn linux_only() -> Int { 2 }\n\
             \n\
             @cfg(target_os = \"macos\")\n\
             public fn mac_only() -> Int { 3 }\n\
             \n\
             public fn everywhere() -> Int { 0 }\n",
        ),
        (
            "leaf.vr",
            "mount super.helper.{win_only, linux_only, mac_only, everywhere};\n\
             \n\
             public fn make() -> Int { everywhere() }\n",
        ),
    ]);

    let (ok, diagnostics) = check_file(leaf);
    assert!(
        !diagnostics.contains("E401"),
        "cfg'd-out items must still resolve as mount items \
         (exports are cfg-unfiltered); diagnostics:\n{}",
        diagnostics
    );
    assert!(
        ok,
        "mounting cfg-gated items must not fail the check; diagnostics:\n{}",
        diagnostics
    );
}

/// BUILTIN CONTROL: `mount base.{Bool, Int64}` names language builtins
/// that no stdlib module declares.  The narrowed leniency
/// (`builtin_ambient_names`) must keep these green.
#[test]
fn builtin_primitive_mount_stays_lenient() {
    let (_tmp, leaf) = write_project(&[(
        "leaf.vr",
        "mount base.{Bool, Int64};\n\
         \n\
         public fn make() -> Bool { true }\n",
    )]);

    let (ok, diagnostics) = check_file(leaf);
    assert!(
        !diagnostics.contains("E401") && !diagnostics.contains("E402"),
        "builtin-primitive mounts must stay lenient; diagnostics:\n{}",
        diagnostics
    );
    assert!(
        ok,
        "builtin-primitive mount must not fail the check; diagnostics:\n{}",
        diagnostics
    );
}
